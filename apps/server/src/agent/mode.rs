//! Decision 147c: an agent asks for auto, the person decides. A Claude Code
//! session reading Storm may call `request_permission_mode {mode: "auto",
//! reason}`; that makes one request card, which only a person approves or
//! denies, and which expires after [`EXPIRES_AFTER_SECS`]. Approval switches
//! the session by resuming its conversation in auto (`ops::switch_to_auto`):
//! G-147 found that Claude Code does not re-read Storm's `--settings` while
//! it runs, so auto cannot be put back in its cycle in place. A child may ask
//! only while its parent is in auto (146b). Only `auto` can be asked for;
//! nothing reaches bypass.

use std::time::{Duration, Instant};

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use super::store::spawn::expired;
use super::{AgentError, AgentManager, AgentResult, now, permission};

/// How long a request waits for the person.
pub const EXPIRES_AFTER_SECS: i64 = 600;

/// The longest reason kept, in bytes; a longer one is cut at a character.
pub const MAX_REASON_BYTES: usize = 1024;

pub const PENDING: &str = "pending";
pub const APPROVED: &str = "approved";
pub const DENIED: &str = "denied";
pub const EXPIRED: &str = "expired";

pub(super) fn init(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "-- 147c: an agent's request for auto, decided once by a person.
         CREATE TABLE IF NOT EXISTS mode_requests (
             id                TEXT PRIMARY KEY,
             session_id        TEXT NOT NULL,
             mode              TEXT NOT NULL CHECK (mode = 'auto'),
             reason            TEXT NOT NULL CHECK (length(CAST(reason AS BLOB)) <= 1024),
             created_at        TEXT NOT NULL,
             expires_at        TEXT NOT NULL,
             outcome           TEXT CHECK (outcome IN ('approved', 'denied', 'expired')),
             decided_by_device TEXT,
             decided_at        TEXT,
             switched_to       TEXT,
             switch_failed     INTEGER NOT NULL DEFAULT 0 CHECK (switch_failed IN (0, 1)),
             CHECK ((outcome IS NULL) = (decided_at IS NULL)),
             CHECK (switched_to IS NULL OR outcome = 'approved')
         );
         CREATE UNIQUE INDEX IF NOT EXISTS mode_requests_one_pending
             ON mode_requests(session_id) WHERE outcome IS NULL;
         CREATE INDEX IF NOT EXISTS mode_requests_by_session
             ON mode_requests(session_id, created_at);
         CREATE TRIGGER IF NOT EXISTS mode_request_fixed
             BEFORE UPDATE OF id, session_id, mode, reason, created_at ON mode_requests
             BEGIN SELECT RAISE(ABORT, 'a request is fixed when it is made'); END;
         CREATE TRIGGER IF NOT EXISTS mode_request_decided_once
             BEFORE UPDATE OF outcome, decided_by_device, decided_at ON mode_requests
             WHEN OLD.outcome IS NOT NULL
             BEGIN SELECT RAISE(ABORT, 'a request is decided once'); END;",
    )?;
    Ok(())
}

/// One request, as the card and the tool see it. The reason is the agent's
/// own text: data to show, never an instruction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ModeRequest {
    pub id: String,
    pub session_id: String,
    pub mode: String,
    pub reason: String,
    pub created_at: String,
    pub expires_at: String,
    /// `pending`, else the outcome.
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decided_at: Option<String>,
    /// The session the approved switch resumed into.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub switched_to: Option<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub switch_failed: bool,
}

impl ModeRequest {
    pub fn pending(&self) -> bool {
        self.state == PENDING
    }
}

const REQUEST_COLUMNS: &str = "id, session_id, mode, reason, created_at, expires_at, outcome, \
    decided_at, switched_to, switch_failed";

fn from_row(r: &rusqlite::Row) -> rusqlite::Result<ModeRequest> {
    let outcome: Option<String> = r.get(6)?;
    Ok(ModeRequest {
        id: r.get(0)?,
        session_id: r.get(1)?,
        mode: r.get(2)?,
        reason: r.get(3)?,
        created_at: r.get(4)?,
        expires_at: r.get(5)?,
        state: outcome.unwrap_or_else(|| PENDING.into()),
        decided_at: r.get(7)?,
        switched_to: r.get(8)?,
        switch_failed: r.get::<_, i64>(9)? == 1,
    })
}

fn by_id(conn: &Connection, id: &str) -> Result<Option<ModeRequest>> {
    Ok(conn
        .query_row(
            &format!("SELECT {REQUEST_COLUMNS} FROM mode_requests WHERE id = ?1"),
            [id],
            from_row,
        )
        .optional()?)
}

fn latest(conn: &Connection, session_id: &str) -> Result<Option<ModeRequest>> {
    Ok(conn
        .query_row(
            &format!(
                "SELECT {REQUEST_COLUMNS} FROM mode_requests WHERE session_id = ?1
                 ORDER BY created_at DESC, rowid DESC LIMIT 1"
            ),
            [session_id],
            from_row,
        )
        .optional()?)
}

/// Decides a pending request that is past its expiry as `expired`.
fn settle_expired(conn: &Connection, r: ModeRequest) -> Result<ModeRequest> {
    let at = now();
    if !r.pending() || !expired(&at, &r.expires_at) {
        return Ok(r);
    }
    conn.execute(
        "UPDATE mode_requests SET outcome = 'expired', decided_at = ?2
         WHERE id = ?1 AND outcome IS NULL",
        params![r.id, at],
    )?;
    Ok(by_id(conn, &r.id)?.unwrap_or(r))
}

fn cut(reason: &str) -> String {
    let reason = reason.trim();
    if reason.len() <= MAX_REASON_BYTES {
        return reason.to_string();
    }
    let mut end = MAX_REASON_BYTES;
    while !reason.is_char_boundary(end) {
        end -= 1;
    }
    reason[..end].to_string()
}

/// Why a request is refused before any card is made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// Only `auto` can be asked for, and only by Claude Code.
    Unavailable(String),
    /// A child whose parent is not in auto (146b).
    Wider(String),
    /// Already in auto: nothing to ask.
    AlreadyAuto,
    /// The switch restarts the session, which its live children would not
    /// survive.
    LiveChildren,
    /// Ended, or not running.
    NotLive,
}

impl AgentManager {
    /// The newest request of `session_id`, an expired one settled.
    pub fn mode_request_of(&self, session_id: &str) -> AgentResult<Option<ModeRequest>> {
        let store = self.store.lock().unwrap();
        let Some(r) = latest(&store.conn, session_id)? else {
            return Ok(None);
        };
        Ok(Some(settle_expired(&store.conn, r)?))
    }

    pub fn mode_request(&self, id: &str) -> AgentResult<Option<ModeRequest>> {
        let store = self.store.lock().unwrap();
        let Some(r) = by_id(&store.conn, id)? else {
            return Ok(None);
        };
        Ok(Some(settle_expired(&store.conn, r)?))
    }

    /// Makes `session_id`'s request for `mode`, or returns the one already
    /// waiting (one card, however often the agent asks). `Ok((request,
    /// true))` when this call made it.
    pub fn request_mode(
        &self,
        session_id: &str,
        mode: &str,
        reason: &str,
    ) -> AgentResult<Result<(ModeRequest, bool), Refusal>> {
        let r = self.get(session_id)?;
        if !matches!(r.status.as_str(), "starting" | "running") {
            return Ok(Err(Refusal::NotLive));
        }
        let mode = mode.trim();
        if r.provider != "claude-code" || mode != permission::AUTO {
            let shown: String = mode.chars().take(40).collect();
            return Ok(Err(Refusal::Unavailable(format!(
                "{shown:?} cannot be asked for here: only a Claude Code session can ask, and \
                 only for \"auto\". Manual, accept-edits and plan are yours to cycle with \
                 Shift+Tab; nothing turns permission checks off."
            ))));
        }
        let store = self.store.lock().unwrap();
        let own = store.launch(session_id)?.and_then(|l| l.permission_mode);
        if own.as_deref() == Some(permission::AUTO) {
            return Ok(Err(Refusal::AlreadyAuto));
        }
        if let Some(parent) = store.lineage(session_id)?.parent_session {
            let parents = store.launch(&parent)?.and_then(|l| l.permission_mode);
            if parents.as_deref() != Some(permission::AUTO) {
                return Ok(Err(Refusal::Wider(
                    "A child is never looser than the agent that started it, and that agent \
                     is not in auto."
                        .into(),
                )));
            }
        }
        if !store.descendants(session_id, true)?.is_empty() {
            return Ok(Err(Refusal::LiveChildren));
        }
        if let Some(open) = latest(&store.conn, session_id)? {
            let open = settle_expired(&store.conn, open)?;
            if open.pending() {
                return Ok(Ok((open, false)));
            }
        }
        let created = now();
        let id = crate::auth::identity::random_id("amr_");
        store
            .conn
            .execute(
                "INSERT INTO mode_requests (id, session_id, mode, reason, created_at, expires_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    id,
                    session_id,
                    mode,
                    cut(reason),
                    created,
                    crate::index::rfc3339_minus_secs(&created, -EXPIRES_AFTER_SECS)
                ],
            )
            .map_err(anyhow::Error::from)?;
        let made = by_id(&store.conn, &id)?.expect("just written");
        drop(store);
        self.bump(session_id);
        Ok(Ok((made, true)))
    }

    /// Waits up to `bound` for request `id` to be decided or to expire.
    pub async fn wait_mode(&self, id: &str, bound: Duration) -> AgentResult<ModeRequest> {
        let deadline = Instant::now() + bound;
        let first = self
            .mode_request(id)?
            .ok_or(AgentError::NotFound("no such request"))?;
        let mut changed = self.watch(&first.session_id);
        loop {
            changed.borrow_and_update();
            let r = self
                .mode_request(id)?
                .ok_or(AgentError::NotFound("no such request"))?;
            let left = deadline.saturating_duration_since(Instant::now());
            if !r.pending() || left.is_zero() {
                return Ok(r);
            }
            // The tick also notices the expiry, which nothing bumps.
            let _ =
                tokio::time::timeout(left.min(Duration::from_millis(500)), changed.changed()).await;
        }
    }

    /// The person's answer to request `id` of `session_id`: `approved` or
    /// `denied`, once. A request past its expiry is `expired`, whatever the
    /// answer.
    pub fn decide_mode(
        &self,
        session_id: &str,
        id: &str,
        approve: bool,
        device: Option<&str>,
    ) -> AgentResult<ModeRequest> {
        let decided =
            {
                let store = self.store.lock().unwrap();
                let r = by_id(&store.conn, id)?
                    .filter(|r| r.session_id == session_id)
                    .ok_or(AgentError::NotFound("no such request"))?;
                let r = settle_expired(&store.conn, r)?;
                if !r.pending() {
                    return Err(AgentError::Conflict(format!("request_{}", r.state)));
                }
                store.conn.execute(
                "UPDATE mode_requests SET outcome = ?2, decided_by_device = ?3, decided_at = ?4
                 WHERE id = ?1 AND outcome IS NULL",
                params![id, if approve { APPROVED } else { DENIED }, device, now()],
            ).map_err(anyhow::Error::from)?;
                by_id(&store.conn, id)?.expect("just decided")
            };
        self.bump(session_id);
        Ok(decided)
    }

    /// Records where the approved switch resumed to, or that it failed.
    pub fn record_switch(&self, id: &str, to: Option<&str>) -> AgentResult<()> {
        let session =
            {
                let store = self.store.lock().unwrap();
                store.conn.execute(
                "UPDATE mode_requests SET switched_to = ?2, switch_failed = ?3 WHERE id = ?1",
                params![id, to, i64::from(to.is_none())],
            ).map_err(anyhow::Error::from)?;
                by_id(&store.conn, id)?.map(|r| r.session_id)
            };
        if let Some(s) = session {
            self.bump(&s);
        }
        if let Some(to) = to {
            self.bump(to);
        }
        Ok(())
    }

    /// Changes a session's stored mode to auto for its next start (a parked
    /// child resumes with it, 145e).
    pub fn set_mode_for_switch(&self, session_id: &str) -> AgentResult<()> {
        let store = self.store.lock().unwrap();
        store
            .conn
            .execute(
                "UPDATE session_launch SET permission_mode = 'auto' WHERE session_id = ?1",
                [session_id],
            )
            .map_err(anyhow::Error::from)?;
        Ok(())
    }

    /// Asks a child's host to park it for the switch (145e's stop, which
    /// keeps its conversation and workspace).
    pub fn park_for_switch(&self, session_id: &str) -> AgentResult<()> {
        let r = self.get(session_id)?;
        if r.status == "parked" {
            return Ok(());
        }
        self.live
            .lock()
            .unwrap()
            .sessions
            .entry(session_id.to_string())
            .or_insert_with(super::SessionLive::new)
            .parking = true;
        let sent = self.send(
            &r.host_id,
            super::Command::Park {
                session: session_id.to_string(),
            },
        );
        if sent.is_err() {
            self.take_parking(session_id);
        }
        sent
    }

    #[cfg(test)]
    pub fn expire_mode_requests_for_tests(&self, session_id: &str) {
        let store = self.store.lock().unwrap();
        store
            .conn
            .execute(
                "UPDATE mode_requests SET expires_at = '2000-01-01T00:00:00Z'
                 WHERE session_id = ?1 AND outcome IS NULL",
                [session_id],
            )
            .unwrap();
    }
}
