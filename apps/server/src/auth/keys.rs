//! MCP keys: a credential a user mints for a machine (A14).
//!
//! Design: *Storm MCP Keys* in the personal vault. The short version:
//!
//! - It travels as an ordinary `Authorization: Bearer stk_…`, because most MCP
//!   clients can send nothing else (A14.1). The `stk_` prefix is what tells it
//!   apart from a session token.
//! - It is accepted on `/mcp` and nowhere else (A14.2). A session token lives
//!   in a platform keychain; a key lives in a plaintext config file on someone's
//!   disk, and the two do not deserve the same reach.
//! - It resolves to its **owner's** identity (A14.3). There is no separate
//!   principal here and there must not be — `authz.rs` records why the
//!   `Actor::Mcp` variant was removed, and a key with an identity of its own
//!   would put that mistake back under a new name.
//!
//! **A key carries the user's authority, and A14 adds no way to narrow it.**
//! Per-vault scoping is a real and useful thing to want; it is also the same
//! question the authorization model has to answer, so it is deferred there
//! rather than guessed at twice. An owner's key is therefore an owner-powered
//! bearer credential sitting in a config file. That is a documented,
//! deliberate property of this release — not an oversight — and it is the
//! strongest argument for the per-vault scoping work landing with RBAC.
//!
//! Hashing is blake3, not Argon2id: a key is 256 bits of randomness with
//! nothing to guess, so a memory-hard KDF would cost ~173 ms per MCP call to
//! defend against an attack that does not exist. See [`super::token`].

use anyhow::{Context, Result};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use super::db::AuthDb;
use super::token;
use super::users::{Status, User};

/// How long a key's `last_used` may lag before another write is worth it.
///
/// An MCP session is chatty — a tool call per keystroke in an agent loop — and
/// a write per call would be a write per keystroke. Same throttle the session
/// path uses, for the same reason.
pub const LAST_USED_THROTTLE_SECS: i64 = 60;

/// The most live keys one user may hold at once.
///
/// A bound on a table that a script could otherwise fill, not a statement about
/// how many machines someone may own — revoke one to mint another past this.
pub const MAX_LIVE_KEYS_PER_USER: i64 = 32;

/// Longest accepted key name.
pub const MAX_NAME_CHARS: usize = 100;

pub const EVENT_KEY_CREATED: &str = "api_key_created";
pub const EVENT_KEY_REVOKED: &str = "api_key_revoked";
pub const EVENT_KEY_REJECTED: &str = "api_key_rejected";

/// A key as stored. **Never carries the secret** — that exists once, in the
/// response that created it.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ApiKey {
    pub id: String,
    pub user_id: String,
    pub name: String,
    pub created: String,
    pub created_via: Option<String>,
    pub expires: Option<String>,
    pub last_used: Option<String>,
    pub revoked: Option<String>,
    pub revoked_reason: Option<String>,
}

impl ApiKey {
    pub fn is_revoked(&self) -> bool {
        self.revoked.is_some()
    }
}

/// A key that authenticated, and the user behind it.
#[derive(Debug, Clone)]
pub struct AuthenticatedKey {
    pub key: ApiKey,
    pub user: User,
}

/// Why a key that looked live is not usable.
///
/// Separate from an internal error, the same way `sessions` splits them: a
/// refusal is an answer, not a failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyFailure {
    /// No such key — and also what an expired-and-swept key looks like.
    Unknown,
    Revoked,
    Expired,
    /// The key is fine; its owner cannot log in.
    UserDisabled,
}

impl KeyFailure {
    /// The wire code. **Deliberately coarse.** `unknown`, `revoked` and
    /// `expired` are all answered to the client as one refusal by the
    /// middleware — telling an unauthenticated caller *which* of those it hit
    /// distinguishes "this key never existed" from "this key existed and was
    /// revoked", which is free reconnaissance. The distinction is kept here so
    /// the **audit row** can be specific.
    pub fn code(&self) -> &'static str {
        match self {
            KeyFailure::Unknown => "unknown",
            KeyFailure::Revoked => "revoked",
            KeyFailure::Expired => "expired",
            KeyFailure::UserDisabled => "user_disabled",
        }
    }
}

#[derive(Debug)]
pub enum KeyError {
    Refused(KeyFailure),
    Internal(anyhow::Error),
}

impl From<anyhow::Error> for KeyError {
    fn from(e: anyhow::Error) -> Self {
        KeyError::Internal(e)
    }
}

fn parse_time(ts: &str) -> Result<OffsetDateTime> {
    OffsetDateTime::parse(ts, &Rfc3339).with_context(|| format!("parsing timestamp `{ts}`"))
}

/// Rejects a name that is empty or absurd.
pub fn validate_name(name: &str) -> std::result::Result<(), String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("a key needs a name — it is how you recognise what to revoke".into());
    }
    if trimmed.chars().count() > MAX_NAME_CHARS {
        return Err(format!("a key name is at most {MAX_NAME_CHARS} characters"));
    }
    Ok(())
}

/// Mints a key for `user_id`, returning the record and the **plaintext once**.
///
/// The caller shows the plaintext to the person who asked and then drops it.
/// Nothing stores it: not this function, not the database, not the client.
pub fn create(
    db: &mut AuthDb,
    user_id: &str,
    name: &str,
    created_via: Option<&str>,
    expires: Option<&str>,
    now: &str,
) -> Result<(ApiKey, String)> {
    validate_name(name).map_err(|e| anyhow::anyhow!(e))?;

    let live = db.live_api_key_count(user_id)?;
    if live >= MAX_LIVE_KEYS_PER_USER {
        anyhow::bail!(
            "this account already holds {MAX_LIVE_KEYS_PER_USER} live keys; revoke one first"
        );
    }

    // An expiry in the past would mint a key that cannot be used, which is a
    // confusing way to fail. Refuse it where the intent is still visible.
    if let Some(at) = expires
        && parse_time(at)? <= parse_time(now)?
    {
        anyhow::bail!("that expiry is already in the past");
    }

    let secret = token::mint(token::KEY_PREFIX);
    let id = format!("key_{}", uuid::Uuid::new_v4());
    db.insert_api_key(
        &id,
        user_id,
        name.trim(),
        &token::hash(&secret),
        now,
        created_via,
        expires,
    )?;

    let key = db
        .api_key_by_id(&id)?
        .context("the key just inserted is missing")?;

    // **The audit row is written here, beside the act**, the way `users.rs`
    // writes its own — an audit trail assembled by callers is one a new caller
    // can forget to write. The detail names the key and never the secret;
    // `security_events` has never held one and a test keeps it that way.
    db.record_event(
        EVENT_KEY_CREATED,
        Some(user_id),
        created_via,
        now,
        &format!(
            r#"{{"key_id":"{}","name":{}}}"#,
            key.id,
            serde_json::Value::String(key.name.clone())
        ),
    )?;

    Ok((key, secret))
}

/// Resolves a presented `stk_…` secret to its key and owner.
///
/// **Four refusal grounds, and the owner's status is one of them.** A key that
/// outlives its owner's ability to log in is a back door around `set_status`:
/// disabling an account has to disable everything that speaks for it, and
/// SQLite cannot express that any more than it can express the last-owner rule.
pub fn authenticate(
    db: &mut AuthDb,
    secret: &str,
    now: &str,
) -> std::result::Result<AuthenticatedKey, KeyError> {
    let key = db
        .api_key_by_hash(&token::hash(secret))
        .map_err(KeyError::Internal)?
        .ok_or(KeyError::Refused(KeyFailure::Unknown))?;

    if key.is_revoked() {
        return Err(KeyError::Refused(KeyFailure::Revoked));
    }

    let at = parse_time(now).map_err(KeyError::Internal)?;
    if let Some(expires) = &key.expires
        && parse_time(expires).map_err(KeyError::Internal)? <= at
    {
        return Err(KeyError::Refused(KeyFailure::Expired));
    }

    let user = db
        .user_by_id(&key.user_id)
        .map_err(KeyError::Internal)?
        .ok_or(KeyError::Refused(KeyFailure::Unknown))?;
    if user.status == Status::Disabled {
        return Err(KeyError::Refused(KeyFailure::UserDisabled));
    }

    let due = match &key.last_used {
        None => true,
        Some(last) => {
            (at - parse_time(last).map_err(KeyError::Internal)?).whole_seconds()
                >= LAST_USED_THROTTLE_SECS
        }
    };
    if due {
        db.touch_api_key(&key.id, now).map_err(KeyError::Internal)?;
    }

    Ok(AuthenticatedKey { key, user })
}

/// Revokes a key. Idempotent: revoking an already-revoked key is not an error.
///
/// `by` is who asked, for the audit row — not necessarily the owner, since an
/// owner may revoke anyone's key (A14).
pub fn revoke(
    db: &mut AuthDb,
    key_id: &str,
    by: Option<&str>,
    reason: &str,
    now: &str,
) -> Result<bool> {
    let changed = db.revoke_api_key(key_id, now, reason)?;
    if changed > 0 {
        db.record_event(
            EVENT_KEY_REVOKED,
            by,
            None,
            now,
            &format!(r#"{{"key_id":"{key_id}"}}"#),
        )?;
    }
    Ok(changed > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::users::{NewUser, Role, create_user};

    const NOW: &str = "2026-08-19T12:00:00Z";

    fn db_with_user() -> (AuthDb, User) {
        let mut db = AuthDb::open_in_memory().unwrap();
        let user = create_user(
            &mut db,
            NewUser {
                username: "dewansh",
                display_name: None,
                password_hash: "x",
                role: Role::Owner,
            },
            NOW,
        )
        .unwrap();
        (db, user)
    }

    #[test]
    fn a_minted_key_authenticates_as_its_owner() {
        let (mut db, user) = db_with_user();
        let (key, secret) = create(&mut db, &user.id, "laptop", None, None, NOW).unwrap();

        assert!(secret.starts_with(token::KEY_PREFIX), "{secret}");
        let authed = authenticate(&mut db, &secret, NOW).unwrap();
        assert_eq!(authed.user.id, user.id);
        assert_eq!(authed.key.id, key.id);
        assert_eq!(
            authed.user.role,
            Role::Owner,
            "a key carries its owner's role"
        );
    }

    #[test]
    fn the_plaintext_is_never_stored() {
        // A14.5. If the secret were recoverable from the database, "shown once"
        // would be decoration — so this greps the whole file for it.
        let (mut db, user) = db_with_user();
        let (_, secret) = create(&mut db, &user.id, "laptop", None, None, NOW).unwrap();

        let mut found = false;
        let mut stmt = db.conn.prepare("SELECT * FROM api_keys").unwrap();
        let cols = stmt.column_count();
        let mut rows = stmt.query([]).unwrap();
        while let Some(row) = rows.next().unwrap() {
            for i in 0..cols {
                if let Ok(v) = row.get::<_, String>(i)
                    && v.contains(&secret)
                {
                    found = true;
                }
            }
        }
        assert!(!found, "the plaintext key must not be in the database");
    }

    #[test]
    fn a_revoked_key_cannot_authenticate() {
        let (mut db, user) = db_with_user();
        let (key, secret) = create(&mut db, &user.id, "laptop", None, None, NOW).unwrap();

        assert!(revoke(&mut db, &key.id, None, "test", NOW).unwrap());
        assert!(matches!(
            authenticate(&mut db, &secret, NOW),
            Err(KeyError::Refused(KeyFailure::Revoked))
        ));
    }

    #[test]
    fn an_expired_key_cannot_authenticate() {
        let (mut db, user) = db_with_user();
        let (_, secret) = create(
            &mut db,
            &user.id,
            "laptop",
            None,
            Some("2026-08-20T12:00:00Z"),
            NOW,
        )
        .unwrap();

        // Still good the moment before.
        assert!(authenticate(&mut db, &secret, "2026-08-20T11:59:59Z").is_ok());
        assert!(matches!(
            authenticate(&mut db, &secret, "2026-08-20T12:00:01Z"),
            Err(KeyError::Refused(KeyFailure::Expired))
        ));
    }

    #[test]
    fn a_disabled_owner_disables_the_key() {
        // The back door this closes: disabling an account has to disable
        // everything that speaks for it, or `set_status` is advisory.
        let (mut db, user) = db_with_user();
        let (_, secret) = create(&mut db, &user.id, "laptop", None, None, NOW).unwrap();
        assert!(authenticate(&mut db, &secret, NOW).is_ok());

        // A second owner, because the last *active* owner cannot be disabled —
        // the rule that exists so a server cannot be left unadministrable.
        create_user(
            &mut db,
            NewUser {
                username: "second",
                display_name: None,
                password_hash: "x",
                role: Role::Owner,
            },
            NOW,
        )
        .unwrap();
        crate::auth::users::set_status(&mut db, "dewansh", Status::Disabled, NOW).unwrap();
        assert!(matches!(
            authenticate(&mut db, &secret, NOW),
            Err(KeyError::Refused(KeyFailure::UserDisabled))
        ));
    }

    #[test]
    fn deleting_the_user_takes_the_keys() {
        // The `ON DELETE CASCADE` half of "a key belongs to a user".
        let (mut db, user) = db_with_user();
        // A second owner, so the last-active-owner rule permits the delete.
        create_user(
            &mut db,
            NewUser {
                username: "second",
                display_name: None,
                password_hash: "x",
                role: Role::Owner,
            },
            NOW,
        )
        .unwrap();
        let (_, secret) = create(&mut db, &user.id, "laptop", None, None, NOW).unwrap();

        crate::auth::users::delete_user(&mut db, "dewansh", NOW).unwrap();
        assert!(matches!(
            authenticate(&mut db, &secret, NOW),
            Err(KeyError::Refused(KeyFailure::Unknown))
        ));
    }

    #[test]
    fn one_users_key_never_resolves_to_another_user() {
        let (mut db, first) = db_with_user();
        let second = create_user(
            &mut db,
            NewUser {
                username: "second",
                display_name: None,
                password_hash: "x",
                role: Role::Member,
            },
            NOW,
        )
        .unwrap();

        let (_, a) = create(&mut db, &first.id, "a", None, None, NOW).unwrap();
        let (_, b) = create(&mut db, &second.id, "b", None, None, NOW).unwrap();

        assert_eq!(authenticate(&mut db, &a, NOW).unwrap().user.id, first.id);
        let authed_b = authenticate(&mut db, &b, NOW).unwrap();
        assert_eq!(authed_b.user.id, second.id);
        assert_eq!(
            authed_b.user.role,
            Role::Member,
            "a member's key must not carry an owner's role"
        );
    }

    #[test]
    fn an_unknown_secret_is_refused() {
        let (mut db, _) = db_with_user();
        assert!(matches!(
            authenticate(&mut db, "stk_nonsense", NOW),
            Err(KeyError::Refused(KeyFailure::Unknown))
        ));
    }

    #[test]
    fn last_used_is_throttled() {
        let (mut db, user) = db_with_user();
        let (key, secret) = create(&mut db, &user.id, "laptop", None, None, NOW).unwrap();

        authenticate(&mut db, &secret, NOW).unwrap();
        let first = db.api_key_by_id(&key.id).unwrap().unwrap().last_used;
        assert_eq!(first.as_deref(), Some(NOW));

        // Inside the window: not written again.
        authenticate(&mut db, &secret, "2026-08-19T12:00:30Z").unwrap();
        assert_eq!(
            db.api_key_by_id(&key.id).unwrap().unwrap().last_used,
            first,
            "a write inside the throttle window defeats the throttle"
        );

        // Past it: written.
        authenticate(&mut db, &secret, "2026-08-19T12:01:30Z").unwrap();
        assert_eq!(
            db.api_key_by_id(&key.id)
                .unwrap()
                .unwrap()
                .last_used
                .as_deref(),
            Some("2026-08-19T12:01:30Z")
        );
    }

    #[test]
    fn the_per_user_cap_counts_only_live_keys() {
        let (mut db, user) = db_with_user();
        let mut ids = Vec::new();
        for i in 0..MAX_LIVE_KEYS_PER_USER {
            let (k, _) = create(&mut db, &user.id, &format!("k{i}"), None, None, NOW).unwrap();
            ids.push(k.id);
        }
        assert!(
            create(&mut db, &user.id, "one too many", None, None, NOW).is_err(),
            "the cap must actually refuse"
        );

        revoke(&mut db, &ids[0], None, "making room", NOW).unwrap();
        assert!(
            create(&mut db, &user.id, "now there is room", None, None, NOW).is_ok(),
            "revoking must free a slot, or the cap is a lifetime limit"
        );
    }

    #[test]
    fn a_name_is_required_and_bounded() {
        let (mut db, user) = db_with_user();
        assert!(create(&mut db, &user.id, "   ", None, None, NOW).is_err());
        let long = "x".repeat(MAX_NAME_CHARS + 1);
        assert!(create(&mut db, &user.id, &long, None, None, NOW).is_err());
    }

    #[test]
    fn an_expiry_in_the_past_is_refused_rather_than_minted_dead() {
        let (mut db, user) = db_with_user();
        assert!(
            create(
                &mut db,
                &user.id,
                "laptop",
                None,
                Some("2020-01-01T00:00:00Z"),
                NOW
            )
            .is_err()
        );
    }

    #[test]
    fn minting_and_revoking_are_audited_and_never_hold_the_secret() {
        // `security_events` has never contained a secret and a test asserts it
        // for every administrative act. Minting a key is one, and it is the
        // act with a plaintext credential closest to hand.
        let (mut db, user) = db_with_user();
        let (key, secret) = create(&mut db, &user.id, "laptop", None, None, NOW).unwrap();
        revoke(&mut db, &key.id, Some(&user.id), "done with it", NOW).unwrap();

        let mut stmt = db
            .conn
            .prepare("SELECT kind, detail FROM security_events ORDER BY seq")
            .unwrap();
        let rows: Vec<(String, Option<String>)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();

        let kinds: Vec<&str> = rows.iter().map(|(k, _)| k.as_str()).collect();
        assert!(kinds.contains(&EVENT_KEY_CREATED), "{kinds:?}");
        assert!(kinds.contains(&EVENT_KEY_REVOKED), "{kinds:?}");

        for (kind, detail) in &rows {
            let detail = detail.clone().unwrap_or_default();
            assert!(
                !detail.contains(&secret),
                "{kind} wrote a key secret into security_events: {detail}"
            );
            // Nor the stored hash, which is a verifier and equally not for logs.
            assert!(
                !detail.contains(&hex(&token::hash(&secret))),
                "{kind} wrote the key hash into security_events: {detail}"
            );
        }
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn revoking_is_idempotent() {
        let (mut db, user) = db_with_user();
        let (key, _) = create(&mut db, &user.id, "laptop", None, None, NOW).unwrap();
        assert!(revoke(&mut db, &key.id, None, "first", NOW).unwrap());
        assert!(
            !revoke(&mut db, &key.id, None, "again", NOW).unwrap(),
            "a second revoke changes nothing and must not error"
        );
    }
}
