//! Token sync (decision 124, slice 124n): the single user's own Claude
//! subscription token, sealed in `gateway.db`, and the hosts it is delivered
//! to. The spec is *Provider Auth/Token Sync Specification* in the vault
//! (§0 pins every name here).
//!
//! Storage, the shape rule, the fingerprint and the expiry arithmetic live
//! here; who may do what is `ops.rs`'s, as for connections.
//!
//! **The token never leaves this module in the clear except as a [`Secret`]**,
//! whose `Debug` is redacted and which implements neither `Display` nor
//! `Serialize`. No view, error or audit detail has a field that could hold it.

use anyhow::Result;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use super::crypto::{Keyring, Sealed};
use super::store::GatewayDb;

pub const KIND: &str = "claude_subscription_token";
/// One user, one token: the row id is fixed, and it is the AAD's first half.
pub const CREDENTIAL_ID: &str = "pc_claude_subscription";
pub const TOKEN_PREFIX: &str = "sk-ant-oat01-";
const TOKEN_BODY_MIN: usize = 20;
const TOKEN_BODY_MAX: usize = 400;
/// `claude setup-token` mints for one year [DOC].
pub const LIFETIME_DAYS: i64 = 365;
/// The `PUT` body is a token and a date; anything near this is not one.
pub const MAX_BODY_BYTES: usize = 4096;

pub mod status {
    pub const ACTIVE: &str = "active";
    /// Its key is gone or it no longer opens: never delivered until re-synced.
    pub const NEEDS_RESYNC: &str = "needs_resync";
}

pub mod event {
    pub const SET: &str = "provider_credential_set";
    pub const REPLACED: &str = "provider_credential_replaced";
    pub const REMOVED: &str = "provider_credential_removed";
    pub const HOSTS_CHANGED: &str = "provider_credential_hosts_changed";
    pub const DELIVERED: &str = "provider_credential_delivered";
    pub const EXPIRED: &str = "provider_credential_expired";
    pub const UNREADABLE: &str = "provider_credential_unreadable";
}

pub type HostId = String;

/// The token in memory. Redacted `Debug`, no `Display`, no `Serialize`, no
/// `Clone`, overwritten on drop (best effort, as `crypto::Plaintext`).
pub struct Secret(String);

impl Secret {
    pub fn new(value: String) -> Self {
        Self(value)
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(<redacted>)")
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        let mut bytes = std::mem::take(&mut self.0).into_bytes();
        bytes.iter_mut().for_each(|b| *b = 0);
    }
}

impl<'de> Deserialize<'de> for Secret {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        String::deserialize(d).map(Secret)
    }
}

/// The `PUT` body. Parsed by hand from bytes so a malformed body is one
/// generic code, never a serde message that could quote the input.
#[derive(Debug, Deserialize)]
pub struct PutBody {
    pub token: Secret,
    #[serde(default)]
    pub minted_at: Option<String>,
}

impl PutBody {
    pub fn parse(bytes: &[u8]) -> Result<Self, Code> {
        serde_json::from_slice(bytes).map_err(|_| Code::bad_request("bad_request_body"))
    }
}

#[derive(Debug, Deserialize)]
pub struct HostsBody {
    pub hosts: Vec<HostId>,
}

/// A refusal: a stable code in `{"error": …}` (the shape every `ApiError`
/// has), with an optional human hint that never echoes input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Code {
    pub status: StatusCode,
    pub code: &'static str,
    pub message: Option<String>,
}

impl Code {
    pub fn new(status: StatusCode, code: &'static str) -> Self {
        Self {
            status,
            code,
            message: None,
        }
    }

    pub fn bad_request(code: &'static str) -> Self {
        Self::new(StatusCode::BAD_REQUEST, code)
    }

    pub fn with_message(mut self, message: impl Into<String>) -> Self {
        self.message = Some(message.into());
        self
    }

    /// The detail goes to the journal; the caller gets the code only.
    pub fn internal(e: impl std::fmt::Display) -> Self {
        tracing::warn!(error = %e, "token sync: internal error");
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "internal")
    }
}

impl IntoResponse for Code {
    fn into_response(self) -> Response {
        let mut body = serde_json::json!({ "error": self.code });
        if let Some(m) = self.message {
            body["message"] = serde_json::Value::String(m);
        }
        (self.status, axum::Json(body)).into_response()
    }
}

impl From<Code> for crate::api::ApiError {
    fn from(c: Code) -> Self {
        crate::api::ApiError(c.status, c.code.to_string())
    }
}

/// Strips what a terminal paste adds (bracketed-paste markers, `\r`,
/// surrounding whitespace), then applies §0.1's shape. A refusal names the
/// rule, never the input.
pub fn normalize_token(raw: &Secret) -> Result<Secret, Code> {
    let stripped = Secret(
        raw.expose()
            .replace("\u{1b}[200~", "")
            .replace("\u{1b}[201~", "")
            .replace('\r', ""),
    );
    let token = Secret(stripped.expose().trim().to_string());
    let ok = token
        .expose()
        .strip_prefix(TOKEN_PREFIX)
        .is_some_and(|body| {
            (TOKEN_BODY_MIN..=TOKEN_BODY_MAX).contains(&body.len())
                && body
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        });
    if ok {
        Ok(token)
    } else {
        Err(Code::bad_request("bad_token_shape").with_message(
            "a Claude token starts with sk-ant-oat01- and has no spaces; \
             paste the whole line `claude setup-token` printed",
        ))
    }
}

/// The first 8 hex characters of `blake3(token)`: enough to tell two tokens
/// apart, no character of the token itself.
pub fn fingerprint(token: &Secret) -> String {
    blake3::hash(token.expose().as_bytes()).to_hex()[..8].to_string()
}

pub fn format_time(t: OffsetDateTime) -> String {
    t.replace_nanosecond(0)
        .unwrap_or(t)
        .format(&Rfc3339)
        .unwrap_or_default()
}

pub fn parse_time(s: &str) -> Option<OffsetDateTime> {
    OffsetDateTime::parse(s, &Rfc3339).ok()
}

/// The user-stated mint time: RFC 3339, or a bare `YYYY-MM-DD` (midnight
/// UTC). `None` is "now". A time in the future is refused (an hour of clock
/// skew allowed), since it would stretch the expiry past what Anthropic gave.
pub fn minted_at(raw: Option<&str>, now: OffsetDateTime) -> Result<OffsetDateTime, Code> {
    let bad = || {
        Code::bad_request("bad_minted_at")
            .with_message("minted_at is an RFC 3339 time or a YYYY-MM-DD date, not in the future")
    };
    let Some(raw) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(now);
    };
    let t = match parse_time(raw) {
        Some(t) => t,
        None => {
            let fmt = time::macros::format_description!("[year]-[month]-[day]");
            time::Date::parse(raw, &fmt)
                .map_err(|_| bad())?
                .midnight()
                .assume_utc()
        }
    };
    if t > now + time::Duration::hours(1) {
        return Err(bad());
    }
    Ok(t)
}

pub fn expires_at(minted: OffsetDateTime) -> OffsetDateTime {
    minted + time::Duration::days(LIFETIME_DAYS)
}

/// Whole days left, rounded up (a fresh token shows 365), never negative.
pub fn days_left(expires: OffsetDateTime, now: OffsetDateTime) -> i64 {
    let secs = (expires - now).whole_seconds();
    if secs <= 0 {
        0
    } else {
        (secs + 86_399) / 86_400
    }
}

/// §6: `30d` from 30 days before, `7d` from 7 days before, `expired` after.
pub fn warn(expires: OffsetDateTime, now: OffsetDateTime) -> Option<&'static str> {
    let left = expires - now;
    if left <= time::Duration::ZERO {
        Some("expired")
    } else if left <= time::Duration::days(7) {
        Some("7d")
    } else if left <= time::Duration::days(30) {
        Some("30d")
    } else {
        None
    }
}

/// The stored row, still sealed.
#[derive(Debug, Clone)]
pub struct Row {
    pub id: String,
    pub kind: String,
    pub owner_user_id: String,
    pub sealed: Sealed,
    pub fingerprint: String,
    pub minted_at: String,
    pub expires_at: String,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
}

impl Row {
    pub fn is_expired(&self, now: OffsetDateTime) -> bool {
        parse_time(&self.expires_at).is_none_or(|e| e <= now)
    }
}

/// `GET …/claude-subscription` (§0.3). **There is no field that returns the
/// token, ever**: the struct has nowhere to put one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderCredentialView {
    pub kind: &'static str,
    pub present: Option<Present>,
    pub hosts: Vec<HostId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Present {
    pub status: String,
    pub fingerprint: String,
    pub minted_at: String,
    pub expires_at: String,
    pub days_left: i64,
    /// `null`, `30d`, `7d` or `expired`; serialized even when `null`.
    pub warn: Option<&'static str>,
}

impl Serialize for ProviderCredentialView {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut m = s.serialize_map(None)?;
        m.serialize_entry("kind", self.kind)?;
        m.serialize_entry("present", &self.present.is_some())?;
        if let Some(p) = &self.present {
            m.serialize_entry("status", &p.status)?;
            m.serialize_entry("fingerprint", &p.fingerprint)?;
            m.serialize_entry("minted_at", &p.minted_at)?;
            m.serialize_entry("expires_at", &p.expires_at)?;
            m.serialize_entry("days_left", &p.days_left)?;
            m.serialize_entry("warn", &p.warn)?;
        }
        m.serialize_entry("hosts", &self.hosts)?;
        m.end()
    }
}

impl ProviderCredentialView {
    pub fn of(row: Option<&Row>, hosts: Vec<HostId>, now: OffsetDateTime) -> Self {
        let present = row.map(|r| {
            let expires = parse_time(&r.expires_at).unwrap_or(now);
            Present {
                status: r.status.clone(),
                fingerprint: r.fingerprint.clone(),
                minted_at: r.minted_at.clone(),
                expires_at: r.expires_at.clone(),
                days_left: days_left(expires, now),
                warn: warn(expires, now),
            }
        });
        Self {
            kind: KIND,
            present,
            hosts,
        }
    }
}

/// Whether, and why not, the token can go to a host right now (§0.4 rules
/// 2 and 3; the provider and capability rules are the launch's).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Availability {
    NotInScope,
    Absent,
    NeedsResync,
    Expired,
    Deliverable,
}

impl Availability {
    pub fn of(db: &GatewayDb, keys: &Keyring, host_id: &str, now: OffsetDateTime) -> Result<Self> {
        if !db.in_provider_scope(CREDENTIAL_ID, host_id)? {
            return Ok(Self::NotInScope);
        }
        let Some(row) = db.provider_credential(KIND)? else {
            return Ok(Self::Absent);
        };
        Ok(
            if row.status != status::ACTIVE || !keys.has_key(&row.sealed.key_id) {
                Self::NeedsResync
            } else if row.is_expired(now) {
                Self::Expired
            } else {
                Self::Deliverable
            },
        )
    }
}

/// Rows sealed under a key no loaded key has. At boot they become
/// `needs_resync` (never a refusal to start); returns their fingerprints for
/// the `provider_credential_unreadable` audit.
pub fn mark_unopenable(db: &GatewayDb, keys: &Keyring, now: &str) -> Result<Vec<String>> {
    let mut out = Vec::new();
    if let Some(row) = db.provider_credential(KIND)?
        && row.status == status::ACTIVE
        && !keys.has_key(&row.sealed.key_id)
        && db.mark_provider_needs_resync(&row.id, now)?
    {
        out.push(row.fingerprint);
    }
    Ok(out)
}

const ROW_COLUMNS: &str = "id, kind, owner_user_id, key_id, nonce, ciphertext, fingerprint, \
    minted_at, expires_at, status, created_at, updated_at";

fn row_from(r: &rusqlite::Row<'_>) -> rusqlite::Result<Row> {
    Ok(Row {
        id: r.get(0)?,
        kind: r.get(1)?,
        owner_user_id: r.get(2)?,
        sealed: Sealed {
            key_id: r.get(3)?,
            nonce: r.get(4)?,
            ciphertext: r.get(5)?,
        },
        fingerprint: r.get(6)?,
        minted_at: r.get(7)?,
        expires_at: r.get(8)?,
        status: r.get(9)?,
        created_at: r.get(10)?,
        updated_at: r.get(11)?,
    })
}

impl GatewayDb {
    pub fn provider_credential(&self, kind: &str) -> Result<Option<Row>> {
        Ok(self
            .conn()
            .query_row(
                &format!("SELECT {ROW_COLUMNS} FROM provider_credentials WHERE kind = ?1"),
                params![kind],
                row_from,
            )
            .optional()?)
    }

    /// Creates or replaces in place: the ciphertext, fingerprint, dates and
    /// owner change, the status goes back to `active`, the scope rows stay.
    /// Returns the replaced row's fingerprint, if there was one.
    pub fn put_provider_credential(&self, row: &Row) -> Result<Option<String>> {
        let tx = self.conn().unchecked_transaction()?;
        let previous: Option<String> = tx
            .query_row(
                "SELECT fingerprint FROM provider_credentials WHERE id = ?1",
                params![row.id],
                |r| r.get(0),
            )
            .optional()?;
        tx.execute(
            "INSERT INTO provider_credentials
                 (id, kind, owner_user_id, key_id, nonce, ciphertext, fingerprint,
                  minted_at, expires_at, status, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
             ON CONFLICT(id) DO UPDATE SET
                 owner_user_id = excluded.owner_user_id, key_id = excluded.key_id,
                 nonce = excluded.nonce, ciphertext = excluded.ciphertext,
                 fingerprint = excluded.fingerprint, minted_at = excluded.minted_at,
                 expires_at = excluded.expires_at, status = excluded.status,
                 updated_at = excluded.updated_at",
            params![
                row.id,
                row.kind,
                row.owner_user_id,
                row.sealed.key_id,
                row.sealed.nonce,
                row.sealed.ciphertext,
                row.fingerprint,
                row.minted_at,
                row.expires_at,
                row.status,
                row.created_at,
                row.updated_at,
            ],
        )?;
        tx.commit()?;
        Ok(previous)
    }

    /// Deletes the row and its scope; whether a row went.
    pub fn delete_provider_credential(&self, id: &str) -> Result<bool> {
        let tx = self.conn().unchecked_transaction()?;
        let gone = tx.execute(
            "DELETE FROM provider_credentials WHERE id = ?1",
            params![id],
        )?;
        tx.execute(
            "DELETE FROM provider_credential_hosts WHERE credential_id = ?1",
            params![id],
        )?;
        tx.commit()?;
        Ok(gone > 0)
    }

    pub fn mark_provider_needs_resync(&self, id: &str, now: &str) -> Result<bool> {
        Ok(self.conn().execute(
            "UPDATE provider_credentials SET status = ?2, updated_at = ?3
             WHERE id = ?1 AND status != ?2",
            params![id, status::NEEDS_RESYNC, now],
        )? > 0)
    }

    /// The scoped hosts, sorted. **Empty means none** (the inverse of
    /// `connection_hosts`, on purpose: opt-in).
    pub fn provider_credential_hosts(&self, id: &str) -> Result<Vec<HostId>> {
        let mut stmt = self.conn().prepare(
            "SELECT host_id FROM provider_credential_hosts WHERE credential_id = ?1
             ORDER BY host_id",
        )?;
        let rows = stmt.query_map(params![id], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn in_provider_scope(&self, id: &str, host_id: &str) -> Result<bool> {
        Ok(self.conn().query_row(
            "SELECT EXISTS (SELECT 1 FROM provider_credential_hosts
                            WHERE credential_id = ?1 AND host_id = ?2)",
            params![id, host_id],
            |r| r.get(0),
        )?)
    }

    /// Replaces the scope; `hosts` sorted and deduplicated. Whether it changed.
    pub fn set_provider_credential_hosts(
        &self,
        id: &str,
        hosts: &[HostId],
        now: &str,
    ) -> Result<bool> {
        let tx = self.conn().unchecked_transaction()?;
        let current: Vec<String> = tx
            .prepare(
                "SELECT host_id FROM provider_credential_hosts WHERE credential_id = ?1
                 ORDER BY host_id",
            )?
            .query_map(params![id], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        if current == hosts {
            return Ok(false);
        }
        tx.execute(
            "DELETE FROM provider_credential_hosts WHERE credential_id = ?1",
            params![id],
        )?;
        for host in hosts {
            tx.execute(
                "INSERT OR IGNORE INTO provider_credential_hosts (credential_id, host_id, added_at)
                 VALUES (?1, ?2, ?3)",
                params![id, host, now],
            )?;
        }
        tx.commit()?;
        Ok(true)
    }

    /// A revoked host leaves every scope (§4: with "no rows = none" there is
    /// no widening hazard). Returns the credentials it was removed from.
    pub fn remove_provider_credential_host(&self, host_id: &str) -> Result<Vec<String>> {
        let tx = self.conn().unchecked_transaction()?;
        let ids: Vec<String> = tx
            .prepare("SELECT credential_id FROM provider_credential_hosts WHERE host_id = ?1")?
            .query_map(params![host_id], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        tx.execute(
            "DELETE FROM provider_credential_hosts WHERE host_id = ?1",
            params![host_id],
        )?;
        tx.commit()?;
        Ok(ids)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CANARY: &str = "sk-ant-oat01-CANARY_storm-124n-0123456789abcdef";

    fn t(s: &str) -> OffsetDateTime {
        parse_time(s).unwrap()
    }

    fn secret(s: &str) -> Secret {
        Secret::new(s.to_string())
    }

    fn sealed_row(keys: &Keyring, token: &str, minted: &str) -> Row {
        let token = secret(token);
        let minted = t(minted);
        Row {
            id: CREDENTIAL_ID.into(),
            kind: KIND.into(),
            owner_user_id: "usr_A".into(),
            sealed: keys
                .seal(CREDENTIAL_ID, KIND, token.expose().as_bytes())
                .unwrap(),
            fingerprint: fingerprint(&token),
            minted_at: format_time(minted),
            expires_at: format_time(expires_at(minted)),
            status: status::ACTIVE.into(),
            created_at: "t".into(),
            updated_at: "t".into(),
        }
    }

    fn gateway(dir: &std::path::Path) -> super::super::Gateway {
        super::super::Gateway::open(dir, "t").unwrap()
    }

    #[test]
    fn the_shape_rule_strips_paste_noise_and_refuses_anything_else() {
        let ok =
            normalize_token(&secret(&format!("\u{1b}[200~  {CANARY}\r\n\u{1b}[201~"))).unwrap();
        assert_eq!(ok.expose(), CANARY);
        let body20 = "a".repeat(20);
        let body400 = "a".repeat(400);
        assert!(normalize_token(&secret(&format!("{TOKEN_PREFIX}{body20}"))).is_ok());
        assert!(normalize_token(&secret(&format!("{TOKEN_PREFIX}{body400}"))).is_ok());
        for bad in [
            "".to_string(),
            "sk-ant-api03-CANARYxxxxxxxxxxxxxxxxxxxxxx".to_string(),
            format!("{TOKEN_PREFIX}{}", "a".repeat(19)),
            format!("{TOKEN_PREFIX}{}", "a".repeat(401)),
            format!("{TOKEN_PREFIX}CANARY with-space-0123456789"),
            format!("{TOKEN_PREFIX}CANARY\n0123456789abcdefghij"),
            format!("{TOKEN_PREFIX}CANARY.0123456789abcdefghij"),
            format!("Bearer {CANARY}"),
        ] {
            let err = normalize_token(&secret(&bad)).unwrap_err();
            assert_eq!(err.code, "bad_token_shape", "{bad:?}");
            assert!(
                !err.message.unwrap().contains("CANARY"),
                "the refusal echoes its input"
            );
        }
    }

    #[test]
    fn the_fingerprint_is_eight_hex_and_holds_no_token_character_run() {
        let fp = fingerprint(&secret(CANARY));
        assert_eq!(fp.len(), 8);
        assert!(fp.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_ne!(fp, fingerprint(&secret(&format!("{CANARY}x"))));
        assert!(!CANARY.contains(&fp));
    }

    #[test]
    fn expiry_is_a_year_from_minting_and_warns_at_30_and_7_days() {
        let minted = minted_at(Some("2026-10-16"), t("2026-10-17T00:00:00Z")).unwrap();
        assert_eq!(format_time(minted), "2026-10-16T00:00:00Z");
        let exp = expires_at(minted);
        assert_eq!(format_time(exp), "2027-10-16T00:00:00Z");
        assert_eq!(days_left(exp, minted), 365);
        let day = time::Duration::days(1);
        let sec = time::Duration::seconds(1);
        assert_eq!(warn(exp, exp - day * 30 - sec), None);
        assert_eq!(warn(exp, exp - day * 30), Some("30d"));
        assert_eq!(warn(exp, exp - day * 7 - sec), Some("30d"));
        assert_eq!(warn(exp, exp - day * 7), Some("7d"));
        assert_eq!(warn(exp, exp - sec), Some("7d"));
        assert_eq!(warn(exp, exp), Some("expired"));
        assert_eq!(days_left(exp, exp + day), 0);

        let now = t("2026-10-17T12:00:00Z");
        assert_eq!(minted_at(None, now).unwrap(), now);
        assert!(minted_at(Some("2026-10-17T12:30:00Z"), now).is_ok());
        for bad in [
            "2026-10-19",
            "2026-10-17T14:00:00Z",
            "yesterday",
            "16/10/2026",
        ] {
            assert_eq!(minted_at(Some(bad), now).unwrap_err().code, "bad_minted_at");
        }
    }

    #[test]
    fn a_sealed_row_opens_and_a_swapped_one_does_not() {
        let dir = tempdir::TempDir::new("storm-pc-seal").unwrap();
        let gw = gateway(dir.path());
        let row = sealed_row(&gw.keys, CANARY, "2026-10-16T09:00:00Z");
        let db = gw.store.lock().unwrap();
        assert!(db.put_provider_credential(&row).unwrap().is_none());
        let back = db.provider_credential(KIND).unwrap().unwrap();
        let opened = gw.keys.open(&back.id, &back.kind, &back.sealed).unwrap();
        assert_eq!(opened.expose(), CANARY.as_bytes());

        // A ciphertext sealed for a connection row, moved into this one, does
        // not open as the token: the AAD binds it to (id, kind).
        let foreign = gw.keys.seal("mcc_A", "static", CANARY.as_bytes()).unwrap();
        db.conn()
            .execute(
                "UPDATE provider_credentials SET nonce = ?1, ciphertext = ?2",
                params![foreign.nonce, foreign.ciphertext],
            )
            .unwrap();
        let swapped = db.provider_credential(KIND).unwrap().unwrap();
        assert!(gw.keys.open(CREDENTIAL_ID, KIND, &swapped.sealed).is_err());
        // And this row's ciphertext opens under no other (id, kind).
        assert!(gw.keys.open("mcc_A", KIND, &back.sealed).is_err());
        assert!(gw.keys.open(CREDENTIAL_ID, "static", &back.sealed).is_err());
    }

    #[test]
    fn a_replace_keeps_the_scope_and_resets_the_status_and_a_delete_takes_both() {
        let dir = tempdir::TempDir::new("storm-pc-replace").unwrap();
        let gw = gateway(dir.path());
        let db = gw.store.lock().unwrap();
        assert!(
            db.set_provider_credential_hosts(CREDENTIAL_ID, &["hst_A".into(), "hst_B".into()], "t")
                .unwrap()
        );
        assert!(
            !db.set_provider_credential_hosts(
                CREDENTIAL_ID,
                &["hst_A".into(), "hst_B".into()],
                "t"
            )
            .unwrap()
        );
        let first = sealed_row(&gw.keys, CANARY, "2026-10-16T09:00:00Z");
        db.put_provider_credential(&first).unwrap();
        db.mark_provider_needs_resync(CREDENTIAL_ID, "t").unwrap();
        let second = sealed_row(&gw.keys, &format!("{CANARY}2"), "2026-10-17T09:00:00Z");
        assert_eq!(
            db.put_provider_credential(&second).unwrap(),
            Some(first.fingerprint.clone())
        );
        let now = db.provider_credential(KIND).unwrap().unwrap();
        assert_eq!(now.status, status::ACTIVE);
        assert_eq!(now.fingerprint, second.fingerprint);
        assert_eq!(now.expires_at, "2027-10-17T09:00:00Z");
        assert_eq!(
            db.provider_credential_hosts(CREDENTIAL_ID).unwrap(),
            vec!["hst_A", "hst_B"]
        );

        assert_eq!(
            db.remove_provider_credential_host("hst_A").unwrap(),
            vec![CREDENTIAL_ID]
        );
        assert!(!db.in_provider_scope(CREDENTIAL_ID, "hst_A").unwrap());
        assert!(db.in_provider_scope(CREDENTIAL_ID, "hst_B").unwrap());

        assert!(db.delete_provider_credential(CREDENTIAL_ID).unwrap());
        assert!(db.provider_credential(KIND).unwrap().is_none());
        assert!(
            db.provider_credential_hosts(CREDENTIAL_ID)
                .unwrap()
                .is_empty()
        );
        assert!(!db.delete_provider_credential(CREDENTIAL_ID).unwrap());
    }

    #[test]
    fn availability_is_scope_first_then_presence_status_and_expiry() {
        let dir = tempdir::TempDir::new("storm-pc-avail").unwrap();
        let gw = gateway(dir.path());
        let db = gw.store.lock().unwrap();
        let now = t("2026-10-17T00:00:00Z");
        let at = |host| Availability::of(&db, &gw.keys, host, now).unwrap();
        assert_eq!(at("hst_A"), Availability::NotInScope);
        db.set_provider_credential_hosts(CREDENTIAL_ID, &["hst_A".into()], "t")
            .unwrap();
        assert_eq!(at("hst_A"), Availability::Absent);
        db.put_provider_credential(&sealed_row(&gw.keys, CANARY, "2026-10-16T00:00:00Z"))
            .unwrap();
        assert_eq!(at("hst_A"), Availability::Deliverable);
        assert_eq!(at("hst_B"), Availability::NotInScope);
        assert_eq!(
            Availability::of(&db, &gw.keys, "hst_A", t("2027-10-16T00:00:00Z")).unwrap(),
            Availability::Expired
        );
        db.mark_provider_needs_resync(CREDENTIAL_ID, "t").unwrap();
        assert_eq!(at("hst_A"), Availability::NeedsResync);
    }

    #[test]
    fn the_view_has_no_token_and_absent_has_only_kind_and_hosts() {
        let dir = tempdir::TempDir::new("storm-pc-view").unwrap();
        let gw = gateway(dir.path());
        let row = sealed_row(&gw.keys, CANARY, "2026-10-16T09:00:00Z");
        let view =
            ProviderCredentialView::of(Some(&row), vec!["hst_A".into()], t("2026-10-16T09:00:00Z"));
        let json = serde_json::to_value(&view).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "kind": KIND, "present": true, "status": "active",
                "fingerprint": row.fingerprint, "minted_at": "2026-10-16T09:00:00Z",
                "expires_at": "2027-10-16T09:00:00Z", "days_left": 365, "warn": null,
                "hosts": ["hst_A"],
            })
        );
        assert!(!json.to_string().contains("CANARY"));
        assert!(!format!("{view:?}").contains("CANARY"));
        let absent = ProviderCredentialView::of(None, Vec::new(), t("2026-10-16T09:00:00Z"));
        assert_eq!(
            serde_json::to_value(&absent).unwrap(),
            serde_json::json!({"kind": KIND, "present": false, "hosts": []})
        );
    }

    #[test]
    fn nothing_secret_is_debug_printed() {
        let s = secret(CANARY);
        assert_eq!(format!("{s:?}"), "Secret(<redacted>)");
        let body = PutBody::parse(
            serde_json::json!({"token": CANARY, "minted_at": "2026-10-16"})
                .to_string()
                .as_bytes(),
        )
        .unwrap();
        let printed = format!("{body:?} {:?}", normalize_token(&body.token));
        assert!(!printed.contains("CANARY"), "{printed}");
        let err = PutBody::parse(format!("{{\"token\": \"{CANARY}\", }}").as_bytes()).unwrap_err();
        assert!(!format!("{err:?}").contains("CANARY"));
        assert_eq!(err.code, "bad_request_body");
    }
}
