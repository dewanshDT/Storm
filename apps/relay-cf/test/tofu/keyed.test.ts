// §5 HELLO.pubkey on a relay with one key per server_id (tofu, as allowlist).
import { describe, expect, it } from "vitest";
import { client, expectError, expectOpenAndAck, keyedClient, keyedHello, openStream, register, uniqueId } from "../harness";

describe("a keyed HELLO on a tofu relay", () => {
  it("reaches the registration only when its key matches", async () => {
    const serverId = uniqueId();
    const { peer: server } = await register(serverId, 1);

    const wrong = await keyedHello(serverId, 2);
    expectError(await wrong.recv(), "server_unreachable");

    const right = await keyedClient(serverId, 1);
    const streamId = await openStream(right, "keyed");
    expect(await expectOpenAndAck(server)).toBe(streamId);

    // A client older than 115 still reaches the one registration.
    const old = await client(serverId);
    const oldStream = await openStream(old, "keyless");
    expect(await expectOpenAndAck(server)).toBe(oldStream);
  });
});
