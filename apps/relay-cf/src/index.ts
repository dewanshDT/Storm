import { validateServerId } from "./auth";
import { ConfigError, configFor, type Env } from "./config";
import { HelloLimiter } from "./limiter";
import { log } from "./log";
import { error, parseFrame, ProtocolError, stringField, type ErrorCode } from "./proto";
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

    return isRegister ? register(env, url) : connect(request, env, url);
  },
};

// `/register` names no server_id, so the Worker reads REGISTER_SERVER before it
// can pick the Durable Object, and then carries the trunk's frames both ways.
// See PLAN.md decision 108b for why this is the one hop that is not spliced.
async function register(env: Env, url: URL): Promise<Response> {
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

  const openRoom = async (first: string, serverId: string) => {
    try {
      const stub = env.ROOMS.get(env.ROOMS.idFromName(serverId));
      const response = await stub.fetch("https://room/register", {
        headers: { upgrade: "websocket", [H_SERVER_ID]: serverId, [H_PUBLIC_BASE]: base },
      });
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
      log("relay_fault", { server_id: serverId, reason: `register: ${String(e)}` });
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
    pending = [];
    clearTimeout(timeout);
    let serverId: string;
    try {
      if (typeof event.data !== "string") throw new ProtocolError();
      const frame = parseFrame(event.data);
      if (frame.type !== "REGISTER_SERVER") throw new ProtocolError();
      serverId = stringField(frame.body, "server_id");
      stringField(frame.body, "pubkey");
      if (!validateServerId(serverId)) throw new ProtocolError();
    } catch {
      log("protocol_error", { reason: "register: first frame" });
      close(server, "protocol_error");
      return;
    }
    void openRoom(event.data, serverId);
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

  const limiter = env.HELLO_LIMITER.get(env.HELLO_LIMITER.idFromName(`ip:${peerIp}`));
  if (!(await limiter.take(config.helloRateLimit, config.helloRateWindowMs))) {
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

  const stub = env.ROOMS.get(env.ROOMS.idFromName(serverId));
  return stub.fetch("https://room/connect", {
    headers: { upgrade: "websocket", [H_SERVER_ID]: serverId, [H_PEER_IP]: peerIp },
  });
}
