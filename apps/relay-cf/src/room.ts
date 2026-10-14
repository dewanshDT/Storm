import { DurableObject } from "cloudflare:workers";
import {
  b64urlEncode,
  decodeKey,
  newClientTrunkId,
  newNonce,
  newTrunkId,
  parsePublicKey,
  sameKey,
  validateNonce,
  validateServerId,
  verifyRelayAuth,
} from "./auth";
import { configFor, MIB, type Config, type Env } from "./config";
import { log } from "./log";
import {
  BODY_REQUEST,
  BODY_RESPONSE,
  closeStream,
  control,
  error,
  errorOnAttempt,
  errorOnStream,
  HEARTBEAT_INTERVAL_SECS,
  isRequestHead,
  parseBodyHeader,
  parseFrame,
  PING,
  PONG,
  ProtocolError,
  streamIdOf,
  stringField,
  type Body,
  type ErrorCode,
} from "./proto";
import { trySend } from "./send";

const H_SERVER_ID = "x-storm-server-id";
const H_PEER_IP = "x-storm-peer-ip";
const H_PUBLIC_BASE = "x-storm-public-base";
const H_PUBKEY = "x-storm-pubkey";

// §5.2: no sender may exceed 16 MiB a frame; storm-relay's WebSocket reader
// drops the connection past it, and so does this one.
const MAX_FRAME_BYTES = 16 * MIB;
const MAX_ATTEMPT_ID_BYTES = 256;
const SEEN_WRITE_MS = 1000;
const STREAM_END_LANE = 256;

// Bytes handed to one socket and bytes its peer has proved it read. `m` holds
// the byte counts at which the relay sent a watermark PING; each PONG proves
// the peer read everything up to the oldest one (a WebSocket is ordered).
interface Flow {
  s: number;
  a: number;
  m: number[];
  // stream ends sent past the budget since the peer last answered
  e?: number;
}

// Everything per socket lives in its attachment, which survives hibernation.
interface ServerAtt {
  k: "s";
  tid: string;
  sid: string;
  base: string;
  // 0 awaiting REGISTER_SERVER, 1 challenged, 2 registered
  ph: 0 | 1 | 2;
  hs: number;
  // the key it registered under, kept for routing a keyed HELLO (§5)
  pk?: string;
  // open mode: the key this room is addressed by
  rk?: string;
  nonce?: string;
  rec?: boolean;
  next: number;
  seen: number;
  sup?: number;
  f: Flow;
}

interface ClientAtt {
  k: "c";
  ctk: string;
  sid: string;
  ip: string;
  // 0 awaiting HELLO, 1 holding for a server trunk, 2 ready
  ph: 0 | 1 | 2;
  hs: number;
  hw?: number;
  tid?: string;
  // ?pubkey= on the URL, and the key the HELLO named (§5)
  qk?: string;
  hk?: string;
  f: Flow;
}

type Att = ServerAtt | ClientAtt;

interface Entry<A extends Att = Att> {
  ws: WebSocket;
  att: A;
  // A server's newest frame; att.seen is its copy written at most once a second.
  seen?: number;
}

interface Stream {
  ctk: string;
  acked: boolean;
  deadline: number | null;
  // the client's one HTTP_REQUEST_HEAD has been forwarded (§5.2)
  head: boolean;
}

const SCHEMA = `
CREATE TABLE IF NOT EXISTS streams (
  tid TEXT NOT NULL, sid INTEGER NOT NULL, ctk TEXT NOT NULL,
  acked INTEGER NOT NULL, deadline INTEGER, PRIMARY KEY (tid, sid));
CREATE TABLE IF NOT EXISTS nonces (
  nonce TEXT PRIMARY KEY, expires INTEGER NOT NULL, spent INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS bindings (
  server_id TEXT PRIMARY KEY, pubkey TEXT NOT NULL);
`;

class Refusal extends Error {
  constructor(readonly code: ErrorCode) {
    super(code);
  }
}

const idOf = (att: Att) => (att.k === "s" ? att.tid : att.ctk);

// One per server_id. Holds that server's trunk (and, during a supersession
// drain, its predecessor) and every client trunk routed to it, so routing a
// frame never leaves the object. All sockets are hibernatable; the stream
// table is mirrored in SQLite and rebuilt on wake.
export class RelayRoom extends DurableObject<Env> {
  private readonly sql: SqlStorage;
  private readonly entries = new Map<string, Entry>();
  private readonly streams = new Map<string, Map<number, Stream>>();
  private readonly chains = new Map<string, Promise<void>>();
  private alarmAt: number | null = null;
  // Σ unread() over every socket in the room.
  private held = 0;

  constructor(ctx: DurableObjectState, env: Env) {
    super(ctx, env);
    this.sql = ctx.storage.sql;
    this.sql.exec(SCHEMA);
    // Rooms created before 116a have a streams table without it.
    if (!this.sql.exec<{ name: string }>("PRAGMA table_info(streams)").toArray().some((c) => c.name === "head")) {
      this.sql.exec("ALTER TABLE streams ADD COLUMN head INTEGER NOT NULL DEFAULT 0");
    }
    // Answered by the runtime without waking the object. The bytes are what
    // storm-server, the Dart client and the Rust client all send; any other
    // spelling of PING still reaches webSocketMessage and is answered there.
    ctx.setWebSocketAutoResponse(new WebSocketRequestResponsePair(PING, PONG));

    for (const ws of ctx.getWebSockets()) {
      const att = ws.deserializeAttachment() as Att | null;
      if (!att) continue;
      this.entries.set(idOf(att), { ws, att });
      this.held += this.unread(att);
    }
    for (const row of this.sql.exec<{
      tid: string;
      sid: number;
      ctk: string;
      acked: number;
      deadline: number | null;
      head: number;
    }>("SELECT tid, sid, ctk, acked, deadline, head FROM streams")) {
      const server = this.entries.get(row.tid);
      const client = this.entries.get(row.ctk);
      if (!server || !client) {
        this.sql.exec("DELETE FROM streams WHERE tid = ? AND sid = ?", row.tid, row.sid);
        continue;
      }
      this.table(row.tid).set(row.sid, {
        ctk: row.ctk,
        acked: row.acked === 1,
        deadline: row.deadline,
        head: row.head === 1,
      });
    }
  }

  private get config(): Config {
    return configFor(this.env);
  }

  async fetch(request: Request): Promise<Response> {
    const url = new URL(request.url);
    const serverId = request.headers.get(H_SERVER_ID) ?? "";
    if (!validateServerId(serverId)) return new Response("bad server_id", { status: 400 });
    const config = this.config;
    const now = Date.now();
    const [client, server] = Object.values(new WebSocketPair());
    const flow: Flow = { s: 0, a: 0, m: [] };
    let att: Att;
    if (url.pathname === "/register") {
      att = {
        k: "s",
        tid: newTrunkId(),
        sid: serverId,
        base: request.headers.get(H_PUBLIC_BASE) ?? "",
        ph: 0,
        hs: now + config.handshakeTimeoutMs,
        next: 1,
        seen: now,
        f: flow,
      };
      const key = request.headers.get(H_PUBKEY);
      if (key !== null) att.rk = key;
      this.ctx.acceptWebSocket(server, ["s", `s:${att.tid}`]);
    } else if (url.pathname === "/connect") {
      att = {
        k: "c",
        ctk: newClientTrunkId(),
        sid: serverId,
        ip: request.headers.get(H_PEER_IP) ?? "",
        ph: 0,
        hs: now + config.handshakeTimeoutMs,
        f: flow,
      };
      const key = request.headers.get(H_PUBKEY);
      if (key !== null) att.qk = key;
      this.ctx.acceptWebSocket(server, ["c", `c:${att.ctk}`]);
    } else {
      return new Response("not found", { status: 404 });
    }
    server.serializeAttachment(att);
    this.entries.set(idOf(att), { ws: server, att });
    this.arm();
    return new Response(null, { status: 101, webSocket: client });
  }

  private entryOf(ws: WebSocket): Entry | undefined {
    const att = ws.deserializeAttachment() as Att | null;
    if (!att) return undefined;
    const id = idOf(att);
    const entry = this.entries.get(id);
    if (entry) {
      entry.ws = ws;
      return entry;
    }
    return undefined;
  }

  private persist(entry: Entry): void {
    try {
      entry.ws.serializeAttachment(entry.att);
    } catch {
      // the socket is already gone; its entry is about to be dropped
    }
  }

  // ---- events --------------------------------------------------------

  async webSocketMessage(ws: WebSocket, message: string | ArrayBuffer): Promise<void> {
    const entry = this.entryOf(ws);
    if (!entry) {
      try {
        ws.close(1000);
      } catch {}
      return;
    }
    // CHALLENGE_RESPONSE awaits WebCrypto, and the runtime delivers the next
    // message meanwhile; each socket's frames are handled strictly in order.
    const id = idOf(entry.att);
    const prior = this.chains.get(id);
    const run = prior ? prior.then(() => this.handle(entry, message)) : this.handle(entry, message);
    if (!run) return;
    const settled: Promise<void> = run.finally(() => {
      if (this.chains.get(id) === settled) this.chains.delete(id);
    });
    this.chains.set(id, settled);
    return settled;
  }

  async webSocketClose(ws: WebSocket): Promise<void> {
    this.gone(ws);
  }

  async webSocketError(ws: WebSocket): Promise<void> {
    this.gone(ws);
  }

  private gone(ws: WebSocket): void {
    const entry = this.entryOf(ws);
    if (entry) this.lost(entry);
  }

  // Also reached from a send that found the peer already closing, before its
  // close event runs. Each drop forgets the entry before it sends anything, so
  // a second loss of the same peer, or one met while telling others, is a no-op.
  private lost(entry: Entry): void {
    if (this.entries.get(idOf(entry.att)) !== entry) return;
    if (entry.att.k === "s") this.dropServer(entry as Entry<ServerAtt>, null, "closed");
    else this.dropClient(entry as Entry<ClientAtt>, null);
  }

  private handle(entry: Entry, message: string | ArrayBuffer): Promise<void> | void {
    if (this.entries.get(idOf(entry.att)) !== entry) return;
    try {
      const size = typeof message === "string" ? message.length : message.byteLength;
      if (size > MAX_FRAME_BYTES) {
        return this.fault(entry, null, "frame over 16 MiB");
      }
      const result = entry.att.k === "s"
        ? this.onServer(entry as Entry<ServerAtt>, message)
        : this.onClient(entry as Entry<ClientAtt>, message);
      if (result) return result.catch((e) => this.fail(entry, e));
    } catch (e) {
      this.fail(entry, e);
    }
  }

  private fail(entry: Entry, e: unknown): void {
    if (e instanceof Refusal) return this.fault(entry, e.code, e.code);
    if (e instanceof ProtocolError) return this.fault(entry, "protocol_error", "protocol_error");
    // No honest code for the relay's own fault (§6): protocol_error, and the
    // real cause stays here.
    log("relay_fault", { reason: String(e) });
    this.fault(entry, "protocol_error", "relay_fault");
  }

  private fault(entry: Entry, code: ErrorCode | null, reason: string): void {
    if (code === "protocol_error") {
      log("protocol_error", entry.att.k === "s" ? { trunk_id: entry.att.tid } : { client_trunk_id: entry.att.ctk });
    }
    if (entry.att.k === "s") this.dropServer(entry as Entry<ServerAtt>, code, reason);
    else this.dropClient(entry as Entry<ClientAtt>, code);
  }

  // ---- server trunk --------------------------------------------------

  private onServer(entry: Entry<ServerAtt>, message: string | ArrayBuffer): Promise<void> | void {
    const att = entry.att;
    if (att.ph === 2) {
      const now = Date.now();
      entry.seen = now;
      if (now - att.seen >= SEEN_WRITE_MS) {
        att.seen = now;
        this.persist(entry);
      }
    }
    if (typeof message !== "string") {
      if (att.ph !== 2) throw new ProtocolError();
      const { kind, streamId } = parseBodyHeader(message);
      if (kind !== BODY_RESPONSE) throw new ProtocolError();
      this.deliver(att.tid, streamId, message, message.byteLength);
      return;
    }
    const frame = parseFrame(message);
    // A PING may arrive at any point on a trunk, mid-handshake included, and is
    // answered in order (§4.2, decision 108c). The auto-response covers only
    // the canonical bytes.
    if (frame.type === "PING") {
      this.send(entry, PONG);
      return;
    }
    if (frame.type === "PONG") {
      if (att.ph === 2) this.watermarkAnswered(entry);
      return;
    }
    if (att.ph === 0) {
      if (frame.type !== "REGISTER_SERVER") throw new ProtocolError();
      return this.challenge(entry, frame.body);
    }
    if (att.ph === 1) {
      if (frame.type !== "CHALLENGE_RESPONSE") throw new ProtocolError();
      return this.completeRegistration(entry, frame.body);
    }
    switch (frame.type) {
      case "DEREGISTER":
        return this.dropServer(entry, null, "deregistered");
      case "STREAM_ACK":
        return this.streamAck(att.tid, frame.body);
      case "HTTP_RESPONSE_HEAD": {
        const streamId = streamIdOf(frame.body);
        const json = control("HTTP_RESPONSE_HEAD", frame.body);
        this.deliver(att.tid, streamId, json, json.length);
        return;
      }
      case "CLOSE":
        return this.serverClose(entry, frame.body);
      case "ERROR":
        return this.serverError(entry, frame.body);
      default:
        throw new ProtocolError();
    }
  }

  // §6 gives an origin no ERROR to send, but servers before 116a refused one
  // stream with ERROR{stream_closed, stream_id}. The trunk is shared by every
  // client of the server, so that ends the stream it names, as aborted.
  private serverError(entry: Entry<ServerAtt>, body: Body): void {
    if (!("stream_id" in body)) throw new ProtocolError();
    const streamId = streamIdOf(body);
    const tid = entry.att.tid;
    const stream = this.streams.get(tid)?.get(streamId);
    log("stream_refused", { trunk_id: tid, stream_id: streamId, reason: "origin error" });
    this.sendCounted(entry, closeStream(streamId), 0, this.config.maxServerBufferBytes);
    if (!stream) return;
    this.removeStream(tid, streamId);
    const client = this.entries.get(stream.ctk) as Entry<ClientAtt> | undefined;
    if (!client) return;
    const json = errorOnStream("stream_aborted", streamId);
    this.sendCounted(client, json, json.length, this.config.maxClientBufferBytes);
  }

  private challenge(entry: Entry<ServerAtt>, body: Body): void {
    const att = entry.att;
    const serverId = stringField(body, "server_id");
    const pubkeyText = stringField(body, "pubkey");
    // Shape problems are protocol_error; only the binding and the signature
    // are auth_failed (as storm-relay's register.rs).
    if (!validateServerId(serverId) || serverId !== att.sid) throw new ProtocolError();
    const pubkey = parsePublicKey(pubkeyText);
    if (!pubkey) throw new ProtocolError();

    // Binding first, challenge second (§4.1): no nonce is spent on a
    // registration that cannot succeed.
    const config = this.config;
    let record = false;
    if (config.binding === "open") {
      // Nothing to bind: the room is the key (§4.1). The Worker routed on it,
      // so a disagreement here is the relay's own fault.
      const room = parsePublicKey(att.rk ?? "");
      if (!room || !sameKey(room, pubkey)) throw new Error("open room addressed by another key");
    } else if (config.binding === "allowlist") {
      const expected = config.allowlist.get(serverId);
      if (!expected || !sameKey(expected, pubkey)) {
        log("registration_refused", { server_id: serverId, reason: "binding" });
        throw new Refusal("auth_failed");
      }
    } else {
      const bound = this.boundKey(serverId);
      if (bound && !sameKey(bound, pubkey)) {
        log("registration_refused", { server_id: serverId, reason: "binding" });
        throw new Refusal("auth_failed");
      }
      record = !bound;
    }

    const nonce = newNonce();
    if (!validateNonce(nonce)) throw new Error("minted an invalid nonce");
    const now = Date.now();
    this.sql.exec("DELETE FROM nonces WHERE expires <= ?", now);
    // INSERT OR IGNORE: a spent nonce is never re-armed by issuing it again.
    this.sql.exec(
      "INSERT OR IGNORE INTO nonces (nonce, expires, spent) VALUES (?, ?, 0)",
      nonce,
      now + config.challengeTtlMs,
    );
    att.ph = 1;
    att.pk = b64urlEncode(pubkey);
    att.nonce = nonce;
    att.rec = record;
    this.persist(entry);
    this.send(entry, control("CHALLENGE", { nonce }));
  }

  private async completeRegistration(entry: Entry<ServerAtt>, body: Body): Promise<void> {
    const att = entry.att;
    const sig = stringField(body, "sig");
    const nonce = att.nonce ?? "";
    const pubkey = parsePublicKey(att.pk ?? "");
    // auth_failed for every cause below, one fixed message (§6).
    if (!validateNonce(nonce) || !pubkey) throw new Refusal("auth_failed");
    const spent = this.sql.exec(
      "UPDATE nonces SET spent = 1 WHERE nonce = ? AND spent = 0 AND expires > ?",
      nonce,
      Date.now(),
    ).rowsWritten;
    if (spent !== 1) {
      log("registration_refused", { server_id: att.sid, reason: "nonce" });
      throw new Refusal("auth_failed");
    }
    if (!(await verifyRelayAuth(pubkey, att.sid, nonce, sig))) {
      log("registration_refused", { server_id: att.sid, reason: "signature" });
      throw new Refusal("auth_failed");
    }
    if (this.entries.get(att.tid) !== entry) return;

    if (att.rec) {
      // Re-read: two first registrations may both have passed the check
      // before either verified. The first to record wins.
      const bound = this.boundKey(att.sid);
      if (bound && !sameKey(bound, pubkey)) {
        log("registration_refused", { server_id: att.sid, reason: "binding race" });
        throw new Refusal("auth_failed");
      }
      if (!bound) {
        // On disk before REGISTERED leaves (decision 69): the output gate
        // holds the send until this write is durable.
        this.sql.exec("INSERT INTO bindings (server_id, pubkey) VALUES (?, ?)", att.sid, b64urlEncode(pubkey));
        log("tofu_recorded", { server_id: att.sid });
      }
    }

    const now = Date.now();
    att.ph = 2;
    att.seen = now;
    delete att.nonce;
    delete att.rec;
    this.persist(entry);
    const registered = control("REGISTERED", {
      trunk_id: att.tid,
      public_address: `${att.base}/connect/${att.sid}`,
      heartbeat_interval_secs: HEARTBEAT_INTERVAL_SECS,
    });
    // A server gone before REGISTERED reached it supersedes nothing.
    if (!this.send(entry, registered)) return;
    log("server_registered", { server_id: att.sid, trunk_id: att.tid });

    for (const other of this.servers()) {
      if (other !== entry && other.att.ph === 2 && other.att.sup === undefined) {
        other.att.sup = now + this.config.supersessionDrainMs;
        this.persist(other);
        log("server_superseded", { server_id: att.sid, trunk_id: other.att.tid, reason: `replaced by ${att.tid}` });
      }
    }

    for (const client of this.clients()) {
      if (client.att.ph === 1 && this.reaches(client.att, entry)) this.ready(client, att.tid);
    }
    this.arm();
  }

  // §5: a keyed HELLO reaches only a registration under that key; a keyless
  // one (allowlist and tofu only) the one registration.
  private reaches(client: ClientAtt, server: Entry<ServerAtt>): boolean {
    if (client.hk === undefined) return true;
    const named = decodeKey(client.hk);
    const key = this.serverKey(server.att);
    return !!named && !!key && sameKey(named, key);
  }

  // Trunks registered before 116n kept no key; theirs is the binding's.
  private serverKey(att: ServerAtt): Uint8Array | null {
    if (att.pk) return parsePublicKey(att.pk);
    if (this.config.binding === "allowlist") return this.config.allowlist.get(att.sid) ?? null;
    return this.boundKey(att.sid);
  }

  private boundKey(serverId: string): Uint8Array | null {
    const row = this.sql.exec<{ pubkey: string }>("SELECT pubkey FROM bindings WHERE server_id = ?", serverId).toArray()[0];
    return row ? parsePublicKey(row.pubkey) : null;
  }

  private streamAck(tid: string, body: Body): void {
    const streamId = streamIdOf(body);
    const stream = this.streams.get(tid)?.get(streamId);
    // An ack for a stream that timed out or was never issued is not worth a
    // trunk carrying other healthy streams.
    if (!stream) return;
    stream.acked = true;
    stream.deadline = null;
    this.sql.exec("UPDATE streams SET acked = 1, deadline = NULL WHERE tid = ? AND sid = ?", tid, streamId);
  }

  // The stream's recorded owner is the only destination: nothing in the frame
  // names a client.
  private deliver(tid: string, streamId: number, data: string | ArrayBuffer, size: number): void {
    const stream = this.streams.get(tid)?.get(streamId);
    if (!stream) return;
    const client = this.entries.get(stream.ctk) as Entry<ClientAtt> | undefined;
    if (!client) return;
    if (!this.admits(client, size, this.config.maxClientBufferBytes)) {
      this.dropBacklogged(tid, streamId, client);
      return;
    }
    this.sendCounted(client, data, size, this.config.maxClientBufferBytes);
  }

  // §5.4 (decision 106): the stream ends toward the client with
  // client_backlogged and toward the origin with CLOSE; other streams go on.
  private dropBacklogged(tid: string, streamId: number, client: Entry<ClientAtt>): void {
    log("client_backlogged", { client_trunk_id: client.att.ctk, stream_id: streamId });
    this.removeStream(tid, streamId);
    const json = errorOnStream("client_backlogged", streamId);
    this.sendCounted(client, json, json.length, this.config.maxClientBufferBytes);
    const server = this.entries.get(tid);
    if (server) this.sendCounted(server, closeStream(streamId), 0, this.config.maxServerBufferBytes);
    // Rule 3: storm-relay's stream-end lane holds 256; past that many ends
    // sent unread, the client trunk goes and the client sees trunk loss.
    client.att.f.e = (client.att.f.e ?? 0) + 1;
    if (client.att.f.e > STREAM_END_LANE) this.dropClient(client, null);
  }

  private serverClose(entry: Entry<ServerAtt>, body: Body): void {
    if (!("stream_id" in body)) return this.dropServer(entry, null, "closed by origin");
    if (body.stream_id === null) throw new ProtocolError();
    const streamId = streamIdOf(body);
    let aborted = false;
    if ("aborted" in body) {
      if (typeof body.aborted !== "boolean") throw new ProtocolError();
      aborted = body.aborted;
    }
    const stream = this.streams.get(entry.att.tid)?.get(streamId);
    if (!stream) return;
    this.removeStream(entry.att.tid, streamId);
    const client = this.entries.get(stream.ctk) as Entry<ClientAtt> | undefined;
    if (!client) return;
    const budget = this.config.maxClientBufferBytes;
    let json: string;
    if (aborted) {
      json = errorOnStream("stream_aborted", streamId);
    } else {
      json = closeStream(streamId);
      // A plain CLOSE with no room would present what is queued as a whole
      // body if anything were ever discarded; it ends the stream as backlogged.
      if (!this.admits(client, json.length, budget)) json = errorOnStream("client_backlogged", streamId);
    }
    this.sendCounted(client, json, json.length, budget);
  }

  // Ends a server trunk: every client bound to it is told and closed (§4.2,
  // §5.4), and its streams are forgotten.
  private dropServer(entry: Entry<ServerAtt>, code: ErrorCode | null, reason: string, fail: ErrorCode = "trunk_lost"): void {
    const att = entry.att;
    if (this.entries.get(att.tid) !== entry) return;
    this.forget(att);
    this.close(entry, code);
    if (att.ph === 2) {
      this.failClientsOf(att.tid, fail);
      log("server_closed", { server_id: att.sid, trunk_id: att.tid, reason, code: fail });
    }
    this.arm();
  }

  private failClientsOf(tid: string, code: ErrorCode): void {
    const told = new Set<string>();
    const table = this.streams.get(tid);
    if (table) {
      for (const [streamId, stream] of table) {
        const client = this.entries.get(stream.ctk);
        if (client) {
          const json = errorOnStream(code, streamId);
          this.sendCounted(client, json, json.length, this.config.maxClientBufferBytes);
        }
        told.add(stream.ctk);
      }
      this.streams.delete(tid);
      this.sql.exec("DELETE FROM streams WHERE tid = ?", tid);
    }
    for (const client of this.clients()) {
      if (client.att.ph !== 2 || client.att.tid !== tid || this.entries.get(client.att.ctk) !== client) continue;
      this.forget(client.att);
      this.close(client, told.has(client.att.ctk) ? null : code);
      log("client_closed", { client_trunk_id: client.att.ctk, code });
    }
  }

  // ---- client trunk --------------------------------------------------

  private onClient(entry: Entry<ClientAtt>, message: string | ArrayBuffer): void {
    const att = entry.att;
    const frame = typeof message === "string" ? parseFrame(message) : null;
    if (frame?.type === "PING") {
      if (att.ph === 2) this.sendCounted(entry, PONG, PONG.length, this.config.maxClientBufferBytes);
      else this.send(entry, PONG);
      return;
    }
    if (frame?.type === "PONG") {
      if (att.ph === 2) this.watermarkAnswered(entry);
      return;
    }
    if (att.ph === 0) {
      if (!frame || frame.type !== "HELLO") throw new ProtocolError();
      return this.hello(entry, frame.body);
    }
    // Stream frames sent before READY are refused (§5, decision 108c), not
    // queued for later.
    if (att.ph === 1) throw new ProtocolError();

    if (!frame) {
      const { kind, streamId } = parseBodyHeader(message as ArrayBuffer);
      if (kind !== BODY_REQUEST) throw new ProtocolError();
      return this.requestBody(entry, streamId, message as ArrayBuffer);
    }
    switch (frame.type) {
      case "OPEN_STREAM":
        return this.openStream(entry, frame.body);
      case "HTTP_REQUEST_HEAD":
        return this.requestHead(entry, frame.body);
      case "CLOSE":
        return this.clientClose(entry, frame.body);
      default:
        throw new ProtocolError();
    }
  }

  private hello(entry: Entry<ClientAtt>, body: Body): void {
    const serverId = stringField(body, "server_id");
    // Named twice, in the path and in HELLO; a disagreement is refused, never
    // resolved (as storm-relay's connect.rs).
    if (serverId !== entry.att.sid) throw new ProtocolError();
    const named = this.helloKey(entry.att, body);
    if (this.config.binding === "open") {
      // No grace for a keyless HELLO on an open relay (§5, the 114f addendum),
      // and a key the URL did not carry was routed to a room with no server.
      if (named === null || entry.att.qk === undefined) throw new Refusal("server_unreachable");
    }
    if (named !== null) entry.att.hk = named;
    const live = this.liveServer(entry.att);
    if (live) return this.ready(entry, live.att.tid);
    entry.att.ph = 1;
    entry.att.hw = Date.now() + this.config.helloWaitMs;
    this.persist(entry);
    this.arm();
  }

  // HELLO.pubkey: 32 bytes of strict base64url, and the URL's key when both
  // are given (§5). A query alone names nothing; §5's table reads the HELLO.
  private helloKey(att: ClientAtt, body: Body): string | null {
    if (!("pubkey" in body)) return null;
    if (typeof body.pubkey !== "string") throw new ProtocolError();
    const bytes = decodeKey(body.pubkey);
    if (!bytes) throw new ProtocolError();
    const key = b64urlEncode(bytes);
    if (att.qk !== undefined && att.qk !== key) throw new ProtocolError();
    return key;
  }

  private ready(entry: Entry<ClientAtt>, tid: string): void {
    entry.att.ph = 2;
    entry.att.tid = tid;
    delete entry.att.hw;
    this.persist(entry);
    const json = control("READY", { client_trunk_id: entry.att.ctk });
    this.sendCounted(entry, json, json.length, this.config.maxClientBufferBytes);
    log("client_opened", { server_id: entry.att.sid, client_trunk_id: entry.att.ctk });
  }

  private openStream(entry: Entry<ClientAtt>, body: Body): void {
    if (!("attempt_id" in body)) throw new ProtocolError();
    const attemptId = body.attempt_id;
    if (JSON.stringify(attemptId).length > MAX_ATTEMPT_ID_BYTES) throw new ProtocolError();
    const server = this.entries.get(entry.att.tid ?? "") as Entry<ServerAtt> | undefined;
    if (!server) throw new Error("client bound to a missing trunk");
    if (!this.alive(server)) return;
    const config = this.config;
    const table = this.table(server.att.tid);

    let inFlight = 0;
    for (const stream of table.values()) if (!stream.acked && stream.ctk === entry.att.ctk) inFlight++;
    const streamId = server.att.next;
    // Refused at once, never queued, naming its attempt; the trunk carries on (§5.1).
    if (table.size >= config.maxTotalStreams || inFlight >= config.maxInFlightStreams || streamId > 0xffffffff) {
      const json = errorOnAttempt("rate_limited", attemptId);
      this.sendCounted(entry, json, json.length, config.maxClientBufferBytes);
      return;
    }
    server.att.next = streamId + 1;
    this.persist(server);
    const ready = control("STREAM_READY", { attempt_id: attemptId, stream_id: streamId });
    // Before the stream exists, so a client already gone leaves the origin
    // nothing to be told about.
    if (!this.sendCounted(entry, ready, ready.length, config.maxClientBufferBytes)) return;
    const deadline = Date.now() + config.streamAckTimeoutMs;
    table.set(streamId, { ctk: entry.att.ctk, acked: false, deadline, head: false });
    this.sql.exec(
      "INSERT INTO streams (tid, sid, ctk, acked, deadline) VALUES (?, ?, ?, 0, ?)",
      server.att.tid,
      streamId,
      entry.att.ctk,
      deadline,
    );
    const open = control("STREAM_OPEN", { stream_id: streamId });
    this.sendCounted(server, open, open.length, config.maxServerBufferBytes);
    this.arm();
  }

  // Both halves of §5.2's relay_peer_ip rule: a client that sends the field is
  // refused, and the relay writes it from CF-Connecting-IP on every head.
  private requestHead(entry: Entry<ClientAtt>, body: Body): void {
    if ("relay_peer_ip" in body) {
      log("protocol_error", { client_trunk_id: entry.att.ctk, reason: "client sent relay_peer_ip" });
      throw new ProtocolError();
    }
    const streamId = streamIdOf(body);
    // The origin's trunk is shared by every client of the server, so a head
    // it could not interpret is refused here, to its sender only (§5.2).
    if (!isRequestHead(body)) throw new ProtocolError();
    const server = this.owned(entry, streamId);
    if (!server) return;
    const tid = server.att.tid;
    const stream = this.streams.get(tid)?.get(streamId);
    if (!stream) return;
    if (stream.head) {
      log("stream_refused", { client_trunk_id: entry.att.ctk, stream_id: streamId, reason: "second head" });
      this.removeStream(tid, streamId);
      const refused = errorOnStream("stream_closed", streamId);
      this.sendCounted(entry, refused, refused.length, this.config.maxClientBufferBytes);
      this.sendCounted(server, closeStream(streamId), 0, this.config.maxServerBufferBytes);
      return;
    }
    stream.head = true;
    this.sql.exec("UPDATE streams SET head = 1 WHERE tid = ? AND sid = ?", tid, streamId);
    const json = control("HTTP_REQUEST_HEAD", { ...body, relay_peer_ip: entry.att.ip });
    this.sendCounted(server, json, json.length, this.config.maxServerBufferBytes);
  }

  private requestBody(entry: Entry<ClientAtt>, streamId: number, raw: ArrayBuffer): void {
    const server = this.owned(entry, streamId);
    if (!server) return;
    const budget = this.config.maxServerBufferBytes;
    if (!this.admits(server, raw.byteLength, budget)) {
      // The origin has not read what the relay already holds for it, and the
      // relay cannot slow this client down; the stream is refused instead.
      log("server_backlogged", { trunk_id: server.att.tid, stream_id: streamId });
      this.removeStream(server.att.tid, streamId);
      const json = errorOnStream("rate_limited", streamId);
      this.sendCounted(entry, json, json.length, this.config.maxClientBufferBytes);
      this.sendCounted(server, closeStream(streamId), 0, budget);
      return;
    }
    this.sendCounted(server, raw, raw.byteLength, budget);
  }

  private clientClose(entry: Entry<ClientAtt>, body: Body): void {
    if (!("stream_id" in body)) return this.dropClient(entry, null);
    if (body.stream_id === null) throw new ProtocolError();
    const streamId = streamIdOf(body);
    const server = this.owned(entry, streamId);
    if (!server) return;
    this.removeStream(server.att.tid, streamId);
    const json = closeStream(streamId);
    this.sendCounted(server, json, json.length, this.config.maxServerBufferBytes);
  }

  // A stream_id this client was given, or stream_closed — the same answer for
  // another client's live stream and for one that never existed.
  private owned(entry: Entry<ClientAtt>, streamId: number): Entry<ServerAtt> | null {
    const tid = entry.att.tid ?? "";
    const stream = this.streams.get(tid)?.get(streamId);
    const server = this.entries.get(tid) as Entry<ServerAtt> | undefined;
    if (stream && stream.ctk === entry.att.ctk && server) return server;
    const json = errorOnStream("stream_closed", streamId);
    this.sendCounted(entry, json, json.length, this.config.maxClientBufferBytes);
    return null;
  }

  private dropClient(entry: Entry<ClientAtt>, code: ErrorCode | null): void {
    const att = entry.att;
    if (this.entries.get(att.ctk) !== entry) return;
    this.forget(att);
    this.close(entry, code);
    if (att.tid) {
      const table = this.streams.get(att.tid);
      if (table) {
        for (const [streamId, stream] of [...table]) {
          if (stream.ctk !== att.ctk) continue;
          this.removeStream(att.tid, streamId);
          const server = this.entries.get(att.tid);
          if (server) this.sendCounted(server, closeStream(streamId), 0, this.config.maxServerBufferBytes);
        }
      }
    }
    if (att.ph === 2) log("client_closed", { client_trunk_id: att.ctk, code: code ?? undefined });
    this.arm();
  }

  // ---- flow: what "fell behind" means without a send buffer -----------

  // With no watermark outstanding the peer has read all but the last step,
  // the nearest this relay can see to storm-relay's "empty queue", which
  // admits any one frame. The room's total is a second bound: every socket
  // here shares one isolate's memory.
  private admits(entry: Entry, size: number, budget: number): boolean {
    const f = entry.att.f;
    if (this.held > 0 && this.held + size > this.config.maxRoomBufferBytes) return false;
    return f.m.length === 0 || f.s - f.a + size <= budget;
  }

  private sendCounted(entry: Entry, data: string | ArrayBuffer, size: number, budget: number): boolean {
    const f = entry.att.f;
    f.s += size;
    if (!this.send(entry, data)) return false;
    const last = f.m.length > 0 ? f.m[f.m.length - 1] : f.a;
    if (f.s - last >= Math.max(1, Math.floor(budget / 8))) {
      f.m.push(f.s);
      this.held += f.s - last;
      if (!this.send(entry, PING)) return false;
      this.persist(entry);
    }
    return true;
  }

  private watermarkAnswered(entry: Entry): void {
    const f = entry.att.f;
    const mark = f.m.shift();
    if (mark === undefined) return;
    this.held -= mark - f.a;
    f.a = mark;
    f.e = 0;
    this.persist(entry);
  }

  // Bytes behind this socket's unanswered watermarks.
  private unread(att: Att): number {
    const f = att.f;
    return f.m.length > 0 ? f.m[f.m.length - 1] - f.a : 0;
  }

  private forget(att: Att): void {
    this.entries.delete(idOf(att));
    this.held -= this.unread(att);
  }

  // ---- timers ----------------------------------------------------------

  async alarm(): Promise<void> {
    this.alarmAt = null;
    const now = Date.now();
    const config = this.config;
    for (const entry of [...this.entries.values()]) {
      if (this.entries.get(idOf(entry.att)) !== entry) continue;
      if (entry.att.k === "s") {
        const server = entry as Entry<ServerAtt>;
        const att = server.att;
        if (att.ph < 2) {
          if (att.hs <= now) this.dropServer(server, null, "handshake timeout");
        } else if (this.lastSeen(server) + config.heartbeatDeadlineMs <= now) {
          log("server_heartbeat_expired", { server_id: att.sid, trunk_id: att.tid });
          this.dropServer(server, null, "heartbeat deadline");
        } else if (att.sup !== undefined && att.sup <= now) {
          // trunk_superseded, not trunk_lost: the server is up on a newer
          // trunk, and a client should reconnect at once.
          log("server_drain_ended", { server_id: att.sid, trunk_id: att.tid });
          this.dropServer(server, null, "drain ended", "trunk_superseded");
        }
      } else {
        const client = entry as Entry<ClientAtt>;
        if (client.att.ph === 0 && client.att.hs <= now) this.dropClient(client, null);
        else if (client.att.ph === 1 && (client.att.hw ?? 0) <= now) this.dropClient(client, "server_unreachable");
      }
    }
    for (const [tid, table] of this.streams) {
      for (const [streamId, stream] of [...table]) {
        // An earlier send in this loop may have ended a peer and its streams.
        if (this.streams.get(tid)?.get(streamId) !== stream) continue;
        if (stream.acked || stream.deadline === null || stream.deadline > now) continue;
        // No retry (§5.1): the client decides whether to try again.
        this.removeStream(tid, streamId);
        const client = this.entries.get(stream.ctk);
        if (client) {
          const json = errorOnStream("server_timeout", streamId);
          this.sendCounted(client, json, json.length, config.maxClientBufferBytes);
        }
        const server = this.entries.get(tid);
        if (server) this.sendCounted(server, closeStream(streamId), 0, config.maxServerBufferBytes);
      }
    }
    this.sql.exec("DELETE FROM nonces WHERE expires <= ?", now);
    this.arm();
  }

  // Any frame keeps a trunk alive, including the PINGs the runtime answered
  // without waking this object.
  private lastSeen(server: Entry<ServerAtt>): number {
    let seen = Math.max(server.att.seen, server.seen ?? 0);
    try {
      const auto = this.ctx.getWebSocketAutoResponseTimestamp(server.ws);
      if (auto) seen = Math.max(seen, auto.getTime());
    } catch {}
    return seen;
  }

  private arm(): void {
    const config = this.config;
    let next = Infinity;
    for (const entry of this.entries.values()) {
      if (entry.att.k === "s") {
        const att = entry.att;
        if (att.ph < 2) next = Math.min(next, att.hs);
        else {
          next = Math.min(next, this.lastSeen(entry as Entry<ServerAtt>) + config.heartbeatDeadlineMs);
          if (att.sup !== undefined) next = Math.min(next, att.sup);
        }
      } else {
        const att = entry.att;
        if (att.ph === 0) next = Math.min(next, att.hs);
        else if (att.ph === 1 && att.hw !== undefined) next = Math.min(next, att.hw);
      }
    }
    for (const table of this.streams.values()) {
      for (const stream of table.values()) if (stream.deadline !== null) next = Math.min(next, stream.deadline);
    }
    if (next === Infinity) {
      if (this.alarmAt !== null) void this.ctx.storage.deleteAlarm();
      this.alarmAt = null;
      return;
    }
    if (this.alarmAt === next) return;
    this.alarmAt = next;
    void this.ctx.storage.setAlarm(next);
  }

  // ---- plumbing --------------------------------------------------------

  private table(tid: string): Map<number, Stream> {
    let table = this.streams.get(tid);
    if (!table) {
      table = new Map();
      this.streams.set(tid, table);
    }
    return table;
  }

  private removeStream(tid: string, streamId: number): void {
    this.streams.get(tid)?.delete(streamId);
    this.sql.exec("DELETE FROM streams WHERE tid = ? AND sid = ?", tid, streamId);
  }

  private *servers(): Generator<Entry<ServerAtt>> {
    for (const entry of this.entries.values()) if (entry.att.k === "s") yield entry as Entry<ServerAtt>;
  }

  private *clients(): Generator<Entry<ClientAtt>> {
    for (const entry of [...this.entries.values()]) if (entry.att.k === "c") yield entry as Entry<ClientAtt>;
  }

  private liveServer(client: ClientAtt): Entry<ServerAtt> | undefined {
    for (const server of this.servers()) {
      if (server.att.ph === 2 && server.att.sup === undefined && this.reaches(client, server) && this.alive(server)) {
        return server;
      }
    }
    return undefined;
  }

  // §4.2: past the deadline a trunk takes no new HELLO or OPEN_STREAM, even
  // if the alarm that ends it has not run yet.
  private alive(server: Entry<ServerAtt>): boolean {
    if (this.lastSeen(server) + this.config.heartbeatDeadlineMs > Date.now()) return true;
    log("server_heartbeat_expired", { server_id: server.att.sid, trunk_id: server.att.tid });
    this.dropServer(server, null, "heartbeat deadline");
    return false;
  }

  // Every frame the room sends goes through here. A peer found closed is
  // ended now, never thrown at; its close event, when it runs, finds it gone.
  private send(entry: Entry, data: string | ArrayBuffer): boolean {
    if (trySend(entry.ws, data)) return true;
    this.lost(entry);
    return false;
  }

  // Only for an entry already forgotten, so a failed send here ends nothing.
  private close(entry: Entry, code: ErrorCode | null): void {
    if (code) this.send(entry, error(code));
    try {
      entry.ws.close(1000);
    } catch {}
  }
}
