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
    /// still resolves.
    pub fn delete(&mut self, id: &str) -> Result<(bool, Option<Crew>)> {
        let tx = self.conn.transaction()?;
        let gone = tx.execute("DELETE FROM sessions WHERE id = ?1", params![id])? > 0;
        let crew = leave_crew(&tx, id)?;
        tx.execute(
            "DELETE FROM session_role WHERE session_id = ?1",
            params![id],
        )?;
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
