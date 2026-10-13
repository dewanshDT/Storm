import type { Env as RelayEnv } from "../src/config";

declare global {
  namespace Cloudflare {
    interface Env extends RelayEnv {}
  }
}

export {};
