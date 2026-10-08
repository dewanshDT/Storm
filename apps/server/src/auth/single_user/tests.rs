//! v6 single-user migration tests (plan §3.3.4). The fixture is built from
//! the v5 schema as it shipped, frozen below — never edit it to match today's.

use std::path::{Path, PathBuf};

use rusqlite::{Connection, params};

use super::*;
use crate::auth::AuthDb;

const V5_SCHEMA: &str = r#"
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

            -- Runtime Hosts (Agent Runtime V1, decision 77b): the execution
            -- plane's machines. Public metadata only — the private key never
            -- leaves the host. Additive, so no schema version bump.
            CREATE TABLE IF NOT EXISTS runtime_hosts (
                id           TEXT PRIMARY KEY,
                name         TEXT NOT NULL,
                public_key   BLOB NOT NULL,
                key_id       TEXT NOT NULL,
                enrolled_by  TEXT REFERENCES users(id) ON DELETE SET NULL,
                enrolled_at  TEXT NOT NULL,
                last_seen    TEXT,
                revoked      TEXT
            );

            -- Single use, 10 minutes. `token_hash` is blake3 of the whole
            -- `sen_…` string; the plaintext is shown once to the owner.
            CREATE TABLE IF NOT EXISTS host_enrollments (
                id          TEXT PRIMARY KEY,
                token_hash  BLOB NOT NULL UNIQUE,
                created_by  TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                created     TEXT NOT NULL,
                expires     TEXT NOT NULL,
                consumed    TEXT,
                consumed_by TEXT
            );

            -- A host's link credential, minted only by proving its key.
            CREATE TABLE IF NOT EXISTS host_tokens (
                id         TEXT PRIMARY KEY,
                host_id    TEXT NOT NULL REFERENCES runtime_hosts(id) ON DELETE CASCADE,
                token_hash BLOB NOT NULL UNIQUE,
                created    TEXT NOT NULL,
                expires    TEXT NOT NULL,
                revoked    TEXT
            );
            CREATE INDEX IF NOT EXISTS host_tokens_by_host ON host_tokens(host_id);

            -- Server-generated nonces for key authentication: single use, 60 s.
            CREATE TABLE IF NOT EXISTS host_challenges (
                nonce   TEXT PRIMARY KEY,
                host_id TEXT NOT NULL,
                expires TEXT NOT NULL
            );

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
            "#;

const NOW: &str = "2026-10-08T12:00:00Z";
pub(crate) const SURVIVOR_PASSWORD: &str = "survivor correct horse battery";

/// Six accounts: `usr_owner_old` survives; an older *disabled* owner must not
/// win; `usr_member_b` shares `dev_shared` with the survivor.
pub(crate) struct Fixture {
    _dir: tempdir::TempDir,
    pub path: PathBuf,
    pub survivor_hash: String,
}

pub(crate) fn v5_fixture(survivor_hash: &str) -> Fixture {
    let dir = tempdir::TempDir::new("storm-single-user").unwrap();
    let path = dir.path().join("auth.db");
    let conn = Connection::open(&path).unwrap();
    conn.pragma_update(None, "journal_mode", "WAL").unwrap();
    conn.pragma_update(None, "foreign_keys", "ON").unwrap();
    conn.execute_batch(V5_SCHEMA).unwrap();
    conn.pragma_update(None, "user_version", 5).unwrap();

    let users: &[(&str, &str, &str, &str)] = &[
        (
            "usr_owner_disabled",
            "owner",
            "disabled",
            "2026-01-01T00:00:00Z",
        ),
        ("usr_owner_old", "owner", "active", "2026-02-01T00:00:00Z"),
        ("usr_owner_new", "owner", "active", "2026-03-01T00:00:00Z"),
        ("usr_member_a", "member", "active", "2026-04-01T00:00:00Z"),
        ("usr_member_b", "member", "active", "2026-05-01T00:00:00Z"),
        (
            "usr_member_disabled",
            "member",
            "disabled",
            "2026-06-01T00:00:00Z",
        ),
    ];
    for (id, role, status, created) in users {
        let name = id.trim_start_matches("usr_");
        let username = format!("uname-{name}");
        let hash = if *id == "usr_owner_old" {
            survivor_hash.to_string()
        } else {
            format!("$argon2id$v=19$m=196608,t=1,p=1$c2FsdA$h{name}")
        };
        conn.execute(
            "INSERT INTO users (id, username, username_fold, display_name, password_hash, role, status, created, updated, last_login, failed_count, locked_until)
             VALUES (?1, ?2, ?2, NULL, ?3, ?4, ?5, ?6, ?6, NULL, ?7, NULL)",
            params![id, username, hash, role, status, created, if *id == "usr_owner_old" { 2 } else { 0 }],
        )
        .unwrap();
    }

    let pairings: &[(&str, &str, Option<&str>, Option<&str>)] = &[
        (
            "pair_survivor",
            "add_device",
            Some("usr_owner_old"),
            Some(NOW),
        ),
        (
            "pair_member_a_pending",
            "add_device",
            Some("usr_member_a"),
            None,
        ),
        (
            "pair_member_disabled",
            "add_device",
            Some("usr_member_disabled"),
            Some(NOW),
        ),
        ("pair_bootstrap", "first_user", None, Some(NOW)),
    ];
    for (id, purpose, by, consumed) in pairings {
        conn.execute(
            "INSERT INTO pairing_sessions (id, nonce_hash, purpose, peer_ip, created_by, created, expires, consumed, consumed_by, attempts)
             VALUES (?1, ?2, ?3, NULL, ?4, ?5, '2026-12-01T00:00:00Z', ?6, NULL, 0)",
            params![id, id.as_bytes(), purpose, by, NOW, consumed],
        )
        .unwrap();
    }

    for id in [
        "dev_survivor1",
        "dev_survivor2",
        "dev_member_a",
        "dev_shared",
        "dev_owner_new",
        "dev_unused",
    ] {
        conn.execute(
            "INSERT INTO client_devices (id, name, platform, client_version, secret_hash, paired, paired_via, last_seen, revoked, revoked_reason)
             VALUES (?1, ?1, 'linux', '0.3.1', ?2, ?3, NULL, NULL, NULL, NULL)",
            params![id, crate::auth::token::hash(&device_secret(id)), NOW],
        )
        .unwrap();
    }

    let sessions: &[(&str, &str, &str, Option<&str>)] = &[
        ("ses_s1", "usr_owner_old", "dev_survivor1", None),
        ("ses_s2", "usr_owner_old", "dev_survivor2", Some(NOW)),
        ("ses_s3", "usr_owner_old", "dev_shared", None),
        ("ses_a1", "usr_member_a", "dev_member_a", None),
        ("ses_b1", "usr_member_b", "dev_shared", None),
        ("ses_n1", "usr_owner_new", "dev_owner_new", None),
    ];
    for (id, user, device, revoked) in sessions {
        conn.execute(
            "INSERT INTO sessions (id, user_id, device_id, access_hash, refresh_hash, previous_refresh_hash, created, expires, refresh_expires, last_used, revoked, revoked_reason)
             VALUES (?1, ?2, ?3, ?4, ?5, NULL, ?6, '2026-11-08T12:00:00Z', '2027-04-08T12:00:00Z', NULL, ?7, NULL)",
            params![id, user, device, crate::auth::token::hash(&access_token(id)), crate::auth::token::hash(&refresh_token(id)), NOW, revoked],
        )
        .unwrap();
    }

    for (id, session) in [("wst_s1", "ses_s1"), ("wst_a1", "ses_a1")] {
        conn.execute(
            "INSERT INTO ws_tickets (id, session_id, access_hash, created, expires, used) VALUES (?1, ?2, ?3, ?4, '2026-10-08T12:01:00Z', NULL)",
            params![id, session, id.as_bytes(), NOW],
        )
        .unwrap();
    }

    let keys: &[(&str, &str, Option<&str>)] = &[
        ("key_s1", "usr_owner_old", None),
        ("key_s2", "usr_owner_old", Some(NOW)),
        ("key_a1", "usr_member_a", None),
        ("key_n1", "usr_owner_new", None),
    ];
    for (id, user, revoked) in keys {
        conn.execute(
            "INSERT INTO api_keys (id, user_id, name, secret_hash, created, created_via, expires, last_used, revoked, revoked_reason)
             VALUES (?1, ?2, ?1, ?3, ?4, NULL, NULL, NULL, ?5, NULL)",
            params![id, user, crate::auth::token::hash(&key_secret(id)), NOW, revoked],
        )
        .unwrap();
    }

    for (id, by) in [
        ("hst_survivor", "usr_owner_old"),
        ("hst_owner_new", "usr_owner_new"),
    ] {
        conn.execute(
            "INSERT INTO runtime_hosts (id, name, public_key, key_id, enrolled_by, enrolled_at, last_seen, revoked)
             VALUES (?1, ?1, ?2, 'k1', ?3, ?4, NULL, NULL)",
            params![id, id.as_bytes(), by, NOW],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO host_tokens (id, host_id, token_hash, created, expires, revoked) VALUES (?1, ?2, ?3, ?4, '2027-01-01T00:00:00Z', NULL)",
            params![format!("htk_{id}"), id, format!("tok-{id}").as_bytes(), NOW],
        )
        .unwrap();
    }
    for (id, by) in [
        ("enr_survivor", "usr_owner_old"),
        ("enr_owner_new", "usr_owner_new"),
    ] {
        conn.execute(
            "INSERT INTO host_enrollments (id, token_hash, created_by, created, expires, consumed, consumed_by) VALUES (?1, ?2, ?3, ?4, '2026-10-08T12:10:00Z', NULL, NULL)",
            params![id, id.as_bytes(), by, NOW],
        )
        .unwrap();
    }

    conn.execute(
        "INSERT INTO vault_grants (user_id, vault_id, access, granted, granted_by) VALUES ('usr_member_a', 'vlt_1', 'read', ?1, 'usr_owner_old')",
        params![NOW],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO security_events (at, kind, user_id, device_id, remote, detail) VALUES (?1, 'login_ok', 'usr_owner_old', 'dev_survivor1', NULL, '{}')",
        params![NOW],
    )
    .unwrap();

    let orphan: Option<String> = conn
        .query_row("PRAGMA foreign_key_check", [], |r| r.get(0))
        .optional()
        .unwrap();
    assert_eq!(orphan, None, "the v5 fixture must start consistent");
    drop(conn);

    Fixture {
        path,
        _dir: dir,
        survivor_hash: survivor_hash.to_string(),
    }
}

pub(crate) fn access_token(session: &str) -> String {
    format!("sta_fixture_{session}")
}
pub(crate) fn refresh_token(session: &str) -> String {
    format!("str_fixture_{session}")
}
pub(crate) fn key_secret(key: &str) -> String {
    format!("stk_fixture_{key}")
}
pub(crate) fn device_secret(device: &str) -> String {
    format!("dvs_fixture_{device}")
}

pub(crate) fn fake_hash() -> String {
    "$argon2id$v=19$m=196608,t=1,p=1$c3Vydml2b3I$c3Vydml2b3JoYXNo".to_string()
}

fn raw(path: &Path) -> Connection {
    let conn = Connection::open(path).unwrap();
    conn.pragma_update(None, "foreign_keys", "ON").unwrap();
    conn
}

fn fingerprint(conn: &Connection) -> String {
    let mut out = String::new();
    let version: i64 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    out.push_str(&format!("user_version={version}\n"));
    let objects: Vec<(String, String, Option<String>)> = conn
        .prepare("SELECT type, name, sql FROM sqlite_master WHERE name NOT LIKE 'sqlite_%' ORDER BY type, name")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    for (kind, name, sql) in &objects {
        out.push_str(&format!(
            "{kind} {name}: {}\n",
            sql.clone().unwrap_or_default()
        ));
        if kind == "table" {
            let mut stmt = conn
                .prepare(&format!("SELECT * FROM \"{name}\" ORDER BY rowid"))
                .unwrap();
            let cols = stmt.column_count();
            let mut rows = stmt.query([]).unwrap();
            while let Some(row) = rows.next().unwrap() {
                let cells: Vec<String> = (0..cols)
                    .map(|i| format!("{:?}", row.get_ref(i).unwrap()))
                    .collect();
                out.push_str(&format!("  {}\n", cells.join("|")));
            }
        }
    }
    out
}

fn count(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get(0)).unwrap()
}

fn ids(conn: &Connection, sql: &str) -> Vec<String> {
    conn.prepare(sql)
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

fn migrated() -> (Fixture, Connection) {
    let f = v5_fixture(&fake_hash());
    AuthDb::open_at(&f.path).expect("the migration succeeds");
    let conn = raw(&f.path);
    (f, conn)
}

#[test]
fn migrates_multi_account_to_the_oldest_active_owner() {
    let (f, conn) = migrated();
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM users"), 1);
    let (id, hash, failed): (String, String, i64) = conn
        .query_row(
            "SELECT id, password_hash, failed_count FROM users",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(id, "usr_owner_old");
    assert_eq!(hash, f.survivor_hash);
    assert_eq!(failed, 2, "lockout counters carry over");
    let columns = ids(&conn, "SELECT name FROM pragma_table_info('users')");
    assert_eq!(columns, V6_USER_COLUMNS);
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM sqlite_master WHERE name = 'vault_grants'"
        ),
        0
    );
    for forbidden in [
        "role",
        "status",
        "username",
        "username_fold",
        "display_name",
    ] {
        assert!(
            !columns.iter().any(|c| c == forbidden),
            "{forbidden} survived"
        );
    }
    assert_eq!(count(&conn, "PRAGMA user_version"), SINGLE_USER_VERSION);
}

#[test]
fn surviving_resources_stay_attached() {
    let f = v5_fixture(&fake_hash());
    let before = raw(&f.path);
    let survivor_sessions = fingerprint_rows(
        &before,
        "SELECT * FROM sessions WHERE user_id = 'usr_owner_old' ORDER BY id",
    );
    let survivor_keys = fingerprint_rows(
        &before,
        "SELECT * FROM api_keys WHERE user_id = 'usr_owner_old' ORDER BY id",
    );
    let host_tokens = fingerprint_rows(&before, "SELECT * FROM host_tokens ORDER BY id");
    drop(before);

    AuthDb::open_at(&f.path).unwrap();
    let conn = raw(&f.path);
    assert_eq!(
        fingerprint_rows(
            &conn,
            "SELECT * FROM sessions WHERE user_id = 'usr_owner_old' ORDER BY id"
        ),
        survivor_sessions
    );
    assert_eq!(
        fingerprint_rows(
            &conn,
            "SELECT * FROM api_keys WHERE user_id = 'usr_owner_old' ORDER BY id"
        ),
        survivor_keys
    );
    assert_eq!(
        fingerprint_rows(&conn, "SELECT * FROM host_tokens ORDER BY id"),
        host_tokens,
        "hosts keep their links"
    );
    assert_eq!(ids(&conn, "SELECT id FROM ws_tickets"), vec!["wst_s1"]);
    assert_eq!(
        ids(&conn, "SELECT id FROM host_enrollments"),
        vec!["enr_survivor"]
    );
    let live_devices = ids(
        &conn,
        "SELECT id FROM client_devices WHERE revoked IS NULL ORDER BY id",
    );
    assert_eq!(
        live_devices,
        vec!["dev_shared", "dev_survivor1", "dev_survivor2", "dev_unused"]
    );
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM sessions WHERE user_id <> 'usr_owner_old'"
        ),
        0
    );
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM api_keys WHERE user_id <> 'usr_owner_old'"
        ),
        0
    );
    assert_eq!(
        ids(
            &conn,
            "SELECT id FROM runtime_hosts WHERE enrolled_by = 'usr_owner_old'"
        ),
        vec!["hst_survivor"]
    );
}

fn fingerprint_rows(conn: &Connection, sql: &str) -> Vec<String> {
    let mut stmt = conn.prepare(sql).unwrap();
    let cols = stmt.column_count();
    let mut rows = stmt.query([]).unwrap();
    let mut out = Vec::new();
    while let Some(row) = rows.next().unwrap() {
        out.push(
            (0..cols)
                .map(|i| format!("{:?}", row.get_ref(i).unwrap()))
                .collect::<Vec<_>>()
                .join("|"),
        );
    }
    out
}

#[test]
fn deleted_accounts_leave_no_orphans() {
    let (_f, conn) = migrated();
    let orphans = count(&conn, "SELECT COUNT(*) FROM pragma_foreign_key_check");
    assert_eq!(orphans, 0);
    let removed = "('usr_owner_disabled','usr_owner_new','usr_member_a','usr_member_b','usr_member_disabled')";
    for (table, column) in [
        ("sessions", "user_id"),
        ("api_keys", "user_id"),
        ("host_enrollments", "created_by"),
        ("pairing_sessions", "created_by"),
        ("runtime_hosts", "enrolled_by"),
    ] {
        assert_eq!(
            count(
                &conn,
                &format!("SELECT COUNT(*) FROM {table} WHERE {column} IN {removed}")
            ),
            0,
            "{table}.{column} still names a removed account"
        );
    }
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM ws_tickets WHERE id = 'wst_a1'"),
        0
    );
    let revoked = ids(
        &conn,
        "SELECT id FROM client_devices WHERE revoked_reason = 'single_user_migration' ORDER BY id",
    );
    assert_eq!(revoked, vec!["dev_member_a", "dev_owner_new"]);
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM pairing_sessions WHERE id = 'pair_member_a_pending'"
        ),
        0
    );
    let consumed: Option<String> = conn
        .query_row(
            "SELECT created_by FROM pairing_sessions WHERE id = 'pair_member_disabled'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(consumed, None);
    assert_eq!(
        conn.query_row::<Option<String>, _, _>(
            "SELECT created_by FROM pairing_sessions WHERE id = 'pair_survivor'",
            [],
            |r| r.get(0)
        )
        .unwrap()
        .as_deref(),
        Some("usr_owner_old")
    );
    let enroller: Option<String> = conn
        .query_row(
            "SELECT enrolled_by FROM runtime_hosts WHERE id = 'hst_owner_new'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(enroller, None);
}

#[test]
fn migration_is_not_applied_twice() {
    let (f, conn) = migrated();
    let once = fingerprint(&conn);
    drop(conn);
    AuthDb::open_at(&f.path).unwrap();
    let conn = raw(&f.path);
    assert_eq!(
        fingerprint(&conn),
        once,
        "I6: a second open changes nothing"
    );
    assert_eq!(
        count(
            &conn,
            &format!("SELECT COUNT(*) FROM security_events WHERE kind = '{EVENT_MIGRATION}'")
        ),
        1
    );
}

#[test]
fn a_v6_schema_under_a_stale_version_is_only_restamped() {
    let (f, conn) = migrated();
    conn.pragma_update(None, "user_version", 5).unwrap();
    let mut before = fingerprint(&conn);
    drop(conn);
    AuthDb::open_at(&f.path).unwrap();
    let conn = raw(&f.path);
    before = before.replacen("user_version=5", "user_version=6", 1);
    assert_eq!(fingerprint(&conn), before);
    assert!(!f.path.with_file_name("auth.db.pre-v6.tmp").exists());
}

#[test]
fn failure_mid_migration_leaves_v5_untouched() {
    for step in 1..=LAST_STEP {
        let f = v5_fixture(&fake_hash());
        let original = fingerprint(&raw(&f.path));

        let conn = raw(&f.path);
        inject_fault_after(Some(step));
        let result = migrate(&conn, Some(&f.path), &Keep::OldestActiveOwner, NOW);
        inject_fault_after(None);
        assert!(result.is_err(), "step {step}: the fault must surface");
        let fk: i64 = conn
            .query_row("PRAGMA foreign_keys", [], |r| r.get(0))
            .unwrap();
        assert_eq!(fk, 1, "step {step}: foreign keys must be back on");
        assert_eq!(
            fingerprint(&conn),
            original,
            "step {step}: v5 must be untouched"
        );
        drop(conn);

        AuthDb::open_at(&f.path).unwrap_or_else(|e| panic!("step {step}: retry failed: {e:#}"));
        assert_eq!(
            count(&raw(&f.path), "SELECT COUNT(*) FROM users"),
            1,
            "step {step}"
        );
    }
}

#[test]
fn a_fault_through_the_real_open_path_refuses_to_start() {
    let f = v5_fixture(&fake_hash());
    let original = fingerprint(&raw(&f.path));
    inject_fault_after(Some(Step::RebuildUsers as u8));
    let opened = AuthDb::open_at(&f.path);
    inject_fault_after(None);
    assert!(opened.is_err());
    assert_eq!(fingerprint(&raw(&f.path)), original);
}

#[test]
fn pre_v6_backup_is_written_first() {
    let f = v5_fixture(&fake_hash());
    let original = fingerprint(&raw(&f.path));
    AuthDb::open_at(&f.path).unwrap();
    let backup = f.path.with_file_name(PRE_V6_BACKUP);
    assert!(backup.exists());
    assert_eq!(
        fingerprint(&raw(&backup)),
        original,
        "the snapshot is the v5 database"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&backup).unwrap().permissions().mode() & 0o777;
        assert_eq!(
            mode, 0o600,
            "the snapshot holds every hash; it gets auth.db's mode"
        );
    }
    assert!(!f.path.with_file_name("auth.db.pre-v6.tmp").exists());
}

#[test]
fn no_backup_means_no_migration() {
    let f = v5_fixture(&fake_hash());
    let original = fingerprint(&raw(&f.path));
    std::fs::create_dir(f.path.with_file_name("auth.db.pre-v6.tmp")).unwrap();
    let opened = AuthDb::open_at(&f.path);
    assert!(opened.is_err(), "no snapshot, no migration");
    assert_eq!(fingerprint(&raw(&f.path)), original);
}

#[test]
fn no_active_owner_refuses_and_changes_nothing() {
    let f = v5_fixture(&fake_hash());
    {
        let conn = raw(&f.path);
        conn.execute(
            "UPDATE users SET status = 'disabled' WHERE role = 'owner'",
            [],
        )
        .unwrap();
    }
    let original = fingerprint(&raw(&f.path));
    let err = AuthDb::open_at(&f.path).err().expect("refused");
    let message = format!("{err:#}");
    assert!(message.contains("no active owner"), "{message}");
    assert!(message.contains("single-user --keep"), "{message}");
    assert_eq!(fingerprint(&raw(&f.path)), original);
    assert!(
        !f.path.with_file_name(PRE_V6_BACKUP).exists(),
        "refused before the snapshot"
    );
}

#[test]
fn single_user_keep_flag_chooses_the_named_account() {
    let f = v5_fixture(&fake_hash());
    AuthDb::open_at_keeping(&f.path, &Keep::Username("UNAME-member_b".into())).unwrap();
    let conn = raw(&f.path);
    assert_eq!(ids(&conn, "SELECT id FROM users"), vec!["usr_member_b"]);
    let live = ids(
        &conn,
        "SELECT id FROM client_devices WHERE revoked IS NULL ORDER BY id",
    );
    assert_eq!(live, vec!["dev_shared", "dev_unused"]);
}

#[test]
fn keep_refuses_an_unknown_name() {
    let f = v5_fixture(&fake_hash());
    let original = fingerprint(&raw(&f.path));
    assert!(AuthDb::open_at_keeping(&f.path, &Keep::Username("nobody".into())).is_err());
    assert_eq!(fingerprint(&raw(&f.path)), original);
}

#[test]
fn a_never_set_up_server_migrates_to_no_account() {
    let dir = tempdir::TempDir::new("storm-single-user-empty").unwrap();
    let path = dir.path().join("auth.db");
    {
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(V5_SCHEMA).unwrap();
        conn.pragma_update(None, "user_version", 5).unwrap();
    }
    let db = AuthDb::open_at(&path).unwrap();
    assert!(db.account().unwrap().is_none());
}

#[test]
fn fresh_install_schema_equals_migrated_schema() {
    let (_f, migrated_conn) = migrated();
    let dir = tempdir::TempDir::new("storm-single-user-fresh").unwrap();
    let fresh_path = dir.path().join("auth.db");
    AuthDb::open_at(&fresh_path).unwrap();
    let fresh_conn = raw(&fresh_path);
    assert_eq!(shape(&fresh_conn), shape(&migrated_conn));
}

fn shape(conn: &Connection) -> Vec<String> {
    let tables = ids(
        conn,
        "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
    );
    let mut out = Vec::new();
    for t in tables {
        out.push(format!("table {t}"));
        out.extend(fingerprint_rows(
            conn,
            &format!(
                "SELECT name, type, \"notnull\", dflt_value, pk FROM pragma_table_info('{t}')"
            ),
        ));
        out.extend(fingerprint_rows(
            conn,
            &format!(
                "SELECT \"unique\", origin, partial FROM pragma_index_list('{t}') ORDER BY name"
            ),
        ));
        out.extend(fingerprint_rows(conn, &format!("SELECT \"table\", \"from\", \"to\", on_delete FROM pragma_foreign_key_list('{t}') ORDER BY \"from\"")));
    }
    out
}

#[test]
fn security_events_record_the_migration_without_secrets() {
    let (_f, conn) = migrated();
    let removals: Vec<(Option<String>, String)> = conn
        .prepare(&format!("SELECT user_id, detail FROM security_events WHERE kind = '{EVENT_ACCOUNT_REMOVED}' ORDER BY user_id"))
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let removed_ids: Vec<_> = removals.iter().map(|(id, _)| id.clone().unwrap()).collect();
    assert_eq!(
        removed_ids,
        vec![
            "usr_member_a",
            "usr_member_b",
            "usr_member_disabled",
            "usr_owner_disabled",
            "usr_owner_new"
        ]
    );
    let member_a: serde_json::Value = serde_json::from_str(&removals[0].1).unwrap();
    assert_eq!(member_a["sessions"], 1);
    assert_eq!(member_a["keys"], 1);
    assert_eq!(member_a["devices_revoked"], 1);
    let summary: String = conn
        .query_row(
            &format!("SELECT detail FROM security_events WHERE kind = '{EVENT_MIGRATION}'"),
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(summary.contains("usr_owner_old"));
    let all: Vec<String> = ids(&conn, "SELECT detail FROM security_events");
    for detail in all {
        for secret in [
            "argon2",
            "sta_fixture",
            "str_fixture",
            "stk_fixture",
            "dvs_fixture",
            "uname-",
        ] {
            assert!(!detail.contains(secret), "{secret} leaked into {detail}");
        }
    }
}

#[test]
fn check_single_user_refuses_a_violated_database() {
    let (f, conn) = migrated();
    conn.execute_batch("PRAGMA foreign_keys = OFF;").unwrap();
    conn.execute(
        "INSERT INTO sessions (id, user_id, device_id, access_hash, refresh_hash, created, expires, refresh_expires)
         VALUES ('ses_orphan', 'usr_gone', 'dev_survivor1', x'01', x'02', ?1, ?1, ?1)",
        params![NOW],
    )
    .unwrap();
    drop(conn);
    let err = AuthDb::open_at(&f.path).err().expect("refused");
    assert!(format!("{err:#}").contains("I3"), "{err:#}");
}

#[test]
fn check_refuses_a_restored_multi_user_table() {
    let f = v5_fixture(&fake_hash());
    raw(&f.path).pragma_update(None, "user_version", 6).unwrap();
    let err = AuthDb::open_at(&f.path).err().expect("refused");
    assert!(
        format!("{err:#}").contains("I5") || format!("{err:#}").contains("I1"),
        "{err:#}"
    );
}

#[tokio::test]
async fn password_login_after_migration() {
    let hasher = crate::auth::Hasher::new();
    let hash = hasher.hash(SURVIVOR_PASSWORD.to_string()).await.unwrap();
    let f = v5_fixture(&hash);
    let mut db = AuthDb::open_at(&f.path).unwrap();
    let now = crate::index::now_rfc3339();

    let issued = crate::auth::sessions::login(
        &mut db,
        &hasher,
        SURVIVOR_PASSWORD.into(),
        "dev_survivor1",
        &now,
    )
    .await
    .expect("the survivor's password signs in");
    assert_eq!(issued.user_id, "usr_owner_old");
    let failed: i64 = db
        .conn
        .query_row("SELECT failed_count FROM users", [], |r| r.get(0))
        .unwrap();
    assert_eq!(failed, 0);

    let refused = crate::auth::sessions::login(
        &mut db,
        &hasher,
        SURVIVOR_PASSWORD.into(),
        "dev_member_a",
        &now,
    )
    .await;
    assert!(refused.is_err());
    let wrong = crate::auth::sessions::login(
        &mut db,
        &hasher,
        "a member's password".into(),
        "dev_survivor1",
        &now,
    )
    .await;
    assert!(wrong.is_err());
}
