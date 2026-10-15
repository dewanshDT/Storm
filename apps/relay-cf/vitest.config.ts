import { cloudflareTest } from "@cloudflare/vitest-plugin";
import { defineConfig } from "vitest/config";

// Shortened timers so the suite proves each deadline in milliseconds.
const timers = {
  STORM_RELAY_CHALLENGE_TTL_MS: "30000",
  STORM_RELAY_HANDSHAKE_TIMEOUT_MS: "3000",
  STORM_RELAY_HELLO_WAIT_MS: "400",
  STORM_RELAY_STREAM_ACK_TIMEOUT_MS: "400",
  STORM_RELAY_SUPERSESSION_DRAIN_MS: "500",
  STORM_RELAY_HEARTBEAT_DEADLINE_MS: "1500",
  STORM_RELAY_MAX_IN_FLIGHT_STREAMS: "3",
  STORM_RELAY_MAX_TOTAL_STREAMS: "50",
  STORM_RELAY_HELLO_RATE_LIMIT: "100",
  STORM_RELAY_HELLO_RATE_WINDOW_MS: "60000",
  STORM_RELAY_MAX_CLIENT_BUFFER_BYTES: String(64 * 1024),
  STORM_RELAY_MAX_SERVER_BUFFER_BYTES: String(256 * 1024),
};

// Test keys: Ed25519 from the seeds [1; 32], [2; 32] and [3; 32].
const KEY_1 = "iojj3XQJ8ZX9UtstPLpdcspnCb8dlBIb83SIAbQPb1w";
const KEY_2 = "gTl3Dqh9F19Wo1Rmw0x-zMuNipG07jeiXfYPW4_Js5Q";
const KEY_3 = "7UkoxijRwsbq6QM4kFmVYSlZJzpcY_k2NsFGFKyHN9E";

function project(name: string, include: string[], bindings: Record<string, string>) {
  return {
    extends: true as const,
    plugins: [
      cloudflareTest({
        wrangler: { configPath: "./wrangler.jsonc" },
        miniflare: { compatibilityFlags: ["nodejs_compat"], bindings: { ...timers, ...bindings } },
      }),
    ],
    test: { name, include },
  };
}

export default defineConfig({
  test: {
    testTimeout: 20_000,
    silent: "passed-only",
    // The runner inside workerd cannot read Vitest 5's on-disk module cache.
    fsModuleCache: false,
    projects: [
      project("unit", ["test/unit/**/*.test.ts"], {}),
      project("tofu", ["test/tofu/**/*.test.ts"], { STORM_RELAY_BINDING: "tofu" }),
      project("open", ["test/open/**/*.test.ts"], { STORM_RELAY_BINDING: "open" }),
      project("allowlist", ["test/allowlist/**/*.test.ts"], {
        STORM_RELAY_BINDING: "allowlist",
        STORM_RELAY_PUBLIC_BASE: "wss://relay.example/",
        STORM_RELAY_ALLOWLIST: `# test allowlist\nsrv_ALLOWED ${KEY_1}  # seed 1\n\nsrv_OTHER ${KEY_2}\nsrv_LIMITS ${KEY_3}\n`,
        STORM_RELAY_HELLO_RATE_LIMIT: "3",
        STORM_RELAY_REGISTER_RATE_LIMIT: "3",
        STORM_RELAY_MAX_ROOM_BUFFER_BYTES: String(96 * 1024),
      }),
    ],
  },
});
