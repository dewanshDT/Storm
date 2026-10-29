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
    /// The role it was launched with (decision 138f), fixed for its life and
    /// handed to the agent through `session_context`. Only a session that
    /// reads Storm has one; for any other the role is a label only (138g).
    pub role: Option<Context>,
    /// The model it was started on (decision 145b), checked for its agent.
    pub model: Option<String>,
}

/// The note a session was started from, or its launch role. The title is a
/// snapshot.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
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
    /// Bumped after a commit that wrote a child event (144a): what a
    /// waiting `wait_agent` wakes on.
    pub(super) events: tokio::sync::watch::Sender<u64>,
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
             );
             -- Each host's session isolation as it last reported it
             -- (decision 136j), for its views while it is away. Never what a
             -- privileged operation is judged by: that is the live link.
             CREATE TABLE IF NOT EXISTS host_isolation (
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
        // Decision 89 on `sessions` and 138f on `session_launch`, additive
        // the same way.
        for (table, column) in [
            ("sessions", "harness_session"),
            ("sessions", "resumed_from"),
            ("session_launch", "role_vault_id"),
            ("session_launch", "role_note_id"),
            ("session_launch", "role_title"),
            ("session_launch", "model"),
        ] {
            let has = conn
                .prepare("SELECT 1 FROM pragma_table_info(?1) WHERE name = ?2")?
                .exists([table, column])?;
            if !has {
                conn.execute(&format!("ALTER TABLE {table} ADD COLUMN {column} TEXT"), [])?;
            }
        }
        spawn::init(&conn)?;
        super::events::init(&conn)?;
        conn.execute_batch(
            "-- One-time upgrades, each recorded with what it did and when the
             -- server audited it.
             CREATE TABLE IF NOT EXISTS agent_migrations (
                 name       TEXT PRIMARY KEY,
                 applied_at TEXT NOT NULL,
                 detail     TEXT NOT NULL,
                 audited_at TEXT
             );",
        )?;
        migrate_crew_trees(&conn)?;
        Ok(Self {
            conn,
            events: tokio::sync::watch::channel(0).0,
        })
    }

    /// At boot, every session that has not ended is `unknown` until its host
    /// reports it (freeze §13). A server that just started knows nothing about
    /// what its hosts did while it was down, and saying `running` would be a
    /// guess.
    ///
    /// A spawned child still `creating` is left as it is: its host was never
    /// told of it, and startup fails it `server_restart` (139-D5). A parked
    /// child (145e) has no process to be unsure of, so it stays parked.
    ///
    /// A child that turns `unknown` here gets an `unknown` event from Storm
    /// (144a), in the same transaction.
    pub fn mark_unknown_on_boot(&self) -> Result<usize> {
        let tx = self.conn.unchecked_transaction()?;
        let children: Vec<String> = {
            let mut stmt = tx.prepare(
                "SELECT id FROM sessions
                 WHERE status NOT IN ('completed', 'failed', 'stopped', 'creating', 'unknown',
                                      'parked')
                   AND id IN (SELECT session_id FROM session_lineage)",
            )?;
            let rows = stmt.query_map([], |r| r.get(0))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        let changed = tx.execute(
            "UPDATE sessions SET status = 'unknown'
             WHERE status NOT IN ('completed', 'failed', 'stopped', 'parked')
               AND NOT (status = 'creating'
                        AND id IN (SELECT session_id FROM session_lineage))",
            [],
        )?;
        let now = crate::index::now_rfc3339();
        for id in children {
            if let Some(r) = get_in(&tx, &id)? {
                super::events::record_in(
                    &tx,
                    &r,
                    super::events::UNKNOWN,
                    None,
                    super::events::STORM,
                    &super::events::LiveSignals::default(),
                    &now,
                )?;
            }
        }
        tx.commit()?;
        Ok(changed)
    }

    pub fn insert(&self, r: &SessionRecord) -> Result<()> {
        insert_session(&self.conn, r)
    }

    pub fn get(&self, id: &str) -> Result<Option<SessionRecord>> {
        get_in(&self.conn, id)
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
    #[cfg(test)]
    pub fn update(&self, r: &SessionRecord) -> Result<()> {
        self.update_with(r, &super::events::EventCtx::storm())
    }

    /// [`Self::update`], with the events the change makes for a child's
    /// parent written in the same transaction (144a): entering `running`
    /// or `unknown`, an end, and `ctx.extra`.
    ///
    /// An ending refuses the session's pending requests, `requester_ended`,
    /// in the same transaction (P-4, L-7).
    pub fn update_with(&self, r: &SessionRecord, ctx: &super::events::EventCtx) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        let before: Option<String> = tx
            .query_row(
                "SELECT status FROM sessions WHERE id = ?1",
                params![r.id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        let was_ended = before
            .as_deref()
            .is_some_and(|s| matches!(s, "completed" | "failed" | "stopped"));
        tx.execute(
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
        if r.is_ended() && !was_ended {
            spawn::on_ended(&tx, r, &crate::index::now_rfc3339())?;
        }
        let now = crate::index::now_rfc3339();
        let mut wrote = false;
        if before.is_some()
            && let Some((kind, reason)) = super::events::status_event(before.as_deref(), r)
        {
            wrote |= super::events::record_in(
                &tx,
                r,
                kind,
                reason.as_deref(),
                ctx.source,
                &ctx.live,
                &now,
            )?;
        }
        for e in &ctx.extra {
            wrote |= super::events::record_in(
                &tx,
                r,
                e.kind,
                e.reason.as_deref(),
                e.source,
                &ctx.live,
                &now,
            )?;
        }
        tx.commit()?;
        if wrote {
            self.notify_events();
        }
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
        insert_grants(&tx, session_id, allow_vault_writes, grants, now)?;
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

    /// A session's live grants: what a spawned child's start names.
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
        insert_integrations(&tx, session_id, integrations)?;
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
        insert_launch(&self.conn, l, now)
    }

    pub fn launch(&self, session_id: &str) -> Result<Option<LaunchRecord>> {
        Ok(self
            .conn
            .query_row(
                "SELECT session_id, name, context_vault_id, context_note_id, context_title,
                     write_vault_id, role_vault_id, role_note_id, role_title, model
                 FROM session_launch WHERE session_id = ?1",
                params![session_id],
                |r| {
                    let note = |at: usize| -> rusqlite::Result<Option<Context>> {
                        let vault: Option<String> = r.get(at)?;
                        let note: Option<String> = r.get(at + 1)?;
                        Ok(vault.zip(note).map(|(vault_id, note_id)| Context {
                            vault_id,
                            note_id,
                            title: r
                                .get::<_, Option<String>>(at + 2)
                                .ok()
                                .flatten()
                                .unwrap_or_default(),
                        }))
                    };
                    Ok(LaunchRecord {
                        session_id: r.get(0)?,
                        name: r.get(1)?,
                        context: note(2)?,
                        write_vault_id: r.get(5)?,
                        role: note(6)?,
                        model: r.get(9)?,
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
        let removed = delete_in(&tx, id)?;
        tx.commit()?;
        Ok(removed)
    }

    /// Dismisses a whole tree in one transaction (decision 134, C8): every
    /// member as `delete` removes one. Returns the crews it emptied, in the
    /// order they emptied.
    pub fn delete_all(&mut self, ids: &[String]) -> Result<Vec<Crew>> {
        let tx = self.conn.transaction()?;
        let mut emptied = Vec::new();
        for id in ids {
            emptied.extend(delete_in(&tx, id)?.1);
        }
        tx.commit()?;
        Ok(emptied)
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

    /// The crew a session is in: its tree root's (decision 145i). A spawned
    /// child has no row of its own.
    pub fn membership(&self, session_id: &str) -> Result<Option<Membership>> {
        let root = crew_root(&self.conn, session_id)?;
        Ok(self
            .conn
            .query_row(
                "SELECT crew_id, ord FROM crew_members WHERE session_id = ?1",
                params![root],
                |r| {
                    Ok(Membership {
                        id: r.get(0)?,
                        order: r.get(1)?,
                    })
                },
            )
            .optional()?)
    }

    /// Upgrades that ran but the server has not audited yet.
    pub fn unaudited_migrations(&self) -> Result<Vec<Migration>> {
        let mut stmt = self.conn.prepare(
            "SELECT name, applied_at, detail FROM agent_migrations
             WHERE audited_at IS NULL ORDER BY applied_at, name",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(Migration {
                name: r.get(0)?,
                applied_at: r.get(1)?,
                detail: r.get(2)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn mark_migration_audited(&self, name: &str, now: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE agent_migrations SET audited_at = ?2 WHERE name = ?1",
            params![name, now],
        )?;
        Ok(())
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
        let mut roots: Vec<String> = Vec::new();
        for s in sessions {
            let root = crew_root(&tx, s)?;
            if !roots.contains(&root) {
                roots.push(root);
            }
        }
        for (i, s) in roots.iter().enumerate() {
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

    /// Puts a session's tree in `crew_id` before `before`'s (or last), or
    /// takes it out of any crew when `crew_id` is `None`: a child moves its
    /// root (145i). A crew the move empties is deleted and returned (138c).
    /// The caller checks that the crew and `before` exist.
    pub fn move_session(
        &mut self,
        session_id: &str,
        crew_id: Option<&str>,
        before: Option<&str>,
    ) -> Result<Option<Crew>> {
        let tx = self.conn.transaction()?;
        let session_id = crew_root(&tx, session_id)?;
        let session_id = session_id.as_str();
        let before = before.map(|b| crew_root(&tx, b)).transpose()?;
        let before = before.as_deref();
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

    pub fn host_label(
        &self,
        kind: super::HostLabel,
        host_id: &str,
    ) -> Result<Option<super::Identity>> {
        let table = label_table(kind);
        Ok(self
            .conn
            .query_row(
                &format!("SELECT state, mechanism, reason FROM {table} WHERE host_id = ?1"),
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

    pub fn set_host_label(
        &self,
        kind: super::HostLabel,
        host_id: &str,
        label: &super::Identity,
        now: &str,
    ) -> Result<()> {
        let table = label_table(kind);
        self.conn.execute(
            &format!(
                "INSERT INTO {table} (host_id, state, mechanism, reason, reported_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT (host_id) DO UPDATE SET state = excluded.state,
                     mechanism = excluded.mechanism, reason = excluded.reason,
                     reported_at = excluded.reported_at"
            ),
            params![host_id, label.state, label.mechanism, label.reason, now],
        )?;
        Ok(())
    }
}

fn label_table(kind: super::HostLabel) -> &'static str {
    match kind {
        super::HostLabel::Identity => "host_identity",
        super::HostLabel::Isolation => "host_isolation",
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

fn insert_grants(
    conn: &Connection,
    session_id: &str,
    allow_vault_writes: bool,
    grants: &[McpGrant],
    now: &str,
) -> Result<()> {
    conn.execute(
        "INSERT INTO session_mcp (session_id, allow_vault_writes, created_at)
         VALUES (?1, ?2, ?3)",
        params![session_id, allow_vault_writes, now],
    )?;
    for g in grants {
        conn.execute(
            "INSERT INTO session_mcp_grants (session_id, connection_id, slug, granted_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![session_id, g.id, g.slug, now],
        )?;
    }
    Ok(())
}

fn insert_integrations(
    conn: &Connection,
    session_id: &str,
    integrations: &[SessionIntegration],
) -> Result<()> {
    for i in integrations {
        conn.execute(
            "INSERT OR REPLACE INTO session_integrations
                 (session_id, connection_id, slug, display_name)
             VALUES (?1, ?2, ?3, ?4)",
            params![session_id, i.id, i.slug, i.display_name],
        )?;
    }
    Ok(())
}

fn insert_launch(conn: &Connection, l: &LaunchRecord, now: &str) -> Result<()> {
    let parts = |c: &Option<Context>| match c {
        Some(c) => (
            Some(c.vault_id.clone()),
            Some(c.note_id.clone()),
            Some(c.title.clone()),
        ),
        None => (None, None, None),
    };
    let (vault, note, title) = parts(&l.context);
    let (role_vault, role_note, role_title) = parts(&l.role);
    conn.execute(
        "INSERT INTO session_launch (session_id, name, context_vault_id, context_note_id,
             context_title, write_vault_id, created_at, role_vault_id, role_note_id,
             role_title, model)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            l.session_id,
            l.name,
            vault,
            note,
            title,
            l.write_vault_id,
            now,
            role_vault,
            role_note,
            role_title,
            l.model
        ],
    )?;
    Ok(())
}

/// The session whose crew row stands for `session_id`'s: its tree's root
/// (decision 145i), or itself.
pub(super) fn crew_root(conn: &Connection, session_id: &str) -> Result<String> {
    Ok(conn
        .query_row(
            "SELECT root_session FROM session_lineage WHERE session_id = ?1",
            params![session_id],
            |r| r.get(0),
        )
        .optional()?
        .unwrap_or_else(|| session_id.to_string()))
}

/// One child's row the 145i upgrade took out of a crew.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MovedRow {
    pub session_id: String,
    pub crew_id: String,
    pub root: String,
    /// The root joined that crew, having been in none.
    pub root_joined: bool,
}

/// A one-time upgrade of `agent.db`, recorded once with what it did, and
/// audited once by the server (decision 145i).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Migration {
    pub name: String,
    pub applied_at: String,
    pub detail: String,
}

pub const CREW_TREES_MIGRATION: &str = "145i_crew_trees";

/// 145i, once: every child's crew row goes, and its root joins that crew if
/// the root is in none, in one transaction with the record that it ran.
fn migrate_crew_trees(conn: &Connection) -> Result<()> {
    let done = conn
        .prepare("SELECT 1 FROM agent_migrations WHERE name = ?1")?
        .exists([CREW_TREES_MIGRATION])?;
    if done {
        return Ok(());
    }
    let tx = conn.unchecked_transaction()?;
    let rows: Vec<(String, String, String)> = {
        let mut stmt = tx.prepare(
            "SELECT m.session_id, m.crew_id, l.root_session FROM crew_members m
             JOIN session_lineage l ON l.session_id = m.session_id
             ORDER BY m.crew_id, m.ord, m.session_id",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let mut moved = Vec::new();
    for (session_id, crew_id, root) in rows {
        tx.execute(
            "DELETE FROM crew_members WHERE session_id = ?1",
            params![session_id],
        )?;
        let root_in_one = tx
            .prepare("SELECT 1 FROM crew_members WHERE session_id = ?1")?
            .exists([&root])?;
        let root_exists = tx
            .prepare("SELECT 1 FROM sessions WHERE id = ?1")?
            .exists([&root])?;
        let root_joined = !root_in_one && root_exists;
        if root_joined {
            tx.execute(
                "INSERT INTO crew_members (session_id, crew_id, ord)
                 VALUES (?1, ?2, (SELECT COALESCE(MAX(ord) + 1, 0) FROM crew_members
                                  WHERE crew_id = ?2))",
                params![root, crew_id],
            )?;
        }
        moved.push(MovedRow {
            session_id,
            crew_id,
            root,
            root_joined,
        });
    }
    let emptied: Vec<String> = moved
        .iter()
        .map(|m| m.crew_id.clone())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .filter_map(|c| delete_if_empty(&tx, &c).transpose())
        .map(|c| c.map(|c| c.id))
        .collect::<Result<_>>()?;
    let detail = serde_json::json!({"moved": moved, "emptied_crews": emptied});
    tx.execute(
        "INSERT INTO agent_migrations (name, applied_at, detail) VALUES (?1, ?2, ?3)",
        params![
            CREW_TREES_MIGRATION,
            crate::index::now_rfc3339(),
            detail.to_string()
        ],
    )?;
    tx.commit()?;
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

fn get_in(conn: &Connection, id: &str) -> Result<Option<SessionRecord>> {
    Ok(conn
        .query_row(
            &format!("SELECT {COLUMNS} FROM sessions WHERE id = ?1"),
            params![id],
            from_row,
        )
        .optional()?)
}

fn delete_in(conn: &Connection, id: &str) -> Result<(bool, Option<Crew>)> {
    let gone = conn.execute("DELETE FROM sessions WHERE id = ?1", params![id])? > 0;
    let crew = leave_crew(conn, id)?;
    conn.execute(
        "DELETE FROM session_role WHERE session_id = ?1",
        params![id],
    )?;
    spawn::forget(conn, id)?;
    super::events::forget_in(conn, id)?;
    Ok((gone, crew))
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
/// nothing older is altered, and of these only `agent_requests` was rebuilt
/// (143a).
pub mod spawn {
    use std::sync::LazyLock;

    use anyhow::{Result, bail};
    use rusqlite::{Connection, OptionalExtension, params};

    use serde::{Deserialize, Serialize};

    use super::{
        COLUMNS, LaunchRecord, McpGrant, SessionIntegration, SessionRecord, Store, from_row,
        get_in, insert_grants, insert_integrations, insert_launch, insert_session,
    };

    pub const MAX_TASK_BYTES: usize = 16 * 1024;
    pub const MAX_MESSAGE_BYTES: usize = 8 * 1024;

    /// What `report_result` may say a session's work came to (134h, fixed in
    /// 134-4). Checked here as well as by the tool, since this is the only
    /// writer; the schema's CHECK stays "non-empty" so no table is rebuilt.
    pub const RESULT_OUTCOMES: &[&str] = &["succeeded", "partial", "failed", "blocked"];
    pub const MAX_SUMMARY_BYTES: usize = 4 * 1024;
    pub const MAX_DETAILS_BYTES: usize = 16 * 1024;

    /// 134o's requests, since decision 145a the log of a parent's acts:
    /// each start, message and end is written here already decided, with no
    /// one asked. A row left `pending` by an older server is expired once,
    /// at open.
    const REQUESTS_TABLE: &str = "
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
            'isolation_unverified', 'agent_unavailable')),
        session           TEXT,
        CHECK ((kind = 'start') = (target IS NULL)),
        CHECK ((state = 'decided') = (outcome IS NOT NULL)),
        CHECK ((COALESCE(outcome, '') = 'refused') = (reason IS NOT NULL)),
        CHECK (session IS NULL OR outcome IN ('started', 'start_failed')),
        CHECK (COALESCE(outcome, '') != 'started' OR session IS NOT NULL)
    );";

    const REQUESTS_INDEXES: &str = "
    CREATE INDEX IF NOT EXISTS requests_by_requester
        ON agent_requests(requester, state);
    CREATE INDEX IF NOT EXISTS requests_by_target ON agent_requests(target, state);
    CREATE INDEX IF NOT EXISTS requests_pending
        ON agent_requests(created_at, id, expires_at) WHERE state = 'pending';
    -- What a person approves is what acts (T8, T9), and once.
    CREATE TRIGGER IF NOT EXISTS request_text_fixed
        BEFORE UPDATE OF id, kind, requester, target, payload, created_at, expires_at
        ON agent_requests
        BEGIN SELECT RAISE(ABORT, 'a request is fixed when it is made'); END;
    CREATE TRIGGER IF NOT EXISTS request_decided_once
        BEFORE UPDATE ON agent_requests WHEN OLD.state = 'decided'
        BEGIN SELECT RAISE(ABORT, 'a request is decided once'); END;";

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

             -- 134-8: the session's start carried spawn_asks, so its CLI's
             -- config holds the pinned asks. Only such a session's switch
             -- turns on.
             CREATE TABLE IF NOT EXISTS session_spawn_asks (
                 session_id TEXT PRIMARY KEY,
                 created_at TEXT NOT NULL
             );

             -- 141e: launched with the switch on, so only it turns on again.
             CREATE TABLE IF NOT EXISTS session_spawn_launch (
                 session_id TEXT PRIMARY KEY,
                 created_at TEXT NOT NULL
             );

             -- 134-5: a child's own workspace as its host made it (137-1):
             -- the mode, and for a clone the commit it starts from.
             CREATE TABLE IF NOT EXISTS session_child_workspace (
                 session_id  TEXT PRIMARY KEY,
                 mode        TEXT NOT NULL CHECK (mode IN ('clone', 'scratch')),
                 base_commit TEXT,
                 branch      TEXT,
                 submodules  INTEGER NOT NULL,
                 large_files INTEGER NOT NULL,
                 created_at  TEXT NOT NULL
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
             CREATE INDEX IF NOT EXISTS cascades_open
                 ON spawn_cascades(created_at) WHERE completed_at IS NULL;
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
        conn.execute_batch(REQUESTS_TABLE)?;
        let requests: String = conn.query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'agent_requests'",
            [],
            |r| r.get(0),
        )?;
        if !requests.contains("'agent_unavailable'") {
            // 143a's reason: a CHECK is not altered in place, so the table is
            // rebuilt with its rows. Its old indexes and triggers go with the
            // old table, and are made again below.
            conn.execute_batch(&format!(
                "BEGIN;
                 ALTER TABLE agent_requests RENAME TO agent_requests_142;
                 {REQUESTS_TABLE}
                 INSERT INTO agent_requests ({REQUEST_COLUMNS})
                     SELECT {REQUEST_COLUMNS} FROM agent_requests_142;
                 DROP TABLE agent_requests_142;
                 COMMIT;"
            ))?;
        }
        conn.execute_batch(REQUESTS_INDEXES)?;
        conn.execute_batch(
            "-- 145: a child's round. Absent is round 1; a message after the
             -- round's result starts the next.
             CREATE TABLE IF NOT EXISTS session_round (
                 session_id TEXT PRIMARY KEY,
                 round      INTEGER NOT NULL CHECK (round >= 1),
                 changed_at TEXT NOT NULL
             );
             -- 145: every round's result, as report_result left it.
             CREATE TABLE IF NOT EXISTS session_round_result (
                 session_id  TEXT NOT NULL,
                 round       INTEGER NOT NULL,
                 outcome     TEXT NOT NULL CHECK (outcome != ''),
                 summary     TEXT NOT NULL,
                 details     TEXT,
                 reported_at TEXT NOT NULL,
                 PRIMARY KEY (session_id, round)
             );",
        )?;
        let has_round = conn
            .prepare("SELECT 1 FROM pragma_table_info('session_result') WHERE name = 'round'")?
            .exists([])?;
        if !has_round {
            conn.execute(
                "ALTER TABLE session_result ADD COLUMN round INTEGER NOT NULL DEFAULT 1",
                [],
            )?;
        }
        // 145a: nothing waits for a person any more. What an older server
        // left waiting expires, and its messages with it.
        let now = crate::index::now_rfc3339();
        let expired = conn.execute(
            "UPDATE agent_requests SET state = 'decided', outcome = 'expired', decided_at = ?1
             WHERE state IN ('pending', 'approving')",
            params![now],
        )?;
        if expired > 0 {
            settle_messages(conn)?;
        }
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
        /// The round it answers (145): 1 for the task, then one per message
        /// that came after a result.
        pub round: i64,
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

    // ---- 145a: a parent's acts (134-3's checks, no approval) -----------------
    //
    // Every act runs in one transaction of the store, whose lock the manager
    // holds throughout: that lock is the serialization point for the caps,
    // for every tree at once.

    /// C4's hard caps. Counted here rather than held as constraints, so a
    /// revisit needs no migration.
    pub const MAX_DEPTH: i64 = 2;
    /// The depth a start may reach today. A child's own workspace is its
    /// user's alone (137-1), so the helper refuses a grandchild
    /// (`wrong_parent`) and 137 does not say how one would read it; until a
    /// decision does, a child starts none. Lower than C4's, never higher.
    pub const BUILT_DEPTH: i64 = 1;
    pub const MAX_CHILDREN: i64 = 3;
    pub const MAX_TREE: i64 = 6;

    /// 134j: at most this many unacknowledged messages (`queued`,
    /// `delivered`) in one inbox. A parent's messages may take all but one:
    /// the last is kept for C9's notice, which is never refused, so the cap
    /// holds with it.
    pub const MAX_INBOX: i64 = 32;
    const UNACKED: &str = "('pending_approval', 'queued', 'delivered')";
    /// C9: Storm's own words, fixed, with nothing of the parent's in them.
    pub const PARENT_ENDED_NOTICE: &str = "Storm: the agent that started you has ended. \
        Report your result now with report_result. This session is stopped in 30 seconds.";
    /// C9: the stop follows the notice after this long; no session extends it.
    pub const NOTICE_GRACE_SECS: i64 = 30;

    /// The longest name a parent may give a child, in characters.
    pub const MAX_NAME_CHARS: usize = 80;

    /// A start's payload, as its act row keeps it (145b).
    #[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
    pub struct StartPayload {
        pub task: String,
        /// The child's agent; the parent's own when absent. Another must be
        /// one the parent's host can start a child in (143a).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub provider: Option<String>,
        /// Unique among the parent's live children; one made from the task
        /// otherwise.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub name: Option<String>,
        /// A note the parent can read, the child's own context note.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub context_note: Option<super::Context>,
        /// Checked for the child's agent before the act (145b).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub model: Option<String>,
        /// A crew role (138), applied as a launch role is.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub role: Option<super::Context>,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct MessagePayload {
        pub text: String,
        /// The round the message belongs to (145).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub round: Option<i64>,
    }

    /// Why an act was refused, and for `limit`, the cap. `not_found` is the
    /// answer for a target out of scope.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct Refusal {
        pub reason: &'static str,
        pub cap: Option<&'static str>,
    }

    impl Refusal {
        fn because(reason: &'static str) -> Self {
            Self { reason, cap: None }
        }

        fn limit(cap: &'static str) -> Self {
            Self {
                reason: "limit",
                cap: Some(cap),
            }
        }
    }

    /// What the host's live link says now, when C0 holds.
    #[derive(Debug, Clone, Default, PartialEq, Eq)]
    pub struct HostNow {
        /// 0 is no limit.
        pub max_sessions: u32,
        /// `(id, kind)` of each agent a child may run there other than its
        /// parent's own (143a): installed, with Storm's servers, not `shell`.
        pub agents: Vec<(String, String)>,
    }

    /// What the host's live link says now: C0's refusal, or what it reports.
    pub type HostCheck<'a> = &'a dyn Fn(&str) -> std::result::Result<HostNow, &'static str>;

    /// The child's agent and its kind: the parent's own when the start
    /// names none or names it, else one its host reports it can start a
    /// child in (143a). `None` is `agent_unavailable`.
    pub fn child_agent(
        parent: &SessionRecord,
        named: Option<&str>,
        host: &HostNow,
    ) -> Option<(String, String)> {
        match named {
            None => Some((parent.provider.clone(), parent.provider_kind.clone())),
            Some(p) if p == parent.provider => {
                Some((parent.provider.clone(), parent.provider_kind.clone()))
            }
            Some(p) => host.agents.iter().find(|(id, _)| id == p).cloned(),
        }
    }

    /// A cascade target whose stop is due (L-3, L-8): immediate for a
    /// person's end, after the grace for a parent's own end or when the
    /// person let children report first.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct DueStop {
        pub cascade_id: String,
        pub cause: String,
        pub session_id: String,
        pub host_id: String,
        /// `pending`, or `unreachable` when an earlier stop found its host
        /// away.
        pub state: String,
    }

    impl Store {
        pub fn lineage(&self, session_id: &str) -> Result<Lineage> {
            lineage_of(&self.conn, session_id)
        }

        /// 139-D5: every spawned child still `creating` is failed
        /// `server_restart`, in one transaction, and returned with its
        /// parent. Each update is conditional on `creating`, so a child past
        /// it is never ended and a second run changes nothing.
        pub fn fail_children_left_creating(
            &mut self,
            now: &str,
        ) -> Result<Vec<(SessionRecord, String)>> {
            let tx = self.conn.transaction()?;
            let left: Vec<(String, String)> = {
                let mut stmt = tx.prepare(
                    "SELECT s.id, l.parent_session FROM sessions s
                     JOIN session_lineage l ON l.session_id = s.id
                     WHERE s.status = 'creating' ORDER BY s.created_at, s.id",
                )?;
                let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
                rows.collect::<rusqlite::Result<_>>()?
            };
            let mut failed = Vec::new();
            for (id, parent) in left {
                let changed = tx.execute(
                    "UPDATE sessions SET status = 'failed', end_reason = 'server_restart',
                         ended_at = ?2
                     WHERE id = ?1 AND status = 'creating'",
                    params![id, now],
                )?;
                if changed == 0 {
                    continue;
                }
                let r = get_in(&tx, &id)?.expect("just updated");
                on_ended(&tx, &r, now)?;
                crate::agent::events::record_in(
                    &tx,
                    &r,
                    crate::agent::events::FAILED,
                    r.end_reason.as_deref(),
                    crate::agent::events::STORM,
                    &crate::agent::events::LiveSignals::default(),
                    now,
                )?;
                failed.push((r, parent));
            }
            tx.commit()?;
            if !failed.is_empty() {
                self.notify_events();
            }
            Ok(failed)
        }

        /// Records a child at its approval, in one transaction: the session,
        /// its lineage under `parent` and its task. The only writer of
        /// lineage; nothing ever changes it after (L-1).
        #[cfg(test)]
        pub fn insert_spawned(
            &mut self,
            child: &SessionRecord,
            parent: &str,
            task: &str,
            now: &str,
        ) -> Result<Lineage> {
            let tx = self.conn.transaction()?;
            let lineage = insert_spawned(&tx, child, parent, task, now)?;
            tx.commit()?;
            Ok(lineage)
        }

        /// A session's descendants that still have a record, nearest first;
        /// with `live_only`, those that have not ended (`unknown` is live, L-6).
        pub fn descendants(&self, session_id: &str, live_only: bool) -> Result<Vec<String>> {
            descendants_of(&self.conn, session_id, live_only)
        }

        /// Whether `target` is a recorded descendant of `ancestor` (L-2),
        /// walked up `target`'s own lineage. A session is never its own.
        pub fn is_descendant(&self, ancestor: &str, target: &str) -> Result<bool> {
            if get_in(&self.conn, target)?.is_none() {
                return Ok(false);
            }
            let mut at = lineage_of(&self.conn, target)?;
            while let Some(parent) = at.parent_session {
                if parent == ancestor {
                    return Ok(true);
                }
                at = lineage_of(&self.conn, &parent)?;
            }
            Ok(false)
        }

        /// A session's own children that still have a record, oldest first.
        pub fn children(&self, parent: &str) -> Result<Vec<SessionRecord>> {
            let mut stmt = self.conn.prepare(&format!(
                "SELECT {COLUMNS} FROM sessions WHERE id IN
                     (SELECT session_id FROM session_lineage WHERE parent_session = ?1)
                 ORDER BY created_at, id"
            ))?;
            let rows = stmt.query_map(params![parent], from_row)?;
            Ok(rows.collect::<rusqlite::Result<_>>()?)
        }

        /// The cap a start by `requester` would hit now, counted as P-2
        /// counts it. Advisory: creation counts again (P-6).
        pub fn start_cap(
            &self,
            requester: &SessionRecord,
            max_sessions: u32,
        ) -> Result<Option<&'static str>> {
            over_cap(&self.conn, requester, max_sessions)
        }

        /// Test-only: a child recorded exactly as an approval records one,
        /// with the parent's grants, left `running` instead of failed.
        #[cfg(test)]
        pub fn spawn_child_for_tests(
            &mut self,
            parent: &str,
            task: &str,
            write_vault: bool,
        ) -> Result<SessionRecord> {
            let now = crate::index::now_rfc3339();
            let tx = self.conn.transaction()?;
            let parent = get_in(&tx, parent)?.expect("a parent");
            let payload = StartPayload {
                task: task.into(),
                ..StartPayload::default()
            };
            let agent = (parent.provider.clone(), parent.provider_kind.clone());
            let mut child = spawn_child(&tx, &parent, agent, &payload, write_vault, &now)?;
            child.status = "running".into();
            tx.execute(
                "UPDATE sessions SET status = 'running' WHERE id = ?1",
                params![child.id],
            )?;
            tx.commit()?;
            Ok(child)
        }

        /// Test-only: a session recorded as resuming `from`.
        #[cfg(test)]
        pub fn set_resumed_from_for_tests(&self, session_id: &str, from: &str) {
            self.conn
                .execute(
                    "UPDATE sessions SET resumed_from = ?2 WHERE id = ?1",
                    params![session_id, from],
                )
                .unwrap();
        }

        /// A child's workspace as its host first reported it; never replaced.
        pub fn record_child_workspace(
            &self,
            session_id: &str,
            made: &crate::agent::ChildWorkspace,
            now: &str,
        ) -> Result<()> {
            self.conn.execute(
                "INSERT OR IGNORE INTO session_child_workspace
                     (session_id, mode, base_commit, branch, submodules, large_files, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    session_id,
                    made.mode,
                    made.commit,
                    made.branch,
                    made.submodules,
                    made.large_files,
                    now
                ],
            )?;
            Ok(())
        }

        pub fn child_workspace(
            &self,
            session_id: &str,
        ) -> Result<Option<crate::agent::ChildWorkspace>> {
            Ok(self
                .conn
                .query_row(
                    "SELECT mode, base_commit, branch, submodules, large_files
                     FROM session_child_workspace WHERE session_id = ?1",
                    params![session_id],
                    |r| {
                        Ok(crate::agent::ChildWorkspace {
                            mode: r.get(0)?,
                            commit: r.get(1)?,
                            branch: r.get(2)?,
                            submodules: r.get(3)?,
                            large_files: r.get(4)?,
                        })
                    },
                )
                .optional()?)
        }

        /// Records a launch with the switch on (141e) and turns it on, before
        /// the host is sent `start`.
        pub fn record_spawn_launch(
            &mut self,
            session_id: &str,
            device: Option<&str>,
            now: &str,
        ) -> Result<()> {
            self.conn.execute(
                "INSERT OR IGNORE INTO session_spawn_launch (session_id, created_at) VALUES (?1, ?2)",
                params![session_id, now],
            )?;
            self.set_spawn(session_id, true, device, now)?;
            Ok(())
        }

        pub fn spawn_launched(&self, session_id: &str) -> Result<bool> {
            Ok(self
                .conn
                .query_row(
                    "SELECT 1 FROM session_spawn_launch WHERE session_id = ?1",
                    params![session_id],
                    |_| Ok(()),
                )
                .optional()?
                .is_some())
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

        /// Turns a session's spawn switch on or off (B2). Off stops its next
        /// act at once (145a): nothing waits to be refused.
        pub fn set_spawn(
            &mut self,
            session_id: &str,
            enabled: bool,
            device: Option<&str>,
            now: &str,
        ) -> Result<()> {
            self.conn.execute(
                "INSERT INTO session_spawn (session_id, enabled, changed_by_device, changed_at)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT (session_id) DO UPDATE SET enabled = excluded.enabled,
                     changed_by_device = excluded.changed_by_device,
                     changed_at = excluded.changed_at",
                params![session_id, enabled, device, now],
            )?;
            Ok(())
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
        #[cfg(test)]
        pub fn set_result(&self, session_id: &str, r: &SpawnResult) -> Result<()> {
            if !RESULT_OUTCOMES.contains(&r.outcome.as_str()) {
                bail!("{} is not a result outcome", r.outcome);
            }
            write_result(&self.conn, session_id, r)
        }

        /// `report_result` (134h) with its events (144a), in one
        /// transaction: `reported` with the outcome only, and
        /// `needs_attention` when a live session's report turns it
        /// `blocked`.
        pub fn record_result(
            &self,
            session_id: &str,
            r: &SpawnResult,
            live: &crate::agent::events::LiveSignals,
            source: &'static str,
        ) -> Result<()> {
            use crate::agent::events;
            let tx = self.conn.unchecked_transaction()?;
            let Some(session) = get_in(&tx, session_id)? else {
                bail!("no session {session_id}");
            };
            let before = events::needs_attention_in(&tx, &session, live.attention.as_deref())?;
            if !RESULT_OUTCOMES.contains(&r.outcome.as_str()) {
                bail!("{} is not a result outcome", r.outcome);
            }
            let r = &SpawnResult {
                round: round_of(&tx, session_id)?,
                ..r.clone()
            };
            write_result(&tx, session_id, r)?;
            let mut wrote = events::record_in(
                &tx,
                &session,
                events::REPORTED,
                Some(&r.outcome),
                source,
                live,
                &r.reported_at,
            )?;
            let after = events::needs_attention_in(&tx, &session, live.attention.as_deref())?;
            if after == Some(crate::agent::attention::BLOCKED) && before != after {
                wrote |= events::record_in(
                    &tx,
                    &session,
                    events::NEEDS_ATTENTION,
                    Some(crate::agent::attention::BLOCKED),
                    events::STORM,
                    live,
                    &r.reported_at,
                )?;
            }
            tx.commit()?;
            if wrote {
                self.notify_events();
            }
            Ok(())
        }

        /// 145c: a round's final message as its result, unless the child
        /// already reported this round itself, and what the agent said the
        /// round cost (145f), in one transaction. Returns whether the
        /// message became the result.
        pub fn record_final_message(
            &self,
            session_id: &str,
            r: &SpawnResult,
            usage: Option<&crate::agent::events::Usage>,
            live: &crate::agent::events::LiveSignals,
            source: &'static str,
        ) -> Result<bool> {
            let round = round_of(&self.conn, session_id)?;
            if let Some(usage) = usage.filter(|u| !u.is_empty()) {
                crate::agent::events::record_usage(
                    &self.conn,
                    session_id,
                    round,
                    usage,
                    &r.reported_at,
                )?;
            }
            let reported = self
                .conn
                .prepare("SELECT 1 FROM session_round_result WHERE session_id = ?1 AND round = ?2")?
                .exists(params![session_id, round])?;
            if reported {
                return Ok(false);
            }
            self.record_result(session_id, r, live, source)?;
            Ok(true)
        }

        /// The parked children on a host (145e): what `MAX_PARKED` counts.
        pub fn parked_on(&self, host_id: &str) -> Result<i64> {
            Ok(self.conn.query_row(
                "SELECT COUNT(*) FROM sessions WHERE host_id = ?1 AND status = 'parked'",
                params![host_id],
                |r| r.get(0),
            )?)
        }

        /// Whether `session_id` has a message it has not read yet: a
        /// parked child with one is resumed (145d).
        pub fn has_unread(&self, session_id: &str) -> Result<bool> {
            Ok(self
                .conn
                .prepare("SELECT 1 FROM session_inbox WHERE session_id = ?1 AND state = 'queued'")?
                .exists(params![session_id])?)
        }

        /// The newest message `session_id` has not read, by its number: the
        /// mark a child's wake is for (145d). 0 with none.
        pub fn unread_mark(&self, session_id: &str) -> Result<i64> {
            Ok(self.conn.query_row(
                "SELECT COALESCE(MAX(seq), 0) FROM session_inbox
                 WHERE session_id = ?1 AND state = 'queued'",
                params![session_id],
                |r| r.get(0),
            )?)
        }

        /// Whether `round`'s result is on record for `session_id`.
        pub fn reported_round(&self, session_id: &str, round: i64) -> Result<bool> {
            Ok(self
                .conn
                .prepare("SELECT 1 FROM session_round_result WHERE session_id = ?1 AND round = ?2")?
                .exists(params![session_id, round])?)
        }

        pub fn result(&self, session_id: &str) -> Result<Option<SpawnResult>> {
            Ok(self
                .conn
                .query_row(
                    "SELECT outcome, summary, details, reported_at, round FROM session_result
                     WHERE session_id = ?1",
                    params![session_id],
                    result_from_row,
                )
                .optional()?)
        }

        /// Every round's result, oldest first (145).
        pub fn results(&self, session_id: &str) -> Result<Vec<SpawnResult>> {
            let mut stmt = self.conn.prepare(
                "SELECT outcome, summary, details, reported_at, round FROM session_round_result
                 WHERE session_id = ?1 ORDER BY round",
            )?;
            let rows = stmt.query_map(params![session_id], result_from_row)?;
            Ok(rows.collect::<rusqlite::Result<_>>()?)
        }

        /// The round a child is in: 1 until a message comes after a result.
        pub fn round(&self, session_id: &str) -> Result<i64> {
            round_of(&self.conn, session_id)
        }

        /// Records a new request, `pending`.
        #[cfg(test)]
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

        #[cfg(test)]
        pub fn request(&self, id: &str) -> Result<Option<Request>> {
            Ok(self
                .conn
                .query_row(
                    &format!("SELECT {REQUEST_COLUMNS} FROM agent_requests WHERE id = ?1"),
                    params![id],
                    request_from_row,
                )
                .optional()?)
        }

        /// Puts a message in a session's inbox at the next sequence number.
        #[cfg(test)]
        pub fn queue_message(
            &mut self,
            session_id: &str,
            sender: &str,
            text: &str,
            request_id: Option<&str>,
            state: &str,
            now: &str,
        ) -> Result<i64> {
            let tx = self.conn.transaction()?;
            let seq = insert_message(&tx, session_id, sender, text, request_id, state, now)?;
            tx.commit()?;
            Ok(seq)
        }

        #[cfg(test)]
        pub fn inbox(&self, session_id: &str) -> Result<Vec<Message>> {
            messages_where(&self.conn, "session_id = ?1", params![session_id])
        }

        /// The messages `sender` asked to put in `session_id`'s inbox, in
        /// order, as their states now stand.
        pub fn messages_from(&self, sender: &str, session_id: &str) -> Result<Vec<Message>> {
            messages_where(
                &self.conn,
                "session_id = ?1 AND sender = ?2",
                params![session_id, sender],
            )
        }

        /// `read_messages` (134j), at least once. A cursor `after` first
        /// acknowledges what was delivered up to it; then every message
        /// queued, and every delivered one past the cursor, is returned and
        /// becomes `delivered`. A message is never returned before its
        /// approval, and one never returned is never acknowledged.
        pub fn read_messages(
            &mut self,
            session_id: &str,
            after: Option<i64>,
        ) -> Result<Vec<Message>> {
            let tx = self.conn.transaction()?;
            if let Some(after) = after {
                tx.execute(
                    "UPDATE session_inbox SET state = 'acknowledged'
                     WHERE session_id = ?1 AND state = 'delivered' AND seq <= ?2",
                    params![session_id, after],
                )?;
            }
            let mut read = messages_where(
                &tx,
                "session_id = ?1 AND (state = 'queued' OR (state = 'delivered' AND seq > ?2))",
                params![session_id, after.unwrap_or(0)],
            )?;
            tx.execute(
                "UPDATE session_inbox SET state = 'delivered'
                 WHERE session_id = ?1 AND state = 'queued'",
                params![session_id],
            )?;
            tx.commit()?;
            for m in &mut read {
                m.state = "delivered".into();
            }
            Ok(read)
        }

        /// Every cascade target whose stop is due at `now`, oldest cascade
        /// first: still live, `pending` or `unreachable`, and either its
        /// cascade stops at once or its grace is over. A child still
        /// `creating` waits for its `start` to go out, so its `end` never
        /// reaches the host first (AP-3).
        pub fn due_stops(&self, now: &str) -> Result<Vec<DueStop>> {
            static SQL: LazyLock<String> = LazyLock::new(|| {
                format!(
                    "SELECT c.id, c.cause, c.report_first, c.created_at, t.session_id, s.host_id,
                         t.state
                     FROM spawn_cascades c
                     JOIN spawn_cascade_targets t ON t.cascade_id = c.id
                     JOIN sessions s ON s.id = t.session_id
                     WHERE c.completed_at IS NULL AND t.state IN ('pending', 'unreachable')
                       AND s.status NOT IN {ENDED} AND s.status <> 'creating'
                     ORDER BY c.created_at, c.rowid, t.rowid"
                )
            });
            let mut stmt = self.conn.prepare_cached(&SQL)?;
            let rows = stmt.query_map([], |r| {
                Ok((
                    DueStop {
                        cascade_id: r.get(0)?,
                        cause: r.get(1)?,
                        session_id: r.get(4)?,
                        host_id: r.get(5)?,
                        state: r.get(6)?,
                    },
                    r.get::<_, bool>(2)?,
                    r.get::<_, String>(3)?,
                ))
            })?;
            Ok(rows
                .collect::<rusqlite::Result<Vec<_>>>()?
                .into_iter()
                .filter(|(d, report_first, at)| stop_due(&d.cause, *report_first, now, at))
                .map(|(d, ..)| d)
                .collect())
        }

        /// The end reason Storm's stop gives `session_id` (L-4), when the
        /// host's report of its end is that stop: it is still a target of
        /// an open cascade, and Storm sent the `end` (`sent`) or, after a
        /// restart, the stop is due. The first cascade that named it wins.
        pub fn stop_reason(
            &self,
            session_id: &str,
            now: &str,
            sent: bool,
        ) -> Result<Option<String>> {
            let mut stmt = self.conn.prepare(
                "SELECT c.cause, c.report_first, c.created_at FROM spawn_cascades c
                 JOIN spawn_cascade_targets t ON t.cascade_id = c.id
                 WHERE t.session_id = ?1 AND t.state IN ('pending', 'unreachable')",
            )?;
            let open = stmt
                .query_map(params![session_id], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, bool>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let due = open
                .iter()
                .any(|(cause, report_first, at)| sent || stop_due(cause, *report_first, now, at));
            if !due {
                return Ok(None);
            }
            Ok(self.end_cause(session_id)?.map(|c| c.end_reason))
        }

        /// Whether `session_id` is ending or doomed: a live target of an
        /// open cascade, or the parent of a person's end not yet recorded.
        #[cfg(test)]
        pub fn doomed(&self, session_id: &str) -> Result<bool> {
            doomed(&self.conn, session_id)
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
            let targets =
                open_cascade_in(&tx, id, cause, parent, parent_end_reason, report_first, now)?;
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
            let changed = set_target_in(&tx, cascade_id, session_id, state, now)?;
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
        #[cfg(test)]
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

        /// A start, at once (145a, 145b): the switch, C0, the child's agent,
        /// its name and every cap are checked, then the child is recorded
        /// (its record, lineage, task, narrowed grants and launch) with the
        /// act, in one transaction. A refusal writes nothing; the host is
        /// told after commit.
        pub fn start_child(
            &mut self,
            act: &str,
            requester: &str,
            payload: &StartPayload,
            now: &str,
            host: HostCheck<'_>,
        ) -> Result<std::result::Result<SessionRecord, Refusal>> {
            let tx = self.conn.transaction()?;
            let Some(r) = get_in(&tx, requester)? else {
                bail!("no session {requester}");
            };
            if let Err(refusal) = may_act(&tx, &r, None, host) {
                return Ok(Err(refusal));
            }
            let live = host(&r.host_id).unwrap_or_default();
            let Some(agent) = child_agent(&r, payload.provider.as_deref(), &live) else {
                return Ok(Err(Refusal::because("agent_unavailable")));
            };
            if let Some(name) = &payload.name
                && live_child_names(&tx, &r.id)?.contains(name)
            {
                return Ok(Err(Refusal::because("name_taken")));
            }
            if let Some(cap) = over_cap(&tx, &r, live.max_sessions)? {
                return Ok(Err(Refusal::limit(cap)));
            }
            let child = spawn_child(&tx, &r, agent, payload, true, now)?;
            insert_act(
                &tx,
                act,
                "start",
                requester,
                None,
                &serde_json::to_string(payload)?,
                "started",
                Some(&child.id),
                now,
            )?;
            tx.commit()?;
            Ok(Ok(child))
        }

        /// A message to a live descendant, at once: into its inbox `queued`,
        /// as the round it belongs to. A message after the target's result
        /// for its round starts the next round (145d); the round is
        /// returned.
        pub fn message_child(
            &mut self,
            act: &str,
            requester: &str,
            target: &str,
            text: &str,
            now: &str,
            host: HostCheck<'_>,
        ) -> Result<std::result::Result<i64, Refusal>> {
            let tx = self.conn.transaction()?;
            let Some(r) = get_in(&tx, requester)? else {
                bail!("no session {requester}");
            };
            if let Err(refusal) = may_act(&tx, &r, Some(target), host) {
                return Ok(Err(refusal));
            }
            if inbox_full(&tx, target)? {
                return Ok(Err(Refusal::limit("inbox")));
            }
            let mut round = round_of(&tx, target)?;
            let answered = tx
                .prepare("SELECT 1 FROM session_round_result WHERE session_id = ?1 AND round = ?2")?
                .exists(params![target, round])?;
            if answered {
                round += 1;
                tx.execute(
                    "INSERT INTO session_round (session_id, round, changed_at) VALUES (?1, ?2, ?3)
                     ON CONFLICT (session_id) DO UPDATE SET round = excluded.round,
                         changed_at = excluded.changed_at",
                    params![target, round, now],
                )?;
            }
            let payload = MessagePayload {
                text: text.to_string(),
                round: Some(round),
            };
            insert_act(
                &tx,
                act,
                "message",
                requester,
                Some(target),
                &serde_json::to_string(&payload)?,
                "delivered",
                None,
                now,
            )?;
            insert_message(&tx, target, requester, text, Some(act), "queued", now)?;
            tx.commit()?;
            Ok(Ok(round))
        }

        /// An end of a live descendant, at once: recorded here, and the
        /// manager sends `end` after commit. Its own descendants follow it
        /// when its end is recorded (`human_end`, C1).
        pub fn end_child(
            &mut self,
            act: &str,
            requester: &str,
            target: &str,
            now: &str,
            host: HostCheck<'_>,
        ) -> Result<std::result::Result<(), Refusal>> {
            let tx = self.conn.transaction()?;
            let Some(r) = get_in(&tx, requester)? else {
                bail!("no session {requester}");
            };
            if let Err(refusal) = may_act(&tx, &r, Some(target), host) {
                return Ok(Err(refusal));
            }
            insert_act(
                &tx,
                act,
                "end",
                requester,
                Some(target),
                "{}",
                "ended",
                None,
                now,
            )?;
            tx.commit()?;
            Ok(Ok(()))
        }

        /// The requests a session made, oldest first.
        pub fn requests_of(&self, requester: &str) -> Result<Vec<Request>> {
            let mut stmt = self.conn.prepare(&format!(
                "SELECT {REQUEST_COLUMNS} FROM agent_requests WHERE requester = ?1
                 ORDER BY created_at, rowid"
            ))?;
            let rows = stmt.query_map(params![requester], request_from_row)?;
            Ok(rows.collect::<rusqlite::Result<_>>()?)
        }
    }

    const REQUEST_COLUMNS: &str = "id, kind, requester, target, payload, created_at, expires_at, \
        state, decided_by_device, decided_at, outcome, reason, session";

    fn request_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Request> {
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
    }

    fn spawn_on(conn: &Connection, session_id: &str) -> Result<bool> {
        Ok(conn
            .query_row(
                "SELECT enabled FROM session_spawn WHERE session_id = ?1",
                params![session_id],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or(false))
    }

    /// Past its expiry at `now`. A stamp that does not parse is expired: an
    /// approval it cannot date never acts.
    pub fn expired(now: &str, expires_at: &str) -> bool {
        use time::format_description::well_known::Rfc3339;
        match (
            time::OffsetDateTime::parse(now, &Rfc3339),
            time::OffsetDateTime::parse(expires_at, &Rfc3339),
        ) {
            (Ok(now), Ok(at)) => now >= at,
            _ => true,
        }
    }

    fn plus_secs(stamp: &str, secs: i64) -> String {
        crate::index::rfc3339_minus_secs(stamp, -secs)
    }

    /// What every act requires of the requester and its target: live, its
    /// switch on, a spawning adapter, C0, and a target that is its live
    /// descendant (`not_found` otherwise).
    fn may_act(
        conn: &Connection,
        r: &SessionRecord,
        target: Option<&str>,
        host: HostCheck<'_>,
    ) -> std::result::Result<(), Refusal> {
        let fail = |e: anyhow::Error| {
            tracing::warn!(error = %e, "a spawn act's check failed");
            Refusal::because("not_permitted")
        };
        // Being ended asks for nothing more: a child it started now could be
        // one its cascade never snapshotted (AP-3).
        if r.is_ended() || doomed(conn, &r.id).map_err(fail)? {
            return Err(Refusal::because("requester_ended"));
        }
        if !spawn_on(conn, &r.id).map_err(fail)? {
            return Err(Refusal::because("spawn_off"));
        }
        if !crate::agent::SPAWN_PROVIDERS.contains(&r.provider.as_str()) {
            return Err(Refusal::because("not_permitted"));
        }
        host(&r.host_id).map_err(Refusal::because)?;
        if let Some(target) = target {
            let live = descendants_of(conn, &r.id, true).map_err(fail)?;
            if !live.iter().any(|d| d == target) {
                return Err(Refusal::because("not_found"));
            }
        }
        Ok(())
    }

    /// The cap a start by `r` would break, counting live sessions and the
    /// start itself (C4).
    fn over_cap(
        conn: &Connection,
        r: &SessionRecord,
        max_sessions: u32,
    ) -> Result<Option<&'static str>> {
        let lineage = lineage_of(conn, &r.id)?;
        if lineage.depth + 1 > MAX_DEPTH.min(BUILT_DEPTH) {
            return Ok(Some("depth"));
        }
        // 145e: a parked child has no process, so the live cap does not
        // count it; the tree's total does.
        let children: i64 = conn.query_row(
            &format!(
                "SELECT COUNT(*) FROM session_lineage l JOIN sessions s ON s.id = l.session_id
                 WHERE l.parent_session = ?1 AND s.status NOT IN {ENDED}
                   AND s.status != 'parked'"
            ),
            params![r.id],
            |row| row.get(0),
        )?;
        if children + 1 > MAX_CHILDREN {
            return Ok(Some("children"));
        }
        let root = &lineage.root_session;
        let root_live = get_in(conn, root)?.is_some_and(|s| !s.is_ended());
        let tree_live = root_live as i64 + descendants_of(conn, root, true)?.len() as i64;
        if tree_live + 1 > MAX_TREE {
            return Ok(Some("tree"));
        }
        if max_sessions > 0 {
            let live: i64 = conn.query_row(
                &format!(
                    "SELECT COUNT(*) FROM sessions WHERE host_id = ?1 AND status NOT IN {ENDED}
                       AND status != 'parked'"
                ),
                params![r.host_id],
                |row| row.get(0),
            )?;
            if live + 1 > max_sessions as i64 {
                return Ok(Some("max_sessions"));
            }
        }
        Ok(None)
    }

    /// An act, written already decided (145a): no one is asked.
    #[allow(clippy::too_many_arguments)]
    fn insert_act(
        conn: &Connection,
        id: &str,
        kind: &str,
        requester: &str,
        target: Option<&str>,
        payload: &str,
        outcome: &str,
        session: Option<&str>,
        now: &str,
    ) -> Result<()> {
        conn.execute(
            "INSERT INTO agent_requests (id, kind, requester, target, payload, created_at,
                 expires_at, state, decided_at, outcome, session)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6, 'decided', ?6, ?7, ?8)",
            params![id, kind, requester, target, payload, now, outcome, session],
        )?;
        Ok(())
    }

    /// The names of `parent`'s live children (145b).
    fn live_child_names(conn: &Connection, parent: &str) -> Result<Vec<String>> {
        let mut stmt = conn.prepare(&format!(
            "SELECT n.name FROM session_launch n
             JOIN session_lineage l ON l.session_id = n.session_id
             JOIN sessions s ON s.id = n.session_id
             WHERE l.parent_session = ?1 AND s.status NOT IN {ENDED}"
        ))?;
        let rows = stmt.query_map(params![parent], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    fn round_of(conn: &Connection, session_id: &str) -> Result<i64> {
        Ok(conn
            .query_row(
                "SELECT round FROM session_round WHERE session_id = ?1",
                params![session_id],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or(1))
    }

    fn result_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<SpawnResult> {
        Ok(SpawnResult {
            outcome: r.get(0)?,
            summary: r.get(1)?,
            details: r.get(2)?,
            reported_at: r.get(3)?,
            round: r.get(4)?,
        })
    }

    /// A result as the latest and as its round's: a later report in the
    /// same round replaces the earlier one.
    fn write_result(conn: &Connection, session_id: &str, r: &SpawnResult) -> Result<()> {
        if !RESULT_OUTCOMES.contains(&r.outcome.as_str()) {
            bail!("{} is not a result outcome", r.outcome);
        }
        for table in ["session_result", "session_round_result"] {
            let key = if table == "session_result" {
                "session_id"
            } else {
                "session_id, round"
            };
            conn.execute(
                &format!(
                    "INSERT INTO {table} (session_id, outcome, summary, details, reported_at,
                         round)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                     ON CONFLICT ({key}) DO UPDATE SET outcome = excluded.outcome,
                         summary = excluded.summary, details = excluded.details,
                         reported_at = excluded.reported_at, round = excluded.round"
                ),
                params![
                    session_id,
                    r.outcome,
                    r.summary,
                    r.details,
                    r.reported_at,
                    r.round
                ],
            )?;
        }
        Ok(())
    }

    /// The workspace name a child is recorded under: its own directory,
    /// which its host makes at its start (137-1) and then reports under
    /// this name in `workspace_paths`, so WS-9 sees it apart from every
    /// other. A name with a separator is never a listed workspace, so no
    /// launch can name it.
    pub fn child_workspace(child: &str) -> String {
        format!("children/{child}")
    }

    /// A start's act: the child's record, lineage and task (L-1), narrowed
    /// to the parent's grants as they are now (134e, AP-7), with the
    /// parent's write vault when `write_vault` and the parent launched with
    /// one it may write (145a: a start inherits it; nothing wider). Spawn is off: a child has no switch row. Its workspace
    /// is its own (137-1), its agent `agent` (143a), its name the one asked
    /// for or one made from the task, unique among its live siblings, and
    /// its model and role the start's (145b).
    fn spawn_child(
        conn: &Connection,
        parent: &SessionRecord,
        (provider, provider_kind): (String, String),
        payload: &StartPayload,
        write_vault: bool,
        now: &str,
    ) -> Result<SessionRecord> {
        let id = crate::auth::identity::random_id("ags_");
        let child = SessionRecord {
            workspace: child_workspace(&id),
            id,
            owner_user_id: parent.owner_user_id.clone(),
            host_id: parent.host_id.clone(),
            provider,
            provider_kind,
            interaction: parent.interaction.clone(),
            provider_fallback: None,
            status: "creating".into(),
            end_reason: None,
            signal: None,
            exit_code: None,
            created_at: now.to_string(),
            started_at: None,
            ended_at: None,
            last_activity: None,
            egress: parent.egress.clone(),
            cols: parent.cols,
            rows: parent.rows,
            title: None,
            activity: None,
            harness_session: None,
            resumed_from: None,
        };
        insert_spawned(conn, &child, &parent.id, &payload.task, now)?;
        let grants: Vec<McpGrant> = {
            let mut stmt = conn.prepare(
                "SELECT connection_id, slug FROM session_mcp_grants
                 WHERE session_id = ?1 AND revoked_at IS NULL ORDER BY granted_at, slug",
            )?;
            let rows = stmt.query_map(params![parent.id], |r| {
                Ok(McpGrant {
                    id: r.get(0)?,
                    slug: r.get(1)?,
                    plugin_origin: None,
                })
            })?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        let builtin = crate::gateway::connections::BUILTIN_ID;
        let reads_storm = grants.iter().any(|g| g.id == builtin);
        let parent_vault: Option<String> = conn
            .query_row(
                "SELECT write_vault_id FROM session_launch WHERE session_id = ?1",
                params![parent.id],
                |r| r.get(0),
            )
            .optional()?
            .flatten();
        // The parent's own write vault, only when it launched able to write
        // there: never wider than the parent's live grant (134e).
        let parent_writes: bool = conn
            .query_row(
                "SELECT allow_vault_writes FROM session_mcp WHERE session_id = ?1",
                params![parent.id],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or(false);
        let write_vault_id = parent_vault.filter(|_| write_vault && reads_storm && parent_writes);
        insert_grants(conn, &child.id, write_vault_id.is_some(), &grants, now)?;
        let name = match &payload.name {
            Some(name) => name.clone(),
            None => {
                let from_task: String = payload
                    .task
                    .split_whitespace()
                    .take(4)
                    .collect::<Vec<_>>()
                    .join(" ");
                let titled = payload
                    .context_note
                    .as_ref()
                    .map_or(from_task.as_str(), |c| c.title.as_str());
                let siblings = live_child_names(conn, &parent.id)?;
                crate::agent::session_name(Some(titled), &parent.workspace, &siblings)
            }
        };
        let role = payload.role.clone().filter(|_| reads_storm);
        insert_launch(
            conn,
            &LaunchRecord {
                session_id: child.id.clone(),
                name,
                context: payload.context_note.clone(),
                write_vault_id,
                role: role.clone(),
                model: payload.model.clone(),
            },
            now,
        )?;
        if let Some(role) = &payload.role {
            conn.execute(
                "INSERT INTO session_role (session_id, vault_id, note_id, set_at)
                 VALUES (?1, ?2, ?3, ?4)",
                params![child.id, role.vault_id, role.note_id, now],
            )?;
        }
        let named: Vec<SessionIntegration> = {
            let mut stmt = conn.prepare(
                "SELECT connection_id, slug, display_name FROM session_integrations
                 WHERE session_id = ?1",
            )?;
            let rows = stmt.query_map(params![parent.id], |r| {
                Ok(SessionIntegration {
                    id: r.get(0)?,
                    slug: r.get(1)?,
                    display_name: r.get(2)?,
                })
            })?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        let integrations: Vec<SessionIntegration> = named
            .into_iter()
            .filter(|i| i.id != builtin && grants.iter().any(|g| g.id == i.id))
            .collect();
        insert_integrations(conn, &child.id, &integrations)?;
        Ok(child)
    }

    fn insert_spawned(
        conn: &Connection,
        child: &SessionRecord,
        parent: &str,
        task: &str,
        now: &str,
    ) -> Result<Lineage> {
        if task.len() > MAX_TASK_BYTES {
            bail!("the task is over {MAX_TASK_BYTES} bytes");
        }
        let exists = conn
            .prepare("SELECT 1 FROM sessions WHERE id = ?1")?
            .exists([parent])?;
        if !exists {
            bail!("no parent session {parent}");
        }
        let up = lineage_of(conn, parent)?;
        let lineage = Lineage {
            parent_session: Some(parent.to_string()),
            root_session: up.root_session,
            depth: up.depth + 1,
        };
        insert_session(conn, child)?;
        conn.execute(
            "INSERT INTO session_lineage
                 (session_id, parent_session, root_session, depth, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![child.id, parent, lineage.root_session, lineage.depth, now],
        )?;
        conn.execute(
            "INSERT INTO session_task (session_id, text, created_at) VALUES (?1, ?2, ?3)",
            params![child.id, task, now],
        )?;
        Ok(lineage)
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

    /// What a dismissal takes with a session (C8): its switch, task,
    /// results, rounds and inbox. Its acts, its lineage and cascades stay as
    /// history.
    pub(super) fn forget(conn: &Connection, session_id: &str) -> Result<()> {
        for table in [
            "session_spawn",
            "session_spawn_asks",
            "session_spawn_launch",
            "session_child_workspace",
            "session_task",
            "session_result",
            "session_round",
            "session_round_result",
            "session_round_usage",
            "session_inbox",
        ] {
            conn.execute(
                &format!("DELETE FROM {table} WHERE session_id = ?1"),
                params![session_id],
            )?;
        }
        Ok(())
    }

    // ---- 134-6: the inbox (134j) and C9's notice --------------------------

    fn messages_where(
        conn: &Connection,
        clause: &str,
        params: impl rusqlite::Params,
    ) -> Result<Vec<Message>> {
        let mut stmt = conn.prepare(&format!(
            "SELECT seq, sender, text, request_id, state, at FROM session_inbox
             WHERE {clause} ORDER BY seq"
        ))?;
        let rows = stmt.query_map(params, |r| {
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

    fn next_seq(conn: &Connection, session_id: &str) -> Result<i64> {
        Ok(conn.query_row(
            "SELECT COALESCE(MAX(seq), 0) + 1 FROM session_inbox WHERE session_id = ?1",
            params![session_id],
            |r| r.get(0),
        )?)
    }

    fn insert_message(
        conn: &Connection,
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
        let seq = next_seq(conn, session_id)?;
        conn.execute(
            "INSERT INTO session_inbox (session_id, seq, sender, text, request_id, state, at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![session_id, seq, sender, text, request_id, state, now],
        )?;
        Ok(seq)
    }

    /// Whether one more message from a parent would break the inbox's cap.
    fn inbox_full(conn: &Connection, session_id: &str) -> Result<bool> {
        let waiting: i64 = conn.query_row(
            &format!(
                "SELECT COUNT(*) FROM session_inbox
                 WHERE session_id = ?1 AND state IN {UNACKED}"
            ),
            params![session_id],
            |r| r.get(0),
        )?;
        Ok(waiting + 1 > MAX_INBOX - 1)
    }

    /// A message an older server left `pending_approval` takes its
    /// request's end: `denied`, `expired`, else `undeliverable`. Only at
    /// open: since 145a no message waits.
    fn settle_messages(conn: &Connection) -> Result<()> {
        conn.execute(
            "UPDATE session_inbox SET state = CASE
                     (SELECT outcome FROM agent_requests r WHERE r.id = session_inbox.request_id)
                 WHEN 'denied' THEN 'denied' WHEN 'expired' THEN 'expired'
                 ELSE 'undeliverable' END
             WHERE state = 'pending_approval' AND request_id IN
                 (SELECT id FROM agent_requests WHERE state = 'decided' AND outcome != 'delivered')",
            [],
        )?;
        Ok(())
    }

    /// C9's notice, `queued` at once: Storm's own fixed text, no approval,
    /// never refused by the cap. One waiting notice is enough.
    fn queue_notice(conn: &Connection, session_id: &str, now: &str) -> Result<()> {
        let waiting = conn
            .prepare(&format!(
                "SELECT 1 FROM session_inbox
                 WHERE session_id = ?1 AND sender = 'storm' AND state IN {UNACKED}"
            ))?
            .exists(params![session_id])?;
        if !waiting {
            insert_message(
                conn,
                session_id,
                "storm",
                PARENT_ENDED_NOTICE,
                None,
                "queued",
                now,
            )?;
        }
        Ok(())
    }

    fn grace_over(now: &str, opened: &str) -> bool {
        expired(now, &plus_secs(opened, NOTICE_GRACE_SECS))
    }

    /// C9 and C1: a parent's own end, or a person who let children report
    /// first, gives the grace; any other person's end stops at once.
    fn stop_due(cause: &str, report_first: bool, now: &str, opened: &str) -> bool {
        if cause == "parent_end" || report_first {
            grace_over(now, opened)
        } else {
            true
        }
    }

    fn doomed(conn: &Connection, session_id: &str) -> Result<bool> {
        let target = conn
            .prepare(
                "SELECT 1 FROM spawn_cascade_targets t JOIN spawn_cascades c ON c.id = t.cascade_id
                 WHERE t.session_id = ?1 AND c.completed_at IS NULL
                   AND t.state IN ('pending', 'unreachable')",
            )?
            .exists(params![session_id])?;
        let ending = conn
            .prepare(
                "SELECT 1 FROM spawn_cascades WHERE parent_session = ?1 AND cause = 'human_end'",
            )?
            .exists(params![session_id])?;
        Ok(target || ending)
    }

    fn open_cascade_in(
        conn: &Connection,
        id: &str,
        cause: &str,
        parent: &str,
        parent_end_reason: Option<&str>,
        report_first: bool,
        now: &str,
    ) -> Result<Vec<String>> {
        let targets = descendants_of(conn, parent, true)?;
        conn.execute(
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
            conn.execute(
                "INSERT INTO spawn_cascade_targets (cascade_id, session_id, state, updated_at)
                 VALUES (?1, ?2, 'pending', ?3)",
                params![id, t, now],
            )?;
            // L-8: a grace always comes with the notice that explains it.
            if cause == "parent_end" || report_first {
                queue_notice(conn, t, now)?;
            }
        }
        Ok(targets)
    }

    fn set_target_in(
        conn: &Connection,
        cascade_id: &str,
        session_id: &str,
        state: &str,
        now: &str,
    ) -> Result<bool> {
        let changed = conn.execute(
            "UPDATE spawn_cascade_targets SET state = ?3, updated_at = ?4
             WHERE cascade_id = ?1 AND session_id = ?2
               AND state IN ('pending', 'unreachable') AND state != ?3",
            params![cascade_id, session_id, state, now],
        )? > 0;
        conn.execute(
            "UPDATE spawn_cascades SET completed_at = ?2
             WHERE id = ?1 AND completed_at IS NULL AND NOT EXISTS (
                 SELECT 1 FROM spawn_cascade_targets
                 WHERE cascade_id = ?1 AND state IN ('pending', 'unreachable'))",
            params![cascade_id, now],
        )?;
        Ok(changed)
    }

    /// What a session's end does, in the transaction that records it:
    /// - its messages never read are `undeliverable` (L-7);
    /// - as a cascade's target it is `ended` when Storm stopped it, else
    ///   `already_ended`, its own reason kept (L-4);
    /// - with live descendants no cascade covers yet, it opens one over
    ///   them (L-3): `parent_end` when it ended by itself (`completed` or
    ///   `failed`), with C9's notice (L-8); `human_end` when a person
    ///   stopped it (`stopped`), immediate (C1). A host restart opens
    ///   nothing (L-4), and neither does Storm's own stop, whose cascade
    ///   already holds its descendants. The stops are the manager's.
    pub(super) fn on_ended(conn: &Connection, r: &SessionRecord, now: &str) -> Result<()> {
        conn.execute(
            "UPDATE session_inbox SET state = 'undeliverable'
             WHERE session_id = ?1 AND state = 'queued'",
            params![r.id],
        )?;
        let by_storm = matches!(r.end_reason.as_deref(), Some("parent_ended" | "cascade"));
        let cascades: Vec<String> = {
            let mut stmt = conn.prepare(
                "SELECT cascade_id FROM spawn_cascade_targets
                 WHERE session_id = ?1 AND state IN ('pending', 'unreachable')",
            )?;
            let rows = stmt.query_map(params![r.id], |row| row.get(0))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        for c in cascades {
            let state = if by_storm { "ended" } else { "already_ended" };
            set_target_in(conn, &c, &r.id, state, now)?;
        }
        if r.end_reason.as_deref() == Some("host_restart") || by_storm {
            return Ok(());
        }
        let cause = match r.status.as_str() {
            "completed" | "failed" => "parent_end",
            "stopped" => "human_end",
            _ => return Ok(()),
        };
        let mut uncovered = false;
        for d in descendants_of(conn, &r.id, true)? {
            if !doomed(conn, &d)? {
                uncovered = true;
                break;
            }
        }
        if !uncovered {
            return Ok(());
        }
        let reason = r.end_reason.as_deref().unwrap_or(&r.status);
        let id = crate::auth::identity::random_id("spc_");
        open_cascade_in(conn, &id, cause, &r.id, Some(reason), false, now)?;
        Ok(())
    }
}
