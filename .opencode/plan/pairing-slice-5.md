# Auth Slice 5 — Pairing

## What

Implement QR-based device pairing: the mechanism by which a new Storm client obtains its first `StormDevice` credential. This is the bridge between the identity tier (slices 1–2) and the session tier (slices 3–4). After this slice, the server can bootstrap a first user over the network, and existing paired devices can issue QR codes for new installations.

## Flows (from Storm Auth Protocol)

### Bootstrap (no users exist)

```
Server boots, users table empty
  → create pairing session (purpose=first_user, 5 min TTL)
  → log storm://pair URI to console
  → client scans QR

Client                              Server
  GET /v1/server ──────────────▶  {server_id, public_key}
  POST /v1/server/challenge ───▶  {key_id, signature}
  (verify signature against pk from QR)
  POST /v1/pair {n, name, platform, version}
  ◀────────────────────────────  {device_id, device_secret}
  (secure storage)

  GET /v1/users                  (device auth)
  ◀────────────────────────────  {users: []}  → show "create first user"
  POST /v1/users/first {username, password}
  ◀────────────────────────────  201
  POST /v1/auth/login {username, password}
  ◀────────────────────────────  {access, refresh, expires}
  → session, vault opens
```

### Add device (users exist, existing client scans QR)

```
Existing client (session)          Server              New client
  POST /v1/pairings ─────────────▶  create nonce, 5 min
  ◀──── {sid, pk, n, exp, addr}    (or just the full QR payload)
  renders QR on screen ─── scanned out of band ──────▶
                                   ◀── challenge / verify ──
                                   ◀── POST /v1/pair ──────
                                   ─── device_id + secret ─▶
                                   ◀── GET /v1/users ──────
                                   ─── user picker ────────▶
                                   ◀── POST /v1/auth/login ─
                                   ─── session ────────────▶
```

## Design decisions (from Storm Remote Decisions)

- **R4**: QR is pairing, not permanent auth. Short-lived, single-use, rate-limited, invalidated on use or expiry. Never carries a permanent secret.
- **A2**: Server credential is Ed25519. Public key in QR, signature over challenge proves server identity.
- **A3**: `server_id` is random, independent of key. Survives key rotation.
- **A8**: First user requires device auth + empty user table. Bootstrap QR is the mechanism.

## QR payload format

```
storm://pair?v=1&sid=<server_id>&pk=<base64url-public-key>&n=<nonce>&exp=<expiry-rfc3339>&addr=<address-hint>
```

No signature on the payload itself — the QR is the trust anchor, and the challenge step proves the server holds the private key. `addr` is a hint, never trusted.

## Schema (already built, no migration needed)

`pairing_sessions` table exists in auth.db (slice 1), currently empty:

```sql
CREATE TABLE IF NOT EXISTS pairing_sessions (
    id           TEXT PRIMARY KEY,        -- 'pair_' + 26
    nonce_hash   BLOB NOT NULL UNIQUE,    -- blake3(nonce)
    purpose      TEXT NOT NULL CHECK (purpose IN ('first_user','add_device')),
    created_by   TEXT REFERENCES users(id),   -- NULL only for 'first_user'
    created      TEXT NOT NULL,
    expires      TEXT NOT NULL,
    consumed     TEXT,
    consumed_by  TEXT REFERENCES client_devices(id),
    attempts     INTEGER NOT NULL DEFAULT 0
);
```

## New code

### 1. `apps/server/src/auth/pairing.rs` (new file)

Domain types and operations for pairing sessions. No HTTP surface here.

```
PAIRING_NONCE_LEN = 24 (bytes of randomness → 32 chars base64url)
PAIRING_TTL_SECS = 300 (5 minutes)
PAIRING_MAX_ATTEMPTS = 10

struct PairingSession { id, purpose, created_by, created, expires, consumed, consumed_by, attempts }
struct ConsumeResult { device_id, device_secret, server_id, public_key, key_id }
struct QrPayload { sid, pk, n, exp, addr }

fn generate_nonce() → String  // 24 random bytes → base64url no-pad
fn create_session(db, purpose, created_by, now) → Result<(String, PairingSession)>
    // generates nonce, hashes it, inserts row, returns (nonce, session)
fn consume_session(db, nonce, device_name, platform, client_version, now) → Result<ConsumeResult>
    // hash nonce → lookup by nonce_hash
    // check: not consumed, not expired, attempts < max
    // increment attempts
    // create device row (client_devices) via devices::insert_paired
    // mark pairing session consumed
    // return device_id + plaintext secret + server info
fn encode_qr(server, pairing_session, nonce, addr) → QrPayload
    // builds the storm://pair URI fields
```

### 2. `apps/server/src/auth/devices.rs` (extend)

Add `insert_paired` (the existing `insert_device` is private to the module):

```
pub fn create_paired(db, name, platform, client_version, secret, paired_via, now) → Result<(Device, String)>
    // generates device id + secret, hashes secret, inserts row, records event
    // returns (device, plaintext_secret)
```

The existing `create_synthetic` generates its own secret and discards it. `create_paired` returns the plaintext secret to the caller (the pairing handler, which sends it to the client).

### 3. `apps/server/src/auth/db.rs` (extend)

Add AuthDb methods for pairing:

```
fn insert_pairing_session(session, nonce_hash) → Result<()>
fn pairing_session_by_nonce_hash(hash) → Result<Option<PairingSession>>
fn mark_pairing_consumed(id, consumed_by, now) → Result<()>
fn increment_pairing_attempts(id) → Result<()>
fn active_pairing_count() → Result<i64>
fn expired_pairing_count(now) → Result<i64>
```

### 4. `apps/server/src/auth/mod.rs` (extend)

Add `pub mod pairing;` and re-export key types.

### 5. `apps/server/src/api.rs` (extend)

#### New routes

**None tier** (below require_auth, no credential needed):
- `POST /v1/pair` — consume a pairing nonce, return device credentials

**Session tier** (requires Bearer token):
- `POST /v1/pairings` — issue a new pairing QR for another device

#### New handlers

`POST /v1/pair` handler:
- Extracts: `{ n, name, platform, version }` from body
- Calls `pairing::consume_session()`
- On success: returns `{ device_id, device_secret, server_id, public_key, key_id }`
- Errors: 409 `pairing_consumed`, 410 `pairing_expired`, 429 `rate_limited`
- Sets `DeviceAuth` extension (the newly paired device is immediately authenticated)

`POST /v1/pairings` handler:
- Extracts: `{ purpose }` from body (default: `add_device`)
- Calls `pairing::create_session()`
- Returns: `{ sid, pk, n, exp, addr }` — enough for the client to render a QR

#### AppState changes

```rust
pub struct AppState {
    // ... existing fields ...
    /// Bootstrap pairing nonce (plaintext), if one was created at boot.
    /// Needed to reconstruct the QR payload for the console log.
    pub bootstrap_nonce: Option<String>,
    /// The server's listen address, used as the `addr` hint in QR payloads.
    pub listen_addr: String,
}
```

#### Error codes added

| Code | When |
|---|---|
| `pairing_consumed` | Nonce already used (409) |
| `pairing_expired` | Nonce TTL exceeded (410) |
| `rate_limited` | Too many attempts on this nonce (429) |
| `already_initialized` | first_user QR but users exist (409) |

### 6. `apps/server/src/main.rs` (extend)

At boot, after loading identity and opening auth.db:

```rust
// If no users exist, create a bootstrap pairing session.
if user_count == 0 {
    let (nonce, session) = pairing::create_session(&mut auth_db, "first_user", None, &now)?;
    app_state.bootstrap_nonce = Some(nonce.clone());
    // Log the QR URI
    let qr = pairing::encode_qr(&identity, &session, &nonce, &listen_addr);
    tracing::info!(uri = %qr.to_uri(), "bootstrap pairing QR — scan with Storm Client");
}
```

### 7. `apps/server/src/ops.rs` (extend)

Add `ops::issue_pairing_qr()` — thin wrapper over the pairing module, following the pattern of `ops::server_info()` and `ops::sign_challenge()`.

### 8. CLI: `storm-server qr` (new subcommand)

Reads the bootstrap pairing nonce from AppState (or regenerates one if none exists), encodes it as a QR payload, and prints the URI. Host-only recovery path — no network involved.

## Test strategy

### Unit tests (`auth/pairing.rs`)

- `generate_nonce_is_printable_ascii_no_colon_no_quote`
- `nonce_has_enough_entropy` (1000 nonces are unique)
- `create_and_consume_session_round_trip`
- `consuming_an_expired_session_fails`
- `consuming_a_twice-used_session_fails`
- `rate_limiting_kicks_in_after_max_attempts`
- `purpose_first_user_has_no_created_by`
- `purpose_add_device_has_created_by`
- `device_created_by_consume_has_correct_paired_via`

### Integration tests (`tests/auth_e2e.py` or inline)

- Bootstrap: boot with no users → QR URI in logs → scan → challenge → pair → first user → login
- Add device: existing session → POST /v1/pairings → scan → challenge → pair → login
- Consumed nonce returns 409
- Expired nonce returns 410
- Rate limiting returns 429
- first_user QR with existing users returns 409

### Existing tests

- `make check` must remain clean (clippy -D warnings, all unit suites)
- `make test-live` must remain clean (19 integration tests)

## What this slice does NOT do

- Client-side pairing logic (that's the client slice, after this)
- QR rendering on the client (Flutter qr_flutter package, later)
- Console QR rendering as ASCII art (later enhancement)
- The `storm-server qr` CLI can be deferred to a follow-up if needed
- Credential rotation / attestation (slice 7)

## Build order

1. `auth/pairing.rs` — domain types + operations + unit tests
2. `auth/devices.rs` — add `create_paired`
3. `auth/db.rs` — add pairing CRUD methods
4. `auth/mod.rs` — add pairing module
5. `ops.rs` — add `issue_pairing_qr`
6. `api.rs` — add routes, handlers, AppState fields
7. `main.rs` — bootstrap nonce at boot, log QR
8. Run `make check` and `make test-live`
