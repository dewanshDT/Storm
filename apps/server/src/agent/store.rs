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

/// A granted connection as the host is told of it (AM23): an id and a slug,
/// **never a credential and never a user**.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpGrant {
    pub id: String,
    pub slug: String,
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
             CREATE INDEX IF NOT EXISTS sessions_by_host ON sessions(host_id);

             -- The MCP Gateway's grants (spec §6, decision 81e), snapshotted at
             -- launch and fixed for the session's life. Additive: two new
             -- tables, the sessions table untouched.
             CREATE TABLE IF NOT EXISTS session_mcp (
                 session_id         TEXT PRIMARY KEY,
                 allow_vault_writes INTEGER NOT NULL,
                 created_at         TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS session_mcp_grants (
                 session_id    TEXT NOT NULL,
                 connection_id TEXT NOT NULL,
                 slug          TEXT NOT NULL,
                 granted_at    TEXT NOT NULL,
                 -- Set when the connection is disconnected (§13). The row is
                 -- kept for the audit; a revoked grant authorizes nothing.
                 revoked_at    TEXT,
                 PRIMARY KEY (session_id, connection_id)
             );
             CREATE INDEX IF NOT EXISTS grants_by_connection
                 ON session_mcp_grants(connection_id);",
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

    /// Records a session's grants and its vault-write flag, in one
    /// transaction with nothing else: the launch writes these before `start`
    /// is sent, so a host can never be told of a grant that is not on disk.
    pub fn insert_grants(
        &mut self,
        session_id: &str,
        allow_vault_writes: bool,
        grants: &[McpGrant],
        now: &str,
    ) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO session_mcp (session_id, allow_vault_writes, created_at)
             VALUES (?1, ?2, ?3)",
            params![session_id, allow_vault_writes, now],
        )?;
        for g in grants {
            tx.execute(
                "INSERT INTO session_mcp_grants (session_id, connection_id, slug, granted_at)
                 VALUES (?1, ?2, ?3, ?4)",
                params![session_id, g.id, g.slug, now],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// A live grant: the connection's slug and the session's write flag, or
    /// `None` when there is no grant or it was revoked.
    pub fn grant(&self, session_id: &str, connection_id: &str) -> Result<Option<(String, bool)>> {
        Ok(self
            .conn
            .query_row(
                "SELECT g.slug, m.allow_vault_writes
                 FROM session_mcp_grants g JOIN session_mcp m ON m.session_id = g.session_id
                 WHERE g.session_id = ?1 AND g.connection_id = ?2 AND g.revoked_at IS NULL",
                params![session_id, connection_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?)
    }

    /// A session's live grants, for the record the owner sees.
    #[cfg(test)]
    pub fn grants_of(&self, session_id: &str) -> Result<Vec<McpGrant>> {
        let mut stmt = self.conn.prepare(
            "SELECT connection_id, slug FROM session_mcp_grants
             WHERE session_id = ?1 AND revoked_at IS NULL ORDER BY slug",
        )?;
        let rows = stmt.query_map(params![session_id], |r| {
            Ok(McpGrant {
                id: r.get(0)?,
                slug: r.get(1)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Marks every grant of a connection revoked (§13); how many changed.
    pub fn revoke_grants_for(&self, connection_id: &str, now: &str) -> Result<usize> {
        Ok(self.conn.execute(
            "UPDATE session_mcp_grants SET revoked_at = ?2
             WHERE connection_id = ?1 AND revoked_at IS NULL",
            params![connection_id, now],
        )?)
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
