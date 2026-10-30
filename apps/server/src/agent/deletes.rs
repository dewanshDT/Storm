//! Decision 149: an agent deletes only with the person's Approve. A
//! writable top-level session's `delete_note`/`delete_folder` makes one
//! request, bound to its target as it was when asked (a note's version and
//! path, a folder's note set), which only a person approves or declines,
//! and which times out after [`EXPIRES_AFTER_SECS`]. One request waits per
//! session; asking again for the same target waits on it, asking for
//! another is `busy`. The deletion itself runs when the person approves
//! (`ops::decide_delete_request`), so a target that changed meanwhile is
//! `gone` and nothing is deleted. The permission mode never skips it.

use std::time::{Duration, Instant};

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use super::store::spawn::expired;
use super::{AgentError, AgentManager, AgentResult, now};

/// How long a request waits for the person.
pub const EXPIRES_AFTER_SECS: i64 = 600;

/// A reason is 1 to this many characters.
pub const MAX_REASON_CHARS: usize = 500;

/// How many of a folder's paths a request keeps for the dialog.
pub const MAX_PATHS: usize = 100;

/// How long an answered request stays listed, so another client can close
/// its dialog with who answered.
pub const SHOWN_AFTER_SECS: i64 = 120;

pub const PENDING: &str = "pending";
pub const DELETED: &str = "deleted";
pub const DECLINED: &str = "declined";
pub const TIMED_OUT: &str = "timed_out";
pub const GONE: &str = "gone";

pub const NOTE: &str = "note";
pub const FOLDER: &str = "folder";

pub(super) fn init(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "-- 149: an agent's deletion, decided once by a person.
         CREATE TABLE IF NOT EXISTS delete_requests (
             id                TEXT PRIMARY KEY,
             session_id        TEXT NOT NULL,
             vault_id          TEXT NOT NULL,
             kind              TEXT NOT NULL CHECK (kind IN ('note', 'folder')),
             note_id           TEXT,
             path              TEXT NOT NULL,
             title             TEXT,
             reason            TEXT NOT NULL CHECK (length(reason) BETWEEN 1 AND 500),
             fingerprint       TEXT NOT NULL,
             notes             INTEGER NOT NULL,
             size              INTEGER NOT NULL,
             modified          TEXT,
             paths             TEXT NOT NULL,
             created_at        TEXT NOT NULL,
             expires_at        TEXT NOT NULL,
             outcome           TEXT CHECK (outcome IN ('deleted', 'declined', 'timed_out', 'gone')),
             decided_by_device TEXT,
             decided_by_name   TEXT,
             decided_at        TEXT,
             trash_id          TEXT,
             CHECK ((outcome IS NULL) = (decided_at IS NULL)),
             CHECK ((kind = 'note') = (note_id IS NOT NULL))
         );
         CREATE UNIQUE INDEX IF NOT EXISTS delete_requests_one_pending
             ON delete_requests(session_id) WHERE outcome IS NULL;
         CREATE INDEX IF NOT EXISTS delete_requests_by_decided
             ON delete_requests(decided_at);
         CREATE TRIGGER IF NOT EXISTS delete_request_fixed
             BEFORE UPDATE OF id, session_id, vault_id, kind, note_id, path, reason,
                 fingerprint, created_at ON delete_requests
             BEGIN SELECT RAISE(ABORT, 'a request is fixed when it is made'); END;
         CREATE TRIGGER IF NOT EXISTS delete_request_decided_once
             BEFORE UPDATE OF outcome, decided_by_device, decided_by_name, decided_at, trash_id
             ON delete_requests
             WHEN OLD.outcome IS NOT NULL
             BEGIN SELECT RAISE(ABORT, 'a request is decided once'); END;",
    )?;
    Ok(())
}

/// What is to be deleted, as it was when the agent asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeleteTarget {
    pub vault_id: String,
    pub kind: &'static str,
    pub note_id: Option<String>,
    pub path: String,
    pub title: Option<String>,
    /// A note's version and path, or a hash of a folder's notes and their
    /// versions: what approval is bound to.
    pub fingerprint: String,
    pub notes: i64,
    pub size: i64,
    pub modified: Option<String>,
    pub paths: Vec<String>,
}

impl DeleteTarget {
    fn same_target(&self, r: &DeleteRequest) -> bool {
        self.vault_id == r.vault_id
            && self.kind == r.kind
            && self.note_id == r.note_id
            && (self.kind == NOTE || self.path == r.path)
    }
}

/// One request, as the dialog and the tool see it. The reason is the
/// agent's own text: data to show, never an instruction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DeleteRequest {
    pub id: String,
    pub session_id: String,
    pub vault_id: String,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note_id: Option<String>,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub reason: String,
    #[serde(skip)]
    pub fingerprint: String,
    /// The notes it holds: 1 for a note.
    pub notes: i64,
    /// Bytes: the note's, or the sum of the folder's notes.
    pub size: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modified: Option<String>,
    /// The first [`MAX_PATHS`] paths, sorted.
    pub paths: Vec<String>,
    pub created_at: String,
    pub expires_at: String,
    /// `pending`, else the outcome.
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decided_at: Option<String>,
    /// The device that answered, by its name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decided_by: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trash_id: Option<String>,
}

impl DeleteRequest {
    pub fn pending(&self) -> bool {
        self.state == PENDING
    }
}

const COLUMNS: &str = "id, session_id, vault_id, kind, note_id, path, title, reason, fingerprint, \
    notes, size, modified, paths, created_at, expires_at, outcome, decided_at, decided_by_name, \
    trash_id";

fn from_row(r: &rusqlite::Row) -> rusqlite::Result<DeleteRequest> {
    let paths: String = r.get(12)?;
    let outcome: Option<String> = r.get(15)?;
    Ok(DeleteRequest {
        id: r.get(0)?,
        session_id: r.get(1)?,
        vault_id: r.get(2)?,
        kind: r.get(3)?,
        note_id: r.get(4)?,
        path: r.get(5)?,
        title: r.get(6)?,
        reason: r.get(7)?,
        fingerprint: r.get(8)?,
        notes: r.get(9)?,
        size: r.get(10)?,
        modified: r.get(11)?,
        paths: serde_json::from_str(&paths).unwrap_or_default(),
        created_at: r.get(13)?,
        expires_at: r.get(14)?,
        state: outcome.unwrap_or_else(|| PENDING.into()),
        decided_at: r.get(16)?,
        decided_by: r.get(17)?,
        trash_id: r.get(18)?,
    })
}

fn by_id(conn: &Connection, id: &str) -> Result<Option<DeleteRequest>> {
    Ok(conn
        .query_row(
            &format!("SELECT {COLUMNS} FROM delete_requests WHERE id = ?1"),
            [id],
            from_row,
        )
        .optional()?)
}

fn pending_of(conn: &Connection, session_id: &str) -> Result<Option<DeleteRequest>> {
    Ok(conn
        .query_row(
            &format!(
                "SELECT {COLUMNS} FROM delete_requests WHERE session_id = ?1 AND outcome IS NULL"
            ),
            [session_id],
            from_row,
        )
        .optional()?)
}

fn shown(conn: &Connection, since: &str) -> Result<Vec<DeleteRequest>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM delete_requests
         WHERE outcome IS NULL OR decided_at >= ?1
         ORDER BY created_at, rowid"
    ))?;
    let rows = stmt
        .query_map([since], from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Times out every pending request past its expiry, or whose session has
/// ended: nothing waits on it any more.
fn settle(conn: &Connection) -> Result<usize> {
    let at = now();
    let mut stmt = conn.prepare(
        "SELECT d.id, d.expires_at, s.status FROM delete_requests d
         LEFT JOIN sessions s ON s.id = d.session_id WHERE d.outcome IS NULL",
    )?;
    let due: Vec<String> = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
            ))
        })?
        .filter_map(|r| r.ok())
        .filter(|(_, expires, status)| {
            expired(&at, expires)
                || matches!(
                    status.as_deref(),
                    None | Some("completed" | "failed" | "stopped")
                )
        })
        .map(|(id, ..)| id)
        .collect();
    for id in &due {
        conn.execute(
            "UPDATE delete_requests SET outcome = 'timed_out', decided_at = ?2
             WHERE id = ?1 AND outcome IS NULL",
            params![id, at],
        )?;
    }
    Ok(due.len())
}

/// Why a request is not made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// Another of the session's deletions waits for the person.
    Busy(Box<DeleteRequest>),
    /// Ended, or not running.
    NotLive,
}

impl AgentManager {
    fn settled(&self) -> AgentResult<()> {
        let n = settle(&self.store.lock().unwrap().conn)?;
        if n > 0 {
            self.poke_deletes();
        }
        Ok(())
    }

    /// Something a client's list of requests shows changed.
    fn poke_deletes(&self) {
        self.deletes_changed.send_modify(|v| *v += 1);
    }

    /// A version of the requests' list, to wait for a change on.
    pub fn watch_deletes(&self) -> tokio::sync::watch::Receiver<u64> {
        self.deletes_changed.subscribe()
    }

    pub fn delete_request(&self, id: &str) -> AgentResult<Option<DeleteRequest>> {
        self.settled()?;
        Ok(by_id(&self.store.lock().unwrap().conn, id)?)
    }

    /// `session_id`'s request waiting for the person, if any.
    pub fn pending_delete_of(&self, session_id: &str) -> AgentResult<Option<DeleteRequest>> {
        self.settled()?;
        Ok(pending_of(&self.store.lock().unwrap().conn, session_id)?)
    }

    /// Every request waiting for the person, and those answered in the last
    /// [`SHOWN_AFTER_SECS`], oldest first.
    pub fn shown_delete_requests(&self) -> AgentResult<Vec<DeleteRequest>> {
        self.settled()?;
        let since = crate::index::rfc3339_minus_secs(&now(), SHOWN_AFTER_SECS);
        Ok(shown(&self.store.lock().unwrap().conn, &since)?)
    }

    /// Makes `session_id`'s request to delete `target`, or returns the one
    /// already waiting for the same target. `Ok((request, true))` when this
    /// call made it.
    pub fn request_delete(
        &self,
        session_id: &str,
        target: &DeleteTarget,
        reason: &str,
    ) -> AgentResult<Result<(DeleteRequest, bool), Refusal>> {
        let r = self.get(session_id)?;
        if !matches!(r.status.as_str(), "starting" | "running") {
            return Ok(Err(Refusal::NotLive));
        }
        self.settled()?;
        let store = self.store.lock().unwrap();
        if let Some(open) = pending_of(&store.conn, session_id)? {
            return Ok(if target.same_target(&open) {
                Ok((open, false))
            } else {
                Err(Refusal::Busy(Box::new(open)))
            });
        }
        let created = now();
        let id = crate::auth::identity::random_id("adr_");
        let paths: Vec<&String> = target.paths.iter().take(MAX_PATHS).collect();
        store
            .conn
            .execute(
                "INSERT INTO delete_requests (id, session_id, vault_id, kind, note_id, path,
                     title, reason, fingerprint, notes, size, modified, paths, created_at,
                     expires_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
                params![
                    id,
                    session_id,
                    target.vault_id,
                    target.kind,
                    target.note_id,
                    target.path,
                    target.title,
                    reason,
                    target.fingerprint,
                    target.notes,
                    target.size,
                    target.modified,
                    serde_json::to_string(&paths).map_err(anyhow::Error::from)?,
                    created,
                    crate::index::rfc3339_minus_secs(&created, -EXPIRES_AFTER_SECS)
                ],
            )
            .map_err(anyhow::Error::from)?;
        let made = by_id(&store.conn, &id)?.expect("just written");
        drop(store);
        self.bump(session_id);
        self.poke_deletes();
        Ok(Ok((made, true)))
    }

    /// Waits up to `bound` for request `id` to be answered or to time out.
    pub async fn wait_delete(&self, id: &str, bound: Duration) -> AgentResult<DeleteRequest> {
        let deadline = Instant::now() + bound;
        let first = self
            .delete_request(id)?
            .ok_or(AgentError::NotFound("no such request"))?;
        let mut changed = self.watch(&first.session_id);
        loop {
            changed.borrow_and_update();
            let r = self
                .delete_request(id)?
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

    /// Records request `id`'s outcome, once: `deleted` with where it went,
    /// `declined` or `gone`. A request no longer pending is a conflict
    /// naming its state.
    pub fn decide_delete(
        &self,
        id: &str,
        outcome: &str,
        device: Option<(&str, &str)>,
        trash_id: Option<&str>,
    ) -> AgentResult<DeleteRequest> {
        self.settled()?;
        let decided = {
            let store = self.store.lock().unwrap();
            let changed = store
                .conn
                .execute(
                    "UPDATE delete_requests SET outcome = ?2, decided_by_device = ?3,
                         decided_by_name = ?4, decided_at = ?5, trash_id = ?6
                     WHERE id = ?1 AND outcome IS NULL",
                    params![
                        id,
                        outcome,
                        device.map(|d| d.0),
                        device.map(|d| d.1),
                        now(),
                        trash_id
                    ],
                )
                .map_err(anyhow::Error::from)?;
            let r = by_id(&store.conn, id)?.ok_or(AgentError::NotFound("no such request"))?;
            if changed == 0 {
                return Err(AgentError::Conflict(format!("request_{}", r.state)));
            }
            r
        };
        self.bump(&decided.session_id);
        self.poke_deletes();
        Ok(decided)
    }

    #[cfg(test)]
    pub fn expire_delete_requests_for_tests(&self, session_id: &str) {
        let store = self.store.lock().unwrap();
        store
            .conn
            .execute(
                "UPDATE delete_requests SET expires_at = '2000-01-01T00:00:00Z'
                 WHERE session_id = ?1 AND outcome IS NULL",
                [session_id],
            )
            .unwrap();
    }
}
