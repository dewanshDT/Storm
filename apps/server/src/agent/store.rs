//! `state/agent/agent.db`: agent session records (freeze §7.1, §13).
//!
//! Durable but rebuildable in spirit for sessions: if this file is lost, the
//! sessions a host still runs are re-adopted from its next `hello`. `auth.db`
//! holds the host registry.
//!
//! **Crews, their members and role labels (decision 138) are the owner's own
//! data and are not rebuilt from anything**: losing this file loses them.

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
    /// The agent's own name for the session, from its terminal title (D15
    /// AM42). Kept after the end.
    pub title: Option<String>,
    /// `working` or `idle` while running; never stored (AM42).
    pub activity: Option<String>,
    /// The agent's own id for the conversation, as its host last reported
    /// it: what a resume reopens (decision 89).
    pub harness_session: Option<String>,
    /// The session this one resumed.
    pub resumed_from: Option<String>,
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
    /// The plugin server this connection was added from (131h). An older
    /// side leaves it out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_origin: Option<PluginOrigin>,
}

/// A plugin's server, by the plugin's name and the server's (131h).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginOrigin {
    pub plugin: String,
    pub server: String,
}

/// An integration a session was granted, as it was named at launch.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SessionIntegration {
    pub id: String,
    pub slug: String,
    pub display_name: String,
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

/// A crew (decision 138a). `color` is a vault palette word.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Crew {
    pub id: String,
    pub name: String,
    pub color: String,
    pub order: i64,
}

/// The crew a session is in, and its place there.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Membership {
    pub id: String,
    pub order: i64,
}

/// A session's role label: a note, by id (138d).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoleLabel {
    pub vault_id: String,
    pub note_id: String,
}

pub struct Store {
    pub(super) conn: Connection,
}

const COLUMNS: &str = "id, owner_user_id, host_id, workspace, provider, provider_kind, \
    interaction, provider_fallback, status, end_reason, signal, exit_code, created_at, \
    started_at, ended_at, last_activity, egress, cols, rows, title, harness_session, resumed_from";

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
                 ON session_writes(vault_id, note_id, at);

             -- The integrations a session was granted, named as they were at
             -- launch: history, so a later rename or disconnect changes none.
             CREATE TABLE IF NOT EXISTS session_integrations (
                 session_id    TEXT NOT NULL,
                 connection_id TEXT NOT NULL,
                 slug          TEXT NOT NULL,
                 display_name  TEXT NOT NULL,
                 PRIMARY KEY (session_id, connection_id)
             );

             -- What a session's host said it would not have (decision 131a),
             -- from its first `running` report: server names and reasons.
             CREATE TABLE IF NOT EXISTS session_mcp_notice (
                 session_id   TEXT PRIMARY KEY,
                 skipped      TEXT NOT NULL,
                 plugin_local TEXT NOT NULL,
                 reported_at  TEXT NOT NULL
             );

             -- Crews (decision 138a): a named, coloured label over sessions.
             -- They change how the Agents page is organised and nothing else.
             CREATE TABLE IF NOT EXISTS crews (
                 id            TEXT PRIMARY KEY,
                 owner_user_id TEXT NOT NULL,
                 name          TEXT NOT NULL,
                 color         TEXT NOT NULL,
                 ord           INTEGER NOT NULL,
                 created_at    TEXT NOT NULL
             );
             -- At most one crew per session (138b): the session is the key.
             CREATE TABLE IF NOT EXISTS crew_members (
                 session_id TEXT PRIMARY KEY,
                 crew_id    TEXT NOT NULL,
                 ord        INTEGER NOT NULL
             );
             CREATE INDEX IF NOT EXISTS crew_members_by_crew
                 ON crew_members(crew_id, ord);
             -- A session's role label (138d): a Kit note, set only by a person.
             CREATE TABLE IF NOT EXISTS session_role (
                 session_id TEXT PRIMARY KEY,
                 vault_id   TEXT NOT NULL,
                 note_id    TEXT NOT NULL,
                 set_at     TEXT NOT NULL
             );
             -- Each host's bridge identity as it last reported it (decision
             -- 135d), so a host offline or not yet back since a restart is
             -- still labelled.
             CREATE TABLE IF NOT EXISTS host_identity (
                 host_id     TEXT PRIMARY KEY,
                 state       TEXT NOT NULL,
                 mechanism   TEXT,
                 reason      TEXT,
                 reported_at TEXT NOT NULL
             );",
        )?;
        // D15 AM42, additive: a database from before it gains the column.
        let has_title = conn
            .prepare("SELECT 1 FROM pragma_table_info('sessions') WHERE name = 'title'")?
            .exists([])?;
        if !has_title {
            conn.execute("ALTER TABLE sessions ADD COLUMN title TEXT", [])?;
        }
        // Decision 89, additive the same way.
        for column in ["harness_session", "resumed_from"] {
            let has = conn
                .prepare("SELECT 1 FROM pragma_table_info('sessions') WHERE name = ?1")?
                .exists([column])?;
            if !has {
                conn.execute(
                    &format!("ALTER TABLE sessions ADD COLUMN {column} TEXT"),
                    [],
                )?;
            }
        }
        spawn::init(&conn)?;
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
        insert_session(&self.conn, r)
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
                 started_at = ?6, ended_at = ?7, last_activity = ?8, cols = ?9, rows = ?10,
                 title = ?11, harness_session = ?12
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
                r.rows,
                r.title,
                r.harness_session
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
                plugin_origin: None,
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

    pub fn insert_integrations(
        &mut self,
        session_id: &str,
        integrations: &[SessionIntegration],
    ) -> Result<()> {
        let tx = self.conn.transaction()?;
        for i in integrations {
            tx.execute(
                "INSERT OR REPLACE INTO session_integrations
                     (session_id, connection_id, slug, display_name)
                 VALUES (?1, ?2, ?3, ?4)",
                params![session_id, i.id, i.slug, i.display_name],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// The integrations a session was granted at launch, revoked ones
    /// included. A session from before names were kept shows its grants'
    /// slugs; `builtin` (Storm itself) is never one.
    /// Keeps the first notice a session reports (131a), as JSON lists.
    pub fn record_mcp_notice(
        &self,
        session_id: &str,
        skipped: &str,
        plugin_local: &str,
        now: &str,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO session_mcp_notice
                 (session_id, skipped, plugin_local, reported_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![session_id, skipped, plugin_local, now],
        )?;
        Ok(())
    }

    pub fn mcp_notice(&self, session_id: &str) -> Result<Option<(String, String)>> {
        Ok(self
            .conn
            .query_row(
                "SELECT skipped, plugin_local FROM session_mcp_notice WHERE session_id = ?1",
                params![session_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?)
    }

    pub fn integrations_of(
        &self,
        session_id: &str,
        builtin: &str,
    ) -> Result<Vec<SessionIntegration>> {
        let named = self.integrations_where(
            "SELECT connection_id, slug, display_name FROM session_integrations
             WHERE session_id = ?1 AND connection_id != ?2 ORDER BY display_name, slug",
            session_id,
            builtin,
        )?;
        if !named.is_empty() {
            return Ok(named);
        }
        self.integrations_where(
            "SELECT connection_id, slug, slug FROM session_mcp_grants
             WHERE session_id = ?1 AND connection_id != ?2 ORDER BY slug",
            session_id,
            builtin,
        )
    }

    fn integrations_where(
        &self,
        sql: &str,
        session_id: &str,
        builtin: &str,
    ) -> Result<Vec<SessionIntegration>> {
        let mut stmt = self.conn.prepare(sql)?;
        let rows = stmt.query_map(params![session_id, builtin], |r| {
            Ok(SessionIntegration {
                id: r.get(0)?,
                slug: r.get(1)?,
                display_name: r.get(2)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
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

    /// Removes a session's record, its crew membership and its role label in
    /// one transaction. A crew left with no members is deleted with it and
    /// returned (decision 138c). Its launch and writes are kept, so provenance
    /// still resolves; so is its lineage (decision 134, L-1).
    pub fn delete(&mut self, id: &str) -> Result<(bool, Option<Crew>)> {
        let tx = self.conn.transaction()?;
        let gone = tx.execute("DELETE FROM sessions WHERE id = ?1", params![id])? > 0;
        let crew = leave_crew(&tx, id)?;
        tx.execute(
            "DELETE FROM session_role WHERE session_id = ?1",
            params![id],
        )?;
        spawn::forget(&tx, id)?;
        tx.commit()?;
        Ok((gone, crew))
    }

    // ---- crews (decision 138) ---------------------------------------------

    pub fn crews(&self) -> Result<Vec<Crew>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, name, color, ord FROM crews ORDER BY ord, created_at, id")?;
        let rows = stmt.query_map([], crew_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn crew(&self, id: &str) -> Result<Option<Crew>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id, name, color, ord FROM crews WHERE id = ?1",
                params![id],
                crew_from_row,
            )
            .optional()?)
    }

    /// The sessions in a crew, in order.
    pub fn members(&self, crew_id: &str) -> Result<Vec<String>> {
        members_of(&self.conn, crew_id)
    }

    pub fn membership(&self, session_id: &str) -> Result<Option<Membership>> {
        Ok(self
            .conn
            .query_row(
                "SELECT crew_id, ord FROM crew_members WHERE session_id = ?1",
                params![session_id],
                |r| {
                    Ok(Membership {
                        id: r.get(0)?,
                        order: r.get(1)?,
                    })
                },
            )
            .optional()?)
    }

    /// Creates a crew at the end of the list and moves `sessions` into it, in
    /// that order. Crews the move empties are deleted and returned.
    pub fn create_crew(
        &mut self,
        crew: &Crew,
        owner_user_id: &str,
        sessions: &[String],
        now: &str,
    ) -> Result<(Crew, Vec<Crew>)> {
        let tx = self.conn.transaction()?;
        let order: i64 = tx.query_row("SELECT COALESCE(MAX(ord) + 1, 0) FROM crews", [], |r| {
            r.get(0)
        })?;
        tx.execute(
            "INSERT INTO crews (id, owner_user_id, name, color, ord, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![crew.id, owner_user_id, crew.name, crew.color, order, now],
        )?;
        let mut removed = Vec::new();
        for (i, s) in sessions.iter().enumerate() {
            if let Some(emptied) = leave_crew(&tx, s)? {
                removed.push(emptied);
            }
            tx.execute(
                "INSERT INTO crew_members (session_id, crew_id, ord) VALUES (?1, ?2, ?3)",
                params![s, crew.id, i as i64],
            )?;
        }
        tx.commit()?;
        let created = Crew {
            order,
            ..crew.clone()
        };
        Ok((created, removed))
    }

    pub fn update_crew(&self, id: &str, name: Option<&str>, color: Option<&str>) -> Result<()> {
        self.conn.execute(
            "UPDATE crews SET name = COALESCE(?2, name), color = COALESCE(?3, color)
             WHERE id = ?1",
            params![id, name, color],
        )?;
        Ok(())
    }

    /// Renumbers the crews in the given order. The caller checks that `ids`
    /// is every crew exactly once.
    pub fn reorder_crews(&mut self, ids: &[String]) -> Result<()> {
        let tx = self.conn.transaction()?;
        for (i, id) in ids.iter().enumerate() {
            tx.execute(
                "UPDATE crews SET ord = ?2 WHERE id = ?1",
                params![id, i as i64],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Renumbers a crew's members. The caller checks that `sessions` is every
    /// member exactly once.
    pub fn reorder_members(&mut self, crew_id: &str, sessions: &[String]) -> Result<()> {
        let tx = self.conn.transaction()?;
        for (i, s) in sessions.iter().enumerate() {
            tx.execute(
                "UPDATE crew_members SET ord = ?3 WHERE session_id = ?1 AND crew_id = ?2",
                params![s, crew_id, i as i64],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Deletes a crew; its members become loose.
    pub fn delete_crew(&mut self, id: &str) -> Result<bool> {
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM crew_members WHERE crew_id = ?1", params![id])?;
        let gone = tx.execute("DELETE FROM crews WHERE id = ?1", params![id])? > 0;
        tx.commit()?;
        Ok(gone)
    }

    /// Puts a session in `crew_id` before `before` (or last), or takes it out
    /// of any crew when `crew_id` is `None`. A crew the move empties is
    /// deleted and returned (138c). The caller checks that the crew and
    /// `before` exist.
    pub fn move_session(
        &mut self,
        session_id: &str,
        crew_id: Option<&str>,
        before: Option<&str>,
    ) -> Result<Option<Crew>> {
        let tx = self.conn.transaction()?;
        let from: Option<String> = tx
            .query_row(
                "SELECT crew_id FROM crew_members WHERE session_id = ?1",
                params![session_id],
                |r| r.get(0),
            )
            .optional()?;
        tx.execute(
            "DELETE FROM crew_members WHERE session_id = ?1",
            params![session_id],
        )?;
        if let Some(to) = crew_id {
            let mut order = members_of(&tx, to)?;
            let at = before
                .and_then(|b| order.iter().position(|s| s == b))
                .unwrap_or(order.len());
            order.insert(at, session_id.to_string());
            for (i, s) in order.iter().enumerate() {
                tx.execute(
                    "INSERT INTO crew_members (session_id, crew_id, ord) VALUES (?1, ?2, ?3)
                     ON CONFLICT (session_id) DO UPDATE SET crew_id = excluded.crew_id,
                         ord = excluded.ord",
                    params![s, to, i as i64],
                )?;
            }
        }
        let emptied = match from {
            Some(from) if Some(from.as_str()) != crew_id => delete_if_empty(&tx, &from)?,
            _ => None,
        };
        tx.commit()?;
        Ok(emptied)
    }

    pub fn role_of(&self, session_id: &str) -> Result<Option<RoleLabel>> {
        Ok(self
            .conn
            .query_row(
                "SELECT vault_id, note_id FROM session_role WHERE session_id = ?1",
                params![session_id],
                |r| {
                    Ok(RoleLabel {
                        vault_id: r.get(0)?,
                        note_id: r.get(1)?,
                    })
                },
            )
            .optional()?)
    }

    pub fn set_role(&self, session_id: &str, role: Option<&RoleLabel>, now: &str) -> Result<()> {
        match role {
            Some(role) => self.conn.execute(
                "INSERT INTO session_role (session_id, vault_id, note_id, set_at)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT (session_id) DO UPDATE SET vault_id = excluded.vault_id,
                     note_id = excluded.note_id, set_at = excluded.set_at",
                params![session_id, role.vault_id, role.note_id, now],
            )?,
            None => self.conn.execute(
                "DELETE FROM session_role WHERE session_id = ?1",
                params![session_id],
            )?,
        };
        Ok(())
    }

    pub fn host_identity(&self, host_id: &str) -> Result<Option<super::Identity>> {
        Ok(self
            .conn
            .query_row(
                "SELECT state, mechanism, reason FROM host_identity WHERE host_id = ?1",
                params![host_id],
                |r| {
                    Ok(super::Identity {
                        state: r.get(0)?,
                        mechanism: r.get(1)?,
                        reason: r.get(2)?,
                    })
                },
            )
            .optional()?)
    }

    pub fn set_host_identity(
        &self,
        host_id: &str,
        identity: &super::Identity,
        now: &str,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO host_identity (host_id, state, mechanism, reason, reported_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (host_id) DO UPDATE SET state = excluded.state,
                 mechanism = excluded.mechanism, reason = excluded.reason,
                 reported_at = excluded.reported_at",
            params![
                host_id,
                identity.state,
                identity.mechanism,
                identity.reason,
                now
            ],
        )?;
        Ok(())
    }
}

fn insert_session(conn: &Connection, r: &SessionRecord) -> Result<()> {
    conn.execute(
        &format!("INSERT INTO sessions ({COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22)"),
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
            r.title,
            r.harness_session,
            r.resumed_from,
        ],
    )?;
    Ok(())
}

fn crew_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Crew> {
    Ok(Crew {
        id: r.get(0)?,
        name: r.get(1)?,
        color: r.get(2)?,
        order: r.get(3)?,
    })
}

fn members_of(conn: &Connection, crew_id: &str) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT session_id FROM crew_members WHERE crew_id = ?1 ORDER BY ord, session_id",
    )?;
    let rows = stmt.query_map(params![crew_id], |r| r.get(0))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Takes a session out of its crew, deleting the crew if that empties it.
fn leave_crew(conn: &Connection, session_id: &str) -> Result<Option<Crew>> {
    let from: Option<String> = conn
        .query_row(
            "SELECT crew_id FROM crew_members WHERE session_id = ?1",
            params![session_id],
            |r| r.get(0),
        )
        .optional()?;
    conn.execute(
        "DELETE FROM crew_members WHERE session_id = ?1",
        params![session_id],
    )?;
    match from {
        Some(from) => delete_if_empty(conn, &from),
        None => Ok(None),
    }
}

/// Deletes a crew with no members and returns what it was.
fn delete_if_empty(conn: &Connection, crew_id: &str) -> Result<Option<Crew>> {
    let left: i64 = conn.query_row(
        "SELECT COUNT(*) FROM crew_members WHERE crew_id = ?1",
        params![crew_id],
        |r| r.get(0),
    )?;
    if left > 0 {
        return Ok(None);
    }
    let crew = conn
        .query_row(
            "SELECT id, name, color, ord FROM crews WHERE id = ?1",
            params![crew_id],
            crew_from_row,
        )
        .optional()?;
    conn.execute("DELETE FROM crews WHERE id = ?1", params![crew_id])?;
    Ok(crew)
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
        title: r.get(19)?,
        activity: None,
        harness_session: r.get(20)?,
        resumed_from: r.get(21)?,
    })
}

/// Agent spawning's records (decision 134): lineage, the spawn switch, task,
/// result, approval requests, the inbox and cascades. Additive tables only;
/// nothing older is altered.
// Dark until 134-2…134-7: nothing outside the tests calls this yet.
#[allow(dead_code)]
pub mod spawn {
    use anyhow::{Result, bail};
    use rusqlite::{Connection, OptionalExtension, params};

    use super::{SessionRecord, Store, insert_session};

    pub const MAX_TASK_BYTES: usize = 16 * 1024;
    pub const MAX_MESSAGE_BYTES: usize = 8 * 1024;

    pub(super) fn init(conn: &Connection) -> Result<()> {
        conn.execute_batch(
            "-- 134d: who started a session. A top-level session has no row.
             -- Set once, at approval, and never changed (L-1); kept when the
             -- session is dismissed, as history.
             CREATE TABLE IF NOT EXISTS session_lineage (
                 session_id     TEXT PRIMARY KEY,
                 parent_session TEXT NOT NULL,
                 root_session   TEXT NOT NULL,
                 depth          INTEGER NOT NULL,
                 created_at     TEXT NOT NULL,
                 CHECK (depth >= 1),
                 CHECK (parent_session != session_id AND root_session != session_id),
                 CHECK ((depth = 1) = (root_session = parent_session))
             );
             CREATE INDEX IF NOT EXISTS lineage_by_parent ON session_lineage(parent_session);
             CREATE INDEX IF NOT EXISTS lineage_by_root ON session_lineage(root_session);
             CREATE TRIGGER IF NOT EXISTS lineage_no_update
                 BEFORE UPDATE ON session_lineage
                 BEGIN SELECT RAISE(ABORT, 'lineage is immutable'); END;
             CREATE TRIGGER IF NOT EXISTS lineage_no_delete
                 BEFORE DELETE ON session_lineage
                 BEGIN SELECT RAISE(ABORT, 'lineage is immutable'); END;
             -- INSERT OR REPLACE deletes without firing a delete trigger.
             CREATE TRIGGER IF NOT EXISTS lineage_no_replace
                 BEFORE INSERT ON session_lineage
                 WHEN EXISTS (SELECT 1 FROM session_lineage WHERE session_id = NEW.session_id)
                 BEGIN SELECT RAISE(ABORT, 'lineage is immutable'); END;

             -- 134c: the spawn switch. No row is off; set only by a person.
             CREATE TABLE IF NOT EXISTS session_spawn (
                 session_id        TEXT PRIMARY KEY,
                 enabled           INTEGER NOT NULL DEFAULT 0 CHECK (enabled IN (0, 1)),
                 changed_by_device TEXT,
                 changed_at        TEXT NOT NULL
             );

             -- 134f: a child's task, the exact text a person approved.
             CREATE TABLE IF NOT EXISTS session_task (
                 session_id TEXT PRIMARY KEY,
                 text       TEXT NOT NULL CHECK (length(CAST(text AS BLOB)) <= 16384),
                 created_at TEXT NOT NULL
             );
             CREATE TRIGGER IF NOT EXISTS task_no_update
                 BEFORE UPDATE ON session_task
                 BEGIN SELECT RAISE(ABORT, 'a task is fixed at approval'); END;

             -- 134h: what a session reported with report_result, on its own record.
             CREATE TABLE IF NOT EXISTS session_result (
                 session_id  TEXT PRIMARY KEY,
                 outcome     TEXT NOT NULL CHECK (outcome != ''),
                 summary     TEXT NOT NULL,
                 details     TEXT,
                 reported_at TEXT NOT NULL
             );

             -- 134o: start, message and end wait here for a person's decision.
             CREATE TABLE IF NOT EXISTS agent_requests (
                 id                TEXT PRIMARY KEY,
                 kind              TEXT NOT NULL CHECK (kind IN ('start', 'message', 'end')),
                 requester         TEXT NOT NULL,
                 target            TEXT,
                 payload           TEXT NOT NULL,
                 created_at        TEXT NOT NULL,
                 expires_at        TEXT NOT NULL,
                 state             TEXT NOT NULL
                     CHECK (state IN ('pending', 'approving', 'decided')),
                 decided_by_device TEXT,
                 decided_at        TEXT,
                 outcome           TEXT CHECK (outcome IN ('started', 'start_failed',
                     'delivered', 'ended', 'denied', 'expired', 'refused')),
                 reason            TEXT CHECK (reason IN ('limit', 'not_permitted', 'revoked',
                     'requester_ended', 'target_gone', 'identity_unverified',
                     'isolation_unverified')),
                 session           TEXT,
                 CHECK ((kind = 'start') = (target IS NULL)),
                 CHECK ((state = 'decided') = (outcome IS NOT NULL)),
                 CHECK ((COALESCE(outcome, '') = 'refused') = (reason IS NOT NULL)),
                 CHECK (session IS NULL OR outcome IN ('started', 'start_failed')),
                 CHECK (COALESCE(outcome, '') != 'started' OR session IS NOT NULL)
             );
             CREATE INDEX IF NOT EXISTS requests_by_requester
                 ON agent_requests(requester, state);
             CREATE INDEX IF NOT EXISTS requests_by_target ON agent_requests(target, state);
             -- What a person approves is what acts (T8, T9), and once.
             CREATE TRIGGER IF NOT EXISTS request_text_fixed
                 BEFORE UPDATE OF id, kind, requester, target, payload, created_at, expires_at
                 ON agent_requests
                 BEGIN SELECT RAISE(ABORT, 'a request is fixed when it is made'); END;
             CREATE TRIGGER IF NOT EXISTS request_decided_once
                 BEFORE UPDATE ON agent_requests WHEN OLD.state = 'decided'
                 BEGIN SELECT RAISE(ABORT, 'a request is decided once'); END;

             -- 134j: a child's inbox. The sender is the parent or `storm` (C9).
             CREATE TABLE IF NOT EXISTS session_inbox (
                 session_id TEXT NOT NULL,
                 seq        INTEGER NOT NULL,
                 sender     TEXT NOT NULL,
                 text       TEXT NOT NULL CHECK (length(CAST(text AS BLOB)) <= 8192),
                 request_id TEXT,
                 state      TEXT NOT NULL CHECK (state IN ('pending_approval', 'queued',
                     'delivered', 'acknowledged', 'denied', 'expired', 'undeliverable')),
                 at         TEXT NOT NULL,
                 PRIMARY KEY (session_id, seq),
                 CHECK ((sender = 'storm') = (request_id IS NULL))
             );

             -- L-3: a parent's end, persisted with the descendants it stops.
             CREATE TABLE IF NOT EXISTS spawn_cascades (
                 id                TEXT PRIMARY KEY,
                 cause             TEXT NOT NULL
                     CHECK (cause IN ('human_end', 'parent_end', 'end_all_children')),
                 parent_session    TEXT NOT NULL,
                 parent_end_reason TEXT,
                 report_first      INTEGER NOT NULL CHECK (report_first IN (0, 1)),
                 created_at        TEXT NOT NULL,
                 completed_at      TEXT,
                 CHECK ((cause = 'end_all_children') = (parent_end_reason IS NULL))
             );
             CREATE TRIGGER IF NOT EXISTS cascade_fixed
                 BEFORE UPDATE OF id, cause, parent_session, parent_end_reason, report_first,
                     created_at ON spawn_cascades
                 BEGIN SELECT RAISE(ABORT, 'a cascade is fixed when it is made'); END;
             CREATE TABLE IF NOT EXISTS spawn_cascade_targets (
                 cascade_id TEXT NOT NULL,
                 session_id TEXT NOT NULL,
                 state      TEXT NOT NULL
                     CHECK (state IN ('pending', 'ended', 'already_ended', 'unreachable')),
                 updated_at TEXT NOT NULL,
                 PRIMARY KEY (cascade_id, session_id)
             );
             CREATE INDEX IF NOT EXISTS cascade_targets_by_session
                 ON spawn_cascade_targets(session_id);
             CREATE TRIGGER IF NOT EXISTS cascade_target_final
                 BEFORE UPDATE ON spawn_cascade_targets
                 WHEN OLD.state IN ('ended', 'already_ended')
                 BEGIN SELECT RAISE(ABORT, 'a cascade target is ended once'); END;",
        )?;
        Ok(())
    }

    /// Who started a session (134d). A top-level session is its own root at
    /// depth 0.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct Lineage {
        pub parent_session: Option<String>,
        pub root_session: String,
        pub depth: i64,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct SpawnResult {
        pub outcome: String,
        pub summary: String,
        pub details: Option<String>,
        pub reported_at: String,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct Request {
        pub id: String,
        pub kind: String,
        pub requester: String,
        pub target: Option<String>,
        pub payload: String,
        pub created_at: String,
        pub expires_at: String,
        pub state: String,
        pub decided_by_device: Option<String>,
        pub decided_at: Option<String>,
        pub outcome: Option<String>,
        pub reason: Option<String>,
        pub session: Option<String>,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct Message {
        pub seq: i64,
        pub sender: String,
        pub text: String,
        pub request_id: Option<String>,
        pub state: String,
        pub at: String,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct Cascade {
        pub id: String,
        pub cause: String,
        pub parent_session: String,
        pub parent_end_reason: Option<String>,
        pub report_first: bool,
        pub created_at: String,
        pub completed_at: Option<String>,
        /// Each target and its state, in the order they were snapshotted.
        pub targets: Vec<(String, String)>,
    }

    /// How a cascaded child ended (L-4, L-5): its end reason, its parent's,
    /// and the cascade that ended it.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct EndCause {
        pub end_reason: String,
        pub parent_end_reason: Option<String>,
        pub cascade_id: String,
    }

    const ENDED: &str = "('completed', 'failed', 'stopped')";

    /// What asks for a workspace under WS-9.
    #[derive(Debug, Clone, Copy)]
    pub enum Claim<'a> {
        /// A new session launching into it.
        Launch,
        /// This session turning its spawn switch on.
        Spawn(&'a str),
    }

    /// The same directory, or one inside the other. The host reports
    /// workspaces as it found them, so only separators are normalized here.
    pub fn same_workspace(a: &str, b: &str) -> bool {
        let sep = |c: char| c == '/' || c == '\\';
        let (a, b) = (a.trim_end_matches(sep), b.trim_end_matches(sep));
        let inside = |inner: &str, outer: &str| {
            inner
                .strip_prefix(outer)
                .is_some_and(|rest| rest.starts_with(sep))
        };
        a == b || inside(a, b) || inside(b, a)
    }

    impl Store {
        pub fn lineage(&self, session_id: &str) -> Result<Lineage> {
            lineage_of(&self.conn, session_id)
        }

        /// Records a child at its approval, in one transaction: the session,
        /// its lineage under `parent` and its task. The only writer of
        /// lineage; nothing ever changes it after (L-1).
        pub fn insert_spawned(
            &mut self,
            child: &SessionRecord,
            parent: &str,
            task: &str,
            now: &str,
        ) -> Result<Lineage> {
            if task.len() > MAX_TASK_BYTES {
                bail!("the task is over {MAX_TASK_BYTES} bytes");
            }
            let tx = self.conn.transaction()?;
            let exists = tx
                .prepare("SELECT 1 FROM sessions WHERE id = ?1")?
                .exists([parent])?;
            if !exists {
                bail!("no parent session {parent}");
            }
            let up = lineage_of(&tx, parent)?;
            let lineage = Lineage {
                parent_session: Some(parent.to_string()),
                root_session: up.root_session,
                depth: up.depth + 1,
            };
            insert_session(&tx, child)?;
            tx.execute(
                "INSERT INTO session_lineage
                     (session_id, parent_session, root_session, depth, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![child.id, parent, lineage.root_session, lineage.depth, now],
            )?;
            tx.execute(
                "INSERT INTO session_task (session_id, text, created_at) VALUES (?1, ?2, ?3)",
                params![child.id, task, now],
            )?;
            tx.commit()?;
            Ok(lineage)
        }

        /// A session's descendants that still have a record, nearest first;
        /// with `live_only`, those that have not ended (`unknown` is live, L-6).
        pub fn descendants(&self, session_id: &str, live_only: bool) -> Result<Vec<String>> {
            descendants_of(&self.conn, session_id, live_only)
        }

        pub fn spawn_enabled(&self, session_id: &str) -> Result<bool> {
            Ok(self
                .conn
                .query_row(
                    "SELECT enabled FROM session_spawn WHERE session_id = ?1",
                    params![session_id],
                    |r| r.get(0),
                )
                .optional()?
                .unwrap_or(false))
        }

        /// Turns a session's spawn switch on or off. Turning it off refuses
        /// that session's pending requests, `revoked`, in the same
        /// transaction (B2, P-4); how many it refused.
        pub fn set_spawn(
            &mut self,
            session_id: &str,
            enabled: bool,
            device: Option<&str>,
            now: &str,
        ) -> Result<usize> {
            let tx = self.conn.transaction()?;
            tx.execute(
                "INSERT INTO session_spawn (session_id, enabled, changed_by_device, changed_at)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT (session_id) DO UPDATE SET enabled = excluded.enabled,
                     changed_by_device = excluded.changed_by_device,
                     changed_at = excluded.changed_at",
                params![session_id, enabled, device, now],
            )?;
            let refused = if enabled {
                0
            } else {
                refuse_where(&tx, "requester = ?1", session_id, "revoked", now)?
            };
            tx.commit()?;
            Ok(refused)
        }

        /// WS-9 (decision 137): whether `workspace` on `host_id` is held
        /// against `claim`. A launch is refused beside a live session whose
        /// spawn is on; spawn is refused beside any other live session.
        /// `unknown` is live. Nested workspaces share files, so they count as
        /// one.
        pub fn workspace_held(
            &self,
            host_id: &str,
            workspace: &str,
            claim: Claim<'_>,
        ) -> Result<bool> {
            for r in self.for_host(host_id)? {
                if r.is_ended() || !same_workspace(&r.workspace, workspace) {
                    continue;
                }
                let held = match claim {
                    Claim::Launch => self.spawn_enabled(&r.id)?,
                    Claim::Spawn(session) => r.id != session,
                };
                if held {
                    return Ok(true);
                }
            }
            Ok(false)
        }

        pub fn task(&self, session_id: &str) -> Result<Option<String>> {
            Ok(self
                .conn
                .query_row(
                    "SELECT text FROM session_task WHERE session_id = ?1",
                    params![session_id],
                    |r| r.get(0),
                )
                .optional()?)
        }

        /// The session's own result; a later report replaces an earlier one.
        pub fn set_result(&self, session_id: &str, r: &SpawnResult) -> Result<()> {
            self.conn.execute(
                "INSERT INTO session_result (session_id, outcome, summary, details, reported_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT (session_id) DO UPDATE SET outcome = excluded.outcome,
                     summary = excluded.summary, details = excluded.details,
                     reported_at = excluded.reported_at",
                params![session_id, r.outcome, r.summary, r.details, r.reported_at],
            )?;
            Ok(())
        }

        pub fn result(&self, session_id: &str) -> Result<Option<SpawnResult>> {
            Ok(self
                .conn
                .query_row(
                    "SELECT outcome, summary, details, reported_at FROM session_result
                     WHERE session_id = ?1",
                    params![session_id],
                    |r| {
                        Ok(SpawnResult {
                            outcome: r.get(0)?,
                            summary: r.get(1)?,
                            details: r.get(2)?,
                            reported_at: r.get(3)?,
                        })
                    },
                )
                .optional()?)
        }

        /// Records a new request, `pending`.
        pub fn insert_request(&self, r: &Request) -> Result<()> {
            self.conn.execute(
                "INSERT INTO agent_requests (id, kind, requester, target, payload, created_at,
                     expires_at, state)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'pending')",
                params![
                    r.id,
                    r.kind,
                    r.requester,
                    r.target,
                    r.payload,
                    r.created_at,
                    r.expires_at
                ],
            )?;
            Ok(())
        }

        pub fn request(&self, id: &str) -> Result<Option<Request>> {
            Ok(self
                .conn
                .query_row(
                    "SELECT id, kind, requester, target, payload, created_at, expires_at, state,
                         decided_by_device, decided_at, outcome, reason, session
                     FROM agent_requests WHERE id = ?1",
                    params![id],
                    |r| {
                        Ok(Request {
                            id: r.get(0)?,
                            kind: r.get(1)?,
                            requester: r.get(2)?,
                            target: r.get(3)?,
                            payload: r.get(4)?,
                            created_at: r.get(5)?,
                            expires_at: r.get(6)?,
                            state: r.get(7)?,
                            decided_by_device: r.get(8)?,
                            decided_at: r.get(9)?,
                            outcome: r.get(10)?,
                            reason: r.get(11)?,
                            session: r.get(12)?,
                        })
                    },
                )
                .optional()?)
        }

        /// Refuses a requester's pending requests with `reason` (L-7, P-4).
        pub fn refuse_requests_of(
            &self,
            requester: &str,
            reason: &str,
            now: &str,
        ) -> Result<usize> {
            refuse_where(&self.conn, "requester = ?1", requester, reason, now)
        }

        /// Puts a message in a session's inbox at the next sequence number.
        pub fn queue_message(
            &mut self,
            session_id: &str,
            sender: &str,
            text: &str,
            request_id: Option<&str>,
            state: &str,
            now: &str,
        ) -> Result<i64> {
            if text.len() > MAX_MESSAGE_BYTES {
                bail!("the message is over {MAX_MESSAGE_BYTES} bytes");
            }
            let tx = self.conn.transaction()?;
            let seq: i64 = tx.query_row(
                "SELECT COALESCE(MAX(seq), 0) + 1 FROM session_inbox WHERE session_id = ?1",
                params![session_id],
                |r| r.get(0),
            )?;
            tx.execute(
                "INSERT INTO session_inbox (session_id, seq, sender, text, request_id, state, at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![session_id, seq, sender, text, request_id, state, now],
            )?;
            tx.commit()?;
            Ok(seq)
        }

        pub fn inbox(&self, session_id: &str) -> Result<Vec<Message>> {
            let mut stmt = self.conn.prepare(
                "SELECT seq, sender, text, request_id, state, at FROM session_inbox
                 WHERE session_id = ?1 ORDER BY seq",
            )?;
            let rows = stmt.query_map(params![session_id], |r| {
                Ok(Message {
                    seq: r.get(0)?,
                    sender: r.get(1)?,
                    text: r.get(2)?,
                    request_id: r.get(3)?,
                    state: r.get(4)?,
                    at: r.get(5)?,
                })
            })?;
            Ok(rows.collect::<rusqlite::Result<_>>()?)
        }

        /// Opens a cascade (L-3): in one transaction, the cause, the parent and
        /// its own end reason, and every live descendant as a `pending`
        /// target. When the parent is ending, its pending requests are refused
        /// `requester_ended` in the same transaction (L-7). A cascade with no
        /// live descendant is complete at once. Returns the targets.
        pub fn open_cascade(
            &mut self,
            id: &str,
            cause: &str,
            parent: &str,
            parent_end_reason: Option<&str>,
            report_first: bool,
            now: &str,
        ) -> Result<Vec<String>> {
            let tx = self.conn.transaction()?;
            let targets = descendants_of(&tx, parent, true)?;
            tx.execute(
                "INSERT INTO spawn_cascades (id, cause, parent_session, parent_end_reason,
                     report_first, created_at, completed_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    id,
                    cause,
                    parent,
                    parent_end_reason,
                    report_first,
                    now,
                    targets.is_empty().then_some(now)
                ],
            )?;
            for t in &targets {
                tx.execute(
                    "INSERT INTO spawn_cascade_targets (cascade_id, session_id, state, updated_at)
                     VALUES (?1, ?2, 'pending', ?3)",
                    params![id, t, now],
                )?;
            }
            if cause != "end_all_children" {
                refuse_where(&tx, "requester = ?1", parent, "requester_ended", now)?;
            }
            tx.commit()?;
            Ok(targets)
        }

        /// Moves a cascade target on. `ended` and `already_ended` are final:
        /// a second end is a no-op (L-3), answered `false`. The cascade is
        /// complete once no target is `pending` or `unreachable`.
        pub fn set_cascade_target(
            &mut self,
            cascade_id: &str,
            session_id: &str,
            state: &str,
            now: &str,
        ) -> Result<bool> {
            let tx = self.conn.transaction()?;
            let changed = tx.execute(
                "UPDATE spawn_cascade_targets SET state = ?3, updated_at = ?4
                 WHERE cascade_id = ?1 AND session_id = ?2
                   AND state IN ('pending', 'unreachable') AND state != ?3",
                params![cascade_id, session_id, state, now],
            )? > 0;
            tx.execute(
                "UPDATE spawn_cascades SET completed_at = ?2
                 WHERE id = ?1 AND completed_at IS NULL AND NOT EXISTS (
                     SELECT 1 FROM spawn_cascade_targets
                     WHERE cascade_id = ?1 AND state IN ('pending', 'unreachable'))",
                params![cascade_id, now],
            )?;
            tx.commit()?;
            Ok(changed)
        }

        pub fn cascade(&self, id: &str) -> Result<Option<Cascade>> {
            let Some(mut c) = self
                .conn
                .query_row(
                    "SELECT id, cause, parent_session, parent_end_reason, report_first,
                         created_at, completed_at
                     FROM spawn_cascades WHERE id = ?1",
                    params![id],
                    |r| {
                        Ok(Cascade {
                            id: r.get(0)?,
                            cause: r.get(1)?,
                            parent_session: r.get(2)?,
                            parent_end_reason: r.get(3)?,
                            report_first: r.get(4)?,
                            created_at: r.get(5)?,
                            completed_at: r.get(6)?,
                            targets: Vec::new(),
                        })
                    },
                )
                .optional()?
            else {
                return Ok(None);
            };
            let mut stmt = self.conn.prepare(
                "SELECT session_id, state FROM spawn_cascade_targets
                 WHERE cascade_id = ?1 ORDER BY rowid",
            )?;
            c.targets = stmt
                .query_map(params![id], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<rusqlite::Result<_>>()?;
            Ok(Some(c))
        }

        /// Cascades not yet complete, oldest first: what resumes after a
        /// server restart (L-3).
        pub fn open_cascades(&self) -> Result<Vec<String>> {
            let mut stmt = self.conn.prepare(
                "SELECT id FROM spawn_cascades WHERE completed_at IS NULL
                 ORDER BY created_at, rowid",
            )?;
            let rows = stmt.query_map([], |r| r.get(0))?;
            Ok(rows.collect::<rusqlite::Result<_>>()?)
        }

        /// The end reason a cascaded session takes: the first cascade that
        /// named it wins (L-4), with its parent's reason copied (L-5).
        pub fn end_cause(&self, session_id: &str) -> Result<Option<EndCause>> {
            Ok(self
                .conn
                .query_row(
                    "SELECT c.cause, c.parent_end_reason, c.id
                     FROM spawn_cascade_targets t JOIN spawn_cascades c ON c.id = t.cascade_id
                     WHERE t.session_id = ?1 ORDER BY c.created_at, c.rowid LIMIT 1",
                    params![session_id],
                    |r| {
                        let cause: String = r.get(0)?;
                        Ok(EndCause {
                            end_reason: if cause == "parent_end" {
                                "parent_ended"
                            } else {
                                "cascade"
                            }
                            .to_string(),
                            parent_end_reason: r.get(1)?,
                            cascade_id: r.get(2)?,
                        })
                    },
                )
                .optional()?)
        }
    }

    fn lineage_of(conn: &Connection, session_id: &str) -> Result<Lineage> {
        Ok(conn
            .query_row(
                "SELECT parent_session, root_session, depth FROM session_lineage
                 WHERE session_id = ?1",
                params![session_id],
                |r| {
                    Ok(Lineage {
                        parent_session: Some(r.get(0)?),
                        root_session: r.get(1)?,
                        depth: r.get(2)?,
                    })
                },
            )
            .optional()?
            .unwrap_or_else(|| Lineage {
                parent_session: None,
                root_session: session_id.to_string(),
                depth: 0,
            }))
    }

    fn descendants_of(conn: &Connection, session_id: &str, live_only: bool) -> Result<Vec<String>> {
        let live = if live_only {
            format!("AND s.status NOT IN {ENDED}")
        } else {
            String::new()
        };
        let mut stmt = conn.prepare(&format!(
            "WITH RECURSIVE tree(id) AS (
                 SELECT session_id FROM session_lineage WHERE parent_session = ?1
                 UNION
                 SELECT l.session_id FROM session_lineage l JOIN tree ON l.parent_session = tree.id
             )
             SELECT tree.id FROM tree
             JOIN sessions s ON s.id = tree.id
             JOIN session_lineage l ON l.session_id = tree.id
             WHERE 1 {live}
             ORDER BY l.depth, s.created_at, tree.id"
        ))?;
        let rows = stmt.query_map(params![session_id], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    fn refuse_where(
        conn: &Connection,
        clause: &str,
        session_id: &str,
        reason: &str,
        now: &str,
    ) -> Result<usize> {
        Ok(conn.execute(
            &format!(
                "UPDATE agent_requests SET state = 'decided', outcome = 'refused', reason = ?2,
                     decided_at = ?3
                 WHERE state IN ('pending', 'approving') AND {clause}"
            ),
            params![session_id, reason, now],
        )?)
    }

    /// What a dismissal takes with a session (C8): its switch, task, result
    /// and inbox. Requests still pending from or to it are refused; decided
    /// ones, its lineage and cascades stay as history.
    pub(super) fn forget(conn: &Connection, session_id: &str) -> Result<()> {
        let now = crate::index::now_rfc3339();
        refuse_where(conn, "requester = ?1", session_id, "requester_ended", &now)?;
        refuse_where(conn, "target = ?1", session_id, "target_gone", &now)?;
        for table in [
            "session_spawn",
            "session_task",
            "session_result",
            "session_inbox",
        ] {
            conn.execute(
                &format!("DELETE FROM {table} WHERE session_id = ?1"),
                params![session_id],
            )?;
        }
        Ok(())
    }
}
