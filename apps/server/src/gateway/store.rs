//! `state/gateway/gateway.db` (spec §8, decision 81b).
//!
//! **This file cannot be rebuilt.** Like `auth.db`, nothing regenerates it by
//! rescanning markdown, so `backup_all()` carries it — and its key directory,
//! because a database of ciphertexts without the key that opens them restores
//! a gateway that knows every connection and can authenticate none.
//!
//! **The schema is additive only.** Every statement is `CREATE … IF NOT
//! EXISTS`; a later slice adds a table or an index, never rewrites one. There
//! is no version number to bump because there is nothing to migrate.
//!
//! **No secret is ever stored in the clear here.** Credentials, a pasted OAuth
//! client secret and a PKCE verifier are [`Sealed`] by the keyring; an OAuth
//! `state` is stored only as its blake3 hash. The `calls` audit holds
//! metadata only — never arguments, never results (G-D18).

use std::path::Path;

use anyhow::Result;
use rusqlite::{Connection as Sqlite, OptionalExtension, params};

use super::connections::Connection;
use super::crypto::Sealed;

pub struct GatewayDb {
    conn: Sqlite,
}

/// How long the call audit keeps a row, and how many rows it keeps at most
/// (spec §14: 30 days / 100k rows). Whichever bound is hit first wins.
pub const CALLS_KEEP_MS: i64 = 30 * 24 * 60 * 60 * 1000;
pub const CALLS_KEEP_ROWS: i64 = 100_000;

const SCHEMA: &str = "
    PRAGMA journal_mode = WAL;

    CREATE TABLE IF NOT EXISTS meta (
        key   TEXT PRIMARY KEY,
        value TEXT NOT NULL
    );

    -- One owner's link to one upstream account (spec §5). `revoked` is the
    -- tombstone a disconnect leaves (§13): the row stays so the audit can name
    -- it, its credentials are deleted, and its slug is free again.
    CREATE TABLE IF NOT EXISTS connections (
        id                     TEXT PRIMARY KEY,
        owner_user_id          TEXT NOT NULL,
        slug                   TEXT NOT NULL,
        display_name           TEXT NOT NULL,
        url                    TEXT NOT NULL,
        auth_kind              TEXT NOT NULL,
        status                 TEXT NOT NULL,
        tool_allowlist         TEXT NOT NULL DEFAULT '[]',
        known_tools            TEXT,
        expose_resources       INTEGER NOT NULL DEFAULT 1,
        expose_prompts         INTEGER NOT NULL DEFAULT 1,
        upstream_account_label TEXT,
        last_ok                TEXT,
        last_error_code        TEXT,
        created_at             TEXT NOT NULL,
        updated_at             TEXT NOT NULL
    );
    CREATE UNIQUE INDEX IF NOT EXISTS connections_owner_slug
        ON connections(owner_user_id, slug) WHERE status != 'revoked';

    -- What Storm presents upstream, sealed (AAD = connection_id, kind).
    CREATE TABLE IF NOT EXISTS credentials (
        connection_id TEXT NOT NULL,
        kind          TEXT NOT NULL,
        key_id        TEXT NOT NULL,
        nonce         BLOB NOT NULL,
        ciphertext    BLOB NOT NULL,
        expires_at    TEXT,
        created_at    TEXT NOT NULL,
        updated_at    TEXT NOT NULL,
        PRIMARY KEY (connection_id, kind)
    );

    -- An OAuth client per authorization server (§10). A pasted secret is
    -- sealed (AAD = id, 'client_secret'); a dynamically registered public
    -- client has none.
    CREATE TABLE IF NOT EXISTS oauth_clients (
        id                TEXT PRIMARY KEY,
        owner_user_id     TEXT NOT NULL,
        issuer            TEXT NOT NULL,
        client_id         TEXT NOT NULL,
        redirect_uri      TEXT NOT NULL,
        registered        INTEGER NOT NULL,
        metadata          TEXT,
        secret_key_id     TEXT,
        secret_nonce      BLOB,
        secret_ciphertext BLOB,
        created_at        TEXT NOT NULL,
        UNIQUE (owner_user_id, issuer, redirect_uri)
    );

    -- A pending authorization (§8): the state only as a hash, the PKCE
    -- verifier sealed (AAD = state_hash, 'pkce_verifier'), single use, ten
    -- minutes.
    CREATE TABLE IF NOT EXISTS oauth_flows (
        state_hash          TEXT PRIMARY KEY,
        connection_id       TEXT NOT NULL,
        owner_user_id       TEXT NOT NULL,
        oauth_client        TEXT NOT NULL,
        redirect_uri        TEXT NOT NULL,
        resource            TEXT NOT NULL,
        verifier_key_id     TEXT NOT NULL,
        verifier_nonce      BLOB NOT NULL,
        verifier_ciphertext BLOB NOT NULL,
        created_at          TEXT NOT NULL,
        expires_at          TEXT NOT NULL,
        used_at             TEXT
    );

    -- The call audit (G-D18): metadata only. `at_ms` is epoch milliseconds so
    -- retention is an exact comparison, not a string one.
    CREATE TABLE IF NOT EXISTS calls (
        id             INTEGER PRIMARY KEY AUTOINCREMENT,
        at_ms          INTEGER NOT NULL,
        owner_user_id  TEXT NOT NULL,
        connection_id  TEXT NOT NULL,
        session_id     TEXT,
        host_id        TEXT,
        method         TEXT NOT NULL,
        tool           TEXT,
        outcome        TEXT NOT NULL,
        error_code     TEXT,
        duration_ms    INTEGER,
        response_bytes INTEGER
    );
    CREATE INDEX IF NOT EXISTS calls_by_time ON calls(at_ms);

    -- Upstream tools that appeared after the owner's first test and that the
    -- owner has not reviewed yet (spec §9, decision 81k). Off by
    -- construction — the allowlist names tools explicitly — and shown as
    -- N new tools, review until the owner saves the connection's tool
    -- list. A table rather than a column on `connections`, so an observation
    -- is one `INSERT OR IGNORE` that no rewrite of the connection row clobbers
    -- (the reason `note_access` is a table in the index database).
    CREATE TABLE IF NOT EXISTS new_tools (
        connection_id TEXT NOT NULL,
        name          TEXT NOT NULL,
        seen_at       TEXT NOT NULL,
        PRIMARY KEY (connection_id, name)
    );

    -- Which OAuth client an `oauth` connection was authorized with, so a
    -- refresh uses the same one (decision 81g).
    CREATE TABLE IF NOT EXISTS connection_oauth (
        connection_id TEXT PRIMARY KEY,
        oauth_client  TEXT NOT NULL,
        updated_at    TEXT NOT NULL
    );
";

/// One audit row. There is deliberately no field that could carry arguments
/// or a result: the type is what keeps G-D18 true.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallRecord {
    pub at_ms: i64,
    pub owner_user_id: String,
    pub connection_id: String,
    pub session_id: Option<String>,
    pub host_id: Option<String>,
    pub method: String,
    pub tool: Option<String>,
    pub outcome: String,
    pub error_code: Option<String>,
    pub duration_ms: Option<i64>,
    pub response_bytes: Option<i64>,
}

/// An OAuth client of one authorization server, for one owner and redirect.
#[derive(Debug, Clone)]
pub struct OAuthClientRow {
    pub id: String,
    pub owner_user_id: String,
    pub issuer: String,
    pub client_id: String,
    pub redirect_uri: String,
    /// Registered dynamically (RFC 7591), or pasted by the owner.
    pub registered: bool,
    /// The authorization server's metadata, as discovered.
    pub metadata: Option<String>,
    /// A pasted client secret, sealed (AAD = id, `client_secret`).
    pub secret: Option<Sealed>,
    pub created_at: String,
}

/// A pending authorization (§8): the state as a hash, the verifier sealed.
#[derive(Debug, Clone)]
pub struct FlowRow {
    pub state_hash: String,
    pub connection_id: String,
    pub owner_user_id: String,
    pub oauth_client: String,
    pub redirect_uri: String,
    pub resource: String,
    pub sealed: Sealed,
    pub created_at: String,
    pub expires_at: String,
}

#[derive(Debug, thiserror::Error)]
pub enum InsertError {
    #[error("that slug is already used by another of your integrations")]
    SlugTaken,
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

/// At most this many tools await review per connection: an upstream decides
/// how many it lists, so the record of them is bounded.
pub const MAX_NEW_TOOLS: i64 = 200;

const CONNECTION_COLUMNS: &str = "id, owner_user_id, slug, display_name, url, auth_kind, \
    status, tool_allowlist, known_tools, expose_resources, expose_prompts, \
    upstream_account_label, last_ok, last_error_code, created_at, updated_at";

fn json_list(items: &[String]) -> String {
    serde_json::to_string(items).expect("a list of strings serializes")
}

fn connection_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Connection> {
    let list = |s: String| serde_json::from_str::<Vec<String>>(&s).unwrap_or_default();
    Ok(Connection {
        id: r.get(0)?,
        owner_user_id: r.get(1)?,
        slug: r.get(2)?,
        display_name: r.get(3)?,
        url: r.get(4)?,
        auth_kind: r.get(5)?,
        status: r.get(6)?,
        tool_allowlist: list(r.get(7)?),
        known_tools: r.get::<_, Option<String>>(8)?.map(list),
        expose_resources: r.get(9)?,
        expose_prompts: r.get(10)?,
        upstream_account_label: r.get(11)?,
        last_ok: r.get(12)?,
        last_error_code: r.get(13)?,
        created_at: r.get(14)?,
        updated_at: r.get(15)?,
    })
}

impl GatewayDb {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Sqlite::open(path)?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self { conn })
    }

    /// A consistent copy for `backup_all()`, through `VACUUM INTO` like every
    /// other database: the server holds this one open in WAL mode, so a plain
    /// file copy could miss committed pages still in the `-wal`.
    pub fn snapshot_to(&self, dest: &Path) -> Result<()> {
        crate::db::snapshot_connection(&self.conn, dest)
    }

    // ---- meta -----------------------------------------------------------

    pub fn meta(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row("SELECT value FROM meta WHERE key = ?1", params![key], |r| {
                r.get(0)
            })
            .optional()?)
    }

    pub fn set_meta(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO meta (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    // ---- connections ----------------------------------------------------

    /// Inserts a connection and, if it has one, its sealed credential, in one
    /// transaction: there is never a connection row whose credential failed
    /// to land, nor a credential with no row. A live slug clash with the same
    /// owner is [`InsertError::SlugTaken`], decided by the unique index rather
    /// than by a read-then-write that two requests could race.
    pub fn insert_connection(
        &mut self,
        c: &Connection,
        credential: Option<(&str, &Sealed)>,
    ) -> Result<(), InsertError> {
        let tx = self.conn.transaction().map_err(anyhow::Error::from)?;
        let result = tx.execute(
            &format!(
                "INSERT INTO connections ({CONNECTION_COLUMNS})
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)"
            ),
            params![
                c.id,
                c.owner_user_id,
                c.slug,
                c.display_name,
                c.url,
                c.auth_kind,
                c.status,
                json_list(&c.tool_allowlist),
                c.known_tools.as_deref().map(json_list),
                c.expose_resources,
                c.expose_prompts,
                c.upstream_account_label,
                c.last_ok,
                c.last_error_code,
                c.created_at,
                c.updated_at,
            ],
        );
        match result {
            Ok(_) => {}
            Err(rusqlite::Error::SqliteFailure(e, _))
                if e.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                return Err(InsertError::SlugTaken);
            }
            Err(e) => return Err(InsertError::Other(e.into())),
        }
        if let Some((kind, sealed)) = credential {
            tx.execute(
                "INSERT INTO credentials
                     (connection_id, kind, key_id, nonce, ciphertext, expires_at, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, NULL, ?6, ?6)",
                params![c.id, kind, sealed.key_id, sealed.nonce, sealed.ciphertext, c.created_at],
            )
            .map_err(anyhow::Error::from)?;
        }
        tx.commit().map_err(anyhow::Error::from)?;
        Ok(())
    }

    /// A connection by id, revoked ones included (the audit names them).
    pub fn connection(&self, id: &str) -> Result<Option<Connection>> {
        Ok(self
            .conn
            .query_row(
                &format!("SELECT {CONNECTION_COLUMNS} FROM connections WHERE id = ?1"),
                params![id],
                connection_from_row,
            )
            .optional()?)
    }

    /// An owner's live connections — everything but the revoked tombstones.
    pub fn connections_of(&self, owner_user_id: &str) -> Result<Vec<Connection>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {CONNECTION_COLUMNS} FROM connections
             WHERE owner_user_id = ?1 AND status != 'revoked'
             ORDER BY created_at, id"
        ))?;
        let rows = stmt.query_map(params![owner_user_id], connection_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn live_connections_not_of(&self, account_id: &str) -> Result<Vec<Connection>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {CONNECTION_COLUMNS} FROM connections
             WHERE owner_user_id != ?1 AND status != 'revoked'
             ORDER BY created_at, id"
        ))?;
        let rows = stmt.query_map(params![account_id], connection_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Writes back the mutable fields. `id`, `owner_user_id`, `slug`, `url`
    /// and `auth_kind` are never rewritten: a credential is presented only to
    /// its own upstream (AM24), so a connection cannot be re-pointed, and the
    /// slug is what live sessions were told.
    ///
    /// **`known_tools` is not among them**: only [`observe_tools`] and
    /// [`review_tools`] write it, each in one step under the store's lock, so
    /// a caller holding an older copy of the row cannot undo a review.
    ///
    /// [`observe_tools`]: Self::observe_tools
    /// [`review_tools`]: Self::review_tools
    pub fn update_connection(&self, c: &Connection) -> Result<()> {
        self.conn.execute(
            "UPDATE connections SET display_name = ?2, status = ?3, tool_allowlist = ?4,
                 expose_resources = ?5, expose_prompts = ?6,
                 upstream_account_label = ?7, last_ok = ?8, last_error_code = ?9,
                 updated_at = ?10
             WHERE id = ?1",
            params![
                c.id,
                c.display_name,
                c.status,
                json_list(&c.tool_allowlist),
                c.expose_resources,
                c.expose_prompts,
                c.upstream_account_label,
                c.last_ok,
                c.last_error_code,
                c.updated_at,
            ],
        )?;
        Ok(())
    }

    // ---- the allowlist rule over time (G-D16, spec §9) ---------------------

    /// Applies a listing of the upstream's tools to the connection's records.
    ///
    /// - **The owner's first test is the baseline** (`baseline`, and no
    ///   `known_tools` yet): every listed tool is turned on and remembered
    ///   ("everything is on at connect time", G-D16).
    /// - **After it, a tool not seen before is recorded as new and stays
    ///   off.** Nothing here ever adds to the allowlist, so a new tool is off
    ///   by construction until the owner turns it on.
    /// - **An agent's listing is never a baseline** (`baseline` false): before
    ///   the owner's first test it records nothing, so an agent cannot cause a
    ///   tool to be enabled.
    /// - A name that fails [`valid_tool_name`] is ignored, and at most
    ///   [`MAX_NEW_TOOLS`] stay pending per connection: an upstream controls
    ///   both its names and how many it lists.
    ///
    /// Returns the names newly recorded as new by this call.
    ///
    /// [`valid_tool_name`]: super::connections::valid_tool_name
    pub fn observe_tools(
        &mut self,
        connection_id: &str,
        listed: &[String],
        baseline: bool,
        now: &str,
    ) -> Result<Vec<String>> {
        let mut listed: Vec<String> = listed
            .iter()
            .filter(|n| super::connections::valid_tool_name(n))
            .cloned()
            .collect();
        listed.sort();
        listed.dedup();
        let tx = self.conn.transaction()?;
        let known: Option<Option<String>> = tx
            .query_row(
                "SELECT known_tools FROM connections WHERE id = ?1",
                params![connection_id],
                |r| r.get(0),
            )
            .optional()?;
        let Some(known) = known else {
            return Ok(Vec::new());
        };
        let Some(known) =
            known.map(|s| serde_json::from_str::<Vec<String>>(&s).unwrap_or_default())
        else {
            if baseline {
                tx.execute(
                    "UPDATE connections SET known_tools = ?2, tool_allowlist = ?2 WHERE id = ?1",
                    params![connection_id, json_list(&listed)],
                )?;
                tx.execute(
                    "DELETE FROM new_tools WHERE connection_id = ?1",
                    params![connection_id],
                )?;
                tx.commit()?;
            }
            return Ok(Vec::new());
        };
        let pending: i64 = tx.query_row(
            "SELECT COUNT(*) FROM new_tools WHERE connection_id = ?1",
            params![connection_id],
            |r| r.get(0),
        )?;
        let mut room = (MAX_NEW_TOOLS - pending).max(0);
        let mut added = Vec::new();
        for name in listed.iter().filter(|n| !known.contains(n)) {
            if room == 0 {
                break;
            }
            let inserted = tx.execute(
                "INSERT OR IGNORE INTO new_tools (connection_id, name, seen_at) VALUES (?1, ?2, ?3)",
                params![connection_id, name, now],
            )?;
            if inserted > 0 {
                room -= 1;
                added.push(name.clone());
            }
        }
        tx.commit()?;
        Ok(added)
    }

    /// The connection's tools awaiting the owner's review, by name.
    pub fn new_tools(&self, connection_id: &str) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT name FROM new_tools WHERE connection_id = ?1 ORDER BY name")?;
        let rows = stmt.query_map(params![connection_id], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// The owner reviewed the connection's tools (saved its tool list): every
    /// pending new tool becomes known, on or off as the owner left it. Enables
    /// nothing by itself.
    pub fn review_tools(&mut self, connection_id: &str) -> Result<()> {
        let tx = self.conn.transaction()?;
        let known: Option<String> = tx
            .query_row(
                "SELECT known_tools FROM connections WHERE id = ?1",
                params![connection_id],
                |r| r.get(0),
            )
            .optional()?
            .flatten();
        let mut known: Vec<String> = known
            .map(|s| serde_json::from_str(&s).unwrap_or_default())
            .unwrap_or_default();
        {
            let mut stmt = tx.prepare("SELECT name FROM new_tools WHERE connection_id = ?1")?;
            for name in stmt.query_map(params![connection_id], |r| r.get::<_, String>(0))? {
                known.push(name?);
            }
        }
        known.sort();
        known.dedup();
        tx.execute(
            "UPDATE connections SET known_tools = ?2 WHERE id = ?1",
            params![connection_id, json_list(&known)],
        )?;
        tx.execute(
            "DELETE FROM new_tools WHERE connection_id = ?1",
            params![connection_id],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// A disconnect (§13): the row becomes the `revoked` tombstone and every
    /// ciphertext it had is deleted, in one transaction, so there is no
    /// moment where a revoked connection still holds a credential.
    pub fn revoke_connection(&mut self, id: &str, now: &str) -> Result<bool> {
        let tx = self.conn.transaction()?;
        let changed = tx.execute(
            "UPDATE connections SET status = 'revoked', updated_at = ?2
             WHERE id = ?1 AND status != 'revoked'",
            params![id, now],
        )?;
        tx.execute(
            "DELETE FROM credentials WHERE connection_id = ?1",
            params![id],
        )?;
        tx.execute(
            "DELETE FROM new_tools WHERE connection_id = ?1",
            params![id],
        )?;
        tx.commit()?;
        Ok(changed > 0)
    }

    // ---- credentials ----------------------------------------------------

    pub fn put_credential(
        &self,
        connection_id: &str,
        kind: &str,
        sealed: &Sealed,
        expires_at: Option<&str>,
        now: &str,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO credentials
                 (connection_id, kind, key_id, nonce, ciphertext, expires_at, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)
             ON CONFLICT(connection_id, kind) DO UPDATE SET
                 key_id = excluded.key_id, nonce = excluded.nonce,
                 ciphertext = excluded.ciphertext, expires_at = excluded.expires_at,
                 updated_at = excluded.updated_at",
            params![
                connection_id,
                kind,
                sealed.key_id,
                sealed.nonce,
                sealed.ciphertext,
                expires_at,
                now
            ],
        )?;
        Ok(())
    }

    /// Whether the connection holds any credential, of any kind.
    pub fn has_credential(&self, connection_id: &str) -> Result<bool> {
        Ok(self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM credentials WHERE connection_id = ?1)",
            params![connection_id],
            |r| r.get(0),
        )?)
    }

    /// A credential and its expiry, still sealed.
    pub fn credential(
        &self,
        connection_id: &str,
        kind: &str,
    ) -> Result<Option<(Sealed, Option<String>)>> {
        Ok(self
            .conn
            .query_row(
                "SELECT key_id, nonce, ciphertext, expires_at FROM credentials
                 WHERE connection_id = ?1 AND kind = ?2",
                params![connection_id, kind],
                |r| {
                    Ok((
                        Sealed {
                            key_id: r.get(0)?,
                            nonce: r.get(1)?,
                            ciphertext: r.get(2)?,
                        },
                        r.get(3)?,
                    ))
                },
            )
            .optional()?)
    }

    /// Deletes every credential of a connection; how many went. A
    /// disconnect does this inside its own transaction (`revoke_connection`).
    #[cfg(test)]
    pub fn delete_credentials(&self, connection_id: &str) -> Result<usize> {
        Ok(self.conn.execute(
            "DELETE FROM credentials WHERE connection_id = ?1",
            params![connection_id],
        )?)
    }

    /// Every `(connection, key_id)` a credential is sealed under. At boot
    /// after a lost key (§8), the ones no loaded key can open need
    /// reconnecting.
    pub fn credential_keys(&self) -> Result<Vec<(String, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT connection_id, key_id FROM credentials ORDER BY connection_id",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Marks connections `needs_reauth`, leaving revoked and disabled ones as
    /// they are: a lost key does not resurrect a disconnected connection.
    pub fn mark_needs_reauth(&self, connection_ids: &[String], now: &str) -> Result<usize> {
        let mut changed = 0;
        for id in connection_ids {
            changed += self.conn.execute(
                "UPDATE connections SET status = 'needs_reauth', updated_at = ?2
                 WHERE id = ?1 AND status NOT IN ('revoked', 'disabled')",
                params![id, now],
            )?;
        }
        Ok(changed)
    }

    // ---- the call audit -------------------------------------------------

    pub fn record_call(&self, c: &CallRecord) -> Result<()> {
        self.conn.execute(
            "INSERT INTO calls (at_ms, owner_user_id, connection_id, session_id, host_id,
                 method, tool, outcome, error_code, duration_ms, response_bytes)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                c.at_ms,
                c.owner_user_id,
                c.connection_id,
                c.session_id,
                c.host_id,
                c.method,
                c.tool,
                c.outcome,
                c.error_code,
                c.duration_ms,
                c.response_bytes
            ],
        )?;
        Ok(())
    }

    /// Applies the retention: rows older than [`CALLS_KEEP_MS`] go, then all
    /// but the newest `keep_rows`. Returns how many were removed.
    pub fn prune_calls(&self, now_ms: i64, keep_rows: i64) -> Result<usize> {
        let old = self.conn.execute(
            "DELETE FROM calls WHERE at_ms < ?1",
            params![now_ms - CALLS_KEEP_MS],
        )?;
        // Ids only grow, so "all but the newest N" is everything at or below
        // the (N+1)th newest id: a walk of the key, not an N-row `NOT IN` set.
        let excess = self.conn.execute(
            "DELETE FROM calls WHERE id <=
                 (SELECT id FROM calls ORDER BY id DESC LIMIT 1 OFFSET ?1)",
            params![keep_rows],
        )?;
        Ok(old + excess)
    }

    #[cfg(test)]
    pub fn recent_calls(&self, limit: i64) -> Result<Vec<CallRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT at_ms, owner_user_id, connection_id, session_id, host_id, method, tool,
                    outcome, error_code, duration_ms, response_bytes
             FROM calls ORDER BY id DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit], |r| {
            Ok(CallRecord {
                at_ms: r.get(0)?,
                owner_user_id: r.get(1)?,
                connection_id: r.get(2)?,
                session_id: r.get(3)?,
                host_id: r.get(4)?,
                method: r.get(5)?,
                tool: r.get(6)?,
                outcome: r.get(7)?,
                error_code: r.get(8)?,
                duration_ms: r.get(9)?,
                response_bytes: r.get(10)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    // ---- OAuth (decision 81g) --------------------------------------------

    /// Deletes one credential of a connection; whether it existed.
    pub fn delete_credential(&self, connection_id: &str, kind: &str) -> Result<bool> {
        Ok(self.conn.execute(
            "DELETE FROM credentials WHERE connection_id = ?1 AND kind = ?2",
            params![connection_id, kind],
        )? > 0)
    }

    pub fn insert_oauth_client(&self, c: &OAuthClientRow) -> Result<()> {
        self.conn.execute(
            "INSERT INTO oauth_clients (id, owner_user_id, issuer, client_id, redirect_uri,
                 registered, metadata, secret_key_id, secret_nonce, secret_ciphertext, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
             ON CONFLICT(owner_user_id, issuer, redirect_uri) DO UPDATE SET
                 id = excluded.id, client_id = excluded.client_id,
                 registered = excluded.registered, metadata = excluded.metadata,
                 secret_key_id = excluded.secret_key_id, secret_nonce = excluded.secret_nonce,
                 secret_ciphertext = excluded.secret_ciphertext",
            params![
                c.id,
                c.owner_user_id,
                c.issuer,
                c.client_id,
                c.redirect_uri,
                c.registered,
                c.metadata,
                c.secret.as_ref().map(|s| s.key_id.clone()),
                c.secret.as_ref().map(|s| s.nonce.clone()),
                c.secret.as_ref().map(|s| s.ciphertext.clone()),
                c.created_at
            ],
        )?;
        Ok(())
    }

    fn oauth_client_where(
        &self,
        clause: &str,
        args: &[&dyn rusqlite::ToSql],
    ) -> Result<Option<OAuthClientRow>> {
        Ok(self
            .conn
            .query_row(
                &format!(
                    "SELECT id, owner_user_id, issuer, client_id, redirect_uri, registered, metadata,
                            secret_key_id, secret_nonce, secret_ciphertext, created_at
                     FROM oauth_clients WHERE {clause}"
                ),
                args,
                |r| {
                    let key: Option<String> = r.get(7)?;
                    Ok(OAuthClientRow {
                        id: r.get(0)?,
                        owner_user_id: r.get(1)?,
                        issuer: r.get(2)?,
                        client_id: r.get(3)?,
                        redirect_uri: r.get(4)?,
                        registered: r.get(5)?,
                        metadata: r.get(6)?,
                        secret: match key {
                            Some(key_id) => Some(Sealed {
                                key_id,
                                nonce: r.get(8)?,
                                ciphertext: r.get(9)?,
                            }),
                            None => None,
                        },
                        created_at: r.get(10)?,
                    })
                },
            )
            .optional()?)
    }

    pub fn oauth_client(&self, id: &str) -> Result<Option<OAuthClientRow>> {
        self.oauth_client_where("id = ?1", &[&id])
    }

    pub fn oauth_client_for(
        &self,
        owner: &str,
        issuer: &str,
        redirect_uri: &str,
    ) -> Result<Option<OAuthClientRow>> {
        self.oauth_client_where(
            "owner_user_id = ?1 AND issuer = ?2 AND redirect_uri = ?3",
            &[&owner, &issuer, &redirect_uri],
        )
    }

    pub fn set_connection_oauth(&self, connection_id: &str, client: &str, now: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO connection_oauth (connection_id, oauth_client, updated_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(connection_id) DO UPDATE SET oauth_client = excluded.oauth_client,
                 updated_at = excluded.updated_at",
            params![connection_id, client, now],
        )?;
        Ok(())
    }

    pub fn connection_oauth(&self, connection_id: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT oauth_client FROM connection_oauth WHERE connection_id = ?1",
                params![connection_id],
                |r| r.get(0),
            )
            .optional()?)
    }

    pub fn insert_flow(&self, f: &FlowRow) -> Result<()> {
        self.conn.execute(
            "INSERT INTO oauth_flows (state_hash, connection_id, owner_user_id, oauth_client,
                 redirect_uri, resource, verifier_key_id, verifier_nonce, verifier_ciphertext,
                 created_at, expires_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                f.state_hash,
                f.connection_id,
                f.owner_user_id,
                f.oauth_client,
                f.redirect_uri,
                f.resource,
                f.sealed.key_id,
                f.sealed.nonce,
                f.sealed.ciphertext,
                f.created_at,
                f.expires_at
            ],
        )?;
        Ok(())
    }

    const FLOW_COLUMNS: &'static str = "state_hash, connection_id, owner_user_id, oauth_client, \
        redirect_uri, resource, verifier_key_id, verifier_nonce, verifier_ciphertext, created_at, \
        expires_at";

    fn flow_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<FlowRow> {
        Ok(FlowRow {
            state_hash: r.get(0)?,
            connection_id: r.get(1)?,
            owner_user_id: r.get(2)?,
            oauth_client: r.get(3)?,
            redirect_uri: r.get(4)?,
            resource: r.get(5)?,
            sealed: Sealed {
                key_id: r.get(6)?,
                nonce: r.get(7)?,
                ciphertext: r.get(8)?,
            },
            created_at: r.get(9)?,
            expires_at: r.get(10)?,
        })
    }

    /// A live flow (unused, unexpired), without claiming it.
    pub fn live_flow(&self, state_hash: &str, now: &str) -> Result<Option<FlowRow>> {
        Ok(self
            .conn
            .query_row(
                &format!(
                    "SELECT {} FROM oauth_flows
                     WHERE state_hash = ?1 AND used_at IS NULL AND expires_at > ?2",
                    Self::FLOW_COLUMNS
                ),
                params![state_hash, now],
                Self::flow_from_row,
            )
            .optional()?)
    }

    /// Marks a flow used and returns it, **only if** it was unused and
    /// unexpired: one `UPDATE … WHERE used_at IS NULL`, so of two racing
    /// callers exactly one gets the row.
    pub fn claim_flow(&self, state_hash: &str, now: &str) -> Result<Option<FlowRow>> {
        let changed = self.conn.execute(
            "UPDATE oauth_flows SET used_at = ?2
             WHERE state_hash = ?1 AND used_at IS NULL AND expires_at > ?2",
            params![state_hash, now],
        )?;
        if changed != 1 {
            return Ok(None);
        }
        Ok(self
            .conn
            .query_row(
                &format!(
                    "SELECT {} FROM oauth_flows WHERE state_hash = ?1",
                    Self::FLOW_COLUMNS
                ),
                params![state_hash],
                Self::flow_from_row,
            )
            .optional()?)
    }

    #[cfg(test)]
    pub(crate) fn conn(&self) -> &Sqlite {
        &self.conn
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db(dir: &Path) -> GatewayDb {
        GatewayDb::open(&dir.join("gateway.db")).unwrap()
    }

    fn call(at_ms: i64) -> CallRecord {
        CallRecord {
            at_ms,
            owner_user_id: "usr_A".into(),
            connection_id: "mcc_A".into(),
            session_id: Some("ags_A".into()),
            host_id: Some("hst_A".into()),
            method: "tools/call".into(),
            tool: Some("search".into()),
            outcome: "ok".into(),
            error_code: None,
            duration_ms: Some(12),
            response_bytes: Some(345),
        }
    }

    #[test]
    fn the_schema_has_the_five_tables_and_reopening_is_harmless() {
        let dir = tempdir::TempDir::new("storm-gw-schema").unwrap();
        let first = db(dir.path());
        first.set_meta("active_key_id", "gwk_X").unwrap();
        drop(first);
        // Additive only: opening an existing database must neither fail nor
        // touch what is in it.
        let again = db(dir.path());
        assert_eq!(
            again.meta("active_key_id").unwrap().as_deref(),
            Some("gwk_X")
        );
        let tables: Vec<String> = again
            .conn()
            .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        for t in [
            "calls",
            "connections",
            "credentials",
            "meta",
            "new_tools",
            "oauth_clients",
            "oauth_flows",
        ] {
            assert!(tables.contains(&t.to_string()), "missing table {t}");
        }
    }

    #[test]
    fn every_schema_statement_is_additive() {
        // A `DROP`, an `ALTER` or a bare `CREATE TABLE` in this batch would
        // rewrite a gateway someone already connected integrations on.
        let upper = SCHEMA.to_uppercase();
        assert!(!upper.contains("DROP "));
        assert!(!upper.contains("ALTER "));
        for line in upper.lines().map(str::trim) {
            if line.starts_with("CREATE ") {
                assert!(line.contains("IF NOT EXISTS"), "not additive: {line}");
            }
        }
    }

    #[test]
    fn a_credential_round_trips_sealed_and_is_replaced_in_place() {
        let dir = tempdir::TempDir::new("storm-gw-cred").unwrap();
        let d = db(dir.path());
        let s1 = Sealed {
            key_id: "gwk_A".into(),
            nonce: vec![1; 24],
            ciphertext: vec![2; 40],
        };
        d.put_credential("mcc_A", "static", &s1, None, "t1")
            .unwrap();
        let s2 = Sealed {
            key_id: "gwk_A".into(),
            nonce: vec![3; 24],
            ciphertext: vec![4; 40],
        };
        d.put_credential("mcc_A", "static", &s2, Some("t9"), "t2")
            .unwrap();
        let (got, expiry) = d.credential("mcc_A", "static").unwrap().unwrap();
        assert_eq!(got, s2);
        assert_eq!(expiry.as_deref(), Some("t9"));
        assert!(d.credential("mcc_A", "oauth_tokens").unwrap().is_none());
        assert_eq!(d.delete_credentials("mcc_A").unwrap(), 1);
        assert!(d.credential("mcc_A", "static").unwrap().is_none());
    }

    #[test]
    fn a_lost_key_finds_exactly_the_connections_it_sealed() {
        let dir = tempdir::TempDir::new("storm-gw-lostkey").unwrap();
        let d = db(dir.path());
        for (conn, key) in [("mcc_A", "gwk_OLD"), ("mcc_B", "gwk_NEW")] {
            d.conn()
                .execute(
                    "INSERT INTO connections (id, owner_user_id, slug, display_name, url,
                         auth_kind, status, created_at, updated_at)
                     VALUES (?1, 'usr_A', ?1, ?1, 'https://x', 'static', 'connected', 't', 't')",
                    params![conn],
                )
                .unwrap();
            let s = Sealed {
                key_id: key.into(),
                nonce: vec![0; 24],
                ciphertext: vec![0; 17],
            };
            d.put_credential(conn, "static", &s, None, "t").unwrap();
        }
        let lost: Vec<String> = d
            .credential_keys()
            .unwrap()
            .into_iter()
            .filter(|(_, key)| key != "gwk_NEW")
            .map(|(conn, _)| conn)
            .collect();
        assert_eq!(lost, vec!["mcc_A".to_string()]);
        assert_eq!(d.mark_needs_reauth(&lost, "t2").unwrap(), 1);
        let status: String = d
            .conn()
            .query_row(
                "SELECT status FROM connections WHERE id = 'mcc_A'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(status, "needs_reauth");
    }

    #[test]
    fn the_call_audit_keeps_thirty_days_and_a_row_cap() {
        let dir = tempdir::TempDir::new("storm-gw-calls").unwrap();
        let d = db(dir.path());
        let now = 100 * CALLS_KEEP_MS;
        d.record_call(&call(now - CALLS_KEEP_MS - 1)).unwrap(); // too old
        for i in 0..5 {
            d.record_call(&call(now - i)).unwrap();
        }
        // Age first, then the cap: 5 recent rows, keep 3.
        assert_eq!(d.prune_calls(now, 3).unwrap(), 3);
        let left = d.recent_calls(10).unwrap();
        assert_eq!(left.len(), 3);
        assert!(left.iter().all(|c| c.at_ms > now - CALLS_KEEP_MS));
        // Newest first, and the newest survived.
        assert_eq!(left[0].at_ms, now - 4);
    }

    #[test]
    fn a_slug_is_unique_per_owner_until_its_connection_is_revoked() {
        let dir = tempdir::TempDir::new("storm-gw-slug").unwrap();
        let d = db(dir.path());
        let insert = |id: &str, owner: &str, status: &str| {
            d.conn().execute(
                "INSERT INTO connections (id, owner_user_id, slug, display_name, url,
                     auth_kind, status, created_at, updated_at)
                 VALUES (?1, ?2, 'notion', 'Notion', 'https://x', 'oauth', ?3, 't', 't')",
                params![id, owner, status],
            )
        };
        insert("mcc_1", "usr_A", "connected").unwrap();
        assert!(insert("mcc_2", "usr_A", "connected").is_err());
        insert("mcc_3", "usr_B", "connected").unwrap();
        d.conn()
            .execute(
                "UPDATE connections SET status = 'revoked' WHERE id = 'mcc_1'",
                [],
            )
            .unwrap();
        insert("mcc_4", "usr_A", "pending_auth").unwrap();
    }

    // ---- new upstream tools (G-D16, spec §9, decision 81k) -----------------

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn with_connection(dir: &Path) -> (GatewayDb, String) {
        let mut d = db(dir);
        let c = crate::gateway::connections::tests::connection();
        d.insert_connection(&c, None).unwrap();
        (d, c.id)
    }

    #[test]
    fn the_first_owner_test_turns_everything_on_and_later_tools_stay_off_until_reviewed() {
        let dir = tempdir::TempDir::new("storm-gw-newtools").unwrap();
        let (mut d, id) = with_connection(dir.path());
        assert!(
            d.observe_tools(&id, &names(&["b", "a"]), true, "t")
                .unwrap()
                .is_empty()
        );
        let c = d.connection(&id).unwrap().unwrap();
        assert_eq!(c.tool_allowlist, names(&["a", "b"]));
        assert_eq!(c.known_tools, Some(names(&["a", "b"])));

        // A later listing names `c`: recorded as new, and off.
        let added = d
            .observe_tools(&id, &names(&["a", "b", "c"]), true, "t")
            .unwrap();
        assert_eq!(added, names(&["c"]));
        assert_eq!(d.new_tools(&id).unwrap(), names(&["c"]));
        let c = d.connection(&id).unwrap().unwrap();
        assert_eq!(
            c.tool_allowlist,
            names(&["a", "b"]),
            "a new tool was enabled"
        );
        // Seen again: still one pending entry, not reported twice.
        assert!(
            d.observe_tools(&id, &names(&["c"]), false, "t")
                .unwrap()
                .is_empty()
        );
        assert_eq!(d.new_tools(&id).unwrap(), names(&["c"]));
    }

    #[test]
    fn an_agents_listing_never_sets_the_baseline() {
        // Before the owner's first test an agent's listing records nothing,
        // so nothing an agent does can turn a tool on.
        let dir = tempdir::TempDir::new("storm-gw-newtools-agent").unwrap();
        let (mut d, id) = with_connection(dir.path());
        assert!(
            d.observe_tools(&id, &names(&["a", "b"]), false, "t")
                .unwrap()
                .is_empty()
        );
        let c = d.connection(&id).unwrap().unwrap();
        assert!(c.tool_allowlist.is_empty());
        assert_eq!(c.known_tools, None);
        assert!(d.new_tools(&id).unwrap().is_empty());
    }

    #[test]
    fn a_review_makes_new_tools_known_without_enabling_them() {
        let dir = tempdir::TempDir::new("storm-gw-newtools-review").unwrap();
        let (mut d, id) = with_connection(dir.path());
        d.observe_tools(&id, &names(&["a"]), true, "t").unwrap();
        d.observe_tools(&id, &names(&["a", "b", "c"]), false, "t")
            .unwrap();
        d.review_tools(&id).unwrap();
        assert!(d.new_tools(&id).unwrap().is_empty());
        let c = d.connection(&id).unwrap().unwrap();
        assert_eq!(c.known_tools, Some(names(&["a", "b", "c"])));
        assert_eq!(
            c.tool_allowlist,
            names(&["a"]),
            "a review enabled something"
        );
        // Reviewed tools are no longer new, even if they vanish and return.
        assert!(
            d.observe_tools(&id, &names(&["a", "b"]), false, "t")
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn an_older_copy_of_the_row_cannot_undo_a_review_or_drop_a_new_tool() {
        let dir = tempdir::TempDir::new("storm-gw-newtools-stale").unwrap();
        let (mut d, id) = with_connection(dir.path());
        d.observe_tools(&id, &names(&["a"]), true, "t").unwrap();
        let stale = d.connection(&id).unwrap().unwrap();
        d.observe_tools(&id, &names(&["a", "b"]), false, "t")
            .unwrap();
        d.update_connection(&stale).unwrap();
        assert_eq!(d.new_tools(&id).unwrap(), names(&["b"]));
        d.review_tools(&id).unwrap();
        d.update_connection(&stale).unwrap();
        let c = d.connection(&id).unwrap().unwrap();
        assert_eq!(c.known_tools, Some(names(&["a", "b"])));
    }

    #[test]
    fn invalid_names_are_never_recorded_and_the_pending_list_is_bounded() {
        let dir = tempdir::TempDir::new("storm-gw-newtools-bound").unwrap();
        let (mut d, id) = with_connection(dir.path());
        d.observe_tools(&id, &names(&["a"]), true, "t").unwrap();
        let long = "x".repeat(129);
        let added = d
            .observe_tools(
                &id,
                &[String::new(), long, "bell\u{7}".into(), "ok".into()],
                false,
                "t",
            )
            .unwrap();
        assert_eq!(added, names(&["ok"]));
        let flood: Vec<String> = (0..500).map(|i| format!("t{i:03}")).collect();
        d.observe_tools(&id, &flood, false, "t").unwrap();
        assert_eq!(d.new_tools(&id).unwrap().len() as i64, MAX_NEW_TOOLS);
    }

    #[test]
    fn a_disconnect_forgets_the_pending_tools() {
        let dir = tempdir::TempDir::new("storm-gw-newtools-revoke").unwrap();
        let (mut d, id) = with_connection(dir.path());
        d.observe_tools(&id, &names(&["a"]), true, "t").unwrap();
        d.observe_tools(&id, &names(&["a", "b"]), false, "t")
            .unwrap();
        d.revoke_connection(&id, "t2").unwrap();
        assert!(d.new_tools(&id).unwrap().is_empty());
    }
}
