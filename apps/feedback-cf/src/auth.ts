import type { Env } from "./env";

const BEARER_PREFIX = "Bearer ";

// As relay-cf's `sameKey`: equal-length byte XOR, so a mismatch never returns
// faster for an early-differing byte.
function timingSafeEqual(a: string, b: string): boolean {
  const ab = new TextEncoder().encode(a);
  const bb = new TextEncoder().encode(b);
  if (ab.length !== bb.length) return false;
  let diff = 0;
  for (let i = 0; i < ab.length; i++) diff |= ab[i] ^ bb[i];
  return diff === 0;
}

// GET/DELETE /v1/reports* only (decision 162d); POST takes no token.
export function authorized(request: Request, env: Env): boolean {
  if (!env.SYNC_TOKEN) return false;
  const header = request.headers.get("authorization") ?? "";
  if (!header.startsWith(BEARER_PREFIX)) return false;
  return timingSafeEqual(header.slice(BEARER_PREFIX.length), env.SYNC_TOKEN);
}
