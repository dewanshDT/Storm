#![allow(dead_code)] // Used by tests; REST/middleware callers land in a later slice.
//! Sessions: `(user, device)`, opaque tokens, revocation that takes effect now.
//!
//! A session is always a pair. A device credential never reads a note, and a
//! session never outlives the device it was issued on — which is what makes
//! "revoke this device" and "sign out of this device" two different, both
//! meaningful, operations.
//!
//! **Lifetimes are long because revocation is immediate** (A6): access 30 days,
//! refresh 180 days sliding. Short tokens are the reflex and they are wrong for
//! this client. Storm is offline-first with an outbox, so a session that expires
//! while a phone is out of range means the outbox cannot drain when it comes
//! back, and the user meets an auth wall in front of edits that exist nowhere
//! else. That trade only works because there is no stateless token to wait out —
//! every check is a lookup against this table (A5).
//!
//! Nothing here is reachable over HTTP yet. These are the operations the
//! three-tier middleware will call; until that slice lands, `require_token` is
//! untouched and the only callers are the operator commands.

use anyhow::{Context, Result};
use rusqlite::{OptionalExtension, params};
use time::format_description::well_known::Rfc3339;
use time::{Duration, OffsetDateTime};

use super::db::AuthDb;
use super::devices::Device;
use super::identity::random_id;
use super::password::{self, Hasher};
use super::token;
use super::users::{self, Status, User};

/// A6. Long on purpose — see the module docs.
pub const ACCESS_LIFETIME_DAYS: i64 = 30;
pub const REFRESH_LIFETIME_DAYS: i64 = 180;

/// `last_used` is written at most this often per session.
///
/// Updating it on every request would turn every read into a write and put
/// `auth.db` in the path of every sync poll — on a phone that polls, that is a
/// write per poll per device, forever.
pub const LAST_USED_THROTTLE_SECS: i64 = 60;

/// WS ticket lifetime: 60 seconds, single-use.
pub const WS_TICKET_LIFETIME_SECS: i64 = 60;

pub const EVENT_LOGIN_OK: &str = "login_ok";
pub const EVENT_LOGIN_FAIL: &str = "login_fail";
pub const EVENT_LOGIN_LOCKED: &str = "login_locked";
pub const EVENT_SESSION_CREATED: &str = "session_created";
pub const EVENT_SESSION_REFRESHED: &str = "session_refreshed";
pub const EVENT_SESSION_REVOKED: &str = "session_revoked";
pub const EVENT_REFRESH_REPLAYED: &str = "refresh_replayed";

/// A real Argon2id hash at the current parameters, for the absent-user path.
///
/// When a login names a user who does not exist, the server verifies against
/// this instead of returning early — otherwise the response time answers "does
/// this account exist?" for free, and an attacker enumerates the user list
/// without ever guessing a password. The password it encodes is irrelevant and
/// deliberately unguessable-by-accident; only the *cost* matters.
///
/// It is a constant rather than something computed at startup because computing
/// it would cost a full hash on every process start, including every CLI
/// invocation. `the_absent_user_hash_costs_what_a_real_one_costs` fails if it
/// ever drifts from the current parameters.
const ABSENT_USER_HASH: &str = "$argon2id$v=19$m=196608,t=1,p=1$c3Rvcm1hYnNlbnR1c2Vy$eIINKY1WRCFNrH6DnrOMUQtDLvtk7kldNz93ger+7lE";

/// Why a credential was refused. The middleware slice maps these onto the
/// protocol's error codes; the mapping lives here so both stay in one place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthFailure {
    /// No session holds this token.
    Unknown,
    Expired,
    Revoked,
    UserDisabled,
    DeviceRevoked,
    /// A refresh token that had already been rotated away. The session is
    /// revoked by the time this is returned.
    ReplayedRefresh,
}

impl AuthFailure {
    /// The `error` string the client sees, from [[Storm Auth Protocol]]'s table.
    pub fn code(self) -> &'static str {
        match self {
            // "Never existed" and "expired" deliberately answer the same. The
            // client's remedy is identical — sign in again — and distinguishing
            // them would turn the endpoint into an oracle for whether a guessed
            // token was ever real.
            AuthFailure::Unknown | AuthFailure::Expired => "session_expired",
            AuthFailure::Revoked | AuthFailure::ReplayedRefresh => "session_revoked",
            AuthFailure::UserDisabled => "user_disabled",
            AuthFailure::DeviceRevoked => "device_revoked",
        }
    }
}

/// Why a login was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginFailure {
    /// Wrong password, or no such user. One answer for both, on purpose.
    InvalidCredentials,
    UserDisabled,
    DeviceRevoked,
    NotPaired,
    RateLimited {
        retry_after_secs: i64,
    },
}

impl LoginFailure {
    pub fn code(self) -> &'static str {
        match self {
            LoginFailure::InvalidCredentials => "invalid_credentials",
            LoginFailure::UserDisabled => "user_disabled",
            LoginFailure::DeviceRevoked => "device_revoked",
            LoginFailure::NotPaired => "not_paired",
            LoginFailure::RateLimited { .. } => "rate_limited",
        }
    }
}

/// A refusal is not an error — it is an answer. Keeping them separate means a
/// caller cannot accidentally treat a database failure as "wrong password" and
/// tell the user to check their typing.
#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("refused: {0:?}")]
    Refused(AuthFailure),
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum LoginError {
    #[error("refused: {0:?}")]
    Refused(LoginFailure),
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

/// A stored session. **No token material** — the hashes stay in the database,
/// for the same reason [`User`] carries no password hash.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Session {
    pub id: String,
    pub user_id: String,
    pub device_id: String,
    pub created: String,
    pub expires: String,
    pub refresh_expires: String,
    pub last_used: Option<String>,
    pub revoked: Option<String>,
}

impl Session {
    pub fn is_revoked(&self) -> bool {
        self.revoked.is_some()
    }
}

/// The one moment the plaintext tokens exist. They are never stored, never
/// logged, and cannot be recovered — a lost token is replaced, not looked up.
#[derive(Clone, serde::Serialize)]
pub struct IssuedSession {
    pub session_id: String,
    pub user_id: String,
    pub device_id: String,
    pub access_token: String,
    pub refresh_token: String,
    pub expires: String,
    pub refresh_expires: String,
}

impl std::fmt::Debug for IssuedSession {
    /// Hand-written so `?issued` in a tracing call is not a credential leak —
    /// the same reason [`super::identity::ServerIdentity`] redacts its key.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IssuedSession")
            .field("session_id", &self.session_id)
            .field("user_id", &self.user_id)
            .field("device_id", &self.device_id)
            .field("access_token", &"<redacted>")
            .field("refresh_token", &"<redacted>")
            .field("expires", &self.expires)
            .finish()
    }
}

/// A caller that has proven who it is.
#[derive(Debug, Clone)]
pub struct Authenticated {
    pub session: Session,
    pub user: User,
    pub device: Device,
}

/// Timestamps are **parsed, never compared as strings.**
///
/// `index::now_rfc3339()` formats with however many fractional digits the
/// instant needs, so `…:44.1Z` and `…:44.05Z` sort the wrong way round
/// lexicographically. A string comparison would decide a session had expired
/// based on the number of trailing zeroes in the second it was created.
fn parse_time(ts: &str) -> Result<OffsetDateTime> {
    OffsetDateTime::parse(ts, &Rfc3339).with_context(|| format!("parsing timestamp `{ts}`"))
}

fn format_time(at: OffsetDateTime) -> String {
    at.format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

fn row_to_session(row: &rusqlite::Row<'_>) -> rusqlite::Result<Session> {
    Ok(Session {
        id: row.get(0)?,
        user_id: row.get(1)?,
        device_id: row.get(2)?,
        created: row.get(3)?,
        expires: row.get(4)?,
        refresh_expires: row.get(5)?,
        last_used: row.get(6)?,
        revoked: row.get(7)?,
    })
}

const SESSION_COLUMNS: &str = "id, user_id, device_id, created, expires,
                               refresh_expires, last_used, revoked";

impl AuthDb {
    fn insert_session(
        &mut self,
        session: &Session,
        access_hash: &[u8],
        refresh_hash: &[u8],
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO sessions
                 (id, user_id, device_id, access_hash, refresh_hash,
                  created, expires, refresh_expires)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                session.id,
                session.user_id,
                session.device_id,
                access_hash,
                refresh_hash,
                session.created,
                session.expires,
                session.refresh_expires,
            ],
        )?;
        Ok(())
    }

    fn session_by_access_hash(&self, hash: &[u8]) -> Result<Option<Session>> {
        Ok(self
            .conn
            .query_row(
                &format!("SELECT {SESSION_COLUMNS} FROM sessions WHERE access_hash = ?1"),
                params![hash],
                row_to_session,
            )
            .optional()?)
    }

    fn session_by_refresh_hash(&self, hash: &[u8]) -> Result<Option<Session>> {
        Ok(self
            .conn
            .query_row(
                &format!("SELECT {SESSION_COLUMNS} FROM sessions WHERE refresh_hash = ?1"),
                params![hash],
                row_to_session,
            )
            .optional()?)
    }

    /// The replay lookup: a token this session has already rotated away from.
    fn session_by_previous_refresh_hash(&self, hash: &[u8]) -> Result<Option<Session>> {
        Ok(self
            .conn
            .query_row(
                &format!("SELECT {SESSION_COLUMNS} FROM sessions WHERE previous_refresh_hash = ?1"),
                params![hash],
                row_to_session,
            )
            .optional()?)
    }

    pub fn session_by_id(&self, id: &str) -> Result<Option<Session>> {
        Ok(self
            .conn
            .query_row(
                &format!("SELECT {SESSION_COLUMNS} FROM sessions WHERE id = ?1"),
                params![id],
                row_to_session,
            )
            .optional()?)
    }

    pub fn list_sessions(&self, user_id: Option<&str>) -> Result<Vec<Session>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {SESSION_COLUMNS} FROM sessions
             WHERE (?1 IS NULL OR user_id = ?1)
             ORDER BY created"
        ))?;
        let rows = stmt.query_map(params![user_id], row_to_session)?;
        let mut sessions = Vec::new();
        for row in rows {
            sessions.push(row?);
        }
        Ok(sessions)
    }

    fn rotate_session(
        &mut self,
        id: &str,
        access_hash: &[u8],
        refresh_hash: &[u8],
        previous_refresh_hash: &[u8],
        expires: &str,
        refresh_expires: &str,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE sessions
                SET access_hash = ?2, refresh_hash = ?3, previous_refresh_hash = ?4,
                    expires = ?5, refresh_expires = ?6
              WHERE id = ?1",
            params![
                id,
                access_hash,
                refresh_hash,
                previous_refresh_hash,
                expires,
                refresh_expires
            ],
        )?;
        Ok(())
    }

    fn mark_session_revoked(&mut self, id: &str, reason: &str, now: &str) -> Result<usize> {
        // `revoked IS NULL` keeps the first revocation's timestamp and reason:
        // when a session was cut off matters more than when someone last asked.
        Ok(self.conn.execute(
            "UPDATE sessions SET revoked = ?2, revoked_reason = ?3
             WHERE id = ?1 AND revoked IS NULL",
            params![id, now, reason],
        )?)
    }

    fn revoke_sessions_where(
        &mut self,
        column: &str,
        value: &str,
        reason: &str,
        now: &str,
    ) -> Result<usize> {
        // `column` is one of two literals chosen in this file, never input.
        Ok(self.conn.execute(
            &format!(
                "UPDATE sessions SET revoked = ?2, revoked_reason = ?3
                 WHERE {column} = ?1 AND revoked IS NULL"
            ),
            params![value, now, reason],
        )?)
    }

    fn touch_session(&mut self, id: &str, now: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE sessions SET last_used = ?2 WHERE id = ?1",
            params![id, now],
        )?;
        Ok(())
    }

    // ---- ws tickets --------------------------------------------------------

    fn insert_ws_ticket(
        &self,
        id: &str,
        session_id: &str,
        access_hash: &[u8],
        created: &str,
        expires: &str,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO ws_tickets (id, session_id, access_hash, created, expires)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id, session_id, access_hash, created, expires],
        )?;
        Ok(())
    }

    fn ws_ticket_by_access_hash(&self, hash: &[u8]) -> Result<Option<(String, String)>> {
        // Returns (ticket_id, session_id) for an unused, unexpired ticket.
        Ok(self
            .conn
            .query_row(
                "SELECT id, session_id FROM ws_tickets
                 WHERE access_hash = ?1 AND used IS NULL",
                params![hash],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?)
    }

    fn mark_ws_ticket_used(&self, id: &str, now: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE ws_tickets SET used = ?2 WHERE id = ?1",
            params![id, now],
        )?;
        Ok(())
    }

    fn purge_expired_ws_tickets(&self, now: &str) -> Result<usize> {
        Ok(self.conn.execute(
            "DELETE FROM ws_tickets WHERE expires <= ?1 OR used IS NOT NULL",
            params![now],
        )?)
    }
}

/// Issues a session for a user on a device.
///
/// This is the credential-minting half, with no opinion about how the caller
/// earned it: [`login`] proves a password first, `storm-server token` is an
/// operator acting on the host. Both end here, so a session looks the same in
/// the device list however it was obtained.
pub fn create(db: &mut AuthDb, user_id: &str, device_id: &str, now: &str) -> Result<IssuedSession> {
    let issued_at = parse_time(now)?;
    let access_token = token::mint(token::ACCESS_PREFIX);
    let refresh_token = token::mint(token::REFRESH_PREFIX);

    let session = Session {
        id: random_id("ses_"),
        user_id: user_id.to_string(),
        device_id: device_id.to_string(),
        created: now.to_string(),
        expires: format_time(issued_at + Duration::days(ACCESS_LIFETIME_DAYS)),
        refresh_expires: format_time(issued_at + Duration::days(REFRESH_LIFETIME_DAYS)),
        last_used: None,
        revoked: None,
    };

    db.insert_session(
        &session,
        &token::hash(&access_token),
        &token::hash(&refresh_token),
    )?;
    db.record_event(
        EVENT_SESSION_CREATED,
        Some(user_id),
        Some(device_id),
        now,
        &format!(r#"{{"session":{:?}}}"#, session.id),
    )?;

    Ok(IssuedSession {
        session_id: session.id,
        user_id: session.user_id,
        device_id: session.device_id,
        access_token,
        refresh_token,
        expires: session.expires,
        refresh_expires: session.refresh_expires,
    })
}

/// Resolves an access token, or says exactly why it will not.
///
/// Takes `&mut` because a successful authentication may write `last_used` —
/// throttled, but a write. The middleware will hold this behind the same lock
/// it already needs, since a rusqlite `Connection` is not `Sync`.
pub fn authenticate(
    db: &mut AuthDb,
    access_token: &str,
    now: &str,
) -> Result<Authenticated, SessionError> {
    // An indexed equality lookup on a hash, not a comparison loop over a
    // secret, so there is no constant-time question here to answer. A
    // `subtle::ConstantTimeEq` would be guarding a comparison that does not
    // happen.
    let session = db
        .session_by_access_hash(&token::hash(access_token))?
        .ok_or(SessionError::Refused(AuthFailure::Unknown))?;

    let checked = check(db, &session, now, parse_time(now)?)?;
    let at = parse_time(now)?;

    let due = match &session.last_used {
        None => true,
        Some(last) => (at - parse_time(last)?).whole_seconds() >= LAST_USED_THROTTLE_SECS,
    };
    if due {
        db.touch_session(&session.id, now)?;
    }

    Ok(checked)
}

/// The four ways a live-looking session is not usable, in one place so
/// [`authenticate`] and [`refresh`] cannot drift apart on them.
fn check(
    db: &AuthDb,
    session: &Session,
    now: &str,
    at: OffsetDateTime,
) -> Result<Authenticated, SessionError> {
    if session.is_revoked() {
        return Err(SessionError::Refused(AuthFailure::Revoked));
    }
    if parse_time(&session.expires)? <= at {
        return Err(SessionError::Refused(AuthFailure::Expired));
    }

    let user = db
        .user_by_id(&session.user_id)?
        .ok_or(SessionError::Refused(AuthFailure::Unknown))?;
    if user.status == Status::Disabled {
        return Err(SessionError::Refused(AuthFailure::UserDisabled));
    }

    let device = db
        .find_device(&session.device_id)?
        .ok_or(SessionError::Refused(AuthFailure::DeviceRevoked))?;
    if device.is_revoked() {
        return Err(SessionError::Refused(AuthFailure::DeviceRevoked));
    }

    let _ = now;
    Ok(Authenticated {
        session: session.clone(),
        user,
        device,
    })
}

/// Exchanges a refresh token for a new pair, rotating both.
///
/// **A replayed refresh token revokes the whole session.** Once a token has
/// been rotated away, the only party who should still hold it is someone who
/// copied it — so its reappearance is treated as evidence of theft rather than
/// as a stale request. The cost of that choice is real and worth stating: a
/// client whose refresh response was lost in flight will retry with the old
/// token and be signed out. That is the accepted trade (A6, and the same one
/// OAuth's rotation guidance makes) — an unnecessary re-login is recoverable,
/// a stolen refresh token quietly minting sessions for 180 days is not.
pub fn refresh(
    db: &mut AuthDb,
    refresh_token: &str,
    now: &str,
) -> Result<IssuedSession, SessionError> {
    let presented = token::hash(refresh_token);
    let at = parse_time(now)?;

    let Some(session) = db.session_by_refresh_hash(&presented)? else {
        // Not a current token. If it is one this session already rotated away,
        // that is theft; anything else is noise.
        if let Some(stolen) = db.session_by_previous_refresh_hash(&presented)? {
            db.mark_session_revoked(&stolen.id, "refresh token replayed", now)?;
            db.record_event(
                EVENT_REFRESH_REPLAYED,
                Some(&stolen.user_id),
                Some(&stolen.device_id),
                now,
                &format!(r#"{{"session":{:?}}}"#, stolen.id),
            )?;
            return Err(SessionError::Refused(AuthFailure::ReplayedRefresh));
        }
        return Err(SessionError::Refused(AuthFailure::Unknown));
    };

    if session.is_revoked() {
        return Err(SessionError::Refused(AuthFailure::Revoked));
    }
    // The refresh window is the one that matters here: an access token that
    // expired weeks ago is exactly the case refresh exists for.
    if parse_time(&session.refresh_expires)? <= at {
        return Err(SessionError::Refused(AuthFailure::Expired));
    }

    let user = db
        .user_by_id(&session.user_id)?
        .ok_or(SessionError::Refused(AuthFailure::Unknown))?;
    if user.status == Status::Disabled {
        return Err(SessionError::Refused(AuthFailure::UserDisabled));
    }
    let device = db
        .find_device(&session.device_id)?
        .ok_or(SessionError::Refused(AuthFailure::DeviceRevoked))?;
    if device.is_revoked() {
        return Err(SessionError::Refused(AuthFailure::DeviceRevoked));
    }

    let access_token = token::mint(token::ACCESS_PREFIX);
    let rotated = token::mint(token::REFRESH_PREFIX);
    // Sliding: an actively used session keeps moving its own deadline, so the
    // 180 days is "since you last used it", not "since you logged in".
    let expires = format_time(at + Duration::days(ACCESS_LIFETIME_DAYS));
    let refresh_expires = format_time(at + Duration::days(REFRESH_LIFETIME_DAYS));

    db.rotate_session(
        &session.id,
        &token::hash(&access_token),
        &token::hash(&rotated),
        &presented,
        &expires,
        &refresh_expires,
    )?;
    db.record_event(
        EVENT_SESSION_REFRESHED,
        Some(&session.user_id),
        Some(&session.device_id),
        now,
        &format!(r#"{{"session":{:?}}}"#, session.id),
    )?;

    Ok(IssuedSession {
        session_id: session.id,
        user_id: session.user_id,
        device_id: session.device_id,
        access_token,
        refresh_token: rotated,
        expires,
        refresh_expires,
    })
}

/// Revokes one session. Idempotent, and effective on the next request.
pub fn revoke(db: &mut AuthDb, session_id: &str, reason: &str, now: &str) -> Result<bool> {
    let Some(session) = db.session_by_id(session_id)? else {
        anyhow::bail!("no session `{session_id}`");
    };
    let changed = db.mark_session_revoked(&session.id, reason, now)? > 0;
    if changed {
        db.record_event(
            EVENT_SESSION_REVOKED,
            Some(&session.user_id),
            Some(&session.device_id),
            now,
            &format!(r#"{{"session":{:?},"reason":{reason:?}}}"#, session.id),
        )?;
    }
    Ok(changed)
}

/// Revokes every live session on a device. Returns how many were closed.
pub fn revoke_for_device(
    db: &mut AuthDb,
    device_id: &str,
    reason: &str,
    now: &str,
) -> Result<usize> {
    let closed = db.revoke_sessions_where("device_id", device_id, reason, now)?;
    if closed > 0 {
        db.record_event(
            EVENT_SESSION_REVOKED,
            None,
            Some(device_id),
            now,
            &format!(r#"{{"scope":"device","count":{closed},"reason":{reason:?}}}"#),
        )?;
    }
    Ok(closed)
}

/// Revokes every live session belonging to a user, on every device.
pub fn revoke_for_user(db: &mut AuthDb, user_id: &str, reason: &str, now: &str) -> Result<usize> {
    let closed = db.revoke_sessions_where("user_id", user_id, reason, now)?;
    if closed > 0 {
        db.record_event(
            EVENT_SESSION_REVOKED,
            Some(user_id),
            None,
            now,
            &format!(r#"{{"scope":"user","count":{closed},"reason":{reason:?}}}"#),
        )?;
    }
    Ok(closed)
}

/// Verifies a password and issues a session — Flow 4, in order.
///
/// The order is the design's and each step is load-bearing:
/// device first (a revoked device gets nothing, whatever the password), then
/// the lockout window, then the hash — and **a missing user still pays for a
/// hash**, because otherwise the response time answers "does this account
/// exist" for free.
pub async fn login(
    db: &mut AuthDb,
    hasher: &Hasher,
    username: &str,
    password: String,
    device_id: &str,
    now: &str,
) -> Result<IssuedSession, LoginError> {
    let device = db
        .find_device(device_id)?
        .ok_or(LoginError::Refused(LoginFailure::NotPaired))?;
    if device.is_revoked() {
        return Err(LoginError::Refused(LoginFailure::DeviceRevoked));
    }

    let user = db.find_user(username)?;

    if let Some(user) = &user
        && let Some(remaining) = users::lockout_remaining(db, &user.id, now)?
    {
        db.record_event(
            EVENT_LOGIN_LOCKED,
            Some(&user.id),
            Some(&device.id),
            now,
            &format!(r#"{{"retry_after_secs":{remaining}}}"#),
        )?;
        return Err(LoginError::Refused(LoginFailure::RateLimited {
            retry_after_secs: remaining,
        }));
    }

    // Both branches run one Argon2id verify through the same bounded hasher, so
    // they cost the same and neither can exhaust memory.
    let stored = match &user {
        Some(user) => db
            .password_hash_of(&user.id)?
            .unwrap_or_else(|| ABSENT_USER_HASH.to_string()),
        None => ABSENT_USER_HASH.to_string(),
    };
    let matched = hasher.verify(password.clone(), stored.clone()).await? && user.is_some();

    let Some(user) = user else {
        return Err(LoginError::Refused(LoginFailure::InvalidCredentials));
    };

    if !matched {
        let locked = users::note_failed_login(db, &user.id, now)?;
        db.record_event(
            EVENT_LOGIN_FAIL,
            Some(&user.id),
            Some(&device.id),
            now,
            &format!(r#"{{"locked":{}}}"#, locked.is_some()),
        )?;
        return Err(LoginError::Refused(LoginFailure::InvalidCredentials));
    }

    // Checked *after* the verify, deliberately: answering "this account is
    // disabled" to someone who has not proven the password would tell a
    // stranger which accounts exist.
    if user.status == Status::Disabled {
        db.record_event(
            EVENT_LOGIN_FAIL,
            Some(&user.id),
            Some(&device.id),
            now,
            r#"{"reason":"disabled"}"#,
        )?;
        return Err(LoginError::Refused(LoginFailure::UserDisabled));
    }

    users::note_successful_login(db, &user.id, now)?;

    // A1's rehash-on-login: a parameter bump reaches existing accounts the next
    // time their owner signs in, so nobody is asked to change a password
    // because the hardware got faster.
    if password::needs_rehash(&stored) {
        let upgraded = hasher.hash(password).await?;
        users::upgrade_password_hash(db, &user.id, &upgraded, now)?;
    }

    let issued = create(db, &user.id, &device.id, now)?;
    db.record_event(
        EVENT_LOGIN_OK,
        Some(&user.id),
        Some(&device.id),
        now,
        &format!(r#"{{"session":{:?}}}"#, issued.session_id),
    )?;
    Ok(issued)
}

/// Mints a short-lived, single-use ticket for WebSocket authentication.
///
/// The client POSTs to get one, then presents it on the `GET /v1/stream`
/// handshake. The ticket is bound to the session that created it and dies
/// after 60 seconds or one use, whichever comes first.
pub fn create_ws_ticket(db: &mut AuthDb, session_id: &str, now: &str) -> Result<IssuedWsTicket> {
    let at = parse_time(now)?;
    let ticket = token::mint("stt_");
    let ticket_id = random_id("stt_");
    let expires = format_time(at + Duration::seconds(WS_TICKET_LIFETIME_SECS));

    db.insert_ws_ticket(&ticket_id, session_id, &token::hash(&ticket), now, &expires)?;

    Ok(IssuedWsTicket {
        ticket,
        expires_in: WS_TICKET_LIFETIME_SECS,
    })
}

/// Validates a WebSocket ticket, consuming it if valid.
///
/// Returns the session id the ticket was bound to, or an error.
pub fn consume_ws_ticket(db: &mut AuthDb, ticket: &str, now: &str) -> Result<String, SessionError> {
    let hash = token::hash(ticket);
    let at = parse_time(now)?;

    let (ticket_id, session_id) = db
        .ws_ticket_by_access_hash(&hash)?
        .ok_or(SessionError::Refused(AuthFailure::Unknown))?;

    // Check expiry.
    let session = db
        .session_by_id(&session_id)?
        .ok_or(SessionError::Refused(AuthFailure::Unknown))?;
    if session.is_revoked() {
        return Err(SessionError::Refused(AuthFailure::Revoked));
    }
    if parse_time(&session.expires).is_ok_and(|exp| at > exp) {
        return Err(SessionError::Refused(AuthFailure::Expired));
    }

    // Mark used (single-use).
    db.mark_ws_ticket_used(&ticket_id, now)?;

    // Purge old tickets to keep the table small.
    let _ = db.purge_expired_ws_tickets(now)?;

    Ok(session_id)
}

/// The response from `POST /v1/auth/ws-ticket`.
pub struct IssuedWsTicket {
    pub ticket: String,
    pub expires_in: i64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::devices;
    use crate::auth::users::{NewUser, Role};

    const NOW: &str = "2026-08-16T12:00:00Z";
    const PASSWORD: &str = "correct horse battery staple";

    async fn fixture() -> (AuthDb, Hasher, User, Device) {
        let mut db = AuthDb::open_in_memory().unwrap();
        let hasher = Hasher::new();
        let hash = hasher.hash(PASSWORD.to_string()).await.unwrap();
        let user = crate::auth::users::create_user(
            &mut db,
            NewUser {
                username: "dewansh",
                display_name: None,
                password_hash: &hash,
                role: Role::Owner,
            },
            NOW,
        )
        .unwrap();
        let device = devices::create_synthetic(&mut db, "test device", NOW).unwrap();
        (db, hasher, user, device)
    }

    fn later(minutes: i64) -> String {
        format_time(parse_time(NOW).unwrap() + Duration::minutes(minutes))
    }

    fn days_later(days: i64) -> String {
        format_time(parse_time(NOW).unwrap() + Duration::days(days))
    }

    #[test]
    fn the_absent_user_hash_costs_what_a_real_one_costs() {
        // If this drifts, a login for a missing account is measurably cheaper
        // than a real one and the endpoint enumerates users by stopwatch.
        assert!(
            !password::needs_rehash(ABSENT_USER_HASH),
            "the absent-user hash is not at the current parameters"
        );
    }

    #[tokio::test]
    async fn a_session_round_trips_and_carries_the_right_pair() {
        let (mut db, _, user, device) = fixture().await;
        let issued = create(&mut db, &user.id, &device.id, NOW).unwrap();

        assert!(issued.session_id.starts_with("ses_"));
        assert!(issued.access_token.starts_with("sta_"));
        assert!(issued.refresh_token.starts_with("str_"));

        let who = authenticate(&mut db, &issued.access_token, NOW).unwrap();
        assert_eq!(who.user.id, user.id);
        assert_eq!(who.device.id, device.id);
        assert_eq!(who.session.id, issued.session_id);
    }

    #[tokio::test]
    async fn the_stored_row_holds_no_token() {
        // A database dump must contain nothing that can be replayed.
        let (mut db, _, user, device) = fixture().await;
        let issued = create(&mut db, &user.id, &device.id, NOW).unwrap();

        let (access, refresh): (Vec<u8>, Vec<u8>) = db
            .conn
            .query_row(
                "SELECT access_hash, refresh_hash FROM sessions WHERE id = ?1",
                params![issued.session_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_ne!(access, issued.access_token.as_bytes());
        assert_ne!(refresh, issued.refresh_token.as_bytes());
        assert_eq!(access, token::hash(&issued.access_token));
    }

    #[tokio::test]
    async fn an_unknown_token_is_refused() {
        let (mut db, _, _, _) = fixture().await;
        let err = authenticate(&mut db, "sta_nonsense", NOW).unwrap_err();
        assert!(matches!(err, SessionError::Refused(AuthFailure::Unknown)));
    }

    #[tokio::test]
    async fn an_expired_session_is_refused_but_can_still_refresh() {
        let (mut db, _, user, device) = fixture().await;
        let issued = create(&mut db, &user.id, &device.id, NOW).unwrap();

        let after = days_later(ACCESS_LIFETIME_DAYS + 1);
        let err = authenticate(&mut db, &issued.access_token, &after).unwrap_err();
        assert!(matches!(err, SessionError::Refused(AuthFailure::Expired)));

        // The whole point of a refresh token outliving the access token: an
        // offline phone coming back after a month must not need a password.
        let renewed = refresh(&mut db, &issued.refresh_token, &after).unwrap();
        assert!(authenticate(&mut db, &renewed.access_token, &after).is_ok());
    }

    #[tokio::test]
    async fn an_expired_refresh_token_is_the_end_of_the_session() {
        let (mut db, _, user, device) = fixture().await;
        let issued = create(&mut db, &user.id, &device.id, NOW).unwrap();
        let after = days_later(REFRESH_LIFETIME_DAYS + 1);
        let err = refresh(&mut db, &issued.refresh_token, &after).unwrap_err();
        assert!(matches!(err, SessionError::Refused(AuthFailure::Expired)));
    }

    #[tokio::test]
    async fn refreshing_rotates_both_tokens() {
        let (mut db, _, user, device) = fixture().await;
        let first = create(&mut db, &user.id, &device.id, NOW).unwrap();
        let second = refresh(&mut db, &first.refresh_token, &later(1)).unwrap();

        assert_eq!(second.session_id, first.session_id, "same session");
        assert_ne!(second.access_token, first.access_token);
        assert_ne!(second.refresh_token, first.refresh_token);

        // The old access token stops working the moment it is rotated away.
        assert!(authenticate(&mut db, &first.access_token, &later(1)).is_err());
        assert!(authenticate(&mut db, &second.access_token, &later(1)).is_ok());
    }

    #[tokio::test]
    async fn a_replayed_refresh_token_kills_the_session() {
        let (mut db, _, user, device) = fixture().await;
        let first = create(&mut db, &user.id, &device.id, NOW).unwrap();
        let second = refresh(&mut db, &first.refresh_token, &later(1)).unwrap();

        // Someone who copied the old token tries it after rotation.
        let err = refresh(&mut db, &first.refresh_token, &later(2)).unwrap_err();
        assert!(matches!(
            err,
            SessionError::Refused(AuthFailure::ReplayedRefresh)
        ));

        // And the session the thief was trying to extend is now dead — for the
        // legitimate holder too, which is the point: the tokens are compromised.
        let err = authenticate(&mut db, &second.access_token, &later(3)).unwrap_err();
        assert!(matches!(err, SessionError::Refused(AuthFailure::Revoked)));
    }

    #[tokio::test]
    async fn revocation_takes_effect_immediately() {
        let (mut db, _, user, device) = fixture().await;
        let issued = create(&mut db, &user.id, &device.id, NOW).unwrap();
        assert!(authenticate(&mut db, &issued.access_token, NOW).is_ok());

        assert!(revoke(&mut db, &issued.session_id, "signed out", NOW).unwrap());
        let err = authenticate(&mut db, &issued.access_token, NOW).unwrap_err();
        assert!(matches!(err, SessionError::Refused(AuthFailure::Revoked)));

        // A revoked session cannot be refreshed back to life either.
        let err = refresh(&mut db, &issued.refresh_token, NOW).unwrap_err();
        assert!(matches!(err, SessionError::Refused(AuthFailure::Revoked)));
    }

    #[tokio::test]
    async fn revoking_a_device_kills_every_session_on_it() {
        let (mut db, _, user, device) = fixture().await;
        let other = devices::create_synthetic(&mut db, "another device", NOW).unwrap();
        let on_device = create(&mut db, &user.id, &device.id, NOW).unwrap();
        let elsewhere = create(&mut db, &user.id, &other.id, NOW).unwrap();

        let (_, killed) = devices::revoke(&mut db, &device.id, "lost", NOW).unwrap();
        assert_eq!(killed, 1);

        assert!(authenticate(&mut db, &on_device.access_token, NOW).is_err());
        assert!(
            authenticate(&mut db, &elsewhere.access_token, NOW).is_ok(),
            "revoking one device must not sign out the others"
        );
    }

    #[tokio::test]
    async fn a_disabled_user_cannot_use_a_session_they_already_had() {
        // Disabling has to reach credentials already in someone's pocket.
        let (mut db, _, user, device) = fixture().await;
        let issued = create(&mut db, &user.id, &device.id, NOW).unwrap();
        crate::auth::users::create_user(
            &mut db,
            NewUser {
                username: "second-owner",
                display_name: None,
                password_hash: "$argon2id$v=19$m=196608,t=1,p=1$c2FsdA$aGFzaA",
                role: Role::Owner,
            },
            NOW,
        )
        .unwrap();
        crate::auth::users::set_status(&mut db, "dewansh", Status::Disabled, NOW).unwrap();

        let err = authenticate(&mut db, &issued.access_token, NOW).unwrap_err();
        assert!(matches!(
            err,
            SessionError::Refused(AuthFailure::UserDisabled)
        ));
        let err = refresh(&mut db, &issued.refresh_token, NOW).unwrap_err();
        assert!(matches!(
            err,
            SessionError::Refused(AuthFailure::UserDisabled)
        ));
    }

    #[tokio::test]
    async fn last_used_is_written_at_most_once_a_minute() {
        let (mut db, _, user, device) = fixture().await;
        let issued = create(&mut db, &user.id, &device.id, NOW).unwrap();

        authenticate(&mut db, &issued.access_token, NOW).unwrap();
        let first = db
            .session_by_id(&issued.session_id)
            .unwrap()
            .unwrap()
            .last_used;
        assert_eq!(first.as_deref(), Some(NOW));

        // Ten seconds later: still the first stamp. Writing here would put
        // auth.db in the path of every sync poll.
        let soon = format_time(parse_time(NOW).unwrap() + Duration::seconds(10));
        authenticate(&mut db, &issued.access_token, &soon).unwrap();
        let again = db
            .session_by_id(&issued.session_id)
            .unwrap()
            .unwrap()
            .last_used;
        assert_eq!(again, first, "last_used was written inside the throttle");

        let after = later(2);
        authenticate(&mut db, &issued.access_token, &after).unwrap();
        let moved = db
            .session_by_id(&issued.session_id)
            .unwrap()
            .unwrap()
            .last_used;
        assert_eq!(moved.as_deref(), Some(after.as_str()));
    }

    #[tokio::test]
    async fn deleting_a_user_takes_their_sessions_with_them() {
        let (mut db, _, user, device) = fixture().await;
        create(&mut db, &user.id, &device.id, NOW).unwrap();
        crate::auth::users::create_user(
            &mut db,
            NewUser {
                username: "second-owner",
                display_name: None,
                password_hash: "$argon2id$v=19$m=196608,t=1,p=1$c2FsdA$aGFzaA",
                role: Role::Owner,
            },
            NOW,
        )
        .unwrap();

        crate::auth::users::delete_user(&mut db, "dewansh", NOW).unwrap();
        assert!(db.list_sessions(Some(&user.id)).unwrap().is_empty());
    }

    // ---- login -----------------------------------------------------------

    #[tokio::test]
    async fn a_correct_password_logs_in() {
        let (mut db, hasher, user, device) = fixture().await;
        let issued = login(
            &mut db,
            &hasher,
            "DEWANSH",
            PASSWORD.to_string(),
            &device.id,
            NOW,
        )
        .await
        .unwrap();
        assert_eq!(issued.user_id, user.id, "the username fold is used");
        assert!(authenticate(&mut db, &issued.access_token, NOW).is_ok());
    }

    #[tokio::test]
    async fn a_wrong_password_is_refused_and_counted() {
        let (mut db, hasher, user, device) = fixture().await;
        let err = login(
            &mut db,
            &hasher,
            "dewansh",
            "not the password".into(),
            &device.id,
            NOW,
        )
        .await
        .unwrap_err();
        assert!(matches!(
            err,
            LoginError::Refused(LoginFailure::InvalidCredentials)
        ));

        let failures: i64 = db
            .conn
            .query_row(
                "SELECT failed_count FROM users WHERE id = ?1",
                params![user.id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(failures, 1);
    }

    #[tokio::test]
    async fn a_missing_user_still_pays_for_a_hash() {
        // The timing defence, tested by what it *does* rather than by a
        // stopwatch: a wall-clock assertion would be flaky on a loaded machine
        // and would pass on a fast one even with the defence removed.
        let (mut db, hasher, _, device) = fixture().await;
        let before = hasher.jobs_run();

        let err = login(
            &mut db,
            &hasher,
            "nobody-at-all",
            PASSWORD.to_string(),
            &device.id,
            NOW,
        )
        .await
        .unwrap_err();
        assert!(matches!(
            err,
            LoginError::Refused(LoginFailure::InvalidCredentials)
        ));
        assert_eq!(
            hasher.jobs_run() - before,
            1,
            "a login for a missing user must still run one verify"
        );
    }

    #[tokio::test]
    async fn five_failures_lock_the_account_and_success_clears_it() {
        let (mut db, hasher, _, device) = fixture().await;
        for _ in 0..5 {
            let _ = login(&mut db, &hasher, "dewansh", "wrong".into(), &device.id, NOW).await;
        }

        // Even the right password is refused while the lock holds — and it is
        // refused *before* the hash, so a locked account costs an attacker
        // nothing to keep locked but costs us nothing either.
        let before = hasher.jobs_run();
        let err = login(
            &mut db,
            &hasher,
            "dewansh",
            PASSWORD.to_string(),
            &device.id,
            NOW,
        )
        .await
        .unwrap_err();
        match err {
            LoginError::Refused(LoginFailure::RateLimited { retry_after_secs }) => {
                assert!(
                    retry_after_secs > 0 && retry_after_secs <= 60,
                    "{retry_after_secs}"
                );
            }
            other => panic!("expected a lockout, got {other:?}"),
        }
        assert_eq!(hasher.jobs_run(), before, "a locked account must not hash");

        // After the window, the right password works and clears the counters.
        let issued = login(
            &mut db,
            &hasher,
            "dewansh",
            PASSWORD.to_string(),
            &device.id,
            &later(2),
        )
        .await
        .unwrap();
        assert!(authenticate(&mut db, &issued.access_token, &later(2)).is_ok());

        let (failures, locked): (i64, Option<String>) = db
            .conn
            .query_row(
                "SELECT failed_count, locked_until FROM users WHERE username_fold = 'dewansh'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(failures, 0);
        assert_eq!(locked, None);
    }

    #[tokio::test]
    async fn the_lockout_doubles_and_then_caps() {
        let (mut db, hasher, user, device) = fixture().await;
        let mut windows = Vec::new();
        for attempt in 0..10 {
            // Step the clock past each lock so the next attempt is counted
            // rather than rejected.
            let at = later(attempt * 60);
            let _ = login(&mut db, &hasher, "dewansh", "wrong".into(), &device.id, &at).await;
            if let Some(remaining) = users::lockout_remaining(&db, &user.id, &at).unwrap() {
                windows.push(remaining / 60);
            }
        }
        assert_eq!(
            windows,
            vec![1, 2, 4, 8, 15, 15],
            "1 min doubling to a 15 min cap"
        );
    }

    #[tokio::test]
    async fn a_disabled_user_cannot_log_in_even_with_the_right_password() {
        let (mut db, hasher, _, device) = fixture().await;
        crate::auth::users::create_user(
            &mut db,
            NewUser {
                username: "second-owner",
                display_name: None,
                password_hash: "$argon2id$v=19$m=196608,t=1,p=1$c2FsdA$aGFzaA",
                role: Role::Owner,
            },
            NOW,
        )
        .unwrap();
        crate::auth::users::set_status(&mut db, "dewansh", Status::Disabled, NOW).unwrap();

        let err = login(
            &mut db,
            &hasher,
            "dewansh",
            PASSWORD.to_string(),
            &device.id,
            NOW,
        )
        .await
        .unwrap_err();
        assert!(matches!(
            err,
            LoginError::Refused(LoginFailure::UserDisabled)
        ));
    }

    #[tokio::test]
    async fn a_revoked_device_cannot_log_in_at_all() {
        let (mut db, hasher, _, device) = fixture().await;
        devices::revoke(&mut db, &device.id, "lost", NOW).unwrap();

        let before = hasher.jobs_run();
        let err = login(
            &mut db,
            &hasher,
            "dewansh",
            PASSWORD.to_string(),
            &device.id,
            NOW,
        )
        .await
        .unwrap_err();
        assert!(matches!(
            err,
            LoginError::Refused(LoginFailure::DeviceRevoked)
        ));
        assert_eq!(
            hasher.jobs_run(),
            before,
            "a revoked device must be turned away before the KDF runs"
        );
    }

    #[tokio::test]
    async fn an_unpaired_device_is_told_to_pair() {
        let (mut db, hasher, _, _) = fixture().await;
        let err = login(
            &mut db,
            &hasher,
            "dewansh",
            PASSWORD.to_string(),
            "dev_nothing",
            NOW,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, LoginError::Refused(LoginFailure::NotPaired)));
    }

    #[tokio::test]
    async fn login_upgrades_a_password_hashed_with_weaker_parameters() {
        // A1's rehash-on-login. The account was created when the box was
        // slower; signing in brings it up to the current cost.
        let (mut db, hasher, user, device) = fixture().await;
        let weak = argon2::Argon2::new(
            argon2::Algorithm::Argon2id,
            argon2::Version::V0x13,
            argon2::Params::new(19_456, 2, 1, None).unwrap(),
        );
        let salt = argon2::password_hash::SaltString::from_b64("c3Rvcm13ZWFr").unwrap();
        let weak_hash = {
            use argon2::password_hash::PasswordHasher;
            weak.hash_password(PASSWORD.as_bytes(), &salt)
                .unwrap()
                .to_string()
        };
        users::upgrade_password_hash(&mut db, &user.id, &weak_hash, NOW).unwrap();
        assert!(password::needs_rehash(
            &db.password_hash_of(&user.id).unwrap().unwrap()
        ));

        login(
            &mut db,
            &hasher,
            "dewansh",
            PASSWORD.to_string(),
            &device.id,
            NOW,
        )
        .await
        .unwrap();

        let stored = db.password_hash_of(&user.id).unwrap().unwrap();
        assert!(
            !password::needs_rehash(&stored),
            "login did not upgrade the stored hash"
        );
        // And the upgrade did not break the password.
        assert!(
            login(
                &mut db,
                &hasher,
                "dewansh",
                PASSWORD.to_string(),
                &device.id,
                NOW
            )
            .await
            .is_ok()
        );
    }

    #[tokio::test]
    async fn every_refusal_has_its_own_code() {
        // The client has to be able to say the true thing. "Couldn't reach the
        // server" is never one of these.
        assert_eq!(AuthFailure::Expired.code(), "session_expired");
        assert_eq!(AuthFailure::Revoked.code(), "session_revoked");
        assert_eq!(AuthFailure::ReplayedRefresh.code(), "session_revoked");
        assert_eq!(AuthFailure::DeviceRevoked.code(), "device_revoked");
        assert_eq!(AuthFailure::UserDisabled.code(), "user_disabled");
        assert_eq!(
            LoginFailure::InvalidCredentials.code(),
            "invalid_credentials"
        );
        assert_eq!(LoginFailure::NotPaired.code(), "not_paired");
        assert_eq!(
            LoginFailure::RateLimited {
                retry_after_secs: 60
            }
            .code(),
            "rate_limited"
        );
    }

    #[tokio::test]
    async fn a_login_writes_a_trail_and_never_a_secret() {
        let (mut db, hasher, _, device) = fixture().await;
        let _ = login(&mut db, &hasher, "dewansh", "wrong".into(), &device.id, NOW).await;
        let issued = login(
            &mut db,
            &hasher,
            "dewansh",
            PASSWORD.to_string(),
            &device.id,
            NOW,
        )
        .await
        .unwrap();

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
        assert!(kinds.contains(&EVENT_LOGIN_FAIL));
        assert!(kinds.contains(&EVENT_LOGIN_OK));

        for (kind, detail) in &rows {
            let detail = detail.clone().unwrap_or_default();
            assert!(
                !detail.contains(&issued.access_token)
                    && !detail.contains(&issued.refresh_token)
                    && !detail.contains(PASSWORD),
                "{kind} leaked a credential into security_events: {detail}"
            );
        }
    }
}
