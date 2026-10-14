// §6.2 abuse controls (decision 116k): a /register limit per source address,
// IPv6 counted per /64, and an unlisted server_id refused before any room.
import { env, runInDurableObject } from "cloudflare:test";
import { describe, expect, it } from "vitest";
import { sourceKey } from "../../src/index";
import type { RelayRoom } from "../../src/room";
import { expectError, Peer, uniqueIp } from "../harness";

describe("register attempts", () => {
  it("are rate limited per source address", async () => {
    const ip = uniqueIp();
    for (let i = 0; i < 3; i++) (await Peer.dial("/register", ip)).close();
    const refused = await Peer.dial("/register", ip);
    expectError(await refused.recv(), "rate_limited");
    // Another address is untouched.
    const other = await Peer.dial("/register", uniqueIp());
    other.send({ v: 1, type: "PING" });
    expect((await other.recv()).type).toBe("PONG");
  });

  it("count an IPv6 /64 as one source", async () => {
    const prefix = `2001:db8:${Math.floor(Math.random() * 0xffff).toString(16)}:1`;
    for (let i = 0; i < 3; i++) (await Peer.dial("/register", `${prefix}::${i + 1}`)).close();
    const refused = await Peer.dial("/register", `${prefix}:ffff::9`);
    expectError(await refused.recv(), "rate_limited");
  });
});

describe("an unlisted server_id", () => {
  // The edge refuses it before picking a room (index.ts); a test can see the
  // answer and that no challenge was issued, not the room that never ran.
  it("is refused auth_failed with no challenge issued", async () => {
    const peer = await Peer.dial("/register");
    peer.send({ v: 1, type: "REGISTER_SERVER", server_id: "srv_UNLISTED_116K", pubkey: "A".repeat(43) });
    expectError(await peer.recv(), "auth_failed");
    const stub = env.ROOMS.get(env.ROOMS.idFromName("srv_UNLISTED_116K"));
    const nonces = await runInDurableObject(stub, (_instance: RelayRoom, state) =>
      state.storage.sql.exec("SELECT COUNT(*) AS n FROM nonces").one().n,
    );
    expect(nonces).toBe(0);
  });
});

describe("sourceKey", () => {
  it("collapses IPv6 to its /64 and leaves IPv4 alone", () => {
    expect(sourceKey("203.0.113.9")).toBe("203.0.113.9");
    expect(sourceKey("::ffff:203.0.113.9")).toBe("203.0.113.9");
    expect(sourceKey("2001:db8:1:2::1")).toBe("2001:db8:1:2::/64");
    expect(sourceKey("2001:db8:1:2:aaaa:bbbb:cccc:dddd")).toBe("2001:db8:1:2::/64");
    expect(sourceKey("2001:db8::1")).toBe("2001:db8:0:0::/64");
    expect(sourceKey("::1")).toBe("0:0:0:0::/64");
  });
});
