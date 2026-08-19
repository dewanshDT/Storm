//! `state/auth.db` — who this server is, and later who may use it.
//!
//! **This is the first thing in `state/` that cannot be rebuilt.** A vault
//! index is derived: delete it and the next boot rescans the markdown and
//! regenerates every row. Nothing regenerates a server identity, a user or a
//! session, so this database is backup-critical in a way `index.db` is not —
//! `backup_all()` in `main.rs` snapshots it for exactly that reason, and a
//! restore that omitted it would produce a server holding every note with
//! nobody able to log in.
//!
//! Server-scoped rather than per vault: a user is a property of the server, and
//! identity living inside a vault's index would vanish when that index is
//! rebuilt (see [`crate::auth`]).
//!
//! Every table in the design is created now even though this slice only writes
//! `server` and `server_credentials`. They are `CREATE TABLE IF NOT EXISTS`
//! additions costing nothing, and creating them later would be a migration.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};

/// The database's name inside the state directory. Used by the backup path too,
/// so the snapshot lands under the same name and a restore is a plain copy.
pub const AUTH_DB_FILE: &str = "auth.db";

/// Bumped only for schema changes that *alter* existing structure.
///
/// Same rule as the index's `SCHEMA_VERSION`. When this needs its first real
/// migration, build the old schema by hand in the test and migrate it: opening
/// a fresh database runs the create path and never exercises the upgrade.
///
/// **1** — the original schema (slice 1).
/// **2** — `sessions.previous_refresh_hash` (slice 3).
/// **3** — `ws_tickets` (slice 4).
/// **4** — `pairing_sessions.peer_ip` + the `web_bootstrap` purpose (slice 15).
/// **5** — `api_keys` (A14).
const SCHEMA_VERSION: i64 = 5;

pub struct AuthDb {
    /// Visible to the rest of `auth` so each area keeps its own SQL beside its
    /// own rules — users in `users.rs`, identity here — rather than growing one
    /// module that knows every table.
    pub(super) conn: Connection,
}

/// The one row in `server`.
#[derive(Debug, Clone)]
pub struct ServerRow {
    pub id: String,
    pub name: String,
    pub created: String,
}

/// A server credential's *public* half. The private bytes are a file — see
/// [`crate::auth::identity`].
#[derive(Debug, Clone)]
pub struct CredentialRow {
    pub key_id: String,
    pub algorithm: String,
    pub public_key: Vec<u8>,
    pub created: String,
}

impl AuthDb {
    /// Opens (creating if absent) `<state_dir>/auth.db`.
    pub fn open(state_dir: &Path) -> Result<Self> {
        Self::open_at(&Self::path_in(state_dir))
    }

    pub fn path_in(state_dir: &Path) -> PathBuf {
        state_dir.join(AUTH_DB_FILE)
    }

    pub fn open_at(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating state dir {}", parent.display()))?;
        }
        let conn = Connection::open(path)
            .with_context(|| format!("opening auth database {}", path.display()))?;
        Self::from_connection(conn)
    }

    /// A throwaway database that touches no disk. Tests only.
    #[cfg(test)]
    pub fn open_in_memory() -> Result<Self> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    fn from_connection(conn: Connection) -> Result<Self> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let db = Self { conn };
        db.migrate()?;
        Ok(db)
    }

    /// The v3→v4 `pairing_sessions` rebuild, in one transaction.
    ///
    /// Called with `foreign_keys` already OFF — see the caller for why that
    /// cannot happen in here. The transaction is what makes a failed attempt
    /// leave nothing behind, so the migration can simply be retried on the
    /// next boot.
    ///
    /// **The column list must match the `CREATE TABLE` above field for field.**
    /// The first version of this dropped `consumed_by` — it was in the fresh
    /// schema and not in the rebuilt one — so a migrated server pairing a
    /// device failed in `mark_pairing_consumed` with `no such column`, *after*
    /// `create_paired` had already written the device row. Fresh installs
    /// worked and upgraded ones did not, which is the hardest shape of this
    /// bug to notice.
    fn rebuild_pairing_sessions_v4(&self) -> Result<()> {
        self.conn.execute_batch(
            r#"
            BEGIN;
            -- A previous attempt that died mid-rebuild leaves this behind.
            DROP TABLE IF EXISTS pairing_sessions_new;
            CREATE TABLE pairing_sessions_new (
              id           TEXT PRIMARY KEY,
              nonce_hash   BLOB NOT NULL UNIQUE,
              purpose      TEXT NOT NULL CHECK (purpose IN ('first_user','add_device','web_bootstrap')),
              peer_ip      TEXT,
              created_by   TEXT REFERENCES users(id),
              created      TEXT NOT NULL,
              expires      TEXT NOT NULL,
              consumed     TEXT,
              consumed_by  TEXT REFERENCES client_devices(id),
              attempts     INTEGER NOT NULL DEFAULT 0
            );
            INSERT INTO pairing_sessions_new
              (id, nonce_hash, purpose, peer_ip, created_by, created, expires, consumed, consumed_by, attempts)
            SELECT id, nonce_hash, purpose, NULL, created_by, created, expires, consumed, consumed_by, attempts
              FROM pairing_sessions;
            DROP TABLE pairing_sessions;
            ALTER TABLE pairing_sessions_new RENAME TO pairing_sessions;
            COMMIT;
            "#,
        )?;
        Ok(())
    }

    fn column_exists(&self, table: &str, column: &str) -> Result<bool> {
        let mut stmt = self.conn.prepare(&format!("PRAGMA table_info({table})"))?;
        let exists = stmt
            .query_map([], |r| r.get::<_, String>(1))?
            .filter_map(Result::ok)
            .any(|name| name == column);
        Ok(exists)
    }

    fn migrate(&self) -> Result<()> {
        self.conn.execute_batch(
            r#"
            -- One row, always. The CHECK is the enforcement: SQLite has no
            -- other way to say "at most one".
            CREATE TABLE IF NOT EXISTS server (
                only_row  INTEGER PRIMARY KEY CHECK (only_row = 1),
                id        TEXT NOT NULL,
                name      TEXT NOT NULL,
                created   TEXT NOT NULL
            );

            -- Credentials rotate; the server's identity does not (A3). Only the
            -- public half is here — the private bytes are a 0600 file, so their
            -- protection is auditable with `ls -l` and a database dump never
            -- contains a usable secret (A2).
            CREATE TABLE IF NOT EXISTS server_credentials (
                key_id          TEXT PRIMARY KEY,
                algorithm       TEXT NOT NULL,
                public_key      BLOB NOT NULL,
                created         TEXT NOT NULL,
                activated       TEXT,
                retired         TEXT,   -- superseded, still verifiable
                revoked         TEXT,   -- compromised, never trust again
                revoked_reason  TEXT
            );

            CREATE TABLE IF NOT EXISTS users (
                id             TEXT PRIMARY KEY,
                username       TEXT NOT NULL,
                username_fold  TEXT NOT NULL UNIQUE,
                display_name   TEXT,
                password_hash  TEXT NOT NULL,
                role           TEXT NOT NULL CHECK (role IN ('owner','admin','member')),
                status         TEXT NOT NULL CHECK (status IN ('active','disabled')),
                created        TEXT NOT NULL,
                updated        TEXT NOT NULL,
                last_login     TEXT,
                failed_count   INTEGER NOT NULL DEFAULT 0,
                locked_until   TEXT
            );

            -- An app installation, not a person. Deliberately not `devices`:
            -- the per-vault index already has one holding client-chosen sync
            -- ids, which are self-asserted and a different thing entirely.
            CREATE TABLE IF NOT EXISTS client_devices (
                id             TEXT PRIMARY KEY,
                name           TEXT NOT NULL,
                platform       TEXT,
                client_version TEXT,
                secret_hash    BLOB NOT NULL,
                paired         TEXT NOT NULL,
                paired_via     TEXT REFERENCES pairing_sessions(id),
                last_seen      TEXT,
                revoked        TEXT,
                revoked_reason TEXT
            );

            CREATE TABLE IF NOT EXISTS sessions (
                id                  TEXT PRIMARY KEY,
                user_id             TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                device_id           TEXT NOT NULL REFERENCES client_devices(id) ON DELETE CASCADE,
                access_hash         BLOB NOT NULL UNIQUE,
                refresh_hash        BLOB NOT NULL UNIQUE,
                previous_refresh_hash BLOB,
                created             TEXT NOT NULL,
                expires             TEXT NOT NULL,
                refresh_expires     TEXT NOT NULL,
                last_used           TEXT,
                revoked             TEXT,
                revoked_reason      TEXT
            );
            CREATE INDEX IF NOT EXISTS sessions_by_user   ON sessions(user_id);
            CREATE INDEX IF NOT EXISTS sessions_by_device ON sessions(device_id);

            CREATE TABLE IF NOT EXISTS pairing_sessions (
                id           TEXT PRIMARY KEY,
                nonce_hash   BLOB NOT NULL UNIQUE,
                purpose      TEXT NOT NULL CHECK (purpose IN ('first_user','add_device','web_bootstrap')),
                -- The peer that was issued this nonce, for web_bootstrap only.
                -- NULL for the QR purposes, which are carried by a human.
                peer_ip      TEXT,
                created_by   TEXT REFERENCES users(id),
                created      TEXT NOT NULL,
                expires      TEXT NOT NULL,
                consumed     TEXT,
                consumed_by  TEXT REFERENCES client_devices(id),
                attempts     INTEGER NOT NULL DEFAULT 0
            );

            -- No foreign key on vault_id: vaults live in vaults.json, not here.
            CREATE TABLE IF NOT EXISTS vault_grants (
                user_id    TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                vault_id   TEXT NOT NULL,
                access     TEXT NOT NULL CHECK (access IN ('read','write')),
                granted    TEXT NOT NULL,
                granted_by TEXT REFERENCES users(id),
                PRIMARY KEY (user_id, vault_id)
            );

            CREATE TABLE IF NOT EXISTS security_events (
                seq       INTEGER PRIMARY KEY AUTOINCREMENT,
                at        TEXT NOT NULL,
                kind      TEXT NOT NULL,
                user_id   TEXT,
                device_id TEXT,
                remote    TEXT,
                detail    TEXT   -- JSON. Never a secret, never a token.
            );

            -- MCP keys (A14): a credential a user mints for a machine.
            --
            -- **The key belongs to the user, and `ON DELETE CASCADE` is the
            -- data-model half of saying so** — deleting the account takes its
            -- keys with it, with no application code left to forget. The
            -- authority a key carries is its owner's; nothing is stored here
            -- about *what* it may reach, because that is the authorization
            -- model's question and it is deliberately not answered in A14.
            --
            -- `secret_hash` is blake3 of the whole `stk_…` string, never the
            -- plaintext (A5, A14.5). The plaintext exists once, in the response
            -- that created it.
            CREATE TABLE IF NOT EXISTS api_keys (
                id             TEXT PRIMARY KEY,
                user_id        TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                name           TEXT NOT NULL,
                secret_hash    BLOB NOT NULL UNIQUE,
                created        TEXT NOT NULL,
                created_via    TEXT REFERENCES client_devices(id),
                expires        TEXT,
                last_used      TEXT,
                revoked        TEXT,
                revoked_reason TEXT
            );
            CREATE INDEX IF NOT EXISTS api_keys_by_user ON api_keys(user_id);

            -- Short-lived, single-use tokens for WebSocket handshakes. The
            -- client POSTs to get one, then presents it on the GET /v1/stream
            -- handshake.
            CREATE TABLE IF NOT EXISTS ws_tickets (
                id          TEXT PRIMARY KEY,
                session_id  TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                access_hash BLOB NOT NULL UNIQUE,
                created     TEXT NOT NULL,
                expires     TEXT NOT NULL,
                used        TEXT
            );
            "#,
        )?;

        let current: i64 = self
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if current < SCHEMA_VERSION {
            // v1→v2: add previous_refresh_hash to sessions.
            if current < 2 && !self.column_exists("sessions", "previous_refresh_hash")? {
                self.conn
                    .execute_batch("ALTER TABLE sessions ADD COLUMN previous_refresh_hash BLOB;")?;
            }

            // v3→v4: pairing_sessions gains the `web_bootstrap` purpose and a
            // `peer_ip` binding. SQLite cannot alter a CHECK constraint, so
            // the table is rebuilt — copy, drop, rename — rather than patched.
            // Pairing sessions are short-lived by nature, so this carries very
            // little and could almost be a truncate; it copies anyway, because
            // "almost" is not a reason to drop a user's pending pairing.
            //
            // **This follows SQLite's documented rebuild procedure exactly, and
            // every part of it is load-bearing.** The obvious version — four
            // statements in one `execute_batch` — is wrong in two ways that
            // only appear on a database someone has actually used:
            //
            //   * `client_devices.paired_via` references this table and
            //     `foreign_keys` is ON, so `DROP TABLE` (an implicit
            //     `DELETE FROM`) is refused the moment any device has ever
            //     been paired. FK enforcement cannot be toggled inside a
            //     transaction, hence the pragma outside it.
            //   * `execute_batch` wraps nothing in a transaction, so that
            //     refusal leaves `pairing_sessions_new` behind with
            //     `user_version` still at 3 — and the *next* boot fails on
            //     "table already exists". A migration that cannot be retried
            //     turns one bad upgrade into a server that never starts again.
            //
            // `auth.db` is the one file in `state/` that cannot be rebuilt by
            // rescanning markdown, so a migration that can strand it is the
            // most expensive bug available here.
            if current < 4 && !self.column_exists("pairing_sessions", "peer_ip")? {
                // Outside the transaction: SQLite ignores this pragma inside one.
                self.conn.pragma_update(None, "foreign_keys", "OFF")?;
                let rebuilt = self.rebuild_pairing_sessions_v4();
                // Restored whether or not the rebuild worked — leaving FK
                // enforcement off would silently disarm every other table.
                self.conn.pragma_update(None, "foreign_keys", "ON")?;
                rebuilt?;
            }

            // v4→v5: `api_keys` (A14). **Deliberately nothing to do here.**
            // The table is created by the `CREATE TABLE IF NOT EXISTS` batch
            // above, which runs on every open, so a v4 database gains it
            // exactly as a fresh one does and this branch would be dead code.
            //
            // That is the point rather than an omission: a purely additive
            // change needs no rebuild, and a rebuild is what made v3→v4 able to
            // strand a database it could not then retry. Keep new tables in the
            // batch and out of here.

            self.conn
                .pragma_update(None, "user_version", SCHEMA_VERSION)?;
        }
        Ok(())
    }

    /// Writes a consistent snapshot, safe against a live WAL database.
    ///
    /// Shares the index's implementation so the "never overwrite a non-file"
    /// guard cannot drift between the two.
    pub fn snapshot_to(&self, dest: &Path) -> Result<()> {
        crate::db::snapshot_connection(&self.conn, dest)
    }

    // ---- server identity ------------------------------------------------

    pub fn server(&self) -> Result<Option<ServerRow>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id, name, created FROM server WHERE only_row = 1",
                [],
                |r| {
                    Ok(ServerRow {
                        id: r.get(0)?,
                        name: r.get(1)?,
                        created: r.get(2)?,
                    })
                },
            )
            .optional()?)
    }

    /// The credential in use: neither retired nor revoked.
    ///
    /// There is exactly one, which SQLite cannot express as a constraint —
    /// [`Self::active_credential_count`] is what the invariant test asserts on.
    pub fn active_credential(&self) -> Result<Option<CredentialRow>> {
        Ok(self
            .conn
            .query_row(
                "SELECT key_id, algorithm, public_key, created
                 FROM server_credentials
                 WHERE retired IS NULL AND revoked IS NULL
                 ORDER BY created DESC",
                [],
                |r| {
                    Ok(CredentialRow {
                        key_id: r.get(0)?,
                        algorithm: r.get(1)?,
                        public_key: r.get(2)?,
                        created: r.get(3)?,
                    })
                },
            )
            .optional()?)
    }

    pub fn active_credential_count(&self) -> Result<i64> {
        Ok(self.conn.query_row(
            "SELECT COUNT(*) FROM server_credentials
             WHERE retired IS NULL AND revoked IS NULL",
            [],
            |r| r.get(0),
        )?)
    }

    /// Appends to the audit trail.
    ///
    /// `detail` is JSON and **never contains a secret** — not a password, not a
    /// hash, not a token. The table exists so an operator can answer "what
    /// happened to this account"; a trail that leaks credentials would be worse
    /// than no trail. `user_id` deliberately has no foreign key, so the record
    /// of a deletion outlives the row it describes.
    pub fn record_event(
        &self,
        kind: &str,
        user_id: Option<&str>,
        device_id: Option<&str>,
        at: &str,
        detail: &str,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO security_events (at, kind, user_id, device_id, detail) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![at, kind, user_id, device_id, detail],
        )?;
        Ok(())
    }

    /// Records the server row and its first credential together.
    ///
    /// One transaction because a server with no credential cannot answer a
    /// challenge, and a credential with no server has nothing to be the
    /// identity *of*. A half-written identity is worse than none: the next boot
    /// would find the row, decide the identity exists, and fail to load a key.
    pub fn insert_identity(
        &mut self,
        server: &ServerRow,
        credential: &CredentialRow,
    ) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO server (only_row, id, name, created) VALUES (1, ?1, ?2, ?3)",
            params![server.id, server.name, server.created],
        )?;
        tx.execute(
            "INSERT INTO server_credentials
                 (key_id, algorithm, public_key, created, activated)
             VALUES (?1, ?2, ?3, ?4, ?4)",
            params![
                credential.key_id,
                credential.algorithm,
                credential.public_key,
                credential.created
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    // ---- api keys (A14) --------------------------------------------------

    #[allow(clippy::too_many_arguments)]
    pub fn insert_api_key(
        &self,
        id: &str,
        user_id: &str,
        name: &str,
        secret_hash: &[u8],
        created: &str,
        created_via: Option<&str>,
        expires: Option<&str>,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO api_keys
                 (id, user_id, name, secret_hash, created, created_via, expires)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                id,
                user_id,
                name,
                secret_hash,
                created,
                created_via,
                expires
            ],
        )?;
        Ok(())
    }

    /// Looks a key up by the hash of its plaintext.
    ///
    /// An indexed equality lookup, not a comparison over a secret — the same
    /// shape as `session_by_access_hash`, and for the same reason: there is no
    /// constant-time question here because no byte-by-byte comparison happens.
    pub fn api_key_by_hash(&self, secret_hash: &[u8]) -> Result<Option<crate::auth::keys::ApiKey>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id, user_id, name, created, created_via, expires,
                        last_used, revoked, revoked_reason
                 FROM api_keys WHERE secret_hash = ?1",
                params![secret_hash],
                Self::api_key_from_row,
            )
            .optional()?)
    }

    pub fn api_key_by_id(&self, id: &str) -> Result<Option<crate::auth::keys::ApiKey>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id, user_id, name, created, created_via, expires,
                        last_used, revoked, revoked_reason
                 FROM api_keys WHERE id = ?1",
                params![id],
                Self::api_key_from_row,
            )
            .optional()?)
    }

    /// Every key a user holds, newest first, revoked ones included.
    ///
    /// Revoked keys are returned on purpose: "this key was revoked" is what
    /// someone needs to see when a machine stops working, and hiding it turns a
    /// two-second answer into a mystery.
    pub fn api_keys_for_user(&self, user_id: &str) -> Result<Vec<crate::auth::keys::ApiKey>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, user_id, name, created, created_via, expires,
                    last_used, revoked, revoked_reason
             FROM api_keys WHERE user_id = ?1 ORDER BY created DESC",
        )?;
        let rows = stmt.query_map(params![user_id], Self::api_key_from_row)?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    /// Live (unrevoked) keys a user holds — the number the per-user cap bounds.
    pub fn live_api_key_count(&self, user_id: &str) -> Result<i64> {
        Ok(self.conn.query_row(
            "SELECT COUNT(*) FROM api_keys WHERE user_id = ?1 AND revoked IS NULL",
            params![user_id],
            |r| r.get(0),
        )?)
    }

    pub fn revoke_api_key(&self, id: &str, at: &str, reason: &str) -> Result<usize> {
        Ok(self.conn.execute(
            "UPDATE api_keys SET revoked = ?2, revoked_reason = ?3
             WHERE id = ?1 AND revoked IS NULL",
            params![id, at, reason],
        )?)
    }

    pub fn touch_api_key(&self, id: &str, at: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE api_keys SET last_used = ?2 WHERE id = ?1",
            params![id, at],
        )?;
        Ok(())
    }

    fn api_key_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<crate::auth::keys::ApiKey> {
        Ok(crate::auth::keys::ApiKey {
            id: r.get(0)?,
            user_id: r.get(1)?,
            name: r.get(2)?,
            created: r.get(3)?,
            created_via: r.get(4)?,
            expires: r.get(5)?,
            last_used: r.get(6)?,
            revoked: r.get(7)?,
            revoked_reason: r.get(8)?,
        })
    }

    // ---- pairing sessions ------------------------------------------------

    #[allow(clippy::too_many_arguments)]
    pub fn insert_pairing_session(
        &self,
        id: &str,
        nonce_hash: &[u8],
        purpose: &str,
        created_by: Option<&str>,
        peer_ip: Option<&str>,
        created: &str,
        expires: &str,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO pairing_sessions
                 (id, nonce_hash, purpose, created_by, peer_ip, created, expires)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                id, nonce_hash, purpose, created_by, peer_ip, created, expires
            ],
        )?;
        Ok(())
    }

    /// How many web-bootstrap nonces this peer has been issued since `since`.
    ///
    /// The rate limit's input. Counts issuance rather than consumption: a
    /// client fetching the page in a loop is the thing being bounded, and it
    /// never consumes anything.
    pub fn web_bootstrap_issued_since(&self, peer_ip: &str, since: &str) -> Result<i64> {
        Ok(self.conn.query_row(
            "SELECT COUNT(*) FROM pairing_sessions
             WHERE purpose = 'web_bootstrap' AND peer_ip = ?1 AND created >= ?2",
            params![peer_ip, since],
            |r| r.get(0),
        )?)
    }

    /// Live (unconsumed, unexpired) web-bootstrap nonces, across all peers.
    pub fn web_bootstrap_outstanding(&self, now: &str) -> Result<i64> {
        Ok(self.conn.query_row(
            "SELECT COUNT(*) FROM pairing_sessions
             WHERE purpose = 'web_bootstrap' AND consumed IS NULL AND expires > ?1",
            params![now],
            |r| r.get(0),
        )?)
    }

    /// Drops expired, unconsumed pairing sessions.
    ///
    /// Every web page load mints one and almost none are consumed, so without a
    /// sweep the table grows with traffic rather than with devices.
    pub fn sweep_expired_pairings(&self, now: &str) -> Result<usize> {
        Ok(self.conn.execute(
            "DELETE FROM pairing_sessions WHERE consumed IS NULL AND expires <= ?1",
            params![now],
        )?)
    }

    pub fn pairing_session_by_nonce_hash(
        &self,
        nonce_hash: &[u8],
    ) -> Result<Option<crate::auth::pairing::PairingRow>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id, purpose, expires, consumed, attempts, peer_ip
                 FROM pairing_sessions WHERE nonce_hash = ?1",
                params![nonce_hash],
                |r| {
                    Ok(crate::auth::pairing::PairingRow {
                        id: r.get(0)?,
                        purpose: r.get(1)?,
                        expires: r.get(2)?,
                        consumed: r.get(3)?,
                        attempts: r.get(4)?,
                        peer_ip: r.get(5)?,
                    })
                },
            )
            .optional()?)
    }

    pub fn count_users(&self) -> Result<i64> {
        Ok(self
            .conn
            .query_row("SELECT COUNT(*) FROM users", [], |r| r.get(0))?)
    }

    pub fn mark_pairing_consumed(&self, id: &str, consumed_by: &str, now: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE pairing_sessions SET consumed = ?2, consumed_by = ?3 WHERE id = ?1",
            params![id, now, consumed_by],
        )?;
        Ok(())
    }

    pub fn increment_pairing_attempts(&self, id: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE pairing_sessions SET attempts = attempts + 1 WHERE id = ?1",
            params![id],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The v1 sessions table, without `previous_refresh_hash`.
    const V1_SESSIONS: &str = "CREATE TABLE sessions (
        id              TEXT PRIMARY KEY,
        user_id         TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
        device_id       TEXT NOT NULL REFERENCES client_devices(id) ON DELETE CASCADE,
        access_hash     BLOB NOT NULL UNIQUE,
        refresh_hash    BLOB NOT NULL UNIQUE,
        created         TEXT NOT NULL,
        expires         TEXT NOT NULL,
        refresh_expires TEXT NOT NULL,
        last_used       TEXT,
        revoked         TEXT,
        revoked_reason  TEXT
    )";

    /// The v2 sessions table, with `previous_refresh_hash`.
    const V2_SESSIONS: &str = "CREATE TABLE sessions (
        id                    TEXT PRIMARY KEY,
        user_id               TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
        device_id             TEXT NOT NULL REFERENCES client_devices(id) ON DELETE CASCADE,
        access_hash           BLOB NOT NULL UNIQUE,
        refresh_hash          BLOB NOT NULL UNIQUE,
        previous_refresh_hash BLOB,
        created               TEXT NOT NULL,
        expires               TEXT NOT NULL,
        refresh_expires       TEXT NOT NULL,
        last_used             TEXT,
        revoked               TEXT,
        revoked_reason        TEXT
    )";

    fn table_names(db: &AuthDb) -> Vec<String> {
        let mut stmt = db
            .conn
            .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
            .unwrap();
        let rows = stmt.query_map([], |r| r.get::<_, String>(0)).unwrap();
        rows.map(|r| r.unwrap()).collect()
    }

    #[test]
    fn the_schema_has_every_table_the_data_model_names() {
        // Named one by one rather than counted: the later slices write these,
        // and adding a table then would be a migration on a database holding
        // the only copy of every user.
        let db = AuthDb::open_in_memory().unwrap();
        let names = table_names(&db);
        for expected in [
            "client_devices",
            "pairing_sessions",
            "security_events",
            "server",
            "server_credentials",
            "sessions",
            "users",
            "vault_grants",
            "ws_tickets",
        ] {
            assert!(
                names.iter().any(|n| n == expected),
                "auth.db is missing `{expected}`; have {names:?}"
            );
        }
    }

    #[test]
    fn a_new_database_is_stamped_with_the_schema_version() {
        let dir = tempdir::TempDir::new("storm-auth-ver").unwrap();
        let db = AuthDb::open(dir.path()).unwrap();
        let version: i64 = db
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
    }

    #[test]
    fn the_server_table_refuses_a_second_row() {
        // The CHECK is the only thing standing between one identity and two.
        let db = AuthDb::open_in_memory().unwrap();
        db.conn
            .execute(
                "INSERT INTO server (only_row, id, name, created)
                 VALUES (1, 'srv_a', 'A', '2026-08-13T00:00:00Z')",
                [],
            )
            .unwrap();
        assert!(
            db.conn
                .execute(
                    "INSERT INTO server (only_row, id, name, created)
                     VALUES (2, 'srv_b', 'B', '2026-08-13T00:00:00Z')",
                    [],
                )
                .is_err(),
            "a second server row must be refused"
        );
    }

    #[test]
    fn opening_an_existing_database_keeps_its_rows() {
        let dir = tempdir::TempDir::new("storm-auth-reopen").unwrap();
        {
            let mut db = AuthDb::open(dir.path()).unwrap();
            db.insert_identity(
                &ServerRow {
                    id: "srv_x".into(),
                    name: "Homelab".into(),
                    created: "2026-08-13T00:00:00Z".into(),
                },
                &CredentialRow {
                    key_id: "key_1".into(),
                    algorithm: "ed25519".into(),
                    public_key: vec![7; 32],
                    created: "2026-08-13T00:00:00Z".into(),
                },
            )
            .unwrap();
        }
        let db = AuthDb::open(dir.path()).unwrap();
        assert_eq!(db.server().unwrap().unwrap().id, "srv_x");
        assert_eq!(db.active_credential().unwrap().unwrap().key_id, "key_1");
        assert_eq!(db.active_credential_count().unwrap(), 1);
    }

    #[test]
    fn a_retired_or_revoked_credential_is_not_active() {
        let mut db = AuthDb::open_in_memory().unwrap();
        db.insert_identity(
            &ServerRow {
                id: "srv_x".into(),
                name: "Homelab".into(),
                created: "2026-08-13T00:00:00Z".into(),
            },
            &CredentialRow {
                key_id: "key_1".into(),
                algorithm: "ed25519".into(),
                public_key: vec![7; 32],
                created: "2026-08-13T00:00:00Z".into(),
            },
        )
        .unwrap();
        assert_eq!(db.active_credential_count().unwrap(), 1);

        db.conn
            .execute(
                "UPDATE server_credentials SET retired = '2026-08-14T00:00:00Z'",
                [],
            )
            .unwrap();
        assert_eq!(db.active_credential_count().unwrap(), 0);
        assert!(db.active_credential().unwrap().is_none());
    }

    #[test]
    fn a_snapshot_of_auth_db_reopens_with_its_rows() {
        let dir = tempdir::TempDir::new("storm-auth-snap").unwrap();
        let mut db = AuthDb::open(dir.path()).unwrap();
        db.insert_identity(
            &ServerRow {
                id: "srv_snapshot".into(),
                name: "Homelab".into(),
                created: "2026-08-13T00:00:00Z".into(),
            },
            &CredentialRow {
                key_id: "key_1".into(),
                algorithm: "ed25519".into(),
                public_key: vec![9; 32],
                created: "2026-08-13T00:00:00Z".into(),
            },
        )
        .unwrap();

        let dest = dir.path().join("copy.db");
        db.snapshot_to(&dest).unwrap();
        let restored = AuthDb::open_at(&dest).unwrap();
        assert_eq!(restored.server().unwrap().unwrap().id, "srv_snapshot");
        assert_eq!(restored.active_credential_count().unwrap(), 1);
    }

    #[test]
    fn a_v1_database_gets_previous_refresh_hash_and_bumps_to_current() {
        let dir = tempdir::TempDir::new("storm-auth-mig-v2").unwrap();
        // Build a v1 database by hand.
        {
            let conn = Connection::open(dir.path().join("auth.db")).unwrap();
            conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON;")
                .unwrap();
            // Create the old schema minus the new column.
            conn.execute_batch(V1_SESSIONS).unwrap();
            conn.execute_batch("PRAGMA user_version = 1;").unwrap();
        }

        let db = AuthDb::open(dir.path()).unwrap();
        let ver: i64 = db
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            ver, SCHEMA_VERSION,
            "an old database should migrate all the way to the current schema"
        );
        assert!(
            db.column_exists("sessions", "previous_refresh_hash")
                .unwrap(),
            "sessions should have previous_refresh_hash after v1→v2 migration"
        );
    }

    #[test]
    fn a_v4_database_gains_api_keys_and_keeps_everything_else() {
        // v4→v5 is **purely additive** — the table comes from the
        // `CREATE TABLE IF NOT EXISTS` batch, not from a migration branch. This
        // asserts that, and asserts the thing a migration must never do: lose
        // state that cannot be rebuilt. `auth.db` is the one file in `state/`
        // that a rescan cannot reconstruct.
        let dir = tempdir::TempDir::new("storm-auth-mig-v5").unwrap();
        {
            let db = AuthDb::open(dir.path()).unwrap();
            db.conn.execute_batch("PRAGMA user_version = 4;").unwrap();
            crate::auth::users::create_user(
                &mut AuthDb::open(dir.path()).unwrap(),
                crate::auth::users::NewUser {
                    username: "dewansh",
                    display_name: None,
                    password_hash: "hash",
                    role: crate::auth::users::Role::Owner,
                },
                "2026-01-01T00:00:00Z",
            )
            .unwrap();
            db.conn
                .execute_batch("DROP TABLE IF EXISTS api_keys;")
                .unwrap();
            db.conn.execute_batch("PRAGMA user_version = 4;").unwrap();
        }

        let db = AuthDb::open(dir.path()).unwrap();
        let ver: i64 = db
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(ver, SCHEMA_VERSION);
        assert!(
            db.column_exists("api_keys", "secret_hash").unwrap(),
            "a v4 database must gain api_keys"
        );

        // The account is still there. A migration that quietly emptied the one
        // unrebuildable file would be the worst defect available here.
        let users: i64 = db
            .conn
            .query_row("SELECT COUNT(*) FROM users", [], |r| r.get(0))
            .unwrap();
        assert_eq!(users, 1, "the upgrade must not drop existing auth state");

        // And a key minted after the upgrade works, so the table is not merely
        // present but usable — FKs on, referencing the carried-over user.
        let fk_on: i64 = db
            .conn
            .query_row("PRAGMA foreign_keys", [], |r| r.get(0))
            .unwrap();
        assert_eq!(fk_on, 1);
        let mut db = db;
        let user = db.find_user("dewansh").unwrap().unwrap();
        let (_, secret) = crate::auth::keys::create(
            &mut db,
            &user.id,
            "after the upgrade",
            None,
            None,
            "2026-01-02T00:00:00Z",
        )
        .unwrap();
        assert!(
            crate::auth::keys::authenticate(&mut db, &secret, "2026-01-02T00:00:00Z").is_ok(),
            "a key minted on an upgraded database must authenticate"
        );
    }

    #[test]
    fn a_v3_database_gains_web_bootstrap_and_keeps_its_pairings() {
        // v3→v4 rebuilds pairing_sessions, because SQLite cannot alter a CHECK
        // constraint. A rebuild that dropped rows would be a silent data loss
        // in a table someone may have a pending pairing in.
        let dir = tempdir::TempDir::new("storm-auth-mig-v4").unwrap();
        {
            let db = AuthDb::open(dir.path()).unwrap();
            db.conn.execute_batch("PRAGMA user_version = 3;").unwrap();
            db.insert_pairing_session(
                "pair_keepme",
                b"hash",
                "first_user",
                None,
                None,
                "2026-01-01T00:00:00Z",
                "2099-01-01T00:00:00Z",
            )
            .unwrap();

            // **A device that was paired through that session.** This is the
            // row the first version of this test lacked, and its absence is
            // exactly why the migration looked fine: `client_devices.paired_via`
            // references `pairing_sessions(id)`, so with `foreign_keys = ON`
            // the rebuild's `DROP TABLE` is refused only when such a row
            // exists. Every real server that has ever paired anything has one.
            db.conn
                .execute(
                    "INSERT INTO client_devices
                       (id, name, secret_hash, paired, paired_via)
                     VALUES ('dev_1', 'Pixel 10', X'00', ?1, 'pair_keepme')",
                    params!["2026-01-01T00:00:00Z"],
                )
                .unwrap();
            // And a pairing that records which device consumed it, so the
            // rebuild has a `consumed_by` value it has to carry rather than an
            // all-NULL column that would survive being dropped unnoticed.
            db.conn
                .execute_batch(
                    "UPDATE pairing_sessions
                        SET consumed = '2026-01-02T00:00:00Z', consumed_by = 'dev_1'
                      WHERE id = 'pair_keepme';",
                )
                .unwrap();

            // Pretend it predates the new column.
            db.conn
                .execute_batch("ALTER TABLE pairing_sessions DROP COLUMN peer_ip;")
                .unwrap();
        }

        let db = AuthDb::open(dir.path()).unwrap();
        let ver: i64 = db
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(ver, SCHEMA_VERSION);
        assert!(db.column_exists("pairing_sessions", "peer_ip").unwrap());
        // `consumed_by` is in the fresh schema, so it has to be in the rebuilt
        // one too — `mark_pairing_consumed` writes it on every pair.
        assert!(
            db.column_exists("pairing_sessions", "consumed_by").unwrap(),
            "the rebuild must not drop a column the fresh schema has"
        );
        let consumed_by: Option<String> = db
            .conn
            .query_row(
                "SELECT consumed_by FROM pairing_sessions WHERE id = 'pair_keepme'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(consumed_by.as_deref(), Some("dev_1"), "and must carry it");

        // Foreign keys are back on, and the device still points at its pairing.
        let fk_on: i64 = db
            .conn
            .query_row("PRAGMA foreign_keys", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            fk_on, 1,
            "FK enforcement must be restored after the rebuild"
        );
        let violations = db
            .conn
            .prepare("PRAGMA foreign_key_check")
            .unwrap()
            .query_map([], |_| Ok(()))
            .unwrap()
            .count();
        assert_eq!(violations, 0, "the rebuild must not orphan a device");

        let kept: i64 = db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM pairing_sessions WHERE id = 'pair_keepme'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(kept, 1, "the rebuild must carry existing pairings across");

        // And the new purpose is now accepted, which the old CHECK refused.
        db.insert_pairing_session(
            "pair_web",
            b"hash2",
            "web_bootstrap",
            None,
            Some("192.168.1.20"),
            "2026-01-01T00:00:00Z",
            "2099-01-01T00:00:00Z",
        )
        .unwrap();
    }

    #[test]
    fn a_v2_database_gets_ws_tickets_and_bumps_to_v3() {
        let dir = tempdir::TempDir::new("storm-auth-mig-v3").unwrap();
        // Build a v2 database: v1 schema + previous_refresh_hash column.
        {
            let conn = Connection::open(dir.path().join("auth.db")).unwrap();
            conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON;")
                .unwrap();
            conn.execute_batch(V2_SESSIONS).unwrap();
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS ws_tickets (
                    id          TEXT PRIMARY KEY,
                    session_id  TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                    access_hash BLOB NOT NULL UNIQUE,
                    created     TEXT NOT NULL,
                    expires     TEXT NOT NULL,
                    used        TEXT
                );",
            )
            .unwrap();
            conn.execute_batch("PRAGMA user_version = 2;").unwrap();
        }

        let db = AuthDb::open(dir.path()).unwrap();
        let ver: i64 = db
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            ver, SCHEMA_VERSION,
            "a v2 database should reach the current schema"
        );
        assert!(
            db.column_exists("sessions", "previous_refresh_hash")
                .unwrap(),
            "v2→v3 must preserve the previous_refresh_hash column"
        );
    }
}
