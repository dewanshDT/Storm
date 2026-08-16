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
const SCHEMA_VERSION: i64 = 2;

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
                purpose      TEXT NOT NULL CHECK (purpose IN ('first_user','add_device')),
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
    fn a_v1_database_gets_previous_refresh_hash_and_bumps_to_v2() {
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
        assert_eq!(ver, 2, "schema should be at v2 after migration");
        assert!(
            db.column_exists("sessions", "previous_refresh_hash")
                .unwrap(),
            "sessions should have previous_refresh_hash after v1→v2 migration"
        );
    }
}
