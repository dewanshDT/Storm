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

/// What a session was launched with, beyond the record: read back on every
/// view, and what Run again prefills from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchRecord {
    pub session_id: String,
    pub name: String,
    pub context: Option<Context>,
    pub write_vault_id: Option<String>,
}

/// The note a session was started from. The title is a snapshot.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Context {
    pub vault_id: String,
    pub note_id: String,
    pub title: String,
}

/// One note or kit script an agent session created or edited. A note has an
/// id and a version; a script has neither, only its vault-relative path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteRecord {
    pub session_id: String,
    pub vault_id: String,
    pub note_id: Option<String>,
    pub path: Option<String>,
    /// `created` or `edited` for a note, `script_created` or `script_edited`
    /// for a script.
    pub kind: String,
    pub version: Option<i64>,
    pub at: String,
}

impl WriteRecord {
    fn target(&self) -> String {
        match (&self.note_id, &self.path) {
            (Some(id), _) => id.clone(),
            (None, Some(path)) => format!("script:{path}"),
            (None, None) => String::new(),
        }
    }
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
                 ON session_mcp_grants(connection_id);

             -- What a session was launched with. Kept
             -- when the session is dismissed, so provenance still resolves.
             CREATE TABLE IF NOT EXISTS session_launch (
                 session_id       TEXT PRIMARY KEY,
                 name             TEXT NOT NULL,
                 context_vault_id TEXT,
                 context_note_id  TEXT,
                 context_title    TEXT,
                 write_vault_id   TEXT,
                 created_at       TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS session_writes (
                 session_id TEXT NOT NULL,
                 vault_id   TEXT NOT NULL,
                 -- The note id, or `script:<path>` for a kit script.
                 target     TEXT NOT NULL,
                 note_id    TEXT,
                 path       TEXT,
                 kind       TEXT NOT NULL CHECK (kind IN
                     ('created', 'edited', 'script_created', 'script_edited')),
                 version    INTEGER,
                 at         TEXT NOT NULL,
                 PRIMARY KEY (session_id, vault_id, target)
             );
             CREATE INDEX IF NOT EXISTS writes_by_note
                 ON session_writes(vault_id, note_id, at);",
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

    pub fn insert_launch(&self, l: &LaunchRecord, now: &str) -> Result<()> {
        let (vault, note, title) = match &l.context {
            Some(c) => (Some(&c.vault_id), Some(&c.note_id), Some(&c.title)),
            None => (None, None, None),
        };
        self.conn.execute(
            "INSERT INTO session_launch (session_id, name, context_vault_id, context_note_id,
                 context_title, write_vault_id, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                l.session_id,
                l.name,
                vault,
                note,
                title,
                l.write_vault_id,
                now
            ],
        )?;
        Ok(())
    }

    pub fn launch(&self, session_id: &str) -> Result<Option<LaunchRecord>> {
        Ok(self
            .conn
            .query_row(
                "SELECT session_id, name, context_vault_id, context_note_id, context_title,
                     write_vault_id
                 FROM session_launch WHERE session_id = ?1",
                params![session_id],
                |r| {
                    let vault: Option<String> = r.get(2)?;
                    let note: Option<String> = r.get(3)?;
                    Ok(LaunchRecord {
                        session_id: r.get(0)?,
                        name: r.get(1)?,
                        context: vault.zip(note).map(|(vault_id, note_id)| Context {
                            vault_id,
                            note_id,
                            title: r
                                .get::<_, Option<String>>(4)
                                .ok()
                                .flatten()
                                .unwrap_or_default(),
                        }),
                        write_vault_id: r.get(5)?,
                    })
                },
            )
            .optional()?)
    }

    /// The names of sessions that have not ended, for de-duplication.
    pub fn live_names(&self) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT l.name FROM session_launch l JOIN sessions s ON s.id = l.session_id
             WHERE s.status NOT IN ('completed', 'failed', 'stopped')",
        )?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// One row per note or script a session wrote. One it created and then
    /// edited stays created; the version and time follow the latest write.
    pub fn record_write(&self, w: &WriteRecord) -> Result<()> {
        self.conn.execute(
            "INSERT INTO session_writes (session_id, vault_id, target, note_id, path, kind,
                 version, at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT (session_id, vault_id, target)
             DO UPDATE SET version = excluded.version, at = excluded.at",
            params![
                w.session_id,
                w.vault_id,
                w.target(),
                w.note_id,
                w.path,
                w.kind,
                w.version,
                w.at
            ],
        )?;
        Ok(())
    }

    /// A session's writes, newest first.
    pub fn writes_of(&self, session_id: &str) -> Result<Vec<WriteRecord>> {
        self.writes_where(
            "session_id = ?1 ORDER BY at DESC, rowid DESC",
            params![session_id],
        )
    }

    pub fn write_count(&self, session_id: &str) -> Result<i64> {
        Ok(self.conn.query_row(
            "SELECT COUNT(*) FROM session_writes WHERE session_id = ?1",
            params![session_id],
            |r| r.get(0),
        )?)
    }

    /// The latest agent write to each note of a vault that agents wrote.
    /// Versions only grow, so the highest version is the latest writer.
    pub fn latest_writes(&self, vault_id: &str) -> Result<Vec<WriteRecord>> {
        self.writes_where(
            "vault_id = ?1 AND note_id IS NOT NULL AND NOT EXISTS (
                 SELECT 1 FROM session_writes newer
                 WHERE newer.vault_id = session_writes.vault_id
                   AND newer.note_id = session_writes.note_id
                   AND (newer.version > session_writes.version
                        OR (newer.version = session_writes.version
                            AND newer.at > session_writes.at)))
             ORDER BY note_id",
            params![vault_id],
        )
    }

    pub fn latest_write(&self, vault_id: &str, note_id: &str) -> Result<Option<WriteRecord>> {
        Ok(self
            .writes_where(
                "vault_id = ?1 AND note_id = ?2 ORDER BY version DESC, at DESC LIMIT 1",
                params![vault_id, note_id],
            )?
            .into_iter()
            .next())
    }

    fn writes_where(&self, clause: &str, args: impl rusqlite::Params) -> Result<Vec<WriteRecord>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT session_id, vault_id, note_id, path, kind, version, at FROM session_writes
             WHERE {clause}"
        ))?;
        let rows = stmt.query_map(args, |r| {
            Ok(WriteRecord {
                session_id: r.get(0)?,
                vault_id: r.get(1)?,
                note_id: r.get(2)?,
                path: r.get(3)?,
                kind: r.get(4)?,
                version: r.get(5)?,
                at: r.get(6)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
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
