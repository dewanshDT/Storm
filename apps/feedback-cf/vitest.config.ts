import { cloudflareTest } from "@cloudflare/vitest-plugin";
import { defineConfig } from "vitest/config";

export default defineConfig({
  plugins: [
    cloudflareTest({
      wrangler: { configPath: "./wrangler.jsonc" },
      miniflare: { bindings: { SYNC_TOKEN: "test-sync-token" } },
    }),
  ],
  test: {
    testTimeout: 20_000,
    silent: "passed-only",
    // The runner inside workerd cannot read Vitest 5's on-disk module cache.
    fsModuleCache: false,
  },
});
