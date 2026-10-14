import { parsePublicKey, validateServerId } from "./auth";
import type { HelloLimiter } from "./limiter";
import type { RelayRoom } from "./room";

export interface Env {
  ROOMS: DurableObjectNamespace<RelayRoom>;
  HELLO_LIMITER: DurableObjectNamespace<HelloLimiter>;
  STORM_RELAY_PUBLIC_BASE?: string;
  STORM_RELAY_BINDING?: string;
  STORM_RELAY_ALLOWLIST?: string;
  STORM_RELAY_CHALLENGE_TTL_MS?: string;
  STORM_RELAY_HANDSHAKE_TIMEOUT_MS?: string;
  STORM_RELAY_HELLO_WAIT_MS?: string;
  STORM_RELAY_STREAM_ACK_TIMEOUT_MS?: string;
  STORM_RELAY_MAX_IN_FLIGHT_STREAMS?: string;
  STORM_RELAY_MAX_TOTAL_STREAMS?: string;
  STORM_RELAY_HELLO_RATE_LIMIT?: string;
  STORM_RELAY_HELLO_RATE_WINDOW_MS?: string;
  STORM_RELAY_MAX_CLIENT_BUFFER_BYTES?: string;
  STORM_RELAY_MAX_SERVER_BUFFER_BYTES?: string;
  STORM_RELAY_MAX_ROOM_BUFFER_BYTES?: string;
  STORM_RELAY_SUPERSESSION_DRAIN_MS?: string;
  STORM_RELAY_HEARTBEAT_DEADLINE_MS?: string;
}

// §4.1. `open` binds nothing and addresses each registration by
// (server_id, pubkey), one Durable Object per pair.
export type BindingMode = "allowlist" | "tofu" | "open";

export interface Config {
  publicBase: string | null;
  binding: BindingMode;
  allowlist: Map<string, Uint8Array>;
  challengeTtlMs: number;
  handshakeTimeoutMs: number;
  helloWaitMs: number;
  streamAckTimeoutMs: number;
  maxInFlightStreams: number;
  maxTotalStreams: number;
  helloRateLimit: number;
  helloRateWindowMs: number;
  maxClientBufferBytes: number;
  maxServerBufferBytes: number;
  maxRoomBufferBytes: number;
  supersessionDrainMs: number;
  heartbeatDeadlineMs: number;
}

export const MIB = 1024 * 1024;

export class ConfigError extends Error {}

function int(value: string | undefined, name: string, fallback: number): number {
  if (value === undefined || value.trim() === "") return fallback;
  const n = Number(value.trim());
  if (!Number.isSafeInteger(n) || n <= 0) throw new ConfigError(`${name} must be a positive integer`);
  return n;
}

// The same format as storm-relay's --allowlist file: `<server_id> <pubkey>` per
// line, `#` comments. A duplicate is an error, not last-wins.
export function parseAllowlist(text: string): Map<string, Uint8Array> {
  const entries = new Map<string, Uint8Array>();
  text.split(/\r?\n/).forEach((raw, index) => {
    const line = raw.split("#")[0].trim();
    if (line === "") return;
    const parts = line.split(/\s+/);
    if (parts.length !== 2) throw new ConfigError(`allowlist line ${index + 1}: expected <server_id> <pubkey>`);
    const [serverId, pubkey] = parts;
    if (!validateServerId(serverId)) throw new ConfigError(`allowlist line ${index + 1}: bad server_id`);
    const key = parsePublicKey(pubkey);
    if (!key) throw new ConfigError(`allowlist line ${index + 1}: bad pubkey`);
    if (entries.has(serverId)) throw new ConfigError(`allowlist line ${index + 1}: ${serverId} is listed twice`);
    entries.set(serverId, key);
  });
  return entries;
}

export function loadConfig(env: Env): Config {
  const binding = (env.STORM_RELAY_BINDING ?? "allowlist").trim() || "allowlist";
  if (binding !== "allowlist" && binding !== "tofu" && binding !== "open") {
    throw new ConfigError("STORM_RELAY_BINDING must be allowlist, tofu or open");
  }
  let publicBase: string | null = null;
  if (env.STORM_RELAY_PUBLIC_BASE && env.STORM_RELAY_PUBLIC_BASE.trim() !== "") {
    publicBase = env.STORM_RELAY_PUBLIC_BASE.trim().replace(/\/+$/, "");
    if (!/^wss?:\/\/[^/]+$/.test(publicBase)) {
      throw new ConfigError("STORM_RELAY_PUBLIC_BASE must be ws:// or wss:// plus a host");
    }
  }
  return {
    publicBase,
    binding,
    allowlist: binding === "allowlist" ? parseAllowlist(env.STORM_RELAY_ALLOWLIST ?? "") : new Map(),
    challengeTtlMs: int(env.STORM_RELAY_CHALLENGE_TTL_MS, "STORM_RELAY_CHALLENGE_TTL_MS", 30_000),
    handshakeTimeoutMs: int(env.STORM_RELAY_HANDSHAKE_TIMEOUT_MS, "STORM_RELAY_HANDSHAKE_TIMEOUT_MS", 30_000),
    helloWaitMs: int(env.STORM_RELAY_HELLO_WAIT_MS, "STORM_RELAY_HELLO_WAIT_MS", 5_000),
    streamAckTimeoutMs: int(env.STORM_RELAY_STREAM_ACK_TIMEOUT_MS, "STORM_RELAY_STREAM_ACK_TIMEOUT_MS", 5_000),
    maxInFlightStreams: int(env.STORM_RELAY_MAX_IN_FLIGHT_STREAMS, "STORM_RELAY_MAX_IN_FLIGHT_STREAMS", 20),
    maxTotalStreams: int(env.STORM_RELAY_MAX_TOTAL_STREAMS, "STORM_RELAY_MAX_TOTAL_STREAMS", 1000),
    helloRateLimit: int(env.STORM_RELAY_HELLO_RATE_LIMIT, "STORM_RELAY_HELLO_RATE_LIMIT", 10),
    helloRateWindowMs: int(env.STORM_RELAY_HELLO_RATE_WINDOW_MS, "STORM_RELAY_HELLO_RATE_WINDOW_MS", 60_000),
    // storm-relay's 32 MiB per peer. The room bound below is what keeps a
    // Durable Object's 128 MB isolate safe when several peers fall behind;
    // server-ward needs room for one 16 MiB frame and its end of body.
    maxClientBufferBytes: int(env.STORM_RELAY_MAX_CLIENT_BUFFER_BYTES, "STORM_RELAY_MAX_CLIENT_BUFFER_BYTES", 32 * MIB),
    maxServerBufferBytes: int(env.STORM_RELAY_MAX_SERVER_BUFFER_BYTES, "STORM_RELAY_MAX_SERVER_BUFFER_BYTES", 32 * MIB),
    maxRoomBufferBytes: int(env.STORM_RELAY_MAX_ROOM_BUFFER_BYTES, "STORM_RELAY_MAX_ROOM_BUFFER_BYTES", 64 * MIB),
    supersessionDrainMs: int(env.STORM_RELAY_SUPERSESSION_DRAIN_MS, "STORM_RELAY_SUPERSESSION_DRAIN_MS", 30_000),
    heartbeatDeadlineMs: int(env.STORM_RELAY_HEARTBEAT_DEADLINE_MS, "STORM_RELAY_HEARTBEAT_DEADLINE_MS", 45_000),
  };
}

// Parsed once per isolate per env object; a bad config fails every connection
// closed rather than serving with a guess.
const cache = new WeakMap<object, Config | ConfigError>();

export function configFor(env: Env): Config {
  let entry = cache.get(env);
  if (!entry) {
    try {
      entry = loadConfig(env);
    } catch (e) {
      entry = e instanceof ConfigError ? e : new ConfigError(String(e));
    }
    cache.set(env, entry);
  }
  if (entry instanceof ConfigError) throw entry;
  return entry;
}
