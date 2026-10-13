// The production mode: the allowlist is the binding (§4.1), no TOFU even on
// first sight. This project's vars list srv_ALLOWED (seed 1) and srv_OTHER
// (seed 2), and set STORM_RELAY_PUBLIC_BASE and a HELLO limit of 3.
import { env, runInDurableObject, SELF } from "cloudflare:test";
import { describe, expect, it } from "vitest";
import type { RelayRoom } from "../../src/room";
import { client, expectError, Peer, register, signingKey, uniqueId, uniqueIp } from "../harness";

const room = (serverId: string) => env.ROOMS.get(env.ROOMS.idFromName(serverId));

async function refused(serverId: string, seed: number) {
  const peer = await Peer.dial("/register");
  peer.send({ v: 1, type: "REGISTER_SERVER", server_id: serverId, pubkey: (await signingKey(seed)).pubkey });
  expectError(await peer.recv(), "auth_failed");
  await peer.expectClosed();
}

const count = (serverId: string, table: string) =>
  runInDurableObject(room(serverId), (_i: RelayRoom, state) =>
    state.storage.sql.exec<{ n: number }>(`SELECT COUNT(*) AS n FROM ${table}`).one().n,
  );

describe("an allowlist", () => {
  it("registers a listed key, with the configured public base", async () => {
    const { registered, peer } = await register("srv_ALLOWED", 1);
    expect(registered.public_address).toBe("wss://relay.example/connect/srv_ALLOWED");
    peer.send({ v: 1, type: "DEREGISTER" });
    await peer.expectClosed();
  });

  it("refuses a mismatched key and an unlisted id on first sight, before any nonce", async () => {
    // The earlier registration's spent nonce stays for its TTL; none is added.
    const before = await count("srv_ALLOWED", "nonces");
    await refused("srv_ALLOWED", 2);
    expect(await count("srv_ALLOWED", "nonces")).toBe(before);
    const unlisted = uniqueId();
    await refused(unlisted, 1);
    expect(await count(unlisted, "nonces")).toBe(0);
    // Nothing was adopted: no trust-on-first-use row exists.
    expect(await count(unlisted, "bindings")).toBe(0);
    await refused(unlisted, 1);
  });

  it("still lets the listed key in after refusals", async () => {
    await refused("srv_OTHER", 1);
    const { registered } = await register("srv_OTHER", 2);
    expect(registered.type).toBe("REGISTERED");
  });
});

describe("the HELLO limit", () => {
  it("refuses a fourth trunk from one address with rate_limited, and not another address", async () => {
    const ip = uniqueIp();
    for (let i = 0; i < 3; i++) {
      const c = await Peer.dial(`/connect/${uniqueId()}`, ip);
      c.close();
    }
    const limited = await Peer.dial(`/connect/${uniqueId()}`, ip);
    expect(await limited.recv()).toEqual({ v: 1, type: "ERROR", code: "rate_limited", message: "rate limited" });
    await limited.expectClosed();
    const { peer: server } = await register("srv_ALLOWED", 1);
    await client("srv_ALLOWED", uniqueIp());
    server.close();
  });
});

describe("the edge", () => {
  it("refuses a connection with no CF-Connecting-IP", async () => {
    const response = await SELF.fetch("https://relay.test/connect/srv_ALLOWED", { headers: { upgrade: "websocket" } });
    const ws = response.webSocket!;
    ws.accept();
    const reply = await new Promise<string>((resolve) =>
      ws.addEventListener("message", (e) => resolve(e.data as string), { once: true }),
    );
    expect(JSON.parse(reply).code).toBe("protocol_error");
  });

  it("serves nothing but the two websocket paths", async () => {
    expect((await SELF.fetch("https://relay.test/")).status).toBe(404);
    expect((await SELF.fetch("https://relay.test/connect/a/b", { headers: { upgrade: "websocket" } })).status).toBe(404);
    expect((await SELF.fetch("https://relay.test/register", { method: "POST" })).status).toBe(405);
    expect((await SELF.fetch("https://relay.test/register")).status).toBe(426);
  });
});
