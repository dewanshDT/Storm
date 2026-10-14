// §5 and §6 over real sockets: streams, routing, limits and trunk loss.
import { describe, expect, it } from "vitest";
import {
  bytes,
  client,
  expectError,
  expectOpenAndAck,
  openStream,
  Peer,
  register,
  requestHead,
  responseHead,
  sleep,
  text,
  uniqueId,
  uniqueIp,
} from "../harness";

async function pair() {
  const serverId = uniqueId();
  const { peer: server } = await register(serverId);
  const ip = uniqueIp();
  const peer = await client(serverId, ip);
  return { serverId, server, client: peer, ip };
}

describe("a stream", () => {
  it("carries a request and response with bodies, and relay_peer_ip from CF-Connecting-IP", async () => {
    const { server, client: c, ip } = await pair();
    const streamId = await openStream(c, "a1");
    expect(await expectOpenAndAck(server)).toBe(streamId);

    c.send(requestHead(streamId, "/v1/notes", "POST"));
    const head = await server.recv();
    expect(head.type).toBe("HTTP_REQUEST_HEAD");
    expect(head.stream_id).toBe(streamId);
    expect(head.method).toBe("POST");
    expect(head.headers).toEqual({ authorization: "Bearer inside-the-tunnel" });
    expect(head.relay_peer_ip).toBe(ip);

    c.sendBody(0x01, streamId, bytes("hello "));
    c.sendBody(0x01, streamId, bytes("world"));
    c.sendBody(0x01, streamId);
    const chunks: Uint8Array[] = [];
    for (let i = 0; i < 3; i++) {
      const f = await server.next();
      expect(f.kind).toBe("body");
      if (f.kind === "body") {
        expect(f.type).toBe(0x01);
        expect(f.streamId).toBe(streamId);
        chunks.push(f.payload);
      }
    }
    expect(text(chunks[0]) + text(chunks[1])).toBe("hello world");
    // §5.2: the empty end-of-body frame is forwarded verbatim.
    expect(chunks[2].length).toBe(0);

    server.send(responseHead(streamId, 201));
    server.sendBody(0x02, streamId, bytes("created"));
    server.send({ v: 1, type: "CLOSE", stream_id: streamId });
    const rh = await c.recv();
    expect(rh).toMatchObject({ type: "HTTP_RESPONSE_HEAD", stream_id: streamId, status: 201 });
    const body = await c.next();
    expect(body.kind === "body" && text(body.payload)).toBe("created");
    expect(await c.recv()).toEqual({ v: 1, type: "CLOSE", stream_id: streamId });
  });

  it("forwards a body chunk of 256 KiB intact and streams chunk by chunk", async () => {
    const { server, client: c } = await pair();
    const streamId = await openStream(c);
    await expectOpenAndAck(server);
    server.send(responseHead(streamId));
    await c.recv();
    const chunk = new Uint8Array(256 * 1024).map((_, i) => i & 0xff);
    for (let i = 0; i < 3; i++) {
      server.sendBody(0x02, streamId, chunk);
      const f = await c.next();
      expect(f.kind).toBe("body");
      if (f.kind === "body") expect(f.payload).toEqual(chunk);
      // Each chunk is four times this project's 64 KiB budget, so the next is
      // admitted only once the client's PONG has proved this one read.
      await sleep(30);
    }
  });

  it("never delivers one client's response to another", async () => {
    const { serverId, server, client: alice } = await pair();
    const bob = await client(serverId);
    const a = await openStream(alice, "a");
    await expectOpenAndAck(server);
    const b = await openStream(bob, "b");
    await expectOpenAndAck(server);
    expect(a).not.toBe(b);
    server.send(responseHead(a));
    server.sendBody(0x02, a, bytes("for alice"));
    expect((await alice.recv()).stream_id).toBe(a);
    expect(await bob.silentFor(200)).toBe(true);
  });

  it("does not honour a stream_id a client asserts", async () => {
    const { serverId, server, client: alice } = await pair();
    const bob = await client(serverId);
    const a = await openStream(alice);
    await expectOpenAndAck(server);
    // Bob names Alice's live stream and one that never existed: the same answer.
    bob.send(requestHead(a));
    const owned = await bob.recv();
    expectError(owned, "stream_closed");
    expect(owned.stream_id).toBe(a);
    bob.sendBody(0x01, 9999, bytes("x"));
    expectError(await bob.recv(), "stream_closed");
    expect(await server.silentFor(200)).toBe(true);
    // Bob's trunk is still usable.
    await openStream(bob);
  });

  it("refuses a client that sends relay_peer_ip, even null", async () => {
    for (const value of ["10.0.0.1", null]) {
      const { server, client: c } = await pair();
      const streamId = await openStream(c);
      await expectOpenAndAck(server);
      c.send({ ...requestHead(streamId), relay_peer_ip: value });
      expect(await c.recv()).toEqual({ v: 1, type: "ERROR", code: "protocol_error", message: "protocol error" });
      await c.expectClosed();
      const freed = await server.recv();
      expect(freed).toEqual({ v: 1, type: "CLOSE", stream_id: streamId });
    }
  });

  it("refuses a request head missing its routing or shape fields", async () => {
    for (const mutate of [
      (h: Record<string, unknown>) => delete h.stream_id,
      (h: Record<string, unknown>) => delete h.method,
      (h: Record<string, unknown>) => (h.headers = ["a"]),
      (h: Record<string, unknown>) => (h.stream_id = -1),
    ]) {
      const { server, client: c } = await pair();
      const streamId = await openStream(c);
      await expectOpenAndAck(server);
      const head = requestHead(streamId);
      mutate(head);
      c.send(head);
      expectError(await c.recv(), "protocol_error");
    }
  });

  it("fails an open the server never acknowledges with server_timeout, and tells the server", async () => {
    const { server, client: c } = await pair();
    const streamId = await openStream(c);
    expect((await server.recv()).type).toBe("STREAM_OPEN");
    const reply = await c.recv(2000);
    expectError(reply, "server_timeout");
    expect(reply.stream_id).toBe(streamId);
    expect(await server.recv()).toEqual({ v: 1, type: "CLOSE", stream_id: streamId });
  });

  it("never times out an acknowledged stream that stays silent", async () => {
    const { server, client: c } = await pair();
    const streamId = await openStream(c);
    await expectOpenAndAck(server);
    await sleep(900);
    server.send({ v: 1, type: "PING" });
    await server.recv();
    expect(await c.silentFor(100)).toBe(true);
    server.send(responseHead(streamId));
    expect((await c.recv()).type).toBe("HTTP_RESPONSE_HEAD");
  });

  it("refuses an open past the in-flight cap at once, naming the attempt, and the trunk carries on", async () => {
    const { serverId, server, client: c } = await pair();
    for (const id of ["a", "b", "c"]) await openStream(c, id);
    const attempt = { n: 4, tag: ["x"] };
    c.send({ v: 1, type: "OPEN_STREAM", attempt_id: attempt });
    const refused = await c.recv();
    expectError(refused, "rate_limited");
    expect(refused.attempt_id).toEqual(attempt);
    expect(refused.stream_id).toBeUndefined();
    // The cap is per client trunk.
    const other = await client(serverId);
    await openStream(other, "other");
    // An ack frees a slot.
    const open = await server.recv();
    server.send({ v: 1, type: "STREAM_ACK", stream_id: open.stream_id });
    await sleep(50);
    await openStream(c, "d");
  });

  it("refuses an OPEN_STREAM without an attempt_id, or with a huge one", async () => {
    for (const body of [{}, { attempt_id: "x".repeat(300) }]) {
      const { client: c } = await pair();
      c.send({ v: 1, type: "OPEN_STREAM", ...body });
      expectError(await c.recv(), "protocol_error");
    }
  });

  it("ends one stream on CLOSE{stream_id} and keeps the trunk", async () => {
    const { server, client: c } = await pair();
    const streamId = await openStream(c);
    await expectOpenAndAck(server);
    c.send({ v: 1, type: "CLOSE", stream_id: streamId });
    expect(await server.recv()).toEqual({ v: 1, type: "CLOSE", stream_id: streamId });
    c.send(requestHead(streamId));
    expectError(await c.recv(), "stream_closed");
    await openStream(c, "next");
  });

  it("delivers a server CLOSE only to the owning client", async () => {
    const { serverId, server, client: alice } = await pair();
    const bob = await client(serverId);
    const a = await openStream(alice);
    await expectOpenAndAck(server);
    server.send({ v: 1, type: "CLOSE", stream_id: a });
    expect(await alice.recv()).toEqual({ v: 1, type: "CLOSE", stream_id: a });
    expect(await bob.silentFor(150)).toBe(true);
  });

  it("turns an aborted response into stream_aborted for its client only, and the trunk carries on", async () => {
    const { serverId, server, client: alice } = await pair();
    const bob = await client(serverId);
    const a = await openStream(alice, "a");
    await expectOpenAndAck(server);
    const b = await openStream(bob, "b");
    await expectOpenAndAck(server);
    server.send(responseHead(a));
    server.sendBody(0x02, a, bytes("half a bo"));
    server.send({ v: 1, type: "CLOSE", stream_id: a, aborted: true });
    await alice.recv();
    await alice.next();
    const err = await alice.recv();
    expectError(err, "stream_aborted");
    expect(err.stream_id).toBe(a);
    server.send(responseHead(b));
    expect((await bob.recv()).stream_id).toBe(b);
    // aborted: false is a plain CLOSE.
    server.send({ v: 1, type: "CLOSE", stream_id: b, aborted: false });
    expect(await bob.recv()).toEqual({ v: 1, type: "CLOSE", stream_id: b });
  });

  it("refuses an aborted flag that is not a boolean", async () => {
    const { server, client: c } = await pair();
    const streamId = await openStream(c);
    await expectOpenAndAck(server);
    server.send({ v: 1, type: "CLOSE", stream_id: streamId, aborted: "yes" });
    expectError(await server.recv(), "protocol_error");
    await server.expectClosed();
    const lost = await c.recv();
    expectError(lost, "trunk_lost");
  });
});

// Every client of a server shares its one trunk, so nothing a client puts in a
// head may reach the origin in a form that ends it (116a).
describe("one client and the shared trunk", () => {
  // Acks the next STREAM_OPEN, past CLOSEs a refused client left behind; no
  // head may reach the origin meanwhile.
  async function ackNextOpen(server: Peer): Promise<number> {
    for (;;) {
      const frame = await server.recv();
      expect(frame.type, JSON.stringify(frame)).not.toBe("HTTP_REQUEST_HEAD");
      if (frame.type === "STREAM_OPEN") {
        server.send({ v: 1, type: "STREAM_ACK", stream_id: frame.stream_id });
        return frame.stream_id;
      }
    }
  }

  async function stillServes(server: Peer, bob: Peer) {
    const streamId = await openStream(bob, "bob");
    expect(await ackNextOpen(server)).toBe(streamId);
    bob.send(requestHead(streamId));
    const head = await server.recv();
    expect(head.type).toBe("HTTP_REQUEST_HEAD");
    expect(head.stream_id).toBe(streamId);
    server.send(responseHead(streamId));
    const reply = await bob.recv();
    expect(reply.type).toBe("HTTP_RESPONSE_HEAD");
    expect(reply.stream_id).toBe(streamId);
  }

  it("refuses a head whose header values are not strings, to its sender only", async () => {
    const serverId = uniqueId();
    const { peer: server } = await register(serverId);
    const bob = await client(serverId);
    for (const headers of [{ x: 1 }, { x: null }, { x: ["a"] }, [["x", "1"]]]) {
      const mallory = await client(serverId);
      const streamId = await openStream(mallory, "m");
      expect(await ackNextOpen(server)).toBe(streamId);
      mallory.send({ ...requestHead(streamId), headers });
      expectError(await mallory.recv(), "protocol_error");
    }
    for (const [field, value] of [
      ["method", ""],
      ["method", "GE T"],
      ["path", "v1/notes"],
      ["path", "/a b"],
    ]) {
      const mallory = await client(serverId);
      const streamId = await openStream(mallory, "m");
      expect(await ackNextOpen(server)).toBe(streamId);
      mallory.send({ ...requestHead(streamId), [field]: value });
      expectError(await mallory.recv(), "protocol_error");
    }
    await stillServes(server, bob);
  });

  it("refuses a second head with stream_closed and never forwards it", async () => {
    const serverId = uniqueId();
    const { peer: server } = await register(serverId);
    const mallory = await client(serverId);
    const bob = await client(serverId);
    const streamId = await openStream(mallory, "m");
    await expectOpenAndAck(server);
    mallory.send(requestHead(streamId));
    expect((await server.recv()).type).toBe("HTTP_REQUEST_HEAD");

    mallory.send(requestHead(streamId));
    const refused = await mallory.recv();
    expectError(refused, "stream_closed");
    expect(refused.stream_id).toBe(streamId);
    expect(await server.recv()).toEqual({ v: 1, type: "CLOSE", stream_id: streamId });

    const next = await openStream(mallory, "m2");
    expect(await ackNextOpen(server)).toBe(next);
    await stillServes(server, bob);
  });

  it("ends only the stream an origin's ERROR names, as stream_aborted", async () => {
    const serverId = uniqueId();
    const { peer: server } = await register(serverId);
    const alice = await client(serverId);
    const bob = await client(serverId);
    const streamId = await openStream(alice, "a");
    await expectOpenAndAck(server);

    server.send({ v: 1, type: "ERROR", code: "stream_closed", message: "stream closed", stream_id: streamId });
    const ended = await alice.recv();
    expectError(ended, "stream_aborted");
    expect(ended.stream_id).toBe(streamId);
    expect(await server.recv()).toEqual({ v: 1, type: "CLOSE", stream_id: streamId });
    await stillServes(server, bob);
  });

  it("still ends the trunk on a trunk-scoped origin ERROR", async () => {
    const { server, client: c } = await pair();
    const streamId = await openStream(c);
    await expectOpenAndAck(server);
    server.send({ v: 1, type: "ERROR", code: "protocol_error", message: "x" });
    const lost = await c.recv();
    expectError(lost, "trunk_lost");
    expect(lost.stream_id).toBe(streamId);
  });
});

describe("a client trunk", () => {
  it("is told server_unreachable when nothing registers within the HELLO wait", async () => {
    const serverId = uniqueId();
    const c = await Peer.dial(`/connect/${serverId}`);
    const started = Date.now();
    c.send({ v: 1, type: "HELLO", server_id: serverId });
    expectError(await c.recv(), "server_unreachable");
    expect(Date.now() - started).toBeGreaterThanOrEqual(350);
    await c.expectClosed();
  });

  it("is held until a server registers during the wait", async () => {
    const serverId = uniqueId();
    const c = await Peer.dial(`/connect/${serverId}`);
    c.send({ v: 1, type: "HELLO", server_id: serverId });
    await sleep(100);
    await register(serverId);
    expect((await c.recv()).type).toBe("READY");
  });

  it("refuses a HELLO that disagrees with the path, and a bad server_id in the path", async () => {
    const serverId = uniqueId();
    await register(serverId);
    const c = await Peer.dial(`/connect/${serverId}`);
    c.send({ v: 1, type: "HELLO", server_id: "srv_ELSEWHERE" });
    expectError(await c.recv(), "protocol_error");
    const bad = await Peer.dial("/connect/srv%3AA");
    expectError(await bad.recv(), "protocol_error");
  });

  it("refuses a client speaking the server's half", async () => {
    for (const send of [
      (p: Peer) => p.send({ v: 1, type: "STREAM_ACK", stream_id: 1 }),
      (p: Peer) => p.send({ v: 1, type: "HTTP_RESPONSE_HEAD", stream_id: 1, status: 200, headers: {} }),
      (p: Peer) => p.sendBody(0x02, 1, bytes("x")),
      (p: Peer) => p.send({ v: 1, type: "REGISTER_SERVER", server_id: "srv_A", pubkey: "x" }),
    ]) {
      const { client: c } = await pair();
      send(c);
      expect(await c.recv()).toEqual({ v: 1, type: "ERROR", code: "protocol_error", message: "protocol error" });
    }
  });

  it("names no part of the framing in a protocol_error", async () => {
    for (const raw of ["{", '{"v":2,"type":"OPEN_STREAM","attempt_id":1}', '{"v":1,"type":"NOPE"}', "[]"]) {
      const { client: c } = await pair();
      c.sendRaw(raw);
      expect(await c.recv()).toEqual({ v: 1, type: "ERROR", code: "protocol_error", message: "protocol error" });
    }
  });

  it("frees its streams on the server side when it dies", async () => {
    const { server, client: c } = await pair();
    const a = await openStream(c, "a");
    await expectOpenAndAck(server);
    const b = await openStream(c, "b");
    await expectOpenAndAck(server);
    c.close();
    const closed = [await server.recv(), await server.recv()].map((m) => m.stream_id).sort();
    expect(closed).toEqual([a, b].sort());
  });

  it("closes the trunk on CLOSE {}", async () => {
    const { client: c } = await pair();
    c.send({ v: 1, type: "CLOSE" });
    await c.expectClosed();
  });
});

describe("trunk loss", () => {
  it("gives every open stream trunk_lost, and a client with none a trunk-level one", async () => {
    const { serverId, server, client: alice } = await pair();
    const idle = await client(serverId);
    const a = await openStream(alice);
    await expectOpenAndAck(server);
    server.send(responseHead(a));
    server.sendBody(0x02, a, bytes("half"));
    await alice.recv();
    await alice.next();
    server.close();
    const lost = await alice.recv();
    expectError(lost, "trunk_lost");
    expect(lost.stream_id).toBe(a);
    await alice.expectClosed();
    const told = await idle.recv();
    expectError(told, "trunk_lost");
    expect(told.stream_id).toBeUndefined();
    await idle.expectClosed();
  });

  it("keeps a heartbeating server past the deadline", async () => {
    const { serverId, server } = await pair();
    for (let i = 0; i < 5; i++) {
      await sleep(500);
      server.send({ v: 1, type: "PING" });
      expect((await server.recv()).type).toBe("PONG");
    }
    await client(serverId);
  });

  // The attachment's copy of last-seen is written at most once a second; the
  // deadline must still count from the newest frame, or a trunk whose frames
  // are under a second apart is dropped as silent.
  it("counts the deadline from a server's newest frame, not its last written one", async () => {
    const { serverId, server } = await pair();
    for (let i = 0; i < 2; i++) {
      await sleep(900);
      server.send({ v: 1, type: "STREAM_ACK", stream_id: 999 });
    }
    expect(await server.silentFor(300)).toBe(true);
    await client(serverId);
  });

  it("drops a silent server at the deadline and tells its clients", async () => {
    const { client: c, server } = await pair();
    const streamId = await openStream(c);
    await expectOpenAndAck(server);
    const lost = await c.recv(4000);
    expectError(lost, "trunk_lost");
    expect(lost.stream_id).toBe(streamId);
    await server.expectClosed();
  });

  it("is server_unreachable for a HELLO after the server went away, and reachable again after a restart", async () => {
    const { serverId, server } = await pair();
    server.send({ v: 1, type: "DEREGISTER" });
    await server.expectClosed();
    const c = await Peer.dial(`/connect/${serverId}`);
    c.send({ v: 1, type: "HELLO", server_id: serverId });
    expectError(await c.recv(), "server_unreachable");
    const { peer: again } = await register(serverId);
    const fresh = await client(serverId);
    await openStream(fresh);
    await expectOpenAndAck(again);
  });
});

describe("a client that falls behind (§5.4, decision 106)", () => {
  it("loses the stream it could not take: client_backlogged to it, CLOSE to the origin", async () => {
    const { server, client: c } = await pair();
    const streamId = await openStream(c, "flooded");
    await expectOpenAndAck(server);
    c.stopReading();
    server.send(responseHead(streamId));
    const chunk = new Uint8Array(32 * 1024);
    for (let i = 0; i < 16; i++) server.sendBody(0x02, streamId, chunk);
    expect(await server.recv()).toEqual({ v: 1, type: "CLOSE", stream_id: streamId });
  });

  it("is told its stream failed, after what was already sent, and its other streams go on", async () => {
    const { server, client: c } = await pair();
    const flooded = await openStream(c, "flooded");
    await expectOpenAndAck(server);
    const other = await openStream(c, "other");
    await expectOpenAndAck(server);
    server.send(responseHead(other));
    server.send(responseHead(flooded));
    c.stopReading();
    const chunk = new Uint8Array(32 * 1024);
    for (let i = 0; i < 16; i++) server.sendBody(0x02, flooded, chunk);
    expect(await server.recv()).toEqual({ v: 1, type: "CLOSE", stream_id: flooded });

    let errored = false;
    while (!errored) {
      const f = await c.next();
      if (f.kind === "body") {
        expect(f.streamId).toBe(flooded);
      } else if (f.kind === "text" && f.value.type === "ERROR") {
        expectError(f.value, "client_backlogged");
        expect(f.value.stream_id).toBe(flooded);
        errored = true;
      } else if (f.kind === "text") {
        expect(f.value.type).toBe("HTTP_RESPONSE_HEAD");
      }
    }
    c.resumeReading();
    await sleep(50);
    server.sendBody(0x02, other, bytes("still here"));
    server.send({ v: 1, type: "CLOSE", stream_id: other });
    const body = await c.next();
    expect(body.kind === "body" && body.streamId === other && text(body.payload)).toBe("still here");
    expect(await c.recv()).toEqual({ v: 1, type: "CLOSE", stream_id: other });
    // Nothing more for the dropped stream ever arrives.
    server.sendBody(0x02, flooded, bytes("late"));
    expect(await c.silentFor(150)).toBe(true);
    await openStream(c, "afterwards");
    await expectOpenAndAck(server);
  });

  it("receives many times the budget while it keeps reading", async () => {
    const { server, client: c } = await pair();
    const streamId = await openStream(c);
    await expectOpenAndAck(server);
    server.send(responseHead(streamId));
    await c.recv();
    const chunk = new Uint8Array(16 * 1024).fill(7);
    let received = 0;
    for (let i = 0; i < 50; i++) {
      server.sendBody(0x02, streamId, chunk);
      const f = await c.next();
      expect(f.kind).toBe("body");
      if (f.kind === "body") received += f.payload.length;
    }
    // 800 KiB through a 64 KiB budget, and the origin was never told to stop.
    expect(received).toBe(50 * chunk.length);
    expect(c.pingsSeen).toBeGreaterThan(0);
    expect(await server.silentFor(150)).toBe(true);
  });
});

describe("an origin that falls behind", () => {
  it("fails the uploading stream with rate_limited instead of buffering without bound", async () => {
    const { server, client: c } = await pair();
    const streamId = await openStream(c);
    await expectOpenAndAck(server);
    c.send(requestHead(streamId, "/v1/upload", "POST"));
    server.stopReading();
    const chunk = new Uint8Array(64 * 1024);
    for (let i = 0; i < 12; i++) c.sendBody(0x01, streamId, chunk);
    let refused: Record<string, unknown> | null = null;
    for (;;) {
      const reply = await c.recv();
      if (reply.type === "ERROR") {
        refused = reply;
        if (reply.code === "rate_limited") break;
      }
    }
    expect(refused).toMatchObject({ code: "rate_limited", stream_id: streamId });
  });
});
