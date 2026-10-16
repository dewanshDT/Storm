// §4 over real sockets, with trust-on-first-use (the parity mode).
import { env, runInDurableObject } from "cloudflare:test";
import { describe, expect, it } from "vitest";
import type { RelayRoom } from "../../src/room";
import { client, expectError, expectOpenAndAck, Peer, register, relayAuth, signingKey, uniqueId } from "../harness";

async function challenged(serverId: string, seed = 1) {
  const key = await signingKey(seed);
  const peer = await Peer.dial("/register");
  peer.send({ v: 1, type: "REGISTER_SERVER", server_id: serverId, pubkey: key.pubkey });
  const challenge = await peer.recv();
  expect(challenge.type).toBe("CHALLENGE");
  return { peer, key, nonce: challenge.nonce as string };
}

const room = (serverId: string) => env.ROOMS.get(env.ROOMS.idFromName(serverId));

describe("registration", () => {
  it("registers and returns a derived public_address", async () => {
    const serverId = uniqueId();
    const { registered } = await register(serverId);
    expect(registered.v).toBe(1);
    expect(registered.trunk_id).toMatch(/^trk_/);
    expect(registered.public_address).toBe(`wss://relay.test/connect/${serverId}`);
    // Three heartbeats to the test config's 3 s deadline; 15 s at 45 s.
    expect(registered.heartbeat_interval_secs).toBe(1);
  });

  it("answers PING and frees the server_id on DEREGISTER", async () => {
    const serverId = uniqueId();
    const { peer } = await register(serverId);
    peer.send({ v: 1, type: "PING" });
    expect((await peer.recv()).type).toBe("PONG");
    // A spelling the runtime's auto-response does not match is answered too.
    peer.sendRaw('{"type":"PING","v":1}');
    expect((await peer.recv()).type).toBe("PONG");
    peer.send({ v: 1, type: "DEREGISTER" });
    await peer.expectClosed();
    const late = await Peer.dial(`/connect/${serverId}`);
    late.send({ v: 1, type: "HELLO", server_id: serverId });
    expectError(await late.recv(), "server_unreachable");
  });

  // Decision 108c: a PING may arrive at any point on a trunk. Both spellings,
  // since the auto-response matches only the canonical bytes.
  it("answers a PING before REGISTER_SERVER and mid-handshake, and registers", async () => {
    const serverId = uniqueId();
    const key = await signingKey(1);
    const peer = await Peer.dial("/register");
    peer.send({ v: 1, type: "PING" });
    expect((await peer.recv()).type).toBe("PONG");
    peer.sendRaw('{"type":"PING","v":1}');
    expect((await peer.recv()).type).toBe("PONG");
    peer.send({ v: 1, type: "REGISTER_SERVER", server_id: serverId, pubkey: key.pubkey });
    const challenge = await peer.recv();
    expect(challenge.type).toBe("CHALLENGE");
    peer.sendRaw('{"type":"PING","v":1}');
    expect((await peer.recv()).type).toBe("PONG");
    peer.send({ v: 1, type: "CHALLENGE_RESPONSE", sig: await key.sign(relayAuth(serverId, challenge.nonce as string)) });
    expect((await peer.recv()).type).toBe("REGISTERED");
  });

  it("answers a client's PING before HELLO and while HELLO waits", async () => {
    const serverId = uniqueId();
    // The server's dial, challenge and signature happen before the HELLO, so
    // only one frame has to land inside the HELLO hold on a slow runner.
    const server = await challenged(serverId);
    const sig = await server.key.sign(relayAuth(serverId, server.nonce));
    const peer = await Peer.dial(`/connect/${serverId}`);
    peer.sendRaw('{"type":"PING","v":1}');
    expect((await peer.recv()).type).toBe("PONG");
    peer.send({ v: 1, type: "HELLO", server_id: serverId });
    peer.sendRaw('{"type":"PING","v":1}');
    expect((await peer.recv()).type).toBe("PONG");
    server.peer.send({ v: 1, type: "CHALLENGE_RESPONSE", sig });
    expect((await server.peer.recv()).type).toBe("REGISTERED");
    expect((await peer.recv()).type).toBe("READY");
  });

  it("refuses a signature over the client-facing domain", async () => {
    const serverId = uniqueId();
    const { peer, key, nonce } = await challenged(serverId);
    peer.send({ v: 1, type: "CHALLENGE_RESPONSE", sig: await key.sign(`storm-challenge:v1:${serverId}:${nonce}`) });
    expectError(await peer.recv(), "auth_failed");
    await peer.expectClosed();
  });

  it("refuses a signature over the prefix and nonce alone", async () => {
    const serverId = uniqueId();
    const { peer, key, nonce } = await challenged(serverId);
    peer.send({ v: 1, type: "CHALLENGE_RESPONSE", sig: await key.sign(`storm-relay-auth:v1:${nonce}`) });
    expectError(await peer.recv(), "auth_failed");
  });

  it("refuses a signature for a different server_id, and by the wrong key", async () => {
    const serverId = uniqueId();
    const a = await challenged(serverId);
    a.peer.send({ v: 1, type: "CHALLENGE_RESPONSE", sig: await a.key.sign(relayAuth("srv_OTHER", a.nonce)) });
    expectError(await a.peer.recv(), "auth_failed");

    const b = await challenged(uniqueId());
    const wrong = await signingKey(9);
    b.peer.send({ v: 1, type: "CHALLENGE_RESPONSE", sig: await wrong.sign(relayAuth(serverId, b.nonce)) });
    expectError(await b.peer.recv(), "auth_failed");
  });

  it("never re-arms a spent nonce, and refuses an expired one", async () => {
    const serverId = uniqueId();
    const first = await challenged(serverId);
    // Spend it, then issue the same value again: INSERT OR IGNORE keeps it spent.
    await runInDurableObject(room(serverId), (_instance: RelayRoom, state) => {
      state.storage.sql.exec("UPDATE nonces SET spent = 1 WHERE nonce = ?", first.nonce);
      state.storage.sql.exec("INSERT OR IGNORE INTO nonces (nonce, expires, spent) VALUES (?, ?, 0)", first.nonce, Date.now() + 30_000);
    });
    first.peer.send({ v: 1, type: "CHALLENGE_RESPONSE", sig: await first.key.sign(relayAuth(serverId, first.nonce)) });
    expectError(await first.peer.recv(), "auth_failed");

    const second = await challenged(serverId);
    await runInDurableObject(room(serverId), (_instance: RelayRoom, state) => {
      state.storage.sql.exec("UPDATE nonces SET expires = ? WHERE nonce = ?", Date.now() - 1, second.nonce);
    });
    second.peer.send({ v: 1, type: "CHALLENGE_RESPONSE", sig: await second.key.sign(relayAuth(serverId, second.nonce)) });
    expectError(await second.peer.recv(), "auth_failed");
  });

  it("does not let a captured signature register a fresh connection", async () => {
    const serverId = uniqueId();
    const first = await challenged(serverId);
    const captured = await first.key.sign(relayAuth(serverId, first.nonce));
    first.peer.send({ v: 1, type: "CHALLENGE_RESPONSE", sig: captured });
    expect((await first.peer.recv()).type).toBe("REGISTERED");

    const replay = await challenged(serverId);
    expect(replay.nonce).not.toBe(first.nonce);
    replay.peer.send({ v: 1, type: "CHALLENGE_RESPONSE", sig: captured });
    expectError(await replay.peer.recv(), "auth_failed");
  });

  it("binds the first key and refuses a different one for ever, without spending a nonce", async () => {
    const serverId = uniqueId();
    const { peer } = await register(serverId, 1);
    peer.send({ v: 1, type: "DEREGISTER" });
    await peer.expectClosed();

    const nonces = () =>
      runInDurableObject(room(serverId), (_i: RelayRoom, state) =>
        state.storage.sql.exec<{ n: number }>("SELECT COUNT(*) AS n FROM nonces").one().n,
      );
    const before = await nonces();
    const other = await signingKey(2);
    const squatter = await Peer.dial("/register");
    squatter.send({ v: 1, type: "REGISTER_SERVER", server_id: serverId, pubkey: other.pubkey });
    expectError(await squatter.recv(), "auth_failed");
    await squatter.expectClosed();
    // Binding before challenge (§4.1): the refusal issued no nonce.
    expect(await nonces()).toBe(before);

    const again = await register(serverId, 1);
    expect(again.registered.type).toBe("REGISTERED");
  });

  it("records the binding only after a successful signature", async () => {
    const serverId = uniqueId();
    const attacker = await challenged(serverId, 2);
    attacker.peer.send({ v: 1, type: "CHALLENGE_RESPONSE", sig: "AAAA" });
    expectError(await attacker.peer.recv(), "auth_failed");
    // Not squatted: a different key may still claim it.
    expect((await register(serverId, 1)).registered.type).toBe("REGISTERED");
  });

  it("binds per server_id", async () => {
    expect((await register(uniqueId(), 1)).registered.type).toBe("REGISTERED");
    expect((await register(uniqueId(), 2)).registered.type).toBe("REGISTERED");
  });

  it("makes every authentication failure look the same from outside", async () => {
    const replies: string[] = [];
    const bad = await challenged(uniqueId());
    bad.peer.send({ v: 1, type: "CHALLENGE_RESPONSE", sig: "AAAA" });
    replies.push(JSON.stringify(await bad.peer.recv()));

    const serverId = uniqueId();
    await register(serverId, 1);
    const rebind = await Peer.dial("/register");
    rebind.send({ v: 1, type: "REGISTER_SERVER", server_id: serverId, pubkey: (await signingKey(2)).pubkey });
    replies.push(JSON.stringify(await rebind.recv()));

    const expiredId = uniqueId();
    const expired = await challenged(expiredId);
    await runInDurableObject(room(expiredId), (_i: RelayRoom, state) => {
      state.storage.sql.exec("UPDATE nonces SET expires = ?", Date.now() - 1);
    });
    expired.peer.send({ v: 1, type: "CHALLENGE_RESPONSE", sig: await expired.key.sign(relayAuth(expiredId, expired.nonce)) });
    replies.push(JSON.stringify(await expired.peer.recv()));

    expect(replies).toHaveLength(3);
    expect(new Set(replies).size).toBe(1);
    expect(JSON.parse(replies[0])).toEqual({ v: 1, type: "ERROR", code: "auth_failed", message: "authentication failed" });
  });

  it("refuses a wrong or missing version, malformed frames and wrong first frames with a bare protocol_error", async () => {
    const cases: (string | Uint8Array)[] = [
      '{"type":"REGISTER_SERVER","server_id":"srv_A","pubkey":"x"}',
      '{"v":2,"type":"REGISTER_SERVER","server_id":"srv_A","pubkey":"x"}',
      "not json",
      '{"v":1,"type":"HELLO","server_id":"srv_A"}',
      '{"v":1,"type":"REGISTER_SERVER","server_id":"srv:A","pubkey":"x"}',
      '{"v":1,"type":"REGISTER_SERVER","server_id":"srv_A"}',
      new Uint8Array([1, 0, 0, 0, 1]),
    ];
    for (const frame of cases) {
      const peer = await Peer.dial("/register");
      peer.sendRaw(typeof frame === "string" ? frame : frame.buffer as ArrayBuffer);
      expect(await peer.recv()).toEqual({ v: 1, type: "ERROR", code: "protocol_error", message: "protocol error" });
      await peer.expectClosed();
    }
  });

  it("refuses a padded or standard-alphabet pubkey as protocol_error", async () => {
    const key = await signingKey(1);
    for (const pubkey of [`${key.pubkey}=`, key.pubkey.replace(/-/g, "+").replace(/_/g, "/") + "+"]) {
      const peer = await Peer.dial("/register");
      peer.send({ v: 1, type: "REGISTER_SERVER", server_id: uniqueId(), pubkey });
      expectError(await peer.recv(), "protocol_error");
    }
  });

  it("refuses a second REGISTER_SERVER and a body frame mid-handshake", async () => {
    const a = await challenged(uniqueId());
    a.peer.send({ v: 1, type: "REGISTER_SERVER", server_id: uniqueId(), pubkey: a.key.pubkey });
    expectError(await a.peer.recv(), "protocol_error");
    await a.peer.expectClosed();

    const b = await challenged(uniqueId());
    b.peer.sendBody(1, 1, new Uint8Array([1]));
    expectError(await b.peer.recv(), "protocol_error");
  });

  it("closes a handshake that never completes", async () => {
    const peer = await Peer.dial("/register");
    await peer.expectClosed(6000);
    const { peer: half } = await challenged(uniqueId());
    await half.expectClosed(6000);
  });

  it("supersedes: the new trunk takes new clients, the old drains then ends trunk_superseded", async () => {
    const serverId = uniqueId();
    const old = await register(serverId);
    const early = await client(serverId);
    early.send({ v: 1, type: "OPEN_STREAM", attempt_id: "x" });
    const ready = await early.recv();
    const open = await old.peer.recv();
    old.peer.send({ v: 1, type: "STREAM_ACK", stream_id: open.stream_id });

    const fresh = await register(serverId);
    expect(fresh.registered.trunk_id).not.toBe(old.registered.trunk_id);
    // During the drain the old trunk still answers its own streams.
    old.peer.send({ v: 1, type: "HTTP_RESPONSE_HEAD", stream_id: ready.stream_id, status: 200, headers: {} });
    expect((await early.recv()).type).toBe("HTTP_RESPONSE_HEAD");

    const late = await client(serverId);
    late.send({ v: 1, type: "OPEN_STREAM", attempt_id: "y" });
    await late.recv();
    await expectOpenAndAck(fresh.peer);

    const ended = await early.recv(3000);
    expectError(ended, "trunk_superseded");
    expect(ended.stream_id).toBe(ready.stream_id);
    await early.expectClosed();
    await old.peer.expectClosed();
    // The replacement is untouched.
    fresh.peer.send({ v: 1, type: "PING" });
    expect((await fresh.peer.recv()).type).toBe("PONG");
  });
});
