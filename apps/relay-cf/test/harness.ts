import { SELF } from "cloudflare:test";
import { expect } from "vitest";
import { b64urlEncode } from "../src/auth";

export type Json = Record<string, any>;

export type Frame =
  | { kind: "text"; value: Json; raw: string }
  | { kind: "body"; type: number; streamId: number; payload: Uint8Array }
  | { kind: "closed" };

const PKCS8_ED25519 = [0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22, 0x04, 0x20];

export async function signingKey(seed: number | Uint8Array): Promise<{ sign(msg: string): Promise<string>; pubkey: string }> {
  const bytes = typeof seed === "number" ? new Uint8Array(32).fill(seed) : seed;
  const pkcs8 = new Uint8Array([...PKCS8_ED25519, ...bytes]);
  const key = await crypto.subtle.importKey("pkcs8", pkcs8, { name: "Ed25519" }, true, ["sign"]);
  const jwk = (await crypto.subtle.exportKey("jwk", key)) as JsonWebKey;
  return {
    pubkey: jwk.x!,
    async sign(msg: string) {
      const sig = await crypto.subtle.sign({ name: "Ed25519" }, key, new TextEncoder().encode(msg));
      return b64urlEncode(new Uint8Array(sig));
    },
  };
}

export const relayAuth = (serverId: string, nonce: string) => `storm-relay-auth:v1:${serverId}:${nonce}`;

export function uniqueId(prefix = "srv_"): string {
  const bytes = new Uint8Array(8);
  crypto.getRandomValues(bytes);
  return prefix + [...bytes].map((b) => b.toString(16).padStart(2, "0")).join("").toUpperCase();
}

export function uniqueIp(): string {
  const b = new Uint8Array(3);
  crypto.getRandomValues(b);
  return `10.${b[0]}.${b[1]}.${b[2]}`;
}

export const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

export class Peer {
  private readonly queue: Frame[] = [];
  private waiter: (() => void) | null = null;
  // Real clients and storm-server answer every PING; the relay's watermark
  // PINGs depend on it. A test of a client that stops reading turns this off.
  answerPings = true;
  // Like real servers and clients, a peer heartbeats once its handshake is
  // done (REGISTERED or READY), so a test's idle moment never reads as
  // silence on a slow runner. The PONGs answering those PINGs are counted
  // and kept out of the queue: the relay answers in order, so a test's own
  // PING still gets exactly one PONG. A test about silence calls silent().
  private beat: ReturnType<typeof setInterval> | null = null;
  private quiet = false;
  private ownPongs = 0;
  pingsSeen = 0;
  closed = false;
  private unanswered = 0;

  // A client that stops reading never sees the relay's PINGs; when it reads
  // again it answers every one it had not reached.
  stopReading() {
    this.answerPings = false;
  }

  resumeReading() {
    this.answerPings = true;
    for (; this.unanswered > 0; this.unanswered--) this.pong();
  }

  // workerd still delivers messages that arrived before a close, and throws
  // on a send after it; a PING queued behind the close goes unanswered.
  private pong() {
    if (this.ws.readyState === WebSocket.READY_STATE_OPEN) this.ws.send(JSON.stringify({ v: 1, type: "PONG" }));
  }

  private constructor(readonly ws: WebSocket) {
    ws.addEventListener("message", (event) => {
      if (typeof event.data === "string") {
        const value = JSON.parse(event.data);
        if (value.type === "PONG" && this.ownPongs > 0) {
          this.ownPongs--;
          return;
        }
        if (value.type === "READY" || value.type === "REGISTERED") this.startBeat();
        if (value.type === "PING") {
          this.pingsSeen++;
          if (this.answerPings) this.pong();
          else this.unanswered++;
          return;
        }
        this.push({ kind: "text", value, raw: event.data });
      } else {
        const bytes = new Uint8Array(event.data as ArrayBuffer);
        const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
        this.push({ kind: "body", type: bytes[0], streamId: view.getUint32(1), payload: bytes.slice(5) });
      }
    });
    ws.addEventListener("close", () => {
      this.stopBeat();
      this.closed = true;
      this.push({ kind: "closed" });
    });
  }

  static async dial(path: string, ip = uniqueIp()): Promise<Peer> {
    const response = await SELF.fetch(`https://relay.test${path}`, {
      headers: { upgrade: "websocket", "cf-connecting-ip": ip },
    });
    const ws = response.webSocket;
    if (!ws) throw new Error(`no websocket: ${response.status}`);
    ws.binaryType = "arraybuffer";
    ws.accept();
    return new Peer(ws);
  }

  private push(frame: Frame) {
    this.queue.push(frame);
    const waiter = this.waiter;
    this.waiter = null;
    waiter?.();
  }

  send(value: Json) {
    this.ws.send(JSON.stringify(value));
  }

  sendRaw(data: string | ArrayBuffer) {
    this.ws.send(data);
  }

  sendBody(type: number, streamId: number, payload: Uint8Array = new Uint8Array()) {
    const frame = new Uint8Array(5 + payload.length);
    frame[0] = type;
    new DataView(frame.buffer).setUint32(1, streamId);
    frame.set(payload, 5);
    this.ws.send(frame.buffer);
  }

  async next(timeoutMs = 6000): Promise<Frame> {
    const deadline = Date.now() + timeoutMs;
    while (this.queue.length === 0) {
      const remaining = deadline - Date.now();
      if (remaining <= 0) throw new Error("timed out waiting for a frame");
      await new Promise<void>((resolve) => {
        const timer = setTimeout(resolve, remaining);
        this.waiter = () => {
          clearTimeout(timer);
          resolve();
        };
      });
    }
    return this.queue.shift()!;
  }

  // The next text frame, skipping body chunks.
  async recv(timeoutMs = 6000): Promise<Json> {
    for (;;) {
      const frame = await this.next(timeoutMs);
      if (frame.kind === "text") return frame.value;
      if (frame.kind === "closed") throw new Error("closed while waiting for a text frame");
    }
  }

  async expectClosed(timeoutMs = 6000): Promise<void> {
    for (;;) {
      const frame = await this.next(timeoutMs);
      if (frame.kind === "closed") return;
    }
  }

  async silentFor(ms: number): Promise<boolean> {
    try {
      const frame = await this.next(ms);
      this.queue.unshift(frame);
      return false;
    } catch {
      return true;
    }
  }

  private startBeat() {
    if (this.beat || this.quiet) return;
    this.beat = setInterval(() => {
      if (this.ws.readyState !== WebSocket.READY_STATE_OPEN) return;
      this.ownPongs++;
      this.send({ v: 1, type: "PING" });
    }, 300);
  }

  private stopBeat() {
    if (this.beat) clearInterval(this.beat);
    this.beat = null;
  }

  // Stops this peer's heartbeat for good: the tests of the relay's deadlines.
  silent(): this {
    this.quiet = true;
    this.stopBeat();
    return this;
  }

  close() {
    this.stopBeat();
    try {
      this.ws.close(1000);
    } catch {}
  }
}

export function expectError(reply: Json, code: string) {
  expect(reply.type).toBe("ERROR");
  expect(reply.v).toBe(1);
  expect(reply.code).toBe(code);
}

export async function register(serverId: string, seed = 1): Promise<{ peer: Peer; registered: Json }> {
  const key = await signingKey(seed);
  const peer = await Peer.dial("/register");
  peer.send({ v: 1, type: "REGISTER_SERVER", server_id: serverId, pubkey: key.pubkey });
  const challenge = await peer.recv();
  expect(challenge.type, JSON.stringify(challenge)).toBe("CHALLENGE");
  peer.send({ v: 1, type: "CHALLENGE_RESPONSE", sig: await key.sign(relayAuth(serverId, challenge.nonce)) });
  const registered = await peer.recv();
  expect(registered.type, JSON.stringify(registered)).toBe("REGISTERED");
  return { peer, registered };
}

// A registration done up to its last frame. A test that registers while a
// client's HELLO is held has only the hold for it; the dial, the challenge and the
// signature must not be inside that window on a slow runner.
export async function preparedRegistration(serverId: string, seed = 1): Promise<() => Promise<Peer>> {
  const key = await signingKey(seed);
  const peer = await Peer.dial("/register");
  peer.send({ v: 1, type: "REGISTER_SERVER", server_id: serverId, pubkey: key.pubkey });
  const challenge = await peer.recv();
  expect(challenge.type, JSON.stringify(challenge)).toBe("CHALLENGE");
  const sig = await key.sign(relayAuth(serverId, challenge.nonce));
  return async () => {
    peer.send({ v: 1, type: "CHALLENGE_RESPONSE", sig });
    const registered = await peer.recv();
    expect(registered.type, JSON.stringify(registered)).toBe("REGISTERED");
    return peer;
  };
}

// A HELLO naming the key of `seed`, in the query and in the HELLO (§5).
export async function keyedHello(serverId: string, seed: number): Promise<Peer> {
  const { pubkey } = await signingKey(seed);
  const peer = await Peer.dial(`/connect/${serverId}?pubkey=${pubkey}`);
  peer.send({ v: 1, type: "HELLO", server_id: serverId, pubkey });
  return peer;
}

export async function keyedClient(serverId: string, seed: number): Promise<Peer> {
  const peer = await keyedHello(serverId, seed);
  const ready = await peer.recv();
  expect(ready.type, JSON.stringify(ready)).toBe("READY");
  return peer;
}

export async function client(serverId: string, ip?: string): Promise<Peer> {
  const peer = await Peer.dial(`/connect/${serverId}`, ip);
  peer.send({ v: 1, type: "HELLO", server_id: serverId });
  const ready = await peer.recv();
  expect(ready.type, JSON.stringify(ready)).toBe("READY");
  expect(typeof ready.client_trunk_id).toBe("string");
  return peer;
}

export async function openStream(peer: Peer, attemptId: unknown = "a1"): Promise<number> {
  peer.send({ v: 1, type: "OPEN_STREAM", attempt_id: attemptId });
  const ready = await peer.recv();
  expect(ready.type, JSON.stringify(ready)).toBe("STREAM_READY");
  expect(ready.attempt_id).toEqual(attemptId);
  return ready.stream_id;
}

export async function expectOpenAndAck(server: Peer): Promise<number> {
  const open = await server.recv();
  expect(open.type, JSON.stringify(open)).toBe("STREAM_OPEN");
  server.send({ v: 1, type: "STREAM_ACK", stream_id: open.stream_id });
  return open.stream_id;
}

export function requestHead(streamId: number, path = "/v1/notes", method = "GET"): Json {
  return {
    v: 1,
    type: "HTTP_REQUEST_HEAD",
    stream_id: streamId,
    method,
    path,
    headers: { authorization: "Bearer inside-the-tunnel" },
  };
}

export function responseHead(streamId: number, status = 200): Json {
  return { v: 1, type: "HTTP_RESPONSE_HEAD", stream_id: streamId, status, headers: { "content-type": "text/plain" } };
}

export const bytes = (text: string) => new TextEncoder().encode(text);
export const text = (b: Uint8Array) => new TextDecoder().decode(b);
