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
        scope               TEXT,
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

#[derive(Debug, thiserror::Error)]
pub enum InsertError {
    #[error("that slug is already used by another of your integrations")]
    SlugTaken,
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

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

    /// Writes back the mutable fields. `id`, `owner_user_id`, `slug`, `url`
    /// and `auth_kind` are never rewritten: a credential is presented only to
    /// its own upstream (AM24), so a connection cannot be re-pointed, and the
    /// slug is what live sessions were told.
    pub fn update_connection(&self, c: &Connection) -> Result<()> {
        self.conn.execute(
            "UPDATE connections SET display_name = ?2, status = ?3, tool_allowlist = ?4,
                 known_tools = ?5, expose_resources = ?6, expose_prompts = ?7,
                 upstream_account_label = ?8, last_ok = ?9, last_error_code = ?10,
                 updated_at = ?11
             WHERE id = ?1",
            params![
                c.id,
                c.display_name,
                c.status,
                json_list(&c.tool_allowlist),
                c.known_tools.as_deref().map(json_list),
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
        let excess = self.conn.execute(
            "DELETE FROM calls WHERE id NOT IN
                 (SELECT id FROM calls ORDER BY id DESC LIMIT ?1)",
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
}
