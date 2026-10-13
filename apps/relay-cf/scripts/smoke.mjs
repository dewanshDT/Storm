#!/usr/bin/env node
// A real SRP request through a running relay to a real storm-server: the
// identity challenge, a bodied POST that writes a note, reads, concurrency,
// and the change feed. A port of apps/runtime/tests/srp_live.rs plus the
// change-feed case of relay_live_test.dart, for `wrangler dev`.
//
//   RELAY=ws://127.0.0.1:18910 ORIGIN=http://127.0.0.1:18920 \
//   STORM_SESSION="Bearer sta_…" node scripts/smoke.mjs
//
// SERVER_ID and SERVER_KEY default to what ORIGIN's GET /v1/server says.
import { createPublicKey, randomBytes, verify } from "node:crypto";

const RELAY = process.env.RELAY ?? "ws://127.0.0.1:18910";
const ORIGIN = must("ORIGIN");
const SESSION = must("STORM_SESSION");
const FRAME = 256 * 1024;

function must(name) {
  const value = process.env[name];
  if (!value) throw new Error(`${name} is not set`);
  return value;
}

function check(ok, what) {
  if (!ok) throw new Error(`FAIL: ${what}`);
  console.log(`ok - ${what}`);
}

class Trunk {
  static async connect(serverId) {
    const ws = new WebSocket(`${RELAY}/connect/${serverId}`);
    ws.binaryType = "arraybuffer";
    const trunk = new Trunk(ws);
    await new Promise((resolve, reject) => {
      ws.onopen = resolve;
      ws.onerror = () => reject(new Error("relay unreachable"));
    });
    ws.send(JSON.stringify({ v: 1, type: "HELLO", server_id: serverId }));
    const first = await trunk.control();
    if (first.type !== "READY") throw Object.assign(new Error(first.code ?? first.type), { reply: first });
    return trunk;
  }

  constructor(ws) {
    this.ws = ws;
    this.waiting = [];
    this.attempts = new Map();
    this.streams = new Map();
    ws.onmessage = (event) => this.onMessage(event.data);
    ws.onclose = () => {
      for (const s of this.streams.values()) s.fail(new Error("trunk closed"));
    };
  }

  control() {
    return new Promise((resolve) => this.waiting.push(resolve));
  }

  onMessage(data) {
    if (typeof data !== "string") {
      const bytes = new Uint8Array(data);
      const id = new DataView(data).getUint32(1);
      this.streams.get(id)?.chunk(bytes.subarray(5));
      return;
    }
    const msg = JSON.parse(data);
    if (msg.type === "PING") return this.ws.send(JSON.stringify({ v: 1, type: "PONG" }));
    if (msg.type === "STREAM_READY") return this.attempts.get(msg.attempt_id)?.(msg.stream_id);
    if (msg.stream_id !== undefined && this.streams.has(msg.stream_id)) return this.streams.get(msg.stream_id).control(msg);
    if (msg.type === "ERROR" && msg.attempt_id !== undefined) return this.attempts.get(msg.attempt_id)?.(msg);
    this.waiting.shift()?.(msg);
  }

  // Resolves with the head; `body` yields chunks until CLOSE, and throws on a
  // stream-scoped ERROR (§5.4 rule 3).
  async request(method, path, headers = {}, body = new Uint8Array()) {
    const attempt = randomBytes(6).toString("hex");
    const opened = new Promise((resolve) => this.attempts.set(attempt, resolve));
    this.ws.send(JSON.stringify({ v: 1, type: "OPEN_STREAM", attempt_id: attempt }));
    const streamId = await opened;
    this.attempts.delete(attempt);
    if (typeof streamId !== "number") throw new Error(`open refused: ${streamId.code}`);

    const queue = [];
    let wake = null;
    let head;
    const headReady = new Promise((resolve, reject) => (head = { resolve, reject }));
    const push = (item) => {
      queue.push(item);
      wake?.();
    };
    this.streams.set(streamId, {
      control: (msg) => {
        if (msg.type === "HTTP_RESPONSE_HEAD") head.resolve(msg);
        else if (msg.type === "CLOSE") push({ end: true });
        else if (msg.type === "ERROR") {
          head.reject(new Error(msg.code));
          push({ error: msg.code });
        }
      },
      chunk: (bytes) => push({ bytes: new Uint8Array(bytes) }),
      fail: (e) => {
        head.reject(e);
        push({ error: e.message });
      },
    });

    this.ws.send(JSON.stringify({ v: 1, type: "HTTP_REQUEST_HEAD", stream_id: streamId, method, path, headers }));
    for (let at = 0; at < body.length; at += FRAME) this.sendBody(streamId, body.subarray(at, at + FRAME));
    this.sendBody(streamId, new Uint8Array());

    const response = await headReady;
    const streams = this.streams;
    response.body = async function* () {
      for (;;) {
        while (queue.length === 0) await new Promise((r) => (wake = r));
        const item = queue.shift();
        if (item.end) break;
        if (item.error) throw new Error(item.error);
        yield item.bytes;
      }
      streams.delete(streamId);
    };
    response.text = async () => {
      const parts = [];
      for await (const part of response.body()) parts.push(Buffer.from(part));
      return Buffer.concat(parts).toString("utf8");
    };
    return response;
  }

  sendBody(streamId, payload) {
    const frame = new Uint8Array(5 + payload.length);
    frame[0] = 0x01;
    new DataView(frame.buffer).setUint32(1, streamId);
    frame.set(payload, 5);
    this.ws.send(frame);
  }

  close() {
    this.ws.close();
  }
}

const json = (obj) => new TextEncoder().encode(JSON.stringify(obj));
const auth = { authorization: SESSION };

const info = await (await fetch(`${ORIGIN}/v1/server`)).json();
const SERVER_ID = process.env.SERVER_ID ?? info.server_id;
const SERVER_KEY = process.env.SERVER_KEY ?? info.public_key;
const trunk = await Trunk.connect(SERVER_ID);
check(true, `HELLO/READY at ${RELAY}/connect/${SERVER_ID}`);

const served = await trunk.request("GET", "/v1/server");
const servedInfo = JSON.parse(await served.text());
check(served.status === 200 && servedInfo.server_id === SERVER_ID, "GET /v1/server through the relay");

const nonce = `smoke-${randomBytes(12).toString("hex")}`;
const challenged = await trunk.request(
  "POST",
  "/v1/server/challenge",
  { "content-type": "application/json" },
  json({ nonce }),
);
const { signature } = JSON.parse(await challenged.text());
const key = createPublicKey({ key: { kty: "OKP", crv: "Ed25519", x: SERVER_KEY }, format: "jwk" });
check(
  challenged.status === 200 &&
    verify(null, Buffer.from(`storm-challenge:v1:${SERVER_ID}:${nonce}`), key, Buffer.from(signature, "base64url")),
  "POST /v1/server/challenge: a bodied request, answered and verified end to end",
);

const health = await Promise.all(
  Array.from({ length: 16 }, async () => {
    const r = await trunk.request("GET", "/v1/health");
    await r.text();
    return r.status;
  }),
);
check(health.every((s) => s === 200), "16 concurrent GET /v1/health on one trunk");

const vaults = JSON.parse(await (await trunk.request("GET", "/v1/vaults", auth)).text());
const vaultId = (vaults.vaults ?? vaults)[0].id;
const big = `# Smoke\n\n${"lorem ipsum dolor sit amet\n".repeat(40000)}`;
const created = await trunk.request(
  "POST",
  `/v1/vaults/${vaultId}/notes`,
  { ...auth, "content-type": "application/json" },
  json({ path: `Smoke/${Date.now()}.md`, content: big }),
);
const note = JSON.parse(await created.text());
check(created.status === 200 || created.status === 201, `POST a ${(big.length / 1024 / 1024).toFixed(1)} MiB note in ${Math.ceil(big.length / FRAME)}+ frames`);
const noteId = note.note.id;
const read = await trunk.request("GET", `/v1/vaults/${vaultId}/notes/${noteId}`, auth);
const readText = await read.text();
const readBack = JSON.parse(readText);
if (read.status !== 200) console.log(`  GET answered ${read.status}: ${readText.slice(0, 200)}`);
check(read.status === 200 && readBack.content.length >= big.length, `GET the note back through the relay, whole (${readText.length} bytes)`);

// Larger than one WebSocket frame may be, both ways: the relay's watermark
// PINGs to the origin and to this client keep its byte budgets moving.
const blob = randomBytes(20 * 1024 * 1024);
const attachment = `Smoke/${Date.now()}.bin`;
const put = await trunk.request("PUT", `/v1/vaults/${vaultId}/attachments/${attachment}`, auth, blob);
await put.text();
check(put.status >= 200 && put.status < 300, `PUT a 20 MiB attachment in ${blob.length / FRAME} frames`);
const got = await trunk.request("GET", `/v1/vaults/${vaultId}/attachments/${attachment}`, auth);
const parts = [];
for await (const part of got.body()) parts.push(Buffer.from(part));
check(got.status === 200 && Buffer.concat(parts).equals(blob), "GET it back byte for byte");

const feed = await trunk.request("GET", "/v1/stream", { ...auth, accept: "text/event-stream" });
check(feed.status === 200, "GET /v1/stream: the change feed opens");
const sawChange = (async () => {
  let buffered = "";
  for await (const part of feed.body()) {
    buffered += new TextDecoder().decode(part);
    if (buffered.includes("event: change")) return true;
  }
  return false;
})();
await new Promise((r) => setTimeout(r, 300));
const lan = await fetch(`${ORIGIN}/v1/vaults/${vaultId}/notes`, {
  method: "POST",
  headers: { ...auth, "content-type": "application/json" },
  body: JSON.stringify({ path: `Smoke/feed-${Date.now()}.md`, content: "# Pushed\n" }),
});
check(lan.ok, "a LAN edit at the origin");
const changed = await Promise.race([sawChange, new Promise((r) => setTimeout(() => r(false), 10000))]);
check(changed === true, "the change reaches the relayed feed as an SSE event");

try {
  await Trunk.connect("srv_not_registered_here");
  check(false, "an unregistered server_id is refused");
} catch (e) {
  check(e.reply?.code === "server_unreachable", "an unregistered server_id is server_unreachable");
}

trunk.close();
process.exit(0);
