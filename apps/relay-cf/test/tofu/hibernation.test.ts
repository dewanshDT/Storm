// The Durable Object is evicted with its sockets hibernated, so every piece of
// state has to come back from attachments and SQLite. Each test continues
// across an eviction what routing.test.ts proves without one.
import { env, evictDurableObject, runDurableObjectAlarm, runInDurableObject } from "cloudflare:test";
import { describe, expect, it } from "vitest";
import type { RelayRoom } from "../../src/room";
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
  signingKey,
  sleep,
  text,
  uniqueId,
  uniqueIp,
} from "../harness";

const room = (serverId: string) => env.ROOMS.get(env.ROOMS.idFromName(serverId));

async function evict(serverId: string) {
  const stub = room(serverId);
  const marker = await runInDurableObject(stub, (instance: RelayRoom) => {
    (instance as unknown as { marker: number }).marker = 42;
    return (instance as unknown as { marker: number }).marker;
  });
  expect(marker).toBe(42);
  await evictDurableObject(stub);
  // A new instance: nothing held in memory survived.
  const after = await runInDurableObject(stub, (instance: RelayRoom) => (instance as unknown as { marker?: number }).marker);
  expect(after).toBeUndefined();
}

describe("after an eviction", () => {
  it("continues a stream in both directions", async () => {
    const serverId = uniqueId();
    const { peer: server } = await register(serverId);
    const ip = uniqueIp();
    const c = await client(serverId, ip);
    const streamId = await openStream(c);
    await expectOpenAndAck(server);
    c.send(requestHead(streamId, "/v1/notes", "PUT"));
    expect((await server.recv()).relay_peer_ip).toBe(ip);

    await evict(serverId);

    c.sendBody(0x01, streamId, bytes("body"));
    c.sendBody(0x01, streamId);
    const chunk = await server.next();
    expect(chunk.kind === "body" && text(chunk.payload)).toBe("body");
    const end = await server.next();
    expect(end.kind === "body" && end.payload.length).toBe(0);

    server.send(responseHead(streamId));
    server.sendBody(0x02, streamId, bytes("ok"));
    server.send({ v: 1, type: "CLOSE", stream_id: streamId });
    expect((await c.recv()).type).toBe("HTTP_RESPONSE_HEAD");
    const body = await c.next();
    expect(body.kind === "body" && text(body.payload)).toBe("ok");
    expect(await c.recv()).toEqual({ v: 1, type: "CLOSE", stream_id: streamId });

    // Ids are not reissued: the next stream continues the trunk's sequence.
    const next = await openStream(c, "next");
    expect(next).toBe(streamId + 1);
    await expectOpenAndAck(server);
  });

  it("still refuses a stream_id the client was not given", async () => {
    const serverId = uniqueId();
    const { peer: server } = await register(serverId);
    const alice = await client(serverId);
    const bob = await client(serverId);
    const a = await openStream(alice);
    await expectOpenAndAck(server);
    await evict(serverId);
    bob.send(requestHead(a));
    expectError(await bob.recv(), "stream_closed");
    expect(await server.silentFor(150)).toBe(true);
  });

  it("still refuses a second head", async () => {
    const serverId = uniqueId();
    const { peer: server } = await register(serverId);
    const c = await client(serverId);
    const streamId = await openStream(c);
    await expectOpenAndAck(server);
    c.send(requestHead(streamId));
    expect((await server.recv()).type).toBe("HTTP_REQUEST_HEAD");
    await evict(serverId);
    c.send(requestHead(streamId));
    expectError(await c.recv(), "stream_closed");
    expect(await server.recv()).toEqual({ v: 1, type: "CLOSE", stream_id: streamId });
  });

  it("adds the head column to a streams table from before 116a", async () => {
    const serverId = uniqueId();
    const { peer: server } = await register(serverId);
    const c = await client(serverId);
    await runInDurableObject(room(serverId), (instance: RelayRoom) => {
      (instance as unknown as { ctx: DurableObjectState }).ctx.storage.sql.exec("ALTER TABLE streams DROP COLUMN head");
    });
    await evict(serverId);
    const streamId = await openStream(c);
    await expectOpenAndAck(server);
    c.send(requestHead(streamId));
    expect((await server.recv()).type).toBe("HTTP_REQUEST_HEAD");
  });

  it("times out an unacknowledged open from the alarm", async () => {
    const serverId = uniqueId();
    const { peer: server } = await register(serverId);
    const c = await client(serverId);
    const streamId = await openStream(c);
    expect((await server.recv()).type).toBe("STREAM_OPEN");
    await evict(serverId);
    const reply = await c.recv(3000);
    expectError(reply, "server_timeout");
    expect(reply.stream_id).toBe(streamId);
  });

  it("holds a HELLO and answers it when the server registers", async () => {
    const serverId = uniqueId();
    const c = await Peer.dial(`/connect/${serverId}`);
    c.send({ v: 1, type: "HELLO", server_id: serverId });
    await sleep(50);
    await evict(serverId);
    await register(serverId);
    expect((await c.recv()).type).toBe("READY");
  });

  it("keeps a server whose PINGs the runtime answered while the object slept", async () => {
    const serverId = uniqueId();
    const { peer: server } = await register(serverId);
    await evict(serverId);
    for (let i = 0; i < 5; i++) {
      await sleep(500);
      server.send({ v: 1, type: "PING" });
      expect((await server.recv()).type).toBe("PONG");
    }
    // Past the 1.5 s deadline from registration, and still routed to.
    const c = await client(serverId);
    await openStream(c);
    await expectOpenAndAck(server);
  });

  it("still drops a silent server, with nothing in memory to remember it", async () => {
    const serverId = uniqueId();
    const { peer: server } = await register(serverId);
    const c = await client(serverId);
    await evict(serverId);
    // Only the server falls silent, or the client could be reaped first.
    const stop = c.keepAlive();
    try {
      expectError(await c.recv(4000), "trunk_lost");
      await server.expectClosed();
    } finally {
      stop();
    }
  });

  it("still finishes a supersession drain", async () => {
    const serverId = uniqueId();
    const { peer: old } = await register(serverId);
    const c = await client(serverId);
    const streamId = await openStream(c);
    await expectOpenAndAck(old);
    await register(serverId);
    await evict(serverId);
    const ended = await c.recv(3000);
    expectError(ended, "trunk_superseded");
    expect(ended.stream_id).toBe(streamId);
  });

  it("remembers what a client has not acknowledged reading", async () => {
    const serverId = uniqueId();
    const { peer: server } = await register(serverId);
    const c = await client(serverId);
    const streamId = await openStream(c);
    await expectOpenAndAck(server);
    c.stopReading();
    server.send(responseHead(streamId));
    server.sendBody(0x02, streamId, new Uint8Array(40 * 1024));
    await sleep(50);
    await evict(serverId);
    server.sendBody(0x02, streamId, new Uint8Array(40 * 1024));
    expect(await server.recv()).toEqual({ v: 1, type: "CLOSE", stream_id: streamId });
  });

  it("keeps a TOFU binding", async () => {
    const serverId = uniqueId();
    await register(serverId, 1);
    await evictDurableObject(room(serverId), { webSockets: "close" });
    const squatter = await Peer.dial("/register");
    squatter.send({ v: 1, type: "REGISTER_SERVER", server_id: serverId, pubkey: (await signingKey(2)).pubkey });
    expectError(await squatter.recv(), "auth_failed");
  });

  it("drops streams whose sockets did not survive a restart", async () => {
    const serverId = uniqueId();
    const { peer: server } = await register(serverId);
    const c = await client(serverId);
    await openStream(c);
    await expectOpenAndAck(server);
    await evictDurableObject(room(serverId), { webSockets: "close" });
    const rows = await runInDurableObject(room(serverId), (_i: RelayRoom, state) =>
      state.storage.sql.exec<{ n: number }>("SELECT COUNT(*) AS n FROM streams").one().n,
    );
    expect(rows).toBe(0);
    expect(await runDurableObjectAlarm(room(serverId))).toBeTypeOf("boolean");
  });
});
