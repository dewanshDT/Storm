//! Local user accounts: creation, roles, status, and the invariants that keep a
//! server administrable.
//!
//! Users are **server-local** (R2). `(server_id, user_id)` is the identity
//! everywhere; a bare user id means nothing, because the same person on two
//! Storm servers is two accounts that share only a habit of using the same
//! name.
//!
//! Two rules here are enforcement rather than validation, and both exist to stop
//! a server becoming unadministrable by an ordinary-looking edit:
//!
//! - **The first account is an owner.** A server whose only user is a member has
//!   nobody who can create the second one.
//! - **The last active owner cannot be deleted, disabled or demoted.** Not a
//!   courtesy check — SQLite cannot express it, and the operator recovery path
//!   (A11, `storm-server passwd`) assumes an account still exists to recover
//!   *to*.
//!
//! Password hashing lives in [`super::password`], deliberately: this module
//! never sees a plaintext password, and takes an already-hashed PHC string.

use anyhow::{Result, bail};
use rusqlite::{OptionalExtension, params};

use super::db::AuthDb;
use super::identity::random_id;

pub const MIN_USERNAME_CHARS: usize = 3;
pub const MAX_USERNAME_CHARS: usize = 32;

/// Security event kinds written by this module. Every one of them is an
/// administrative act on an account, which is exactly what an operator needs a
/// trail of when something looks wrong.
pub const EVENT_USER_CREATED: &str = "user_created";
pub const EVENT_PASSWORD_CHANGED: &str = "user_password_changed";
pub const EVENT_ROLE_CHANGED: &str = "user_role_changed";
pub const EVENT_DISABLED: &str = "user_disabled";
pub const EVENT_ENABLED: &str = "user_enabled";
pub const EVENT_DELETED: &str = "user_deleted";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Owner,
    Admin,
    Member,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Owner => "owner",
            Role::Admin => "admin",
            Role::Member => "member",
        }
    }

    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "owner" => Ok(Role::Owner),
            "admin" => Ok(Role::Admin),
            "member" => Ok(Role::Member),
            other => bail!("unknown role `{other}`; expected owner, admin or member"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Active,
    Disabled,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Active => "active",
            Status::Disabled => "disabled",
        }
    }

    fn parse(value: &str) -> Result<Self> {
        match value {
            "active" => Ok(Status::Active),
            "disabled" => Ok(Status::Disabled),
            other => bail!("unknown user status `{other}`"),
        }
    }
}

/// A user as everything outside this module sees one.
///
/// **There is no `password_hash` field, on purpose.** A struct that carries the
/// hash ends up in a log line or a JSON body eventually; the hash is fetched by
/// [`AuthDb::password_hash_of`], a call whose name says what it is doing.
#[derive(Debug, Clone)]
pub struct User {
    pub id: String,
    pub username: String,
    pub display_name: Option<String>,
    pub role: Role,
    pub status: Status,
    pub created: String,
    pub last_login: Option<String>,
}

/// Usernames are ASCII, and that is a decision rather than an oversight.
///
/// Uniqueness is decided by the casefold, so the fold has to be unambiguous.
/// Full Unicode brings case rules that differ by locale and homoglyphs that make
/// two visually identical usernames distinct rows — on a server where the user
/// list is how you tell people apart, `dewansh` and a Cyrillic-`е` lookalike
/// being different accounts is a security bug, not a feature. `display_name` is
/// unrestricted, so nobody is stuck with an ASCII name on screen.
pub fn validate_username(name: &str) -> std::result::Result<(), String> {
    let chars = name.chars().count();
    if !(MIN_USERNAME_CHARS..=MAX_USERNAME_CHARS).contains(&chars) {
        return Err(format!(
            "username must be {MIN_USERNAME_CHARS}–{MAX_USERNAME_CHARS} characters; `{name}` is {chars}"
        ));
    }
    if !name
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphanumeric())
    {
        return Err("username must start with a letter or a digit".to_string());
    }
    if let Some(bad) = name
        .chars()
        .find(|c| !(c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')))
    {
        return Err(format!(
            "username may contain letters, digits, dot, underscore and hyphen; `{bad}` is not allowed"
        ));
    }
    Ok(())
}

/// The casefold uniqueness is decided on. ASCII-only by [`validate_username`],
/// so this is total and locale-independent.
pub fn fold_username(name: &str) -> String {
    name.to_ascii_lowercase()
}

/// What [`create_user`] needs. The password arrives already hashed.
pub struct NewUser<'a> {
    pub username: &'a str,
    pub display_name: Option<&'a str>,
    pub password_hash: &'a str,
    pub role: Role,
}

impl AuthDb {
    pub fn user_count(&self) -> Result<i64> {
        Ok(self
            .conn
            .query_row("SELECT COUNT(*) FROM users", [], |r| r.get(0))?)
    }

    /// Owners who could actually log in and administer the server.
    ///
    /// Disabled owners do not count: an account that cannot log in cannot
    /// administer, so leaving only disabled owners is the same lockout as
    /// leaving none.
    pub fn active_owner_count(&self) -> Result<i64> {
        Ok(self.conn.query_row(
            "SELECT COUNT(*) FROM users WHERE role = 'owner' AND status = 'active'",
            [],
            |r| r.get(0),
        )?)
    }

    fn active_owner_count_excluding(&self, user_id: &str) -> Result<i64> {
        Ok(self.conn.query_row(
            "SELECT COUNT(*) FROM users
             WHERE role = 'owner' AND status = 'active' AND id <> ?1",
            params![user_id],
            |r| r.get(0),
        )?)
    }

    pub fn list_users(&self) -> Result<Vec<User>> {
        // `updated` is stamped by every write but read by nothing yet — it is an
        // audit field for whoever opens auth.db with sqlite3, so it is not in
        // `User` and should not be dropped from the table either.
        let mut stmt = self.conn.prepare(
            "SELECT id, username, display_name, role, status, created, last_login
             FROM users ORDER BY created",
        )?;
        let rows = stmt.query_map([], row_to_user)?;
        let mut users = Vec::new();
        for row in rows {
            users.push(row??);
        }
        Ok(users)
    }

    /// Looks a user up by name, casefolded — `Dewansh` and `dewansh` are the
    /// same account.
    pub fn find_user(&self, username: &str) -> Result<Option<User>> {
        let found = self
            .conn
            .query_row(
                "SELECT id, username, display_name, role, status, created, last_login
                 FROM users WHERE username_fold = ?1",
                params![fold_username(username)],
                row_to_user,
            )
            .optional()?;
        found.transpose()
    }

    /// The stored PHC string. Named for what it returns so it cannot be reached
    /// for by accident.
    pub fn password_hash_of(&self, user_id: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT password_hash FROM users WHERE id = ?1",
                params![user_id],
                |r| r.get(0),
            )
            .optional()?)
    }

    fn insert_user_row(&mut self, user: &User, password_hash: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO users
                 (id, username, username_fold, display_name, password_hash,
                  role, status, created, updated)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
            params![
                user.id,
                user.username,
                fold_username(&user.username),
                user.display_name,
                password_hash,
                user.role.as_str(),
                user.status.as_str(),
                user.created,
            ],
        )?;
        Ok(())
    }

    fn update_password_hash(&mut self, user_id: &str, hash: &str, now: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE users SET password_hash = ?2, updated = ?3,
                 failed_count = 0, locked_until = NULL
             WHERE id = ?1",
            params![user_id, hash, now],
        )?;
        Ok(())
    }

    fn update_role(&mut self, user_id: &str, role: Role, now: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE users SET role = ?2, updated = ?3 WHERE id = ?1",
            params![user_id, role.as_str(), now],
        )?;
        Ok(())
    }

    fn update_status(&mut self, user_id: &str, status: Status, now: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE users SET status = ?2, updated = ?3 WHERE id = ?1",
            params![user_id, status.as_str(), now],
        )?;
        Ok(())
    }

    fn delete_user_row(&mut self, user_id: &str) -> Result<()> {
        // Sessions and vault grants carry ON DELETE CASCADE, and `foreign_keys`
        // is ON for every connection — without both, deleting a user would
        // leave live sessions pointing at nobody.
        self.conn
            .execute("DELETE FROM users WHERE id = ?1", params![user_id])?;
        Ok(())
    }
}

fn row_to_user(row: &rusqlite::Row<'_>) -> rusqlite::Result<Result<User>> {
    let role: String = row.get(3)?;
    let status: String = row.get(4)?;
    Ok((|| {
        Ok(User {
            id: row.get(0)?,
            username: row.get(1)?,
            display_name: row.get(2)?,
            role: Role::parse(&role)?,
            status: Status::parse(&status)?,
            created: row.get(5)?,
            last_login: row.get(6)?,
        })
    })())
}

/// Creates an account.
///
/// The first one is forced to be an owner rather than defaulted to it: a server
/// whose only account is a member has nobody who can promote anyone, and the
/// only way out is editing SQLite by hand.
pub fn create_user(db: &mut AuthDb, spec: NewUser<'_>, now: &str) -> Result<User> {
    if let Err(why) = validate_username(spec.username) {
        bail!(why);
    }
    if db.find_user(spec.username)?.is_some() {
        bail!(
            "a user named `{}` already exists (usernames are compared case-insensitively)",
            spec.username
        );
    }
    let first = db.user_count()? == 0;
    if first && spec.role != Role::Owner {
        bail!(
            "the first user must be an owner — a server whose only account is a \
             `{}` has nobody who can create the next one",
            spec.role.as_str()
        );
    }

    let user = User {
        id: random_id("usr_"),
        username: spec.username.to_string(),
        display_name: spec.display_name.map(str::to_string),
        role: spec.role,
        status: Status::Active,
        created: now.to_string(),
        last_login: None,
    };
    db.insert_user_row(&user, spec.password_hash)?;
    db.record_event(
        EVENT_USER_CREATED,
        Some(&user.id),
        now,
        &format!(
            r#"{{"username":{:?},"role":"{}"}}"#,
            user.username,
            user.role.as_str()
        ),
    )?;
    Ok(user)
}

/// Replaces a password. The caller hashes; this never sees plaintext.
pub fn set_password(
    db: &mut AuthDb,
    username: &str,
    password_hash: &str,
    now: &str,
) -> Result<User> {
    let user = require_user(db, username)?;
    db.update_password_hash(&user.id, password_hash, now)?;
    // Clearing the lockout counters is part of the change, not a side effect:
    // A11 exists so an operator can rescue an account, and handing back one
    // that is still locked out would only half-work.
    db.record_event(EVENT_PASSWORD_CHANGED, Some(&user.id), now, "{}")?;
    Ok(user)
}

pub fn set_role(db: &mut AuthDb, username: &str, role: Role, now: &str) -> Result<User> {
    let user = require_user(db, username)?;
    if user.role == role {
        return Ok(user);
    }
    if role != Role::Owner {
        refuse_if_last_owner(db, &user, "demoted")?;
    }
    db.update_role(&user.id, role, now)?;
    db.record_event(
        EVENT_ROLE_CHANGED,
        Some(&user.id),
        now,
        &format!(
            r#"{{"from":"{}","to":"{}"}}"#,
            user.role.as_str(),
            role.as_str()
        ),
    )?;
    Ok(user)
}

pub fn set_status(db: &mut AuthDb, username: &str, status: Status, now: &str) -> Result<User> {
    let user = require_user(db, username)?;
    if status == Status::Disabled {
        refuse_if_last_owner(db, &user, "disabled")?;
    }
    db.update_status(&user.id, status, now)?;
    let kind = match status {
        Status::Disabled => EVENT_DISABLED,
        Status::Active => EVENT_ENABLED,
    };
    db.record_event(kind, Some(&user.id), now, "{}")?;
    Ok(user)
}

pub fn delete_user(db: &mut AuthDb, username: &str, now: &str) -> Result<User> {
    let user = require_user(db, username)?;
    refuse_if_last_owner(db, &user, "deleted")?;
    db.delete_user_row(&user.id)?;
    // The event outlives the row deliberately: `user_id` has no foreign key, so
    // the trail of who was removed survives the removal.
    db.record_event(
        EVENT_DELETED,
        Some(&user.id),
        now,
        &format!(r#"{{"username":{:?}}}"#, user.username),
    )?;
    Ok(user)
}

fn require_user(db: &AuthDb, username: &str) -> Result<User> {
    match db.find_user(username)? {
        Some(user) => Ok(user),
        None => bail!("no user named `{username}`"),
    }
}

/// The last-owner guard, in one place so every path enforces the same rule.
fn refuse_if_last_owner(db: &AuthDb, user: &User, verb: &str) -> Result<()> {
    if user.role != Role::Owner || user.status != Status::Active {
        return Ok(());
    }
    if db.active_owner_count_excluding(&user.id)? == 0 {
        bail!(
            "`{}` is the only active owner and cannot be {verb}. \
             Promote another user to owner first.",
            user.username
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: &str = "2026-08-16T12:00:00Z";
    const HASH: &str = "$argon2id$v=19$m=196608,t=1,p=1$c29tZXNhbHQ$aGFzaA";

    fn add(db: &mut AuthDb, username: &str, role: Role) -> Result<User> {
        create_user(
            db,
            NewUser {
                username,
                display_name: None,
                password_hash: HASH,
                role,
            },
            NOW,
        )
    }

    fn db_with_owner() -> AuthDb {
        let mut db = AuthDb::open_in_memory().unwrap();
        add(&mut db, "owner", Role::Owner).unwrap();
        db
    }

    #[test]
    fn a_username_must_be_ascii_and_reasonably_shaped() {
        assert!(validate_username("dewansh").is_ok());
        assert!(validate_username("de.wansh_1-x").is_ok());
        assert!(validate_username("ab").is_err(), "too short");
        assert!(validate_username(&"a".repeat(33)).is_err(), "too long");
        assert!(validate_username("_leading").is_err());
        assert!(validate_username("has space").is_err());
        assert!(validate_username("has@sign").is_err());
        // The homoglyph the ASCII rule exists for: Cyrillic е, visually
        // identical to the ASCII one.
        assert!(validate_username("d\u{0435}wansh").is_err());
    }

    #[test]
    fn usernames_are_unique_case_insensitively() {
        let mut db = db_with_owner();
        add(&mut db, "Dewansh", Role::Member).unwrap();
        let err = add(&mut db, "dewansh", Role::Member).unwrap_err();
        assert!(err.to_string().contains("already exists"), "{err}");
        // And the fold is what finds it, whichever case is asked for.
        assert_eq!(
            db.find_user("DEWANSH").unwrap().unwrap().username,
            "Dewansh"
        );
    }

    #[test]
    fn the_first_user_must_be_an_owner() {
        let mut db = AuthDb::open_in_memory().unwrap();
        let err = add(&mut db, "member", Role::Member).unwrap_err();
        assert!(
            err.to_string().contains("first user must be an owner"),
            "{err}"
        );
        assert_eq!(db.user_count().unwrap(), 0, "nothing was written");

        add(&mut db, "boss", Role::Owner).unwrap();
        // Only the *first* is forced; after that any role is fine.
        add(&mut db, "member", Role::Member).unwrap();
        assert_eq!(db.user_count().unwrap(), 2);
    }

    #[test]
    fn a_created_user_round_trips() {
        let mut db = AuthDb::open_in_memory().unwrap();
        let created = create_user(
            &mut db,
            NewUser {
                username: "Dewansh",
                display_name: Some("Dewansh Thakur"),
                password_hash: HASH,
                role: Role::Owner,
            },
            NOW,
        )
        .unwrap();
        assert!(created.id.starts_with("usr_"), "{}", created.id);

        let found = db.find_user("dewansh").unwrap().unwrap();
        assert_eq!(found.id, created.id);
        assert_eq!(found.username, "Dewansh", "display case is preserved");
        assert_eq!(found.display_name.as_deref(), Some("Dewansh Thakur"));
        assert_eq!(found.role, Role::Owner);
        assert_eq!(found.status, Status::Active);
        assert_eq!(db.password_hash_of(&found.id).unwrap().unwrap(), HASH);
    }

    #[test]
    fn the_last_active_owner_cannot_be_deleted_disabled_or_demoted() {
        let mut db = db_with_owner();
        add(&mut db, "member", Role::Member).unwrap();

        for outcome in [
            delete_user(&mut db, "owner", NOW).err(),
            set_status(&mut db, "owner", Status::Disabled, NOW).err(),
            set_role(&mut db, "owner", Role::Member, NOW).err(),
        ] {
            let err = outcome.expect("the last owner must be protected");
            assert!(err.to_string().contains("only active owner"), "{err}");
        }

        // Still there, still an owner, still active.
        let owner = db.find_user("owner").unwrap().unwrap();
        assert_eq!(owner.role, Role::Owner);
        assert_eq!(owner.status, Status::Active);
    }

    #[test]
    fn a_second_owner_unlocks_the_first() {
        let mut db = db_with_owner();
        add(&mut db, "second", Role::Owner).unwrap();
        assert_eq!(db.active_owner_count().unwrap(), 2);

        // Ownership transfer: promote, then the original may step down.
        set_role(&mut db, "owner", Role::Member, NOW).unwrap();
        assert_eq!(db.find_user("owner").unwrap().unwrap().role, Role::Member);
        assert_eq!(db.active_owner_count().unwrap(), 1);

        // And now the remaining one is protected in its turn.
        assert!(delete_user(&mut db, "second", NOW).is_err());
    }

    #[test]
    fn a_disabled_owner_does_not_count_as_an_owner() {
        // The lockout this prevents: two owners, disable one, then disable the
        // other because "there are two owners" — and nobody can log in.
        let mut db = db_with_owner();
        add(&mut db, "second", Role::Owner).unwrap();
        set_status(&mut db, "second", Status::Disabled, NOW).unwrap();
        assert_eq!(db.active_owner_count().unwrap(), 1);

        let err = set_status(&mut db, "owner", Status::Disabled, NOW).unwrap_err();
        assert!(err.to_string().contains("only active owner"), "{err}");
    }

    #[test]
    fn disabling_and_re_enabling_moves_only_the_status() {
        let mut db = db_with_owner();
        add(&mut db, "member", Role::Member).unwrap();

        set_status(&mut db, "member", Status::Disabled, NOW).unwrap();
        assert_eq!(
            db.find_user("member").unwrap().unwrap().status,
            Status::Disabled
        );
        set_status(&mut db, "member", Status::Active, NOW).unwrap();
        let user = db.find_user("member").unwrap().unwrap();
        assert_eq!(user.status, Status::Active);
        assert_eq!(user.role, Role::Member);
    }

    #[test]
    fn changing_a_password_clears_the_lockout_counters() {
        // A11 is a rescue path. Handing back an account that is still locked
        // out would only half-rescue it.
        let mut db = db_with_owner();
        let user = db.find_user("owner").unwrap().unwrap();
        db.conn
            .execute(
                "UPDATE users SET failed_count = 5, locked_until = '2099-01-01T00:00:00Z'
                 WHERE id = ?1",
                params![user.id],
            )
            .unwrap();

        set_password(
            &mut db,
            "owner",
            "$argon2id$v=19$m=196608,t=1,p=1$c2FsdA$bmV3",
            NOW,
        )
        .unwrap();

        let (failed, locked): (i64, Option<String>) = db
            .conn
            .query_row(
                "SELECT failed_count, locked_until FROM users WHERE id = ?1",
                params![user.id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(failed, 0);
        assert_eq!(locked, None);
        assert_eq!(
            db.password_hash_of(&user.id).unwrap().unwrap(),
            "$argon2id$v=19$m=196608,t=1,p=1$c2FsdA$bmV3"
        );
    }

    #[test]
    fn deleting_a_user_takes_their_sessions_and_grants_with_them() {
        let mut db = db_with_owner();
        add(&mut db, "member", Role::Member).unwrap();
        let member = db.find_user("member").unwrap().unwrap();

        db.conn
            .execute(
                "INSERT INTO vault_grants (user_id, vault_id, access, granted)
                 VALUES (?1, 'vault-1', 'read', ?2)",
                params![member.id, NOW],
            )
            .unwrap();

        delete_user(&mut db, "member", NOW).unwrap();

        let grants: i64 = db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM vault_grants WHERE user_id = ?1",
                params![member.id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(grants, 0, "a deleted user must not leave grants behind");
        assert!(db.find_user("member").unwrap().is_none());
    }

    #[test]
    fn every_administrative_act_writes_a_security_event_and_no_secret() {
        let mut db = db_with_owner();
        add(&mut db, "member", Role::Member).unwrap();
        set_password(&mut db, "member", HASH, NOW).unwrap();
        set_role(&mut db, "member", Role::Admin, NOW).unwrap();
        set_status(&mut db, "member", Status::Disabled, NOW).unwrap();
        set_status(&mut db, "member", Status::Active, NOW).unwrap();
        delete_user(&mut db, "member", NOW).unwrap();

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
        assert_eq!(
            kinds,
            vec![
                EVENT_USER_CREATED,
                EVENT_USER_CREATED,
                EVENT_PASSWORD_CHANGED,
                EVENT_ROLE_CHANGED,
                EVENT_DISABLED,
                EVENT_ENABLED,
                EVENT_DELETED,
            ]
        );
        // The table must never become a place secrets leak to.
        for (kind, detail) in &rows {
            let detail = detail.clone().unwrap_or_default();
            assert!(
                !detail.contains(HASH) && !detail.contains("argon2"),
                "{kind} wrote a password hash into security_events: {detail}"
            );
        }
    }

    #[test]
    fn acting_on_an_unknown_user_says_so() {
        let mut db = db_with_owner();
        let err = set_password(&mut db, "nobody", HASH, NOW).unwrap_err();
        assert!(err.to_string().contains("no user named `nobody`"), "{err}");
    }
}
