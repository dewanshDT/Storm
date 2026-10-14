// Bounds that need the allowlist project's settings: a 96 KiB room total over
// 64 KiB per client.
import { env, runInDurableObject } from "cloudflare:test";
import { describe, expect, it } from "vitest";
import type { RelayRoom } from "../../src/room";
import { client, expectError, expectOpenAndAck, openStream, register, responseHead, sleep } from "../harness";

const SERVER = "srv_LIMITS";

describe("the room's total", () => {
  // §5.4 shared bounds: never by ending a stream whose own client keeps up.
  it("sheds the client furthest behind, never one that keeps up, and takes no new streams while full", async () => {
    const { peer: server } = await register(SERVER, 3);
    const stop = server.keepAlive();
    try {
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
      expect((await third.recv()).type).toBe("HTTP_RESPONSE_HEAD");
      const body = await third.next();
      expect(body.kind, "the reader keeping up gets its body").toBe("body");

      // One slow reader's stream is what ended, and the origin was told.
      const closed = await server.recv();
      expect(closed.type).toBe("CLOSE");
      expect(slow.map((s) => s.id)).toContain(closed.stream_id);
      const shed = slow.find((s) => s.id === closed.stream_id)!;
      const frames = [];
      for (;;) {
        const f = await shed.c.next();
        if (f.kind === "text" && f.value.type === "ERROR") {
          expectError(f.value, "client_backlogged");
          expect(f.value.stream_id).toBe(shed.id);
          break;
        }
        frames.push(f);
      }

      // Still over the bound: a new stream is refused, by its attempt.
      third.send({ v: 1, type: "OPEN_STREAM", attempt_id: "d" });
      const refused = await third.recv();
      expectError(refused, "rate_limited");
      expect(refused.attempt_id).toBe("d");

      // The slow readers catch up; the room takes streams again.
      for (const { c } of slow) c.resumeReading();
      await sleep(50);
      const again = await openStream(third, "e");
      await expectOpenAndAck(server);
      server.send(responseHead(again));
      expect((await third.recv()).type).toBe("HTTP_RESPONSE_HEAD");
    } finally {
      stop();
      server.close();
    }
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
