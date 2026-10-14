import { b64urlEncode, decodeKey, parsePublicKey, validateServerId } from "./auth";
import { ConfigError, configFor, type Env } from "./config";
import { HelloLimiter } from "./limiter";
import { log } from "./log";
import { error, parseFrame, PONG, ProtocolError, stringField, type ErrorCode } from "./proto";
import { RelayRoom } from "./room";
import { trySend } from "./send";

export { HelloLimiter, RelayRoom };

const REGISTER_PATH = "/register";
const CONNECT_PREFIX = "/connect/";

// Headers on the Worker → Durable Object hop only. The DO is unreachable from
// outside, so these are facts the Worker established, not client claims.
const H_SERVER_ID = "x-storm-server-id";
const H_PEER_IP = "x-storm-peer-ip";
const H_PUBLIC_BASE = "x-storm-public-base";
const H_PUBKEY = "x-storm-pubkey";

// One room per server_id in allowlist and tofu (the binding lives there). In
// open mode one per (server_id, pubkey), so nothing done under one key can
// reach a registration under another (§4.1 rule 2). server_id has no ':', so
// the two namings never meet.
function roomName(serverId: string, key: string | null): string {
  return key === null ? serverId : `${serverId}:${key}`;
}

function refuse(code: ErrorCode): Response {
  const [client, server] = Object.values(new WebSocketPair());
  server.accept();
  trySend(server, error(code));
  server.close(1000);
  return new Response(null, { status: 101, webSocket: client });
}

function publicBase(env: Env, url: URL): string {
  const configured = configFor(env).publicBase;
  if (configured) return configured;
  return `${url.protocol === "https:" ? "wss" : "ws"}://${url.host}`;
}

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    const url = new URL(request.url);
    const isRegister = url.pathname === REGISTER_PATH;
    const isConnect = url.pathname.startsWith(CONNECT_PREFIX) && !url.pathname.slice(CONNECT_PREFIX.length).includes("/");
    if (!isRegister && !isConnect) return new Response("not found", { status: 404 });
    if (request.method !== "GET") return new Response("method not allowed", { status: 405 });
    if (request.headers.get("upgrade")?.toLowerCase() !== "websocket") {
      return new Response("expected a websocket upgrade", { status: 426 });
    }

    try {
      configFor(env);
    } catch (e) {
      log("config_invalid", { reason: e instanceof ConfigError ? e.message : String(e) });
      return refuse("protocol_error");
    }

    return isRegister ? register(request, env, url) : connect(request, env, url);
  },
};

// §6.2: a limit counts against an IPv6 source's /64, since one subscriber
// holds the whole prefix, and against an IPv4-mapped source's IPv4 address.
export function sourceKey(ip: string): string {
  if (!ip.includes(":")) return ip;
  const mapped = /^::ffff:(\d+\.\d+\.\d+\.\d+)$/i.exec(ip);
  if (mapped) return mapped[1];
  const [head, tail = ""] = ip.split("::");
  const left = head ? head.split(":") : [];
  const right = ip.includes("::") && tail ? tail.split(":") : [];
  const groups = ip.includes("::")
    ? [...left, ...Array(8 - left.length - right.length).fill("0"), ...right]
    : left;
  return `${groups
    .slice(0, 4)
    .map((g) => parseInt(g || "0", 16).toString(16))
    .join(":")}::/64`;
}

async function limited(env: Env, scope: string, ip: string, limit: number, windowMs: number): Promise<boolean> {
  const limiter = env.HELLO_LIMITER.get(env.HELLO_LIMITER.idFromName(`${scope}:${sourceKey(ip)}`));
  return !(await limiter.take(limit, windowMs));
}

// `/register` names no server_id, so the Worker reads REGISTER_SERVER before it
// can pick the Durable Object, and then carries the trunk's frames both ways.
// See PLAN.md decision 108b for why this is the one hop that is not spliced.
async function register(request: Request, env: Env, url: URL): Promise<Response> {
  const config = configFor(env);
  // §6.2: counted before a socket, a challenge or a room is spent on it. The
  // log says it happened, never from where (§6.3).
  const peerIp = request.headers.get("cf-connecting-ip");
  if (!peerIp) {
    log("relay_fault", { reason: "register: no CF-Connecting-IP" });
    return refuse("protocol_error");
  }
  if (await limited(env, "reg", peerIp, config.registerRateLimit, config.registerRateWindowMs)) {
    log("register_rate_limited");
    return refuse("rate_limited");
  }
  const base = publicBase(env, url);
  const [client, server] = Object.values(new WebSocketPair());
  // The standard default is "blob"; body frames are forwarded as bytes.
  server.binaryType = "arraybuffer";
  server.accept();
  const timeout = setTimeout(() => close(server), configFor(env).handshakeTimeoutMs);
  let room: WebSocket | null = null;
  let pending: (string | ArrayBuffer)[] | null = null;
  let serverGone = false;

  const close = (ws: WebSocket, code?: ErrorCode) => {
    if (code) trySend(ws, error(code));
    try {
      ws.close(1000);
    } catch {}
  };
  // Either side of this hop can be gone while the other's frames are still
  // being delivered; the frame is dropped and both sides are closed, and
  // closing the room's side is what runs the room's cleanup for this trunk.
  const carry = (to: WebSocket, from: WebSocket, data: string | ArrayBuffer): boolean => {
    if (trySend(to, data)) return true;
    close(to);
    close(from);
    return false;
  };

  const openRoom = async (first: string, serverId: string, key: string | null) => {
    try {
      const stub = env.ROOMS.get(env.ROOMS.idFromName(roomName(serverId, key)));
      const headers: Record<string, string> = { upgrade: "websocket", [H_SERVER_ID]: serverId, [H_PUBLIC_BASE]: base };
      if (key !== null) headers[H_PUBKEY] = key;
      const response = await stub.fetch("https://room/register", { headers });
      const ws = response.webSocket;
      if (!ws) throw new Error(`room answered ${response.status}`);
      ws.binaryType = "arraybuffer";
      ws.accept();
      ws.addEventListener("message", (e) => void carry(server, ws, e.data));
      ws.addEventListener("close", () => close(server));
      ws.addEventListener("error", () => close(server));
      for (const data of [first, ...(pending ?? [])]) if (!carry(ws, server, data)) return;
      room = ws;
      if (serverGone) close(ws);
    } catch (e) {
      // The error's class only: a message can carry anything (§6.3).
      log("relay_fault", { server_id: serverId, reason: `register: ${e instanceof Error ? e.name : "unknown"}` });
      close(server, "protocol_error");
    }
  };

  server.addEventListener("message", (event) => {
    if (room) {
      carry(room, server, event.data);
      return;
    }
    if (pending) {
      pending.push(event.data);
      return;
    }
    let serverId: string;
    let key: string | null = null;
    try {
      if (typeof event.data !== "string") throw new ProtocolError();
      const frame = parseFrame(event.data);
      // A PING before REGISTER_SERVER is answered like any other (§4.2).
      if (frame.type === "PING") {
        trySend(server, PONG);
        return;
      }
      if (frame.type === "PONG") return;
      pending = [];
      clearTimeout(timeout);
      if (frame.type !== "REGISTER_SERVER") throw new ProtocolError();
      serverId = stringField(frame.body, "server_id");
      const pubkey = stringField(frame.body, "pubkey");
      if (!validateServerId(serverId)) throw new ProtocolError();
      // §6.2: on an allowlist relay an unlisted id gets no room at all. It is
      // the same answer the room would give, without the Durable Object.
      if (config.binding === "allowlist" && !config.allowlist.has(serverId)) {
        pending = [];
        log("registration_refused", { server_id: serverId, reason: "not on the allowlist" });
        close(server, "auth_failed");
        return;
      }
      if (config.binding === "open") {
        const bytes = parsePublicKey(pubkey);
        if (!bytes) throw new ProtocolError();
        key = b64urlEncode(bytes);
      }
    } catch {
      pending = [];
      log("protocol_error", { reason: "register: first frame" });
      close(server, "protocol_error");
      return;
    }
    void openRoom(event.data, serverId, key);
  });
  const serverClosed = () => {
    serverGone = true;
    clearTimeout(timeout);
    if (room) close(room);
  };
  server.addEventListener("close", serverClosed);
  server.addEventListener("error", serverClosed);

  return new Response(null, { status: 101, webSocket: client });
}

async function connect(request: Request, env: Env, url: URL): Promise<Response> {
  const config = configFor(env);
  // Set by Cloudflare's edge, which overwrites any value a client sends: the
  // relay's only source for relay_peer_ip (§5.2, rule 5 of the plan note).
  const peerIp = request.headers.get("cf-connecting-ip");
  if (!peerIp) {
    log("relay_fault", { reason: "connect: no CF-Connecting-IP" });
    return refuse("protocol_error");
  }

  if (await limited(env, "ip", peerIp, config.helloRateLimit, config.helloRateWindowMs)) {
    log("hello_rate_limited");
    return refuse("rate_limited");
  }

  let serverId: string;
  try {
    serverId = decodeURIComponent(url.pathname.slice(CONNECT_PREFIX.length));
  } catch {
    serverId = "";
  }
  if (!validateServerId(serverId)) {
    log("protocol_error", { reason: "connect: bad server_id in path" });
    return refuse("protocol_error");
  }

  // §4.3: the key a client names on the URL, so this edge can pick the room
  // before it reads HELLO. Checked against HELLO.pubkey in the room (§5).
  let key: string | null = null;
  const query = url.searchParams.get("pubkey");
  if (query !== null) {
    const bytes = decodeKey(query);
    if (!bytes) {
      log("protocol_error", { reason: "connect: bad pubkey in query" });
      return refuse("protocol_error");
    }
    key = b64urlEncode(bytes);
  }

  // Without a key on the URL an open relay cannot route; that room holds no
  // server and answers the HELLO itself.
  const room = roomName(serverId, config.binding === "open" ? key : null);
  const headers: Record<string, string> = { upgrade: "websocket", [H_SERVER_ID]: serverId, [H_PEER_IP]: peerIp };
  if (key !== null) headers[H_PUBKEY] = key;
  return env.ROOMS.get(env.ROOMS.idFromName(room)).fetch("https://room/connect", { headers });
}
