import { DurableObject } from "cloudflare:workers";

interface Bucket {
  tokens: number;
  last: number;
}

export interface TakeResult {
  allowed: boolean;
  retryAfterMs: number;
}

// One per install_id (`idFromName(installId)`): a token bucket of `limit`
// reports per `windowMs`, refilled continuously, as apps/relay-cf's
// HelloLimiter. Persisted so eviction never resets a limit; deleted once
// untouched for two windows, when a fresh bucket is identical.
export class RateLimiter extends DurableObject {
  async take(limit: number, windowMs: number): Promise<TakeResult> {
    const now = Date.now();
    const stored = await this.ctx.storage.get<Bucket>("bucket");
    const bucket = stored ?? { tokens: limit, last: now };
    const refill = ((now - bucket.last) * limit) / windowMs;
    bucket.tokens = Math.min(limit, bucket.tokens + Math.max(0, refill));
    bucket.last = now;
    const allowed = bucket.tokens >= 1;
    if (allowed) bucket.tokens -= 1;
    await this.ctx.storage.put("bucket", bucket);
    await this.ctx.storage.setAlarm(now + 2 * windowMs);
    const deficit = allowed ? 0 : 1 - bucket.tokens;
    return { allowed, retryAfterMs: Math.ceil((deficit * windowMs) / limit) };
  }

  async alarm(): Promise<void> {
    await this.ctx.storage.deleteAll();
  }
}
