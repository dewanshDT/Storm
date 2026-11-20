import type { RateLimiter } from "./limiter";

export interface Env {
  REPORTS: KVNamespace;
  RATE_LIMITER: DurableObjectNamespace<RateLimiter>;
  SYNC_TOKEN?: string;
}
