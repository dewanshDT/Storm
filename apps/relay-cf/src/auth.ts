// The signed bytes of SRP v1 registration and the rules on each field.
// Re-derived from apps/server and apps/relay; docs/srp-vectors.json is what
// keeps the five implementations in agreement (test/vectors.test.ts).

export const RELAY_AUTH_DOMAIN = "storm-relay-auth:v1:";

export function relayAuthMessage(serverId: string, nonce: string): Uint8Array {
  return new TextEncoder().encode(`${RELAY_AUTH_DOMAIN}${serverId}:${nonce}`);
}

export function validateNonce(nonce: string): boolean {
  if (nonce.length < 16 || nonce.length > 128) return false;
  for (let i = 0; i < nonce.length; i++) {
    const c = nonce.charCodeAt(i);
    if (c < 0x21 || c > 0x7e || c === 0x3a || c === 0x22) return false;
  }
  return true;
}

export function validateServerId(serverId: string): boolean {
  if (serverId.length < 1 || serverId.length > 128) return false;
  return /^[A-Za-z0-9_-]+$/.test(serverId);
}

const ALPHABET = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
const DECODE = new Int16Array(128).fill(-1);
for (let i = 0; i < ALPHABET.length; i++) DECODE[ALPHABET.charCodeAt(i)] = i;

// Strict: the url-safe alphabet only, no padding, and zero trailing bits, as
// data-encoding's BASE64URL_NOPAD. One key has one spelling (decision 61).
export function b64urlDecode(text: string): Uint8Array | null {
  if (text.length % 4 === 1) return null;
  const out = new Uint8Array(Math.floor((text.length * 3) / 4));
  let acc = 0;
  let bits = 0;
  let o = 0;
  for (let i = 0; i < text.length; i++) {
    const c = text.charCodeAt(i);
    const v = c < 128 ? DECODE[c] : -1;
    if (v < 0) return null;
    acc = (acc << 6) | v;
    bits += 6;
    if (bits >= 8) {
      bits -= 8;
      out[o++] = (acc >> bits) & 0xff;
    }
  }
  if (bits > 0 && (acc & ((1 << bits) - 1)) !== 0) return null;
  return out;
}

export function b64urlEncode(bytes: Uint8Array): string {
  let out = "";
  let i = 0;
  for (; i + 2 < bytes.length; i += 3) {
    const n = (bytes[i] << 16) | (bytes[i + 1] << 8) | bytes[i + 2];
    out += ALPHABET[n >> 18] + ALPHABET[(n >> 12) & 63] + ALPHABET[(n >> 6) & 63] + ALPHABET[n & 63];
  }
  if (i < bytes.length) {
    const n = (bytes[i] << 16) | ((bytes[i + 1] ?? 0) << 8);
    out += ALPHABET[n >> 18] + ALPHABET[(n >> 12) & 63];
    if (i + 1 < bytes.length) out += ALPHABET[(n >> 6) & 63];
  }
  return out;
}

const P = (1n << 255n) - 19n;
const D = (-121665n * modPow(121666n, P - 2n)) % P;

function mod(a: bigint): bigint {
  const r = a % P;
  return r < 0n ? r + P : r;
}

function modPow(base: bigint, exp: bigint): bigint {
  let result = 1n;
  let b = mod(base);
  let e = exp;
  while (e > 0n) {
    if (e & 1n) result = (result * b) % P;
    b = (b * b) % P;
    e >>= 1n;
  }
  return result;
}

// ed25519-dalek's VerifyingKey::from_bytes refuses an encoding that is not a
// curve point; WebCrypto would accept it at import and fail only at verify,
// turning storm-relay's protocol_error into auth_failed.
function isCurvePoint(bytes: Uint8Array): boolean {
  let y = 0n;
  for (let i = 31; i >= 0; i--) y = (y << 8n) | BigInt(i === 31 ? bytes[i] & 0x7f : bytes[i]);
  y = mod(y);
  const y2 = (y * y) % P;
  const u = mod(y2 - 1n);
  const v = mod(D * y2 + 1n);
  if (u === 0n) return true;
  const ratio = (u * modPow(v, P - 2n)) % P;
  return modPow(ratio, (P - 1n) / 2n) === 1n;
}

export function parsePublicKey(text: string): Uint8Array | null {
  const bytes = decodeKey(text);
  return bytes && isCurvePoint(bytes) ? bytes : null;
}

// A key a client names (HELLO.pubkey, ?pubkey=): 32 bytes, strict base64url
// (§5). Not checked as a point; it is only compared with keys that were.
export function decodeKey(text: string): Uint8Array | null {
  const bytes = b64urlDecode(text);
  return bytes && bytes.length === 32 ? bytes : null;
}

export function sameKey(a: Uint8Array, b: Uint8Array): boolean {
  if (a.length !== b.length) return false;
  let diff = 0;
  for (let i = 0; i < a.length; i++) diff |= a[i] ^ b[i];
  return diff === 0;
}

// Every failure is `false`: the caller answers one auth_failed whatever the cause.
export async function verifyRelayAuth(
  pubkey: Uint8Array,
  serverId: string,
  nonce: string,
  sigText: string,
): Promise<boolean> {
  const sig = b64urlDecode(sigText);
  if (!sig || sig.length !== 64) return false;
  try {
    const key = await crypto.subtle.importKey("raw", pubkey, { name: "Ed25519" }, false, ["verify"]);
    return await crypto.subtle.verify({ name: "Ed25519" }, key, sig, relayAuthMessage(serverId, nonce));
  } catch {
    return false;
  }
}

function randomB64(bytes: number): string {
  const buf = new Uint8Array(bytes);
  crypto.getRandomValues(buf);
  return b64urlEncode(buf);
}

export const newNonce = () => randomB64(24);
export const newTrunkId = () => `trk_${randomB64(16)}`;
export const newClientTrunkId = () => `ctk_${randomB64(16)}`;
