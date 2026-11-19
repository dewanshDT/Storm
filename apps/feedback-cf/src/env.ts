import type { RateLimiter } from "./limiter";

export interface Env {
  FEEDBACK: R2Bucket;
  RATE_LIMITER: DurableObjectNamespace<RateLimiter>;
  SYNC_TOKEN?: string;
}
