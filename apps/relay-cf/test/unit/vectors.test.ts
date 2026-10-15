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
import {
  closeStream,
  error,
  errorOnAttempt,
  errorOnStream,
  isRequestHead,
  parseBodyHeader,
  PING,
  PONG,
} from "../../src/proto";

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

// SRP §3, §5, §6 (decision 116l): the frames this Worker sends and parses.
describe("docs/srp-vectors.json framing", () => {
  const framing = vectors.framing as unknown as {
    body_frames: { name: string; type: number; stream_id: number; frame_hex: string }[];
    too_short_frames: { frame_hex: string }[];
    control_messages: { name: string; exact_text?: string; json?: Record<string, unknown> }[];
    malformed_request_heads: { cases: { name: string; json: Record<string, unknown> }[] };
  };
  const bytes = (hex: string): ArrayBuffer => {
    const out = new Uint8Array(hex.length / 2);
    for (let i = 0; i < out.length; i++) out[i] = parseInt(hex.slice(i * 2, i * 2 + 2), 16);
    return out.buffer;
  };
  const control = (name: string) => framing.control_messages.find((c) => c.name === name)!;

  it("parses every body frame header as recorded", () => {
    for (const c of framing.body_frames) {
      expect(parseBodyHeader(bytes(c.frame_hex)), c.name).toEqual({ kind: c.type, streamId: c.stream_id });
    }
    for (const c of framing.too_short_frames) {
      expect(() => parseBodyHeader(bytes(c.frame_hex))).toThrow();
    }
  });

  it("sends the canonical PING and PONG it auto-answers (SG-38)", () => {
    expect(PING).toBe(control("canonical_ping").exact_text);
    expect(PONG).toBe(control("canonical_pong").exact_text);
  });

  it("sends the control frames the vectors record", () => {
    expect(JSON.parse(closeStream(7))).toEqual(control("close_stream").json);
    expect(JSON.parse(error("rate_limited"))).toEqual(control("error_trunk_scoped").json);
    expect(JSON.parse(errorOnAttempt("rate_limited", "a1"))).toEqual(control("error_attempt_scoped").json);
    expect(JSON.parse(errorOnStream("client_backlogged", 7))).toEqual(control("error_stream_scoped").json);
  });

  it("refuses every malformed request head", () => {
    for (const c of framing.malformed_request_heads.cases) {
      expect(isRequestHead(c.json), c.name).toBe(false);
    }
    expect(isRequestHead(control("request_head").json!)).toBe(true);
  });
});
