import { DurableObject } from "cloudflare:workers";

interface Bucket {
  tokens: number;
  last: number;
}

// One per source address (`idFromName("ip:" + CF-Connecting-IP)`): a token
// bucket of `limit` HELLOs per `windowMs`, refilled continuously, as
// storm-relay's rate_limit.rs. Persisted so eviction never resets a limit;
// deleted once untouched for two windows, when a fresh bucket is identical.
export class HelloLimiter extends DurableObject {
  async take(limit: number, windowMs: number): Promise<boolean> {
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
    return allowed;
  }

  async alarm(): Promise<void> {
    await this.ctx.storage.deleteAll();
  }
}
