// A peer can be gone by the time the relay writes to it: its socket is closing
// but its close event has not run. The relay must not throw, and must end that
// peer then and there as the close event would, telling the other side.
import { env, runInDurableObject } from "cloudflare:test";
import { describe, expect, it } from "vitest";
import type { RelayRoom } from "../../src/room";
import { bytes, client, expectOpenAndAck, openStream, register, requestHead, responseHead, uniqueId } from "../harness";

const room = (serverId: string) => env.ROOMS.get(env.ROOMS.idFromName(serverId));

function bodyFrame(kind: number, streamId: number, payload: Uint8Array): ArrayBuffer {
  const frame = new Uint8Array(5 + payload.length);
  frame[0] = kind;
  new DataView(frame.buffer).setUint32(1, streamId);
  frame.set(payload, 5);
  return frame.buffer;
}

async function stream() {
  const serverId = uniqueId();
  const { peer: server } = await register(serverId);
  const c = await client(serverId);
  const streamId = await openStream(c);
  await expectOpenAndAck(server);
  return { serverId, server, c, streamId };
}

// Closes one side inside the room, then has the other side's frame make the
// room send to it, before any close event can run.
function sendToClosed(serverId: string, closed: "s" | "c", from: "s" | "c", message: string | ArrayBuffer) {
  return runInDurableObject(room(serverId), async (instance: RelayRoom, state) => {
    const [gone] = state.getWebSockets(closed);
    const [sender] = state.getWebSockets(from);
    gone.close(1000);
    let threw: unknown = null;
    let pending: Promise<void> | undefined;
    try {
      pending = instance.webSocketMessage(sender, message);
    } catch (e) {
      threw = e;
    }
    const streams = state.storage.sql.exec<{ n: number }>("SELECT count(*) AS n FROM streams").one().n;
    const sockets = state.getWebSockets().filter((ws) => ws.readyState === WebSocket.READY_STATE_OPEN).length;
    await pending;
    return { threw: threw === null ? null : String(threw), streams, sockets };
  });
}

describe("a send to a peer that has already closed", () => {
  it("client-ward: ends the client's streams and sends the origin CLOSE", async () => {
    const { serverId, server, c, streamId } = await stream();
    server.send(responseHead(streamId));
    expect((await c.recv()).type).toBe("HTTP_RESPONSE_HEAD");

    const after = await sendToClosed(serverId, "c", "s", bodyFrame(0x02, streamId, bytes("late")));
    expect(after.threw).toBeNull();
    expect(after.streams).toBe(0);
    expect(await server.recv()).toEqual({ v: 1, type: "CLOSE", stream_id: streamId });
    expect(await server.silentFor(200)).toBe(true);
    await c.expectClosed();
  });

  it("server-ward: ends the trunk and sends each client ERROR{trunk_lost}", async () => {
    const { serverId, server, c, streamId } = await stream();

    const after = await sendToClosed(serverId, "s", "c", JSON.stringify(requestHead(streamId)));
    expect(after.threw).toBeNull();
    expect(after.streams).toBe(0);
    expect(after.sockets).toBe(0);
    expect(await c.recv()).toEqual({ v: 1, type: "ERROR", code: "trunk_lost", message: expect.any(String), stream_id: streamId });
    await c.expectClosed();
    await server.expectClosed();
  });
});
