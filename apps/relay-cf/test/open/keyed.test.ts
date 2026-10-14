// §4.1 open mode and §5 HELLO.pubkey: registrations addressed by
// (server_id, pubkey), one room per pair.
import { describe, expect, it } from "vitest";
import vectors from "../../../../docs/srp-vectors.json";
import {
  expectError,
  expectOpenAndAck,
  keyedClient,
  keyedHello,
  openStream,
  Peer,
  register,
  signingKey,
  sleep,
  uniqueId,
} from "../harness";

describe("an open relay", () => {
  it("keeps two keys of one server_id apart, each reached only by its own key", async () => {
    const serverId = uniqueId();
    const { peer: owner } = await register(serverId, 1);
    const { peer: stranger } = await register(serverId, 2);

    const toOwner = await keyedClient(serverId, 1);
    const ownerStream = await openStream(toOwner, "owner");
    expect(await expectOpenAndAck(owner)).toBe(ownerStream);
    expect(await stranger.silentFor(200)).toBe(true);

    const toStranger = await keyedClient(serverId, 2);
    const strangerStream = await openStream(toStranger, "stranger");
    expect(await expectOpenAndAck(stranger)).toBe(strangerStream);
    expect(await owner.silentFor(200)).toBe(true);
  });

  it("never hands a squatter the owner's clients", async () => {
    const serverId = uniqueId();
    await register(serverId, 2);
    const waiting = await keyedHello(serverId, 1);
    expectError(await waiting.recv(), "server_unreachable");

    const { peer: owner } = await register(serverId, 1);
    const stop = owner.keepAlive();
    try {
      await register(serverId, 2);
      const c = await keyedClient(serverId, 1);
      const streamId = await openStream(c);
      expect(await expectOpenAndAck(owner)).toBe(streamId);
    } finally {
      stop();
    }
  });

  it("supersedes only within one key", async () => {
    const serverId = uniqueId();
    const { peer: first } = await register(serverId, 1);
    const { peer: other } = await register(serverId, 2);
    const onFirst = await keyedClient(serverId, 1);
    const firstStream = await openStream(onFirst, "a1");
    await expectOpenAndAck(first);
    const onOther = await keyedClient(serverId, 2);
    const otherStream = await openStream(onOther, "a2");
    await expectOpenAndAck(other);
    // Silence below is the point; the trunks themselves must stay alive.
    const beats = [first, other, onFirst, onOther].map((p) => p.keepAlive());

    await register(serverId, 1);
    const superseded = await onFirst.recv(3000);
    expectError(superseded, "trunk_superseded");
    expect(superseded.stream_id).toBe(firstStream);

    expect(await onOther.silentFor(300)).toBe(true);
    other.send({ v: 1, type: "CLOSE", stream_id: otherStream });
    expect(await onOther.recv()).toEqual({ v: 1, type: "CLOSE", stream_id: otherStream });
    for (const stop of beats) stop();
  });

  it("refuses a keyless HELLO at once, with or without a query", async () => {
    const serverId = uniqueId();
    const { pubkey } = await signingKey(1);
    await register(serverId, 1);
    for (const path of [`/connect/${serverId}`, `/connect/${serverId}?pubkey=${pubkey}`]) {
      const started = Date.now();
      const peer = await Peer.dial(path);
      peer.send({ v: 1, type: "HELLO", server_id: serverId });
      expectError(await peer.recv(), "server_unreachable");
      // The project's HELLO hold is 400 ms.
      expect(Date.now() - started).toBeLessThan(350);
    }
  });

  it("cannot route a keyed HELLO whose URL names no key", async () => {
    const serverId = uniqueId();
    const { pubkey } = await signingKey(1);
    await register(serverId, 1);
    const peer = await Peer.dial(`/connect/${serverId}`);
    peer.send({ v: 1, type: "HELLO", server_id: serverId, pubkey });
    expectError(await peer.recv(), "server_unreachable");
  });

  it("refuses a malformed or disagreeing HELLO.pubkey with protocol_error", async () => {
    const serverId = uniqueId();
    const { pubkey } = await signingKey(1);
    await register(serverId, 1);
    for (const v of vectors.hello_pubkey.cases.filter((c) => !c.accepted)) {
      const peer = await Peer.dial(`/connect/${serverId}?pubkey=${pubkey}`);
      peer.send({ v: 1, type: "HELLO", server_id: serverId, pubkey: v.pubkey });
      expectError(await peer.recv(), "protocol_error");
    }
    const agree = vectors.hello_pubkey.query_and_hello_must_agree;
    const peer = await Peer.dial(`/connect/${serverId}?pubkey=${agree.query_pubkey}`);
    peer.send({ v: 1, type: "HELLO", server_id: serverId, pubkey: agree.hello_pubkey });
    expectError(await peer.recv(), agree.result);

    const bad = await Peer.dial(`/connect/${serverId}?pubkey=${pubkey}=`);
    expectError(await bad.recv(), "protocol_error");
  });

  it("holds a keyed HELLO for a server mid-restart under that key", async () => {
    const serverId = uniqueId();
    const waiting = await keyedHello(serverId, 1);
    await sleep(100);
    const { peer: server } = await register(serverId, 1);
    expect((await waiting.recv()).type).toBe("READY");
    const streamId = await openStream(waiting);
    expect(await expectOpenAndAck(server)).toBe(streamId);
  });
});
