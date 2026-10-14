// docs/srp-vectors.json against this Worker's copy of the signed bytes — the
// fourth implementation the file keeps in agreement with apps/server,
// apps/relay and apps/client.
import { describe, expect, it } from "vitest";
import vectors from "../../../../docs/srp-vectors.json";
import {
  b64urlEncode,
  decodeKey,
  parsePublicKey,
  RELAY_AUTH_DOMAIN,
  relayAuthMessage,
  validateNonce,
  validateServerId,
  verifyRelayAuth,
} from "../../src/auth";

const text = (b: Uint8Array) => new TextDecoder().decode(b);

describe("docs/srp-vectors.json", () => {
  it("is the version this test understands", () => {
    expect(vectors.version).toBe(1);
  });

  it("names the domain this Worker signs and keeps it apart from the client's", () => {
    expect(vectors.domains.relay_auth).toBe(RELAY_AUTH_DOMAIN);
    expect(vectors.domains.relay_auth).not.toBe(vectors.domains.client_challenge);
    expect(text(relayAuthMessage("s", "n")).startsWith(vectors.domains.client_challenge)).toBe(false);
  });

  it("matches every relay_auth_message byte for byte", () => {
    expect(vectors.relay_auth_message.length).toBeGreaterThan(0);
    for (const v of vectors.relay_auth_message) {
      const built = relayAuthMessage(v.server_id, v.nonce);
      expect(text(built), v.name).toBe(v.message_utf8);
      expect(b64urlEncode(built), v.name).toBe(v.message_b64);
    }
  });

  it("still describes a delimiter collision, and refuses both readings", () => {
    for (const pair of vectors.delimiter_confusion.colliding_pairs) {
      const a = text(relayAuthMessage(pair.a.server_id, pair.a.nonce));
      const b = text(relayAuthMessage(pair.b.server_id, pair.b.nonce));
      expect(a).toBe(b);
      expect(a).toBe(pair.shared_message_utf8);
      expect(validateServerId(pair.a.server_id) && validateNonce(pair.a.nonce)).toBe(false);
      expect(validateServerId(pair.b.server_id) && validateNonce(pair.b.nonce)).toBe(false);
    }
  });

  it("gives every nonce the recorded verdict", () => {
    for (const v of vectors.validate_nonce) expect(validateNonce(v.nonce), v.name).toBe(v.valid);
  });

  it("gives every server_id the recorded verdict", () => {
    for (const v of vectors.validate_server_id) expect(validateServerId(v.server_id), v.name).toBe(v.valid);
  });

  it("verifies every signature exactly as recorded", async () => {
    expect(vectors.verify_relay_auth.some((v) => v.verifies)).toBe(true);
    for (const v of vectors.verify_relay_auth) {
      const key = parsePublicKey(v.public_key_b64);
      const ok = key ? await verifyRelayAuth(key, v.server_id, v.nonce, v.signature_b64) : false;
      expect(ok, v.name).toBe(v.verifies);
    }
  });

  it("accepts no client-challenge vector as a registration but the relay-domain one", async () => {
    // The client's valid signature is over the other domain, and the strict
    // decoder refuses the standard-alphabet and padded spellings.
    for (const v of vectors.verify_client_challenge) {
      const key = parsePublicKey(v.public_key_b64);
      const ok = key ? await verifyRelayAuth(key, v.server_id, v.nonce, v.signature_b64) : false;
      expect(ok, v.name).toBe(v.name === "signature_over_the_relay_auth_domain");
    }
  });

  it("parses every public key exactly as recorded", () => {
    for (const v of vectors.public_key_parsing.cases) {
      expect(parsePublicKey(v.public_key_b64) !== null, v.name).toBe(v.accepted);
    }
  });

  it("round-trips the test key to its one unpadded spelling", () => {
    const key = parsePublicKey(vectors.keys.test_public_key_b64);
    expect(key).not.toBeNull();
    expect(b64urlEncode(key!)).toBe(vectors.keys.test_public_key_b64);
  });

  it("decodes every HELLO.pubkey spelling exactly as recorded", () => {
    const section = vectors.hello_pubkey;
    expect(section.cases.some((v) => v.accepted)).toBe(true);
    for (const v of section.cases) expect(decodeKey(v.pubkey) !== null, v.name).toBe(v.accepted);
    const agree = section.query_and_hello_must_agree;
    expect(b64urlEncode(decodeKey(agree.query_pubkey)!)).not.toBe(b64urlEncode(decodeKey(agree.hello_pubkey)!));
    const url = section.connect_url;
    expect(b64urlEncode(decodeKey(url.pubkey_b64)!)).toBe(url.pubkey_b64);
    expect(`${url.relay_base}/connect/${url.server_id}?pubkey=${url.pubkey_b64}`).toBe(url.url);
  });
});
