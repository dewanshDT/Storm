//! `auth.db` v6: collapse a multi-account database onto one account
//! (decision 82; plan §3.3). The oldest active owner survives; everything else
//! is deleted in one transaction after an `auth.db.pre-v6` snapshot.
//! `agent.db`/`gateway.db` are swept separately by `ops::reconcile_single_user`.

use std::cell::Cell;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};

pub const SINGLE_USER_VERSION: i64 = 6;

pub const PRE_V6_BACKUP: &str = "auth.db.pre-v6";

pub const EVENT_ACCOUNT_REMOVED: &str = "account_removed_single_user";
pub const EVENT_MIGRATION: &str = "single_user_migration";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Keep {
    OldestActiveOwner,
    Username(String),
}

/// Shared by the fresh-install path and the rebuild so the two cannot drift.
pub fn users_table_sql(name: &str) -> String {
    format!(
        "CREATE TABLE {name} (
            id            TEXT PRIMARY KEY,
            only_row      INTEGER NOT NULL UNIQUE DEFAULT 1 CHECK (only_row = 1),
            password_hash TEXT NOT NULL,
            created       TEXT NOT NULL,
            updated       TEXT NOT NULL,
            last_login    TEXT,
            failed_count  INTEGER NOT NULL DEFAULT 0,
            locked_until  TEXT
        )"
    )
}

pub const V6_USER_COLUMNS: &[&str] = &[
    "id",
    "only_row",
    "password_hash",
    "created",
    "updated",
    "last_login",
    "failed_count",
    "locked_until",
];

thread_local! {
    static FAULT_AFTER: Cell<Option<u8>> = const { Cell::new(None) };
}

#[cfg(test)]
pub(crate) fn inject_fault_after(step: Option<u8>) {
    FAULT_AFTER.with(|f| f.set(step));
}

fn checkpoint(step: u8) -> Result<()> {
    if FAULT_AFTER.with(|f| f.get()) == Some(step) {
        bail!("injected fault after step {step}");
    }
    Ok(())
}

#[allow(dead_code)]
pub(crate) enum Step {
    Detach = 1,
    RevokeDevices = 2,
    DeleteDependents = 3,
    Audit = 4,
    RebuildUsers = 5,
    DropGrants = 6,
    ForeignKeyCheck = 7,
}

#[cfg(test)]
pub(crate) const LAST_STEP: u8 = 7;

fn column_exists(conn: &Connection, table: &str, column: &str) -> Result<bool> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let found = stmt
        .query_map([], |r| r.get::<_, String>(1))?
        .filter_map(Result::ok)
        .any(|name| name == column);
    Ok(found)
}

fn table_exists(conn: &Connection, table: &str) -> Result<bool> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1",
            params![table],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

/// `db_path` is `None` only for in-memory test databases.
pub(super) fn migrate(
    conn: &Connection,
    db_path: Option<&Path>,
    keep: &Keep,
    now: &str,
) -> Result<bool> {
    let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if version >= SINGLE_USER_VERSION {
        return Ok(false);
    }
    if !column_exists(conn, "users", "role")? {
        conn.pragma_update(None, "user_version", SINGLE_USER_VERSION)?;
        return Ok(false);
    }

    let user_count: i64 = conn.query_row("SELECT COUNT(*) FROM users", [], |r| r.get(0))?;
    let survivor: Option<String> = match keep {
        Keep::OldestActiveOwner => conn
            .query_row(
                "SELECT id FROM users WHERE role = 'owner' AND status = 'active'
                 ORDER BY created, id LIMIT 1",
                [],
                |r| r.get(0),
            )
            .optional()?,
        Keep::Username(name) => {
            let found: Option<String> = conn
                .query_row(
                    "SELECT id FROM users WHERE username_fold = ?1",
                    params![name.to_ascii_lowercase()],
                    |r| r.get(0),
                )
                .optional()?;
            if found.is_none() {
                bail!("no account named `{name}` in this auth.db; nothing was changed");
            }
            found
        }
    };
    if user_count > 0 && survivor.is_none() {
        bail!(
            "Storm is single-user now (decision 82) and keeps the oldest active owner, \
             but this server has {user_count} account(s) and no active owner. Nothing was \
             changed. Either re-enable an owner with the previous storm-server \
             (`storm-server user enable <name>`) and start again, or choose the account to \
             keep: `storm-server single-user --keep <username>`."
        );
    }

    if let Some(path) = db_path {
        write_pre_v6_backup(conn, path)
            .context("writing auth.db.pre-v6; the migration was not started")?;
    }

    // Must be outside the transaction: SQLite ignores it inside one.
    conn.pragma_update(None, "foreign_keys", "OFF")?;
    let result = run_transaction(conn, survivor.as_deref(), now);
    let restored = conn.pragma_update(None, "foreign_keys", "ON");
    result?;
    restored?;
    Ok(true)
}

fn write_pre_v6_backup(conn: &Connection, db_path: &Path) -> Result<()> {
    let dir = db_path.parent().unwrap_or_else(|| Path::new("."));
    let final_path = dir.join(PRE_V6_BACKUP);
    let tmp_path: PathBuf = dir.join(format!("{PRE_V6_BACKUP}.tmp"));
    crate::db::snapshot_connection(conn, &tmp_path)?;
    {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&tmp_path)
            .with_context(|| format!("opening {}", tmp_path.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        }
        file.sync_all()?;
    }
    std::fs::rename(&tmp_path, &final_path)
        .with_context(|| format!("renaming into {}", final_path.display()))?;
    if let Ok(d) = std::fs::File::open(dir) {
        let _ = d.sync_all();
    }
    Ok(())
}

fn run_transaction(conn: &Connection, survivor: Option<&str>, now: &str) -> Result<()> {
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;

    tx.execute_batch(
        "DROP TABLE IF EXISTS temp.removed;
         CREATE TEMP TABLE removed (id TEXT PRIMARY KEY, role TEXT);",
    )?;
    tx.execute(
        "INSERT INTO temp.removed (id, role)
         SELECT id, role FROM users WHERE ?1 IS NULL OR id <> ?1",
        params![survivor],
    )?;

    // `pairing_sessions.created_by` has no ON DELETE: drop pending ones, keep
    // consumed ones without their author.
    tx.execute(
        "DELETE FROM pairing_sessions
         WHERE consumed IS NULL AND created_by IN (SELECT id FROM temp.removed)",
        [],
    )?;
    tx.execute(
        "UPDATE pairing_sessions SET created_by = NULL
         WHERE created_by IN (SELECT id FROM temp.removed)",
        [],
    )?;
    tx.execute(
        "UPDATE runtime_hosts SET enrolled_by = NULL
         WHERE enrolled_by IN (SELECT id FROM temp.removed)",
        [],
    )?;
    checkpoint(Step::Detach as u8)?;

    // A device the survivor ever used stays, even if shared.
    tx.execute(
        "UPDATE client_devices
            SET revoked = ?1, revoked_reason = 'single_user_migration'
          WHERE revoked IS NULL
            AND id IN (SELECT device_id FROM sessions
                        WHERE user_id IN (SELECT id FROM temp.removed))
            AND id NOT IN (SELECT device_id FROM sessions WHERE ?2 IS NOT NULL AND user_id = ?2)",
        params![now, survivor],
    )?;
    checkpoint(Step::RevokeDevices as u8)?;

    let mut stmt = tx.prepare(
        "SELECT r.id, r.role,
                (SELECT COUNT(*) FROM sessions s WHERE s.user_id = r.id),
                (SELECT COUNT(*) FROM api_keys k WHERE k.user_id = r.id),
                (SELECT COUNT(DISTINCT s.device_id) FROM sessions s
                   JOIN client_devices d ON d.id = s.device_id
                  WHERE s.user_id = r.id AND d.revoked_reason = 'single_user_migration')
           FROM temp.removed r ORDER BY r.id",
    )?;
    let removed: Vec<(String, String, i64, i64, i64)> = stmt
        .query_map([], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })?
        .collect::<rusqlite::Result<_>>()?;
    drop(stmt);

    // FKs are off, so nothing cascades: delete dependents explicitly.
    tx.execute_batch(
        "DELETE FROM ws_tickets WHERE session_id IN
            (SELECT id FROM sessions WHERE user_id IN (SELECT id FROM temp.removed));
         DELETE FROM sessions WHERE user_id IN (SELECT id FROM temp.removed);
         DELETE FROM api_keys WHERE user_id IN (SELECT id FROM temp.removed);
         DELETE FROM host_enrollments WHERE created_by IN (SELECT id FROM temp.removed);",
    )?;
    checkpoint(Step::DeleteDependents as u8)?;

    for (id, role, sessions, keys, devices) in &removed {
        let detail = serde_json::json!({
            "user_id": id,
            "role": role,
            "sessions": sessions,
            "keys": keys,
            "devices_revoked": devices,
        });
        tx.execute(
            "INSERT INTO security_events (at, kind, user_id, device_id, remote, detail)
             VALUES (?1, ?2, ?3, NULL, NULL, ?4)",
            params![now, EVENT_ACCOUNT_REMOVED, id, detail.to_string()],
        )?;
    }
    let summary = serde_json::json!({
        "kept": survivor,
        "removed": removed.len(),
    });
    tx.execute(
        "INSERT INTO security_events (at, kind, user_id, device_id, remote, detail)
         VALUES (?1, ?2, ?3, NULL, NULL, ?4)",
        params![now, EVENT_MIGRATION, survivor, summary.to_string()],
    )?;
    checkpoint(Step::Audit as u8)?;

    tx.execute_batch("DROP TABLE IF EXISTS users_new;")?;
    tx.execute_batch(&users_table_sql("users_new"))?;
    tx.execute(
        "INSERT INTO users_new (id, password_hash, created, updated, last_login,
                                failed_count, locked_until)
         SELECT id, password_hash, created, updated, last_login, failed_count, locked_until
           FROM users WHERE id = ?1",
        params![survivor],
    )?;
    tx.execute_batch(
        "DROP TABLE users;
         ALTER TABLE users_new RENAME TO users;",
    )?;
    checkpoint(Step::RebuildUsers as u8)?;

    tx.execute_batch("DROP TABLE IF EXISTS vault_grants;")?;
    checkpoint(Step::DropGrants as u8)?;

    let orphans: Vec<(String, i64)> = {
        let mut stmt = tx.prepare("PRAGMA foreign_key_check")?;
        stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?
            .collect::<rusqlite::Result<_>>()?
    };
    if !orphans.is_empty() {
        bail!(
            "the single-user migration would leave {} orphaned row(s) (first: {:?}); \
             nothing was changed",
            orphans.len(),
            orphans[0]
        );
    }
    checkpoint(Step::ForeignKeyCheck as u8)?;

    tx.execute_batch("DROP TABLE IF EXISTS temp.removed;")?;
    tx.pragma_update(None, "user_version", SINGLE_USER_VERSION)?;
    tx.commit()?;
    Ok(())
}

/// Plan §3.3.3 invariants I1/I3/I5/I6, checked on every open.
pub(super) fn check(conn: &Connection) -> Result<()> {
    let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if version < SINGLE_USER_VERSION {
        bail!("I6: auth.db is at schema {version}, expected {SINGLE_USER_VERSION}");
    }
    let accounts: i64 = conn.query_row("SELECT COUNT(*) FROM users", [], |r| r.get(0))?;
    if accounts > 1 {
        bail!("I1: auth.db holds {accounts} accounts; Storm has exactly one");
    }
    let columns: Vec<String> = {
        let mut stmt = conn.prepare("PRAGMA table_info(users)")?;
        stmt.query_map([], |r| r.get(1))?
            .collect::<rusqlite::Result<_>>()?
    };
    if columns != V6_USER_COLUMNS {
        bail!("I5: the account table has columns {columns:?}, expected {V6_USER_COLUMNS:?}");
    }
    if table_exists(conn, "vault_grants")? {
        bail!("I5: vault_grants still exists");
    }
    let orphan = conn
        .query_row("PRAGMA foreign_key_check", [], |r| r.get::<_, String>(0))
        .optional()?;
    if let Some(table) = orphan {
        bail!("I3: auth.db has a row in `{table}` referencing something that does not exist");
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests;
