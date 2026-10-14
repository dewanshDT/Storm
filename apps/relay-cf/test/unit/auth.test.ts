import { describe, expect, it } from "vitest";
import { b64urlDecode, b64urlEncode, newNonce, parsePublicKey, validateNonce } from "../../src/auth";
import { ConfigError, loadConfig, parseAllowlist } from "../../src/config";
import { parseBodyHeader, parseFrame, ProtocolError, streamIdOf } from "../../src/proto";
import devVarsExample from "../../.dev.vars.example?raw";

const KEY_1 = "iojj3XQJ8ZX9UtstPLpdcspnCb8dlBIb83SIAbQPb1w";

describe("base64url", () => {
  it("refuses padding, the standard alphabet and non-zero trailing bits", () => {
    expect(b64urlDecode("AAAA")).toEqual(new Uint8Array([0, 0, 0]));
    expect(b64urlDecode("AA==")).toBeNull();
    expect(b64urlDecode("A+/A")).toBeNull();
    expect(b64urlDecode("AB")).toBeNull();
    expect(b64urlDecode("AA")).toEqual(new Uint8Array([0]));
    expect(b64urlDecode("A")).toBeNull();
  });

  it("round-trips", () => {
    for (let n = 0; n < 40; n++) {
      const bytes = new Uint8Array(n).map((_, i) => (i * 37 + n) & 0xff);
      expect(b64urlDecode(b64urlEncode(bytes))).toEqual(bytes);
    }
  });

  it("refuses a 32-byte key that is not a curve point, as ed25519-dalek does", () => {
    // y = 2 has no x on edwards25519.
    const bytes = new Uint8Array(32);
    bytes[0] = 2;
    expect(parsePublicKey(b64urlEncode(bytes))).toBeNull();
    expect(parsePublicKey(KEY_1)).not.toBeNull();
  });
});

describe("nonces", () => {
  it("are generated valid", () => {
    for (let i = 0; i < 64; i++) {
      const nonce = newNonce();
      expect(nonce.length).toBe(32);
      expect(validateNonce(nonce)).toBe(true);
    }
  });
});

describe("frames", () => {
  it("check the version before anything else", () => {
    expect(() => parseFrame('{"type":"PING"}')).toThrow(ProtocolError);
    expect(() => parseFrame('{"v":2,"type":"REGISTER_SERVER","server_id":"srv_A","pubkey":"k"}')).toThrow(ProtocolError);
    expect(() => parseFrame('{"v":"1","type":"PING"}')).toThrow(ProtocolError);
    expect(() => parseFrame('{"v":1}')).toThrow(ProtocolError);
    expect(() => parseFrame("[1]")).toThrow(ProtocolError);
    expect(() => parseFrame("not json")).toThrow(ProtocolError);
    expect(parseFrame('{"v":1,"type":"PING"}')).toEqual({ type: "PING", body: {} });
  });

  it("route only on an in-range u32 stream_id", () => {
    expect(streamIdOf({ stream_id: 7 })).toBe(7);
    for (const bad of [undefined, null, "7", -1, 4294967296, 1.5]) {
      expect(() => streamIdOf({ stream_id: bad })).toThrow(ProtocolError);
    }
  });

  it("have exactly two body types", () => {
    const frame = (b: number[]) => new Uint8Array(b).buffer;
    expect(parseBodyHeader(frame([1, 0, 0, 0, 1]))).toEqual({ kind: 1, streamId: 1 });
    expect(parseBodyHeader(frame([2, 255, 255, 255, 255])).streamId).toBe(0xffffffff);
    expect(() => parseBodyHeader(frame([3, 0, 0, 0, 1]))).toThrow(ProtocolError);
    expect(() => parseBodyHeader(frame([1, 0, 0, 0]))).toThrow(ProtocolError);
  });
});

describe("config", () => {
  it("parses storm-relay's allowlist format", () => {
    const list = parseAllowlist(`# comment\n\n  srv_A  ${KEY_1}   # the homelab box\n`);
    expect(list.size).toBe(1);
    expect(list.has("srv_A")).toBe(true);
  });

  it("refuses a duplicate, a third field, a bad key or a bad id", () => {
    expect(() => parseAllowlist(`srv_A ${KEY_1}\nsrv_A ${KEY_1}\n`)).toThrow(/listed twice/);
    expect(() => parseAllowlist(`srv_A ${KEY_1} extra\n`)).toThrow(ConfigError);
    expect(() => parseAllowlist("srv_A notbase64!!\n")).toThrow(ConfigError);
    expect(() => parseAllowlist(`srv:A ${KEY_1}\n`)).toThrow(ConfigError);
    expect(() => parseAllowlist(`srv_A ${KEY_1}=\n`)).toThrow(ConfigError);
  });

  it("ships an example allowlist that parses", () => {
    const value = /STORM_RELAY_ALLOWLIST="([^"]*)"/.exec(devVarsExample)?.[1];
    expect(value).toBeDefined();
    expect(parseAllowlist(value!).size).toBe(1);
  });

  it("defaults to the allowlist, never to trust-on-first-use", () => {
    const config = loadConfig({} as never);
    expect(config.binding).toBe("allowlist");
    expect(config.allowlist.size).toBe(0);
    expect(config.challengeTtlMs).toBe(30_000);
    expect(config.heartbeatDeadlineMs).toBe(45_000);
    expect(config.maxClientBufferBytes).toBe(32 * 1024 * 1024);
    expect(config.maxRoomBufferBytes).toBe(64 * 1024 * 1024);
  });

  it("refuses an unknown mode, a bad number and a public base that is not ws(s)", () => {
    expect(() => loadConfig({ STORM_RELAY_BINDING: "account" } as never)).toThrow(ConfigError);
    expect(() => loadConfig({ STORM_RELAY_HELLO_WAIT_MS: "soon" } as never)).toThrow(ConfigError);
    expect(() => loadConfig({ STORM_RELAY_PUBLIC_BASE: "https://relay.example" } as never)).toThrow(ConfigError);
    expect(loadConfig({ STORM_RELAY_PUBLIC_BASE: "wss://relay.example/" } as never).publicBase).toBe("wss://relay.example");
  });

  it("takes open mode, which reads no allowlist", () => {
    const config = loadConfig({ STORM_RELAY_BINDING: "open", STORM_RELAY_ALLOWLIST: "not parsed" } as never);
    expect(config.binding).toBe("open");
    expect(config.allowlist.size).toBe(0);
  });
});
