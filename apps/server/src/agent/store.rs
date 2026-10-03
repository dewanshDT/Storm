//! `state/agent/agent.db`: agent session records (freeze §7.1, §13).
//!
//! Durable but rebuildable in spirit: if this file is lost, the sessions a
//! host still runs are re-adopted from its next `hello`. `auth.db` holds the
//! host registry; this holds only sessions.

use std::path::Path;

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

/// What the client and the server's logs see of a session.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SessionRecord {
    pub id: String,
    pub owner_user_id: String,
    pub host_id: String,
    pub workspace: String,
    pub provider: String,
    pub provider_kind: String,
    pub interaction: String,
    pub provider_fallback: Option<Fallback>,
    pub status: String,
    pub end_reason: Option<String>,
    pub signal: Option<i32>,
    pub exit_code: Option<i32>,
    pub created_at: String,
    pub started_at: Option<String>,
    pub ended_at: Option<String>,
    pub last_activity: Option<String>,
    /// Sessions inherit their host's network policy in V1 (freeze §12.2).
    pub egress: String,
    pub cols: u16,
    pub rows: u16,
}

impl SessionRecord {
    pub fn is_ended(&self) -> bool {
        matches!(self.status.as_str(), "completed" | "failed" | "stopped")
    }
}

/// A provider other than the one asked for, said out loud (freeze §6).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Fallback {
    pub requested: String,
    pub used: String,
    pub reason: String,
}

pub struct Store {
    conn: Connection,
}

const COLUMNS: &str = "id, owner_user_id, host_id, workspace, provider, provider_kind, \
    interaction, provider_fallback, status, end_reason, signal, exit_code, created_at, \
    started_at, ended_at, last_activity, egress, cols, rows";

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        Self::init(conn)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             CREATE TABLE IF NOT EXISTS sessions (
                 id                TEXT PRIMARY KEY,
                 owner_user_id     TEXT NOT NULL,
                 host_id           TEXT NOT NULL,
                 workspace         TEXT NOT NULL,
                 provider          TEXT NOT NULL,
                 provider_kind     TEXT NOT NULL,
                 interaction       TEXT NOT NULL,
                 provider_fallback TEXT,
                 status            TEXT NOT NULL,
                 end_reason        TEXT,
                 signal            INTEGER,
                 exit_code         INTEGER,
                 created_at        TEXT NOT NULL,
                 started_at        TEXT,
                 ended_at          TEXT,
                 last_activity     TEXT,
                 egress            TEXT NOT NULL DEFAULT 'host',
                 cols              INTEGER NOT NULL,
                 rows              INTEGER NOT NULL
             );
             CREATE INDEX IF NOT EXISTS sessions_by_host ON sessions(host_id);",
        )?;
        Ok(Self { conn })
    }

    /// At boot, every session that has not ended is `unknown` until its host
    /// reports it (freeze §13). A server that just started knows nothing about
    /// what its hosts did while it was down, and saying `running` would be a
    /// guess.
    pub fn mark_unknown_on_boot(&self) -> Result<usize> {
        Ok(self.conn.execute(
            "UPDATE sessions SET status = 'unknown'
             WHERE status NOT IN ('completed', 'failed', 'stopped')",
            [],
        )?)
    }

    pub fn insert(&self, r: &SessionRecord) -> Result<()> {
        self.conn.execute(
            &format!("INSERT INTO sessions ({COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)"),
            params![
                r.id,
                r.owner_user_id,
                r.host_id,
                r.workspace,
                r.provider,
                r.provider_kind,
                r.interaction,
                r.provider_fallback
                    .as_ref()
                    .map(|f| serde_json::to_string(f).expect("serializable")),
                r.status,
                r.end_reason,
                r.signal,
                r.exit_code,
                r.created_at,
                r.started_at,
                r.ended_at,
                r.last_activity,
                r.egress,
                r.cols,
                r.rows,
            ],
        )?;
        Ok(())
    }

    pub fn get(&self, id: &str) -> Result<Option<SessionRecord>> {
        Ok(self
            .conn
            .query_row(
                &format!("SELECT {COLUMNS} FROM sessions WHERE id = ?1"),
                params![id],
                from_row,
            )
            .optional()?)
    }

    pub fn list(&self) -> Result<Vec<SessionRecord>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {COLUMNS} FROM sessions ORDER BY created_at, id"
        ))?;
        let rows = stmt.query_map([], from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn for_host(&self, host_id: &str) -> Result<Vec<SessionRecord>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {COLUMNS} FROM sessions WHERE host_id = ?1 ORDER BY created_at, id"
        ))?;
        let rows = stmt.query_map(params![host_id], from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Writes the whole record back. Records are small and every change is a
    /// state transition worth persisting in full.
    pub fn update(&self, r: &SessionRecord) -> Result<()> {
        self.conn.execute(
            "UPDATE sessions SET status = ?2, end_reason = ?3, signal = ?4, exit_code = ?5,
                 started_at = ?6, ended_at = ?7, last_activity = ?8, cols = ?9, rows = ?10
             WHERE id = ?1",
            params![
                r.id,
                r.status,
                r.end_reason,
                r.signal,
                r.exit_code,
                r.started_at,
                r.ended_at,
                r.last_activity,
                r.cols,
                r.rows
            ],
        )?;
        Ok(())
    }

    pub fn delete(&self, id: &str) -> Result<bool> {
        Ok(self
            .conn
            .execute("DELETE FROM sessions WHERE id = ?1", params![id])?
            > 0)
    }
}

fn from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<SessionRecord> {
    let fallback: Option<String> = r.get(7)?;
    Ok(SessionRecord {
        id: r.get(0)?,
        owner_user_id: r.get(1)?,
        host_id: r.get(2)?,
        workspace: r.get(3)?,
        provider: r.get(4)?,
        provider_kind: r.get(5)?,
        interaction: r.get(6)?,
        provider_fallback: fallback.and_then(|s| serde_json::from_str(&s).ok()),
        status: r.get(8)?,
        end_reason: r.get(9)?,
        signal: r.get(10)?,
        exit_code: r.get(11)?,
        created_at: r.get(12)?,
        started_at: r.get(13)?,
        ended_at: r.get(14)?,
        last_activity: r.get(15)?,
        egress: r.get(16)?,
        cols: r.get(17)?,
        rows: r.get(18)?,
    })
}
