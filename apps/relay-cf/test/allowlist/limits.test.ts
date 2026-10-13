// Bounds that need the allowlist project's settings: a 96 KiB room total over
// 64 KiB per client.
import { env, runInDurableObject } from "cloudflare:test";
import { describe, expect, it } from "vitest";
import type { RelayRoom } from "../../src/room";
import { client, expectError, expectOpenAndAck, openStream, register, responseHead, sleep } from "../harness";

const SERVER = "srv_LIMITS";

describe("the room's total", () => {
  it("fails a stream once the room holds too much unread, though its own client is idle", async () => {
    const { peer: server } = await register(SERVER, 3);
    const chunk = new Uint8Array(40 * 1024);
    const slow = [];
    for (const name of ["a", "b"]) {
      const c = await client(SERVER);
      const id = await openStream(c, name);
      await expectOpenAndAck(server);
      c.stopReading();
      server.send(responseHead(id));
      server.sendBody(0x02, id, chunk);
      slow.push({ c, id });
    }
    await sleep(50);

    const third = await client(SERVER);
    const id = await openStream(third, "c");
    await expectOpenAndAck(server);
    server.send(responseHead(id));
    server.sendBody(0x02, id, chunk);
    expect(await server.recv()).toEqual({ v: 1, type: "CLOSE", stream_id: id });
    const head = await third.recv();
    expect(head.type).toBe("HTTP_RESPONSE_HEAD");
    const refused = await third.recv();
    expectError(refused, "client_backlogged");
    expect(refused.stream_id).toBe(id);

    // The slow readers catch up; the room has room again.
    for (const { c } of slow) c.resumeReading();
    await sleep(50);
    const again = await openStream(third, "d");
    await expectOpenAndAck(server);
    server.send(responseHead(again));
    server.sendBody(0x02, again, chunk);
    expect((await third.recv()).type).toBe("HTTP_RESPONSE_HEAD");
    const body = await third.next();
    expect(body.kind).toBe("body");
    server.close();
  });
});

describe("the heartbeat deadline", () => {
  it("stops a trunk taking new streams even before its alarm has run", async () => {
    const { peer: server } = await register(SERVER, 3);
    const c = await client(SERVER);
    await runInDurableObject(env.ROOMS.get(env.ROOMS.idFromName(SERVER)), (_i: RelayRoom, state) =>
      state.storage.deleteAlarm(),
    );
    await sleep(1700);
    c.send({ v: 1, type: "OPEN_STREAM", attempt_id: "late" });
    expectError(await c.recv(), "trunk_lost");
    await c.expectClosed();
    await server.expectClosed();
  });
});
