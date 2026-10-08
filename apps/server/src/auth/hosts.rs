//! Runtime Hosts: enrollment, key authentication and host tokens (Agent Runtime
//! V1, freeze §5; `PLAN.md` decision 77b).
//!
//! A Runtime Host is a machine that runs agents for this server. It is **not a
//! user and not a device**: it holds no user credential, never learns who owns
//! a session, and its token reaches `/v1/runtime/*` and nothing else. The three
//! secrets here are never interchangeable:
//!
//! | Secret | Lifetime | Purpose |
//! |---|---|---|
//! | Enrollment token `sen_…` | single use, 10 min | the owner authorised this enrollment |
//! | Host key (Ed25519) | until revoked | the host's identity; the private half never leaves it |
//! | Host token `sht_…` | 24 h | the link's credential, re-minted by proving the key |
//!
//! Tokens are blake3-hashed like every other 256-bit credential ([`token`]);
//! the host's key is stored as its public half only.

use anyhow::{Context, Result};
use data_encoding::BASE64URL_NOPAD;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use rand::Rng;
use rusqlite::{OptionalExtension, params};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use super::db::AuthDb;
use super::identity::{random_id, validate_nonce, validate_server_id};
use super::token;

/// An enrollment token: `sen_<26 Crockford>.<43 base64url>`.
pub const ENROLLMENT_PREFIX: &str = "sen_";
/// A host token, sent as `Authorization: Bearer sht_…` on `/v1/runtime/*`.
///
/// **The prefix is what routes it**, exactly as `stk_` routes an MCP key:
/// `require_auth` sends a `Bearer` value to the host path by this prefix, and
/// refuses it on every route that is not the `Host` tier.
pub const HOST_TOKEN_PREFIX: &str = "sht_";
/// The record id an enrollment token's public half names.
const ENROLLMENT_ID_PREFIX: &str = "hen_";
const HOST_ID_PREFIX: &str = "hst_";

/// The signing domain under a host's key. A third domain on a different key
/// from `storm-challenge:v1:` and `storm-relay-auth:v1:` (freeze §5.4).
pub const HOST_AUTH_PREFIX: &str = "storm-host-auth:v1:";

/// Operational defaults, not architectural limits (freeze §5.7).
pub const ENROLLMENT_TTL_SECS: i64 = 10 * 60;
pub const HOST_TOKEN_TTL_SECS: i64 = 24 * 60 * 60;
pub const CHALLENGE_TTL_SECS: i64 = 60;

pub const MAX_NAME_CHARS: usize = 64;

pub const EVENT_ENROLLMENT_ISSUED: &str = "host_enrollment_issued";
pub const EVENT_HOST_ENROLLED: &str = "host_enrolled";
pub const EVENT_HOST_RENAMED: &str = "host_renamed";
pub const EVENT_HOST_REVOKED: &str = "host_revoked";
pub const EVENT_HOST_AUTH_REJECTED: &str = "host_auth_rejected";

/// A Runtime Host as stored: public metadata only.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Host {
    pub id: String,
    pub name: String,
    pub key_id: String,
    /// base64url, no padding: the wire's one spelling of a key.
    pub public_key: String,
    pub enrolled_by: Option<String>,
    pub enrolled_at: String,
    pub last_seen: Option<String>,
    pub revoked: Option<String>,
}

impl Host {
    pub fn is_revoked(&self) -> bool {
        self.revoked.is_some()
    }
}

/// Why a host's attempt was refused. Answered to the host as one generic
/// refusal; kept specific for the audit row, as `keys::KeyFailure` is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostFailure {
    Unknown,
    Expired,
    AlreadyUsed,
    Revoked,
    BadSignature,
    Malformed,
}

impl HostFailure {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Expired => "expired",
            Self::AlreadyUsed => "already_used",
            Self::Revoked => "revoked",
            Self::BadSignature => "bad_signature",
            Self::Malformed => "malformed",
        }
    }
}

#[derive(Debug)]
pub enum HostError {
    Refused(HostFailure),
    Internal(anyhow::Error),
}

impl From<anyhow::Error> for HostError {
    fn from(e: anyhow::Error) -> Self {
        Self::Internal(e)
    }
}

impl From<rusqlite::Error> for HostError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Internal(e.into())
    }
}

type HostResult<T> = std::result::Result<T, HostError>;

fn refused<T>(failure: HostFailure) -> HostResult<T> {
    Err(HostError::Refused(failure))
}

// ---- wire formats (pinned by docs/runtime-vectors.json) -----------------

/// The public id an enrollment token names: `sen_<id>.<secret>` → `hen_<id>`.
///
/// This is the freeze's `<token_id>`. The host derives it from the token it
/// was given, without asking, and the enrollment signature covers it — so the
/// signed message carries no secret.
pub fn enrollment_token_id(token: &str) -> Option<String> {
    let rest = token.strip_prefix(ENROLLMENT_PREFIX)?;
    let (public, secret) = rest.split_once('.')?;
    let crockford = |s: &str| {
        s.len() == 26
            && s.bytes()
                .all(|b| b"0123456789ABCDEFGHJKMNPQRSTVWXYZ".contains(&b))
    };
    let secret_ok = BASE64URL_NOPAD
        .decode(secret.as_bytes())
        .is_ok_and(|raw| raw.len() == 32);
    (crockford(public) && secret_ok).then(|| format!("{ENROLLMENT_ID_PREFIX}{public}"))
}

/// The bytes an enrollment signature covers.
pub fn enroll_message(server_id: &str, token_id: &str) -> Vec<u8> {
    format!("{HOST_AUTH_PREFIX}{server_id}:{token_id}").into_bytes()
}

/// The bytes a connect signature covers.
///
/// Five fields where the enrollment message has four, and every field is
/// validated against the delimiter, so neither can be read as the other.
pub fn connect_message(server_id: &str, host_id: &str, nonce: &str) -> Vec<u8> {
    format!("{HOST_AUTH_PREFIX}{server_id}:{host_id}:{nonce}").into_bytes()
}

/// A host id: `hst_` plus 26 Crockford characters, so it can sit between the
/// colons of a signed message without re-splitting it.
pub fn validate_host_id(host_id: &str) -> std::result::Result<(), &'static str> {
    let ok = host_id.strip_prefix(HOST_ID_PREFIX).is_some_and(|rest| {
        rest.len() == 26
            && rest
                .bytes()
                .all(|b| b"0123456789ABCDEFGHJKMNPQRSTVWXYZ".contains(&b))
    });
    if ok {
        Ok(())
    } else {
        Err("host_id must be hst_ plus 26 Crockford characters")
    }
}

/// Decodes a base64url public key. **Unpadded, and the alternatives are
/// refused**: one key has one spelling (the SRP rule, applied here).
pub fn decode_public_key(b64: &str) -> std::result::Result<VerifyingKey, &'static str> {
    let raw = BASE64URL_NOPAD
        .decode(b64.as_bytes())
        .map_err(|_| "public_key must be unpadded base64url")?;
    let bytes: [u8; 32] = raw.try_into().map_err(|_| "public_key must be 32 bytes")?;
    VerifyingKey::from_bytes(&bytes).map_err(|_| "public_key is not an Ed25519 point")
}

fn verify(key: &VerifyingKey, message: &[u8], signature_b64: &str) -> bool {
    let Ok(raw) = BASE64URL_NOPAD.decode(signature_b64.as_bytes()) else {
        return false;
    };
    let Ok(bytes) = <[u8; 64]>::try_from(raw) else {
        return false;
    };
    key.verify(message, &Signature::from_bytes(&bytes)).is_ok()
}

pub fn validate_name(name: &str) -> std::result::Result<(), String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("a host needs a name".into());
    }
    if trimmed.chars().count() > MAX_NAME_CHARS {
        return Err(format!(
            "a host name is at most {MAX_NAME_CHARS} characters"
        ));
    }
    Ok(())
}

fn parse_time(ts: &str) -> Result<OffsetDateTime> {
    OffsetDateTime::parse(ts, &Rfc3339).with_context(|| format!("parsing timestamp `{ts}`"))
}

fn plus_secs(now: &str, secs: i64) -> Result<String> {
    Ok((parse_time(now)? + time::Duration::seconds(secs)).format(&Rfc3339)?)
}

fn expired(expires: &str, now: &str) -> Result<bool> {
    Ok(parse_time(expires)? <= parse_time(now)?)
}

// ---- the enrollment -------------------------------------------------------

/// An issued enrollment. The token is returned once, beside it.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Enrollment {
    pub id: String,
    pub expires: String,
}

/// Mints a single-use enrollment for `user_id` (who must be the owner — the
/// caller checks). Returns the record and the **token, once**.
pub fn issue_enrollment(db: &mut AuthDb, user_id: &str, now: &str) -> Result<(Enrollment, String)> {
    let id = random_id(ENROLLMENT_ID_PREFIX);
    let mut secret = [0u8; 32];
    rand::rng().fill_bytes(&mut secret);
    let token = format!(
        "{ENROLLMENT_PREFIX}{}.{}",
        &id[ENROLLMENT_ID_PREFIX.len()..],
        BASE64URL_NOPAD.encode(&secret)
    );
    let expires = plus_secs(now, ENROLLMENT_TTL_SECS)?;
    db.conn.execute(
        "INSERT INTO host_enrollments (id, token_hash, created_by, created, expires)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![id, token::hash(&token), user_id, now, expires],
    )?;
    db.record_event(
        EVENT_ENROLLMENT_ISSUED,
        Some(user_id),
        None,
        now,
        &format!(r#"{{"enrollment_id":"{id}"}}"#),
    )?;
    Ok((Enrollment { id, expires }, token))
}

/// What a host sends to enroll.
pub struct EnrollRequest<'a> {
    pub token: &'a str,
    pub public_key: &'a str,
    pub key_id: &'a str,
    pub name: &'a str,
    /// Over [`enroll_message`], by the key being enrolled.
    pub signature: &'a str,
}

/// Consumes an enrollment token and registers the host it names.
///
/// The signature proves the host holds the private half of the key it is
/// registering *for this enrollment on this server*, so neither a captured
/// request nor a key from somewhere else can be replayed into one.
pub fn enroll(
    db: &mut AuthDb,
    server_id: &str,
    req: &EnrollRequest<'_>,
    now: &str,
) -> HostResult<Host> {
    let Some(token_id) = enrollment_token_id(req.token) else {
        return refused(HostFailure::Malformed);
    };
    if validate_name(req.name).is_err() || !valid_key_id(req.key_id) {
        return refused(HostFailure::Malformed);
    }
    let Ok(key) = decode_public_key(req.public_key) else {
        return refused(HostFailure::Malformed);
    };

    let row: Option<(String, String, Option<String>, String)> = db
        .conn
        .query_row(
            "SELECT id, expires, consumed, created_by FROM host_enrollments WHERE token_hash = ?1",
            params![token::hash(req.token)],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?;
    let Some((id, expires, consumed, created_by)) = row else {
        return refused(HostFailure::Unknown);
    };
    // The public half must name the row the secret found: a token whose two
    // halves came from different enrollments is not one token.
    if id != token_id {
        return refused(HostFailure::Unknown);
    }
    if consumed.is_some() {
        return refused(HostFailure::AlreadyUsed);
    }
    if expired(&expires, now)? {
        return refused(HostFailure::Expired);
    }
    if !verify(&key, &enroll_message(server_id, &token_id), req.signature) {
        return refused(HostFailure::BadSignature);
    }

    let host_id = random_id(HOST_ID_PREFIX);
    let tx = db.conn.transaction()?;
    // Consumed in the same transaction that creates the host, and only if
    // still unconsumed: two concurrent enrollments with one token cannot both
    // win.
    let claimed = tx.execute(
        "UPDATE host_enrollments SET consumed = ?1, consumed_by = ?2
         WHERE id = ?3 AND consumed IS NULL",
        params![now, host_id, id],
    )?;
    if claimed == 0 {
        return refused(HostFailure::AlreadyUsed);
    }
    tx.execute(
        "INSERT INTO runtime_hosts (id, name, public_key, key_id, enrolled_by, enrolled_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            host_id,
            req.name.trim(),
            key.as_bytes().as_slice(),
            req.key_id,
            created_by,
            now
        ],
    )?;
    tx.commit()?;

    db.record_event(
        EVENT_HOST_ENROLLED,
        Some(&created_by),
        None,
        now,
        &format!(
            r#"{{"host_id":"{host_id}","enrollment_id":"{id}","name":{}}}"#,
            serde_json::Value::String(req.name.trim().to_string())
        ),
    )?;
    db.host_by_id(&host_id)?
        .context("the host just enrolled is missing")
        .map_err(HostError::Internal)
}

/// A key id as the host chose it: `key_` plus up to 64 id-safe characters.
fn valid_key_id(key_id: &str) -> bool {
    key_id.strip_prefix("key_").is_some_and(|rest| {
        (1..=64).contains(&rest.len())
            && rest
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    })
}

// ---- key authentication ----------------------------------------------------

/// A server-generated, single-use nonce for one connect (freeze §5.4).
pub fn issue_challenge(db: &mut AuthDb, host_id: &str, now: &str) -> HostResult<(String, String)> {
    if validate_host_id(host_id).is_err() {
        return refused(HostFailure::Malformed);
    }
    // Issued only for a live host, so an unknown id gets nothing to sign.
    match db.host_by_id(host_id)? {
        None => return refused(HostFailure::Unknown),
        Some(h) if h.is_revoked() => return refused(HostFailure::Revoked),
        Some(_) => {}
    }
    let mut raw = [0u8; 24];
    rand::rng().fill_bytes(&mut raw);
    let nonce = BASE64URL_NOPAD.encode(&raw);
    let expires = plus_secs(now, CHALLENGE_TTL_SECS)?;
    // Expired challenges are swept here, so the table never grows unbounded.
    db.conn.execute(
        "DELETE FROM host_challenges WHERE expires <= ?1",
        params![now],
    )?;
    db.conn.execute(
        "INSERT INTO host_challenges (nonce, host_id, expires) VALUES (?1, ?2, ?3)",
        params![nonce, host_id, expires],
    )?;
    Ok((nonce, expires))
}

/// An issued host token, shown once.
#[derive(Debug, Clone, serde::Serialize)]
pub struct IssuedHostToken {
    pub token: String,
    pub expires: String,
}

/// Proves the host's key against a nonce this server issued, and mints a host
/// token. The nonce is consumed whether or not the signature verifies, so one
/// nonce is one attempt.
pub fn authenticate_key(
    db: &mut AuthDb,
    server_id: &str,
    host_id: &str,
    nonce: &str,
    signature: &str,
    now: &str,
) -> HostResult<IssuedHostToken> {
    if validate_host_id(host_id).is_err()
        || validate_nonce(nonce).is_err()
        || validate_server_id(server_id).is_err()
    {
        return refused(HostFailure::Malformed);
    }
    let challenge: Option<(String, String)> = db
        .conn
        .query_row(
            "DELETE FROM host_challenges WHERE nonce = ?1 RETURNING host_id, expires",
            params![nonce],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((challenged, expires)) = challenge else {
        return refused(HostFailure::Unknown);
    };
    if challenged != host_id {
        return refused(HostFailure::Unknown);
    }
    if expired(&expires, now)? {
        return refused(HostFailure::Expired);
    }
    let Some(host) = db.host_by_id(host_id)? else {
        return refused(HostFailure::Unknown);
    };
    if host.is_revoked() {
        return refused(HostFailure::Revoked);
    }
    let key = decode_public_key(&host.public_key)
        .map_err(|e| HostError::Internal(anyhow::anyhow!("stored host key unreadable: {e}")))?;
    if !verify(&key, &connect_message(server_id, host_id, nonce), signature) {
        return refused(HostFailure::BadSignature);
    }

    let token = token::mint(HOST_TOKEN_PREFIX);
    let expires = plus_secs(now, HOST_TOKEN_TTL_SECS)?;
    db.conn.execute(
        "INSERT INTO host_tokens (id, host_id, token_hash, created, expires)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            format!("htk_{}", uuid::Uuid::new_v4()),
            host_id,
            token::hash(&token),
            now,
            expires
        ],
    )?;
    db.touch_host(host_id, now)?;
    Ok(IssuedHostToken { token, expires })
}

/// Resolves a presented `sht_…` token to its host.
pub fn authenticate_token(db: &mut AuthDb, token: &str, now: &str) -> HostResult<Host> {
    let row: Option<(String, String, Option<String>)> = db
        .conn
        .query_row(
            "SELECT host_id, expires, revoked FROM host_tokens WHERE token_hash = ?1",
            params![token::hash(token)],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    let Some((host_id, expires, revoked)) = row else {
        return refused(HostFailure::Unknown);
    };
    if revoked.is_some() {
        return refused(HostFailure::Revoked);
    }
    if expired(&expires, now)? {
        return refused(HostFailure::Expired);
    }
    match db.host_by_id(&host_id)? {
        Some(host) if !host.is_revoked() => Ok(host),
        Some(_) => refused(HostFailure::Revoked),
        None => refused(HostFailure::Unknown),
    }
}

// ---- administration --------------------------------------------------------

pub fn rename(
    db: &mut AuthDb,
    host_id: &str,
    name: &str,
    by: &str,
    now: &str,
) -> Result<Option<Host>> {
    validate_name(name).map_err(|e| anyhow::anyhow!(e))?;
    let changed = db.conn.execute(
        "UPDATE runtime_hosts SET name = ?1 WHERE id = ?2",
        params![name.trim(), host_id],
    )?;
    if changed == 0 {
        return Ok(None);
    }
    db.record_event(
        EVENT_HOST_RENAMED,
        Some(by),
        None,
        now,
        &format!(
            r#"{{"host_id":"{host_id}","name":{}}}"#,
            serde_json::Value::String(name.trim().to_string())
        ),
    )?;
    db.host_by_id(host_id)
}

/// Revokes a host and every token it holds. Idempotent. Returns whether the
/// host exists.
pub fn revoke(db: &mut AuthDb, host_id: &str, by: &str, now: &str) -> Result<bool> {
    let Some(host) = db.host_by_id(host_id)? else {
        return Ok(false);
    };
    if host.is_revoked() {
        return Ok(true);
    }
    let tx = db.conn.transaction()?;
    tx.execute(
        "UPDATE runtime_hosts SET revoked = ?1 WHERE id = ?2",
        params![now, host_id],
    )?;
    tx.execute(
        "UPDATE host_tokens SET revoked = ?1 WHERE host_id = ?2 AND revoked IS NULL",
        params![now, host_id],
    )?;
    tx.execute(
        "DELETE FROM host_challenges WHERE host_id = ?1",
        params![host_id],
    )?;
    tx.commit()?;
    db.record_event(
        EVENT_HOST_REVOKED,
        Some(by),
        None,
        now,
        &format!(r#"{{"host_id":"{host_id}"}}"#),
    )?;
    Ok(true)
}

impl AuthDb {
    pub fn host_by_id(&self, host_id: &str) -> Result<Option<Host>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id, name, key_id, public_key, enrolled_by, enrolled_at, last_seen, revoked
                 FROM runtime_hosts WHERE id = ?1",
                params![host_id],
                host_from_row,
            )
            .optional()?)
    }

    pub fn list_hosts(&self) -> Result<Vec<Host>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, key_id, public_key, enrolled_by, enrolled_at, last_seen, revoked
             FROM runtime_hosts ORDER BY enrolled_at, id",
        )?;
        let rows = stmt.query_map([], host_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn touch_host(&self, host_id: &str, now: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE runtime_hosts SET last_seen = ?1 WHERE id = ?2",
            params![now, host_id],
        )?;
        Ok(())
    }
}

fn host_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Host> {
    let key: Vec<u8> = r.get(3)?;
    Ok(Host {
        id: r.get(0)?,
        name: r.get(1)?,
        key_id: r.get(2)?,
        public_key: BASE64URL_NOPAD.encode(&key),
        enrolled_by: r.get(4)?,
        enrolled_at: r.get(5)?,
        last_seen: r.get(6)?,
        revoked: r.get(7)?,
    })
}

/// Refuses the obviously wrong before the database is asked.
pub fn validate_server_url(url: &str) -> std::result::Result<(), &'static str> {
    let rest = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))
        .ok_or("server_url must be http:// or https://")?;
    if rest.is_empty() || url.len() > 512 || url.bytes().any(|b| b.is_ascii_whitespace()) {
        return Err("server_url is not a usable URL");
    }
    Ok(())
}

/// The enrollment string the owner pastes into `storm-runtime enroll`.
pub fn enrollment_string(
    server_url: &str,
    server_id: &str,
    server_pubkey: &str,
    token: &str,
) -> String {
    format!("storm-enroll:v1:{server_url}:{server_id}:{server_pubkey}:{token}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::account::create_account;
    use ed25519_dalek::{Signer, SigningKey};

    const NOW: &str = "2026-10-03T12:00:00Z";
    const LATER: &str = "2026-10-03T12:05:00Z";
    const MUCH_LATER: &str = "2026-10-03T13:00:00Z";
    const SERVER_ID: &str = "srv_01ARZ3NDEKTSV4RRFFQ69G5FAV";

    fn vectors() -> serde_json::Value {
        serde_json::from_str(include_str!("../../../../docs/runtime-vectors.json")).unwrap()
    }

    fn db_with_owner() -> (AuthDb, String) {
        let mut db = AuthDb::open_in_memory().unwrap();
        let account = create_account(&mut db, "x", NOW).unwrap();
        (db, account.id)
    }

    fn key() -> SigningKey {
        SigningKey::from_bytes(&[7u8; 32])
    }

    fn sign(key: &SigningKey, message: &[u8]) -> String {
        BASE64URL_NOPAD.encode(&key.sign(message).to_bytes())
    }

    fn enroll_with(db: &mut AuthDb, token: &str, key: &SigningKey, now: &str) -> HostResult<Host> {
        let tid = enrollment_token_id(token).unwrap();
        let sig = sign(key, &enroll_message(SERVER_ID, &tid));
        let pk = BASE64URL_NOPAD.encode(key.verifying_key().as_bytes());
        enroll(
            db,
            SERVER_ID,
            &EnrollRequest {
                token,
                public_key: &pk,
                key_id: "key_test",
                name: "build-vm",
                signature: &sig,
            },
            now,
        )
    }

    fn refusal<T: std::fmt::Debug>(r: HostResult<T>) -> HostFailure {
        match r {
            Err(HostError::Refused(f)) => f,
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    #[test]
    fn the_wire_formats_match_the_shared_vectors() {
        // docs/runtime-vectors.json is the only thing that makes this crate
        // and apps/runtime agree (decision 77b); they cannot share code.
        let v = vectors();
        let e = &v["enroll"];
        let token = e["token"].as_str().unwrap();
        assert_eq!(enrollment_token_id(token).unwrap(), e["token_id"]);
        let msg = enroll_message(
            e["server_id"].as_str().unwrap(),
            e["token_id"].as_str().unwrap(),
        );
        assert_eq!(msg, e["message"].as_str().unwrap().as_bytes());
        let host_pk =
            decode_public_key(v["host_key"]["public_key_b64url"].as_str().unwrap()).unwrap();
        assert!(verify(
            &host_pk,
            &msg,
            e["signature_b64url"].as_str().unwrap()
        ));

        let c = &v["connect"];
        let msg = connect_message(
            c["server_id"].as_str().unwrap(),
            c["host_id"].as_str().unwrap(),
            c["nonce"].as_str().unwrap(),
        );
        assert_eq!(msg, c["message"].as_str().unwrap().as_bytes());
        assert!(verify(
            &host_pk,
            &msg,
            c["signature_b64url"].as_str().unwrap()
        ));
        validate_host_id(c["host_id"].as_str().unwrap()).unwrap();

        let good = &v["enrollment_string"]["valid"][0];
        assert_eq!(
            enrollment_string(
                good["server_url"].as_str().unwrap(),
                good["server_id"].as_str().unwrap(),
                good["server_pubkey_b64url"].as_str().unwrap(),
                good["token"].as_str().unwrap(),
            ),
            good["string"].as_str().unwrap()
        );
    }

    #[test]
    fn an_enrollment_registers_one_host_once() {
        let (mut db, owner) = db_with_owner();
        let (enrollment, token) = issue_enrollment(&mut db, &owner, NOW).unwrap();
        assert!(token.starts_with(ENROLLMENT_PREFIX));
        assert_eq!(enrollment_token_id(&token).unwrap(), enrollment.id);

        let host = enroll_with(&mut db, &token, &key(), LATER).unwrap();
        assert!(validate_host_id(&host.id).is_ok());
        assert_eq!(host.name, "build-vm");
        assert_eq!(host.enrolled_by.as_deref(), Some(owner.as_str()));

        // Single use: the same token, even with a valid signature, is refused.
        assert_eq!(
            refusal(enroll_with(&mut db, &token, &key(), LATER)),
            HostFailure::AlreadyUsed
        );
        assert_eq!(db.list_hosts().unwrap().len(), 1);
    }

    #[test]
    fn an_enrollment_is_refused_after_ten_minutes() {
        let (mut db, owner) = db_with_owner();
        let (_, token) = issue_enrollment(&mut db, &owner, NOW).unwrap();
        assert_eq!(
            refusal(enroll_with(&mut db, &token, &key(), MUCH_LATER)),
            HostFailure::Expired
        );
    }

    #[test]
    fn an_enrollment_signed_for_another_server_is_refused() {
        // The signature binds the server id: a request captured on the way to
        // one server cannot enroll at another.
        let (mut db, owner) = db_with_owner();
        let (_, token) = issue_enrollment(&mut db, &owner, NOW).unwrap();
        let tid = enrollment_token_id(&token).unwrap();
        let k = key();
        let sig = sign(&k, &enroll_message("srv_SOMEONEELSE", &tid));
        let pk = BASE64URL_NOPAD.encode(k.verifying_key().as_bytes());
        let r = enroll(
            &mut db,
            SERVER_ID,
            &EnrollRequest {
                token: &token,
                public_key: &pk,
                key_id: "key_test",
                name: "x",
                signature: &sig,
            },
            LATER,
        );
        assert_eq!(refusal(r), HostFailure::BadSignature);
        // And the token is still usable: a forged attempt does not burn it.
        enroll_with(&mut db, &token, &k, LATER).unwrap();
    }

    #[test]
    fn a_token_with_the_halves_of_two_enrollments_is_refused() {
        let (mut db, owner) = db_with_owner();
        let (_, a) = issue_enrollment(&mut db, &owner, NOW).unwrap();
        let (_, b) = issue_enrollment(&mut db, &owner, NOW).unwrap();
        let spliced = format!(
            "{}.{}",
            a.split_once('.').unwrap().0,
            b.split_once('.').unwrap().1
        );
        assert_eq!(
            refusal(enroll_with(&mut db, &spliced, &key(), LATER)),
            HostFailure::Unknown
        );
    }

    #[test]
    fn key_authentication_mints_a_token_and_burns_the_nonce() {
        let (mut db, owner) = db_with_owner();
        let (_, token) = issue_enrollment(&mut db, &owner, NOW).unwrap();
        let k = key();
        let host = enroll_with(&mut db, &token, &k, NOW).unwrap();

        let (nonce, _) = issue_challenge(&mut db, &host.id, NOW).unwrap();
        let sig = sign(&k, &connect_message(SERVER_ID, &host.id, &nonce));
        let issued = authenticate_key(&mut db, SERVER_ID, &host.id, &nonce, &sig, NOW).unwrap();
        assert!(issued.token.starts_with(HOST_TOKEN_PREFIX));
        assert_eq!(
            authenticate_token(&mut db, &issued.token, NOW).unwrap().id,
            host.id
        );

        // The same nonce again is gone.
        assert_eq!(
            refusal(authenticate_key(
                &mut db, SERVER_ID, &host.id, &nonce, &sig, NOW
            )),
            HostFailure::Unknown
        );
    }

    #[test]
    fn a_wrong_key_burns_the_nonce_and_gets_nothing() {
        let (mut db, owner) = db_with_owner();
        let (_, token) = issue_enrollment(&mut db, &owner, NOW).unwrap();
        let host = enroll_with(&mut db, &token, &key(), NOW).unwrap();
        let (nonce, _) = issue_challenge(&mut db, &host.id, NOW).unwrap();
        let impostor = SigningKey::from_bytes(&[9u8; 32]);
        let sig = sign(&impostor, &connect_message(SERVER_ID, &host.id, &nonce));
        assert_eq!(
            refusal(authenticate_key(
                &mut db, SERVER_ID, &host.id, &nonce, &sig, NOW
            )),
            HostFailure::BadSignature
        );
        // One nonce, one attempt.
        let sig = sign(&key(), &connect_message(SERVER_ID, &host.id, &nonce));
        assert_eq!(
            refusal(authenticate_key(
                &mut db, SERVER_ID, &host.id, &nonce, &sig, NOW
            )),
            HostFailure::Unknown
        );
    }

    #[test]
    fn a_nonce_expires_after_sixty_seconds() {
        let (mut db, owner) = db_with_owner();
        let (_, token) = issue_enrollment(&mut db, &owner, NOW).unwrap();
        let host = enroll_with(&mut db, &token, &key(), NOW).unwrap();
        let (nonce, _) = issue_challenge(&mut db, &host.id, NOW).unwrap();
        let sig = sign(&key(), &connect_message(SERVER_ID, &host.id, &nonce));
        assert_eq!(
            refusal(authenticate_key(
                &mut db, SERVER_ID, &host.id, &nonce, &sig, LATER
            )),
            HostFailure::Expired
        );
    }

    #[test]
    fn revoking_a_host_kills_its_tokens_and_its_future() {
        let (mut db, owner) = db_with_owner();
        let (_, token) = issue_enrollment(&mut db, &owner, NOW).unwrap();
        let k = key();
        let host = enroll_with(&mut db, &token, &k, NOW).unwrap();
        let (nonce, _) = issue_challenge(&mut db, &host.id, NOW).unwrap();
        let sig = sign(&k, &connect_message(SERVER_ID, &host.id, &nonce));
        let issued = authenticate_key(&mut db, SERVER_ID, &host.id, &nonce, &sig, NOW).unwrap();

        assert!(revoke(&mut db, &host.id, &owner, NOW).unwrap());
        assert_eq!(
            refusal(authenticate_token(&mut db, &issued.token, NOW)),
            HostFailure::Revoked
        );
        assert_eq!(
            refusal(issue_challenge(&mut db, &host.id, NOW)),
            HostFailure::Revoked
        );
        assert!(
            revoke(&mut db, &host.id, &owner, NOW).unwrap(),
            "idempotent"
        );
    }

    #[test]
    fn a_host_token_expires_after_a_day() {
        let (mut db, owner) = db_with_owner();
        let (_, token) = issue_enrollment(&mut db, &owner, NOW).unwrap();
        let k = key();
        let host = enroll_with(&mut db, &token, &k, NOW).unwrap();
        let (nonce, _) = issue_challenge(&mut db, &host.id, NOW).unwrap();
        let sig = sign(&k, &connect_message(SERVER_ID, &host.id, &nonce));
        let issued = authenticate_key(&mut db, SERVER_ID, &host.id, &nonce, &sig, NOW).unwrap();
        assert_eq!(
            refusal(authenticate_token(
                &mut db,
                &issued.token,
                "2026-10-04T12:00:01Z"
            )),
            HostFailure::Expired
        );
    }

    #[test]
    fn no_secret_reaches_the_security_events() {
        // The invariant every auth area holds: `security_events` never contains
        // a token. Checked across the whole lifecycle.
        let (mut db, owner) = db_with_owner();
        let (_, token) = issue_enrollment(&mut db, &owner, NOW).unwrap();
        let k = key();
        let host = enroll_with(&mut db, &token, &k, NOW).unwrap();
        let (nonce, _) = issue_challenge(&mut db, &host.id, NOW).unwrap();
        let sig = sign(&k, &connect_message(SERVER_ID, &host.id, &nonce));
        let issued = authenticate_key(&mut db, SERVER_ID, &host.id, &nonce, &sig, NOW).unwrap();
        rename(&mut db, &host.id, "renamed", &owner, NOW).unwrap();
        revoke(&mut db, &host.id, &owner, NOW).unwrap();

        let secret_half = token.split_once('.').unwrap().1;
        let mut stmt = db
            .conn
            .prepare("SELECT kind, detail FROM security_events")
            .unwrap();
        let rows: Vec<(String, String)> = stmt
            .query_map([], |r| {
                Ok((
                    r.get(0)?,
                    r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                ))
            })
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        let kinds: Vec<&str> = rows.iter().map(|(k, _)| k.as_str()).collect();
        for kind in [
            EVENT_ENROLLMENT_ISSUED,
            EVENT_HOST_ENROLLED,
            EVENT_HOST_RENAMED,
            EVENT_HOST_REVOKED,
        ] {
            assert!(kinds.contains(&kind), "{kind} missing from {kinds:?}");
        }
        for (_, detail) in rows {
            assert!(!detail.contains(secret_half), "{detail}");
            assert!(!detail.contains(&issued.token), "{detail}");
            assert!(!detail.contains(&nonce), "{detail}");
        }
    }

    #[test]
    fn ids_and_keys_have_one_spelling() {
        assert!(validate_host_id("hst_01HB6V3Z7Q2M4N8P0R5S9T1W3X").is_ok());
        assert!(validate_host_id("hst_01HB6V3Z7Q2M4N8P0R5S9T1W3:").is_err());
        assert!(validate_host_id("srv_01HB6V3Z7Q2M4N8P0R5S9T1W3X").is_err());
        let pk = BASE64URL_NOPAD.encode(key().verifying_key().as_bytes());
        assert!(decode_public_key(&pk).is_ok());
        assert!(
            decode_public_key(&format!("{pk}=")).is_err(),
            "padding refused"
        );
        assert!(validate_server_url("http://192.168.1.51:8484").is_ok());
        assert!(validate_server_url("ftp://x").is_err());
        assert!(validate_server_url("http://").is_err());
    }
}
