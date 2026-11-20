// ULID-shaped: time prefix first, in Crockford base32, so KV's (and R2's,
// before decision 162's switch) lexicographic listing is oldest-first for
// free and the time can be decoded back out.
const ALPHABET = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";
const TIME_CHARS = 10;
const RANDOM_CHARS = 16;

function encodeTime(ms: number): string {
  let value = Math.floor(ms);
  let out = "";
  for (let i = 0; i < TIME_CHARS; i++) {
    out = ALPHABET[value % 32] + out;
    value = Math.floor(value / 32);
  }
  return out;
}

function encodeRandom(): string {
  const bytes = new Uint8Array(RANDOM_CHARS);
  crypto.getRandomValues(bytes);
  let out = "";
  for (const b of bytes) out += ALPHABET[b % 32];
  return out;
}

export function newReportId(now: number = Date.now()): string {
  return encodeTime(now) + encodeRandom();
}

export function isValidReportId(id: string): boolean {
  if (id.length !== TIME_CHARS + RANDOM_CHARS) return false;
  for (let i = 0; i < id.length; i++) if (!ALPHABET.includes(id[i])) return false;
  return true;
}

// NaN for anything not shaped like a minted id; callers must check it.
export function timeOfReportId(id: string): number {
  if (!isValidReportId(id)) return NaN;
  let value = 0;
  for (let i = 0; i < TIME_CHARS; i++) value = value * 32 + ALPHABET.indexOf(id[i]);
  return value;
}
