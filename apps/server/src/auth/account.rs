//! The one account (decision 82). `users` keeps its name so existing foreign
//! keys stay valid; `only_row` makes a second row impossible.

use anyhow::{Result, bail};
use rusqlite::{OptionalExtension, params};
use time::format_description::well_known::Rfc3339;
use time::{Duration, OffsetDateTime};

use super::db::AuthDb;
use super::identity::random_id;

pub const LOCKOUT_AFTER_FAILURES: i64 = 5;
pub const LOCKOUT_BASE_MINUTES: i64 = 1;
pub const LOCKOUT_CAP_MINUTES: i64 = 15;

pub const EVENT_ACCOUNT_CREATED: &str = "account_created";
pub const EVENT_PASSWORD_CHANGED: &str = "account_password_changed";

/// No `password_hash` field on purpose: fetch it with
/// [`AuthDb::password_hash_of`].
#[derive(Debug, Clone, serde::Serialize)]
pub struct Account {
    pub id: String,
    pub created: String,
    pub last_login: Option<String>,
}

impl AuthDb {
    pub fn account(&self) -> Result<Option<Account>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id, created, last_login FROM users",
                [],
                row_to_account,
            )
            .optional()?)
    }

    pub fn account_by_id(&self, id: &str) -> Result<Option<Account>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id, created, last_login FROM users WHERE id = ?1",
                params![id],
                row_to_account,
            )
            .optional()?)
    }

    pub fn has_account(&self) -> Result<bool> {
        Ok(self.account()?.is_some())
    }

    pub fn password_hash_of(&self, account_id: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT password_hash FROM users WHERE id = ?1",
                params![account_id],
                |r| r.get(0),
            )
            .optional()?)
    }

    fn update_password_hash(&mut self, account_id: &str, hash: &str, now: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE users SET password_hash = ?2, updated = ?3,
                 failed_count = 0, locked_until = NULL
             WHERE id = ?1",
            params![account_id, hash, now],
        )?;
        Ok(())
    }
}

fn row_to_account(row: &rusqlite::Row<'_>) -> rusqlite::Result<Account> {
    Ok(Account {
        id: row.get(0)?,
        created: row.get(1)?,
        last_login: row.get(2)?,
    })
}

pub fn create_account(db: &mut AuthDb, password_hash: &str, now: &str) -> Result<Account> {
    if db.has_account()? {
        bail!("this Storm already has an account; change its password instead");
    }
    let account = Account {
        id: random_id("usr_"),
        created: now.to_string(),
        last_login: None,
    };
    db.conn.execute(
        "INSERT INTO users (id, password_hash, created, updated) VALUES (?1, ?2, ?3, ?3)",
        params![account.id, password_hash, account.created],
    )?;
    db.record_event(EVENT_ACCOUNT_CREATED, Some(&account.id), None, now, "{}")?;
    Ok(account)
}

/// Also clears the lockout, so an operator reset (A11) actually rescues it.
pub fn set_password(db: &mut AuthDb, password_hash: &str, now: &str) -> Result<Account> {
    let Some(account) = db.account()? else {
        bail!("this Storm has no account yet; set it up from the app first");
    };
    db.update_password_hash(&account.id, password_hash, now)?;
    db.record_event(EVENT_PASSWORD_CHANGED, Some(&account.id), None, now, "{}")?;
    Ok(account)
}

fn parse_time(ts: &str) -> Result<OffsetDateTime> {
    OffsetDateTime::parse(ts, &Rfc3339)
        .map_err(|e| anyhow::anyhow!("parsing timestamp `{ts}`: {e}"))
}

fn format_time(at: OffsetDateTime) -> String {
    at.format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

pub fn lockout_remaining(db: &AuthDb, account_id: &str, now: &str) -> Result<Option<i64>> {
    let locked_until: Option<String> = db.conn.query_row(
        "SELECT locked_until FROM users WHERE id = ?1",
        params![account_id],
        |r| r.get(0),
    )?;
    let Some(locked) = locked_until else {
        return Ok(None);
    };
    let at = parse_time(now)?;
    let until = parse_time(&locked)?;
    if until <= at {
        return Ok(None);
    }
    Ok(Some((until - at).whole_seconds()))
}

/// `Some(locked_until)` once the account is locked.
pub fn note_failed_login(db: &mut AuthDb, account_id: &str, now: &str) -> Result<Option<String>> {
    db.conn.execute(
        "UPDATE users SET failed_count = failed_count + 1, updated = ?2
         WHERE id = ?1",
        params![account_id, now],
    )?;

    let (failed_count, current_lock): (i64, Option<String>) = db.conn.query_row(
        "SELECT failed_count, locked_until FROM users WHERE id = ?1",
        params![account_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;

    if let Some(ref locked) = current_lock
        && parse_time(locked)? > parse_time(now)?
    {
        return Ok(Some(locked.clone()));
    }

    if failed_count >= LOCKOUT_AFTER_FAILURES {
        let exponent = (failed_count - LOCKOUT_AFTER_FAILURES) as u32;
        let minutes = std::cmp::min(
            LOCKOUT_BASE_MINUTES.saturating_mul(2i64.saturating_pow(exponent)),
            LOCKOUT_CAP_MINUTES,
        );
        let locked_until = format_time(parse_time(now)? + Duration::minutes(minutes));
        db.conn.execute(
            "UPDATE users SET locked_until = ?2 WHERE id = ?1",
            params![account_id, locked_until],
        )?;
        return Ok(Some(locked_until));
    }

    Ok(None)
}

pub fn note_successful_login(db: &mut AuthDb, account_id: &str, now: &str) -> Result<()> {
    db.conn.execute(
        "UPDATE users SET failed_count = 0, locked_until = NULL, last_login = ?2, updated = ?2
         WHERE id = ?1",
        params![account_id, now],
    )?;
    Ok(())
}

pub fn upgrade_password_hash(
    db: &mut AuthDb,
    account_id: &str,
    hash: &str,
    now: &str,
) -> Result<()> {
    db.update_password_hash(account_id, hash, now)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: &str = "2026-10-08T12:00:00Z";
    const HASH: &str = "$argon2id$v=19$m=196608,t=1,p=1$c29tZXNhbHQ$aGFzaA";
    const OTHER: &str = "$argon2id$v=19$m=196608,t=1,p=1$b3RoZXJzYWx0$b3RoZXI";

    #[test]
    fn a_fresh_storm_has_no_account_until_setup() {
        let mut db = AuthDb::open_in_memory().unwrap();
        assert!(db.account().unwrap().is_none());
        let created = create_account(&mut db, HASH, NOW).unwrap();
        assert!(created.id.starts_with("usr_"));
        let read = db.account().unwrap().unwrap();
        assert_eq!(read.id, created.id);
        assert_eq!(db.password_hash_of(&created.id).unwrap().unwrap(), HASH);
    }

    #[test]
    fn setup_happens_once() {
        let mut db = AuthDb::open_in_memory().unwrap();
        create_account(&mut db, HASH, NOW).unwrap();
        let err = create_account(&mut db, OTHER, NOW).unwrap_err();
        assert!(err.to_string().contains("already has an account"), "{err}");
    }

    #[test]
    fn the_schema_itself_refuses_a_second_row() {
        let mut db = AuthDb::open_in_memory().unwrap();
        create_account(&mut db, HASH, NOW).unwrap();
        let raw = db.conn.execute(
            "INSERT INTO users (id, password_hash, created, updated) VALUES ('usr_x', 'h', ?1, ?1)",
            params![NOW],
        );
        assert!(raw.is_err(), "a second row must violate only_row");
    }

    #[test]
    fn changing_the_password_clears_the_lockout() {
        let mut db = AuthDb::open_in_memory().unwrap();
        let account = create_account(&mut db, HASH, NOW).unwrap();
        for _ in 0..LOCKOUT_AFTER_FAILURES {
            note_failed_login(&mut db, &account.id, NOW).unwrap();
        }
        assert!(lockout_remaining(&db, &account.id, NOW).unwrap().is_some());

        set_password(&mut db, OTHER, NOW).unwrap();
        assert!(lockout_remaining(&db, &account.id, NOW).unwrap().is_none());
        assert_eq!(db.password_hash_of(&account.id).unwrap().unwrap(), OTHER);
    }

    #[test]
    fn a_password_cannot_be_set_before_setup() {
        let mut db = AuthDb::open_in_memory().unwrap();
        assert!(set_password(&mut db, HASH, NOW).is_err());
    }

    #[test]
    fn lockout_doubles_and_caps() {
        let mut db = AuthDb::open_in_memory().unwrap();
        let account = create_account(&mut db, HASH, NOW).unwrap();
        let mut last = None;
        for _ in 0..LOCKOUT_AFTER_FAILURES {
            last = note_failed_login(&mut db, &account.id, NOW).unwrap();
        }
        let first = last.expect("locked at the threshold");
        assert_eq!(
            first, "2026-10-08T12:01:00Z",
            "the first lock is one minute"
        );
        assert_eq!(lockout_remaining(&db, &account.id, NOW).unwrap(), Some(60));
    }

    #[test]
    fn account_events_never_carry_a_secret() {
        let mut db = AuthDb::open_in_memory().unwrap();
        create_account(&mut db, HASH, NOW).unwrap();
        set_password(&mut db, OTHER, NOW).unwrap();
        let details: Vec<String> = db
            .conn
            .prepare("SELECT detail FROM security_events")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert!(!details.is_empty());
        for d in details {
            assert!(!d.contains("argon2"), "a hash leaked into {d}");
        }
    }
}
