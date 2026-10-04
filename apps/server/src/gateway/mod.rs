//! The MCP Gateway's server half (decision 81; the spec is *MCP Gateway/V1
//! Specification* in the personal vault).
//!
//! storm-server owns every integration: its connection record, its encrypted
//! credential, and the MCP client that talks to its upstream. A Runtime Host
//! only forwards, and never sees a credential (G-D2, G-D3).
//!
//! **This module holds storage, crypto and (later) the upstream client — never
//! a policy decision.** Who may manage a connection, and whether a call is
//! allowed, are answered in `ops.rs` (decision 37, spec §7), so REST and any
//! future caller share one answer.
//!
//! Slice 1 (81b) is the store: `gateway.db`, the data key, and their place in
//! `backup_all()`. Slice 2 (81c) adds the connection rows and the owner's
//! operations on them; slice 3 (81d) the upstream client, the owner's test
//! and tool listing, and the call audit.

pub mod connections;
pub mod crypto;
pub mod store;
pub mod upstream;

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Context, Result};

/// `state/gateway/` — beside `state/agent/`, never inside a vault.
pub const GATEWAY_DIR: &str = "gateway";
pub const GATEWAY_DB_FILE: &str = "gateway.db";
/// Under [`GATEWAY_DIR`].
pub const KEYS_DIR: &str = "keys";

const ACTIVE_KEY: &str = "active_key_id";

pub struct Gateway {
    /// A `std::sync::Mutex`, never held across an `.await` (the `agent/` rule).
    pub store: Mutex<store::GatewayDb>,
    pub keys: crypto::Keyring,
    /// Test suites only: accept `http://` upstream URLs (a mock on loopback
    /// has no certificate). Set by the hidden `--gateway-allow-http-upstreams`.
    allow_http_upstreams: std::sync::atomic::AtomicBool,
    /// Audit rows written since the last prune.
    calls_since_prune: std::sync::atomic::AtomicU64,
}

/// Epoch milliseconds, the call audit's clock.
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Prune the call audit once per this many rows, and at boot.
const PRUNE_EVERY: u64 = 1000;

pub fn db_path(state_dir: &Path) -> PathBuf {
    state_dir.join(GATEWAY_DIR).join(GATEWAY_DB_FILE)
}

impl Gateway {
    /// Opens `state/gateway/`, loading or creating the data key.
    ///
    /// A lost key is **not** a refusal to start (§8): a new key becomes
    /// active, every connection sealed under the lost one becomes
    /// `needs_reauth`, and the journal says so. The notes stay online; the
    /// owner reconnects those integrations.
    pub fn open(state_dir: &Path, now: &str) -> Result<Self> {
        let dir = state_dir.join(GATEWAY_DIR);
        std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        let db = store::GatewayDb::open(&db_path(state_dir)).context("opening gateway.db")?;

        let recorded = db.meta(ACTIVE_KEY)?;
        let loaded = crypto::load_or_create(state_dir, recorded.as_deref())?;
        if recorded.as_deref() != Some(loaded.keyring.active_key_id()) {
            db.set_meta(ACTIVE_KEY, loaded.keyring.active_key_id())?;
        }
        if let Some(lost) = &loaded.replaced_lost_key {
            let unopenable = unopenable_connections(&db, &loaded.keyring)?;
            let marked = db.mark_needs_reauth(&unopenable, now)?;
            tracing::warn!(
                lost_key = %lost,
                new_key = %loaded.keyring.active_key_id(),
                connections = marked,
                "the gateway's data key is missing; a new one is active and every integration \
                 sealed under the old one needs reconnecting (restore state/gateway/keys/ from a \
                 backup to avoid this)"
            );
        }
        let _ = db.prune_calls(now_ms(), store::CALLS_KEEP_ROWS);
        Ok(Self {
            store: Mutex::new(db),
            keys: loaded.keyring,
            allow_http_upstreams: std::sync::atomic::AtomicBool::new(false),
            calls_since_prune: std::sync::atomic::AtomicU64::new(0),
        })
    }

    pub fn allow_http_upstreams(&self) -> bool {
        self.allow_http_upstreams
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// For the test suites only; see the field.
    pub fn set_allow_http_upstreams(&self, allow: bool) {
        self.allow_http_upstreams
            .store(allow, std::sync::atomic::Ordering::Relaxed);
    }

    /// Writes one audit row (G-D18: metadata only, by type) and applies the
    /// retention every [`PRUNE_EVERY`] rows. A failure to audit is logged and
    /// never fails the call it describes.
    pub fn record_call(&self, call: store::CallRecord) {
        let store = self.store.lock().expect("gateway store lock");
        if let Err(e) = store.record_call(&call) {
            tracing::warn!(error = %e, "could not write a gateway call audit row");
        }
        let n = self
            .calls_since_prune
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if n + 1 >= PRUNE_EVERY {
            self.calls_since_prune
                .store(0, std::sync::atomic::Ordering::Relaxed);
            let _ = store.prune_calls(now_ms(), store::CALLS_KEEP_ROWS);
        }
    }
}

/// Connections holding a credential that no loaded key can open.
fn unopenable_connections(db: &store::GatewayDb, keys: &crypto::Keyring) -> Result<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    for (connection, key_id) in db.credential_keys()? {
        if !keys.has_key(&key_id) && !out.contains(&connection) {
            out.push(connection);
        }
    }
    Ok(out)
}

/// Copies `gateway.db` and its key directory into `dest`, mirroring the state
/// layout (`dest/gateway/gateway.db`, `dest/gateway/keys/*.key`).
///
/// **Both or the backup is not one** (the A4 corollary, spec §8): ciphertexts
/// without their key restore a gateway that can authenticate nothing, and a
/// key without its database restores nothing to open. Keys are copied as
/// files — written once, never modified — and re-tightened to `0600` in a
/// `0700` directory rather than trusting the copy to carry the mode.
pub fn backup(state_dir: &Path, dest: &Path) -> Result<()> {
    let src = db_path(state_dir);
    if !src.exists() {
        return Ok(());
    }
    let out_dir = dest.join(GATEWAY_DIR);
    std::fs::create_dir_all(&out_dir).with_context(|| format!("creating {}", out_dir.display()))?;
    let db = store::GatewayDb::open(&src).context("opening gateway.db")?;
    let out = out_dir.join(GATEWAY_DB_FILE);
    db.snapshot_to(&out)
        .with_context(|| format!("writing snapshot to {}", out.display()))?;
    println!("  gateway -> {}", out.display());

    let keys = crypto::keys_dir(state_dir);
    if !keys.is_dir() {
        return Ok(());
    }
    let keys_out = crypto::keys_dir(dest);
    crypto::create_private_dir(&keys_out)?;
    let mut copied = 0;
    for entry in std::fs::read_dir(&keys).with_context(|| format!("reading {}", keys.display()))? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let to = keys_out.join(entry.file_name());
        std::fs::copy(entry.path(), &to)
            .with_context(|| format!("copying {}", entry.path().display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&to, std::fs::Permissions::from_mode(0o600))
                .with_context(|| format!("tightening {}", to.display()))?;
        }
        copied += 1;
    }
    println!("  gateway keys -> {} ({copied})", keys_out.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;

    fn connection(gw: &Gateway, id: &str) {
        gw.store
            .lock()
            .unwrap()
            .conn()
            .execute(
                "INSERT INTO connections (id, owner_user_id, slug, display_name, url,
                     auth_kind, status, created_at, updated_at)
                 VALUES (?1, 'usr_A', ?1, ?1, 'https://x', 'static', 'connected', 't', 't')",
                params![id],
            )
            .unwrap();
    }

    fn status(gw: &Gateway, id: &str) -> String {
        gw.store
            .lock()
            .unwrap()
            .conn()
            .query_row(
                "SELECT status FROM connections WHERE id = ?1",
                params![id],
                |r| r.get(0),
            )
            .unwrap()
    }

    fn seal_into(gw: &Gateway, id: &str, secret: &[u8]) {
        let sealed = gw.keys.seal(id, "static", secret).unwrap();
        gw.store
            .lock()
            .unwrap()
            .put_credential(id, "static", &sealed, None, "t")
            .unwrap();
    }

    fn open_from(gw: &Gateway, id: &str) -> Vec<u8> {
        let (sealed, _) = gw
            .store
            .lock()
            .unwrap()
            .credential(id, "static")
            .unwrap()
            .unwrap();
        gw.keys
            .open(id, "static", &sealed)
            .unwrap()
            .expose()
            .to_vec()
    }

    #[test]
    fn the_first_boot_records_its_key_and_the_next_boot_reuses_it() {
        let dir = tempdir::TempDir::new("storm-gw-boot").unwrap();
        let first = Gateway::open(dir.path(), "t").unwrap();
        connection(&first, "mcc_A");
        seal_into(&first, "mcc_A", b"pat-value");
        let key = first.keys.active_key_id().to_string();
        drop(first);

        let again = Gateway::open(dir.path(), "t").unwrap();
        assert_eq!(again.keys.active_key_id(), key);
        assert_eq!(open_from(&again, "mcc_A"), b"pat-value");
        assert_eq!(status(&again, "mcc_A"), "connected");
    }

    #[test]
    fn a_lost_key_marks_its_connections_needs_reauth_and_the_server_still_starts() {
        let dir = tempdir::TempDir::new("storm-gw-boot-lost").unwrap();
        let first = Gateway::open(dir.path(), "t").unwrap();
        connection(&first, "mcc_A");
        connection(&first, "mcc_NONE");
        seal_into(&first, "mcc_A", b"pat-value");
        let key = first.keys.active_key_id().to_string();
        drop(first);
        std::fs::remove_file(crypto::key_path(dir.path(), &key)).unwrap();

        let again = Gateway::open(dir.path(), "t2").expect("a lost key is not a lockout");
        assert_ne!(again.keys.active_key_id(), key);
        assert_eq!(status(&again, "mcc_A"), "needs_reauth");
        // A connection with nothing sealed (`none`) had nothing to lose.
        assert_eq!(status(&again, "mcc_NONE"), "connected");
        // And the new key is the recorded one from now on.
        let active = again.keys.active_key_id().to_string();
        drop(again);
        let third = Gateway::open(dir.path(), "t3").unwrap();
        assert_eq!(third.keys.active_key_id(), active);
        assert_eq!(
            third
                .store
                .lock()
                .unwrap()
                .meta(ACTIVE_KEY)
                .unwrap()
                .as_deref(),
            Some(active.as_str())
        );
    }

    #[test]
    fn a_backup_carries_the_database_and_its_key_and_a_restore_opens_the_credential() {
        let dir = tempdir::TempDir::new("storm-gw-backup").unwrap();
        let state = dir.path().join("state");
        let gw = Gateway::open(&state, "t").unwrap();
        connection(&gw, "mcc_A");
        seal_into(&gw, "mcc_A", b"upstream-canary-restore");
        let key = gw.keys.active_key_id().to_string();
        drop(gw);

        let dest = dir.path().join("snapshot");
        backup(&state, &dest).unwrap();
        assert!(db_path(&dest).exists(), "gateway.db is not in the backup");
        let backed_key = crypto::key_path(&dest, &key);
        assert!(
            backed_key.exists(),
            "the data key is not in the backup; the ciphertexts would restore unreadable"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&backed_key).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "the backed-up key is {mode:o}");
            let dmode = std::fs::metadata(crypto::keys_dir(&dest))
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(dmode, 0o700, "the backed-up key directory is {dmode:o}");
        }

        // Wipe and restore by plain copy: the backup mirrors the state layout.
        std::fs::remove_dir_all(&state).unwrap();
        std::fs::create_dir_all(state.join(GATEWAY_DIR)).unwrap();
        std::fs::copy(db_path(&dest), db_path(&state)).unwrap();
        crypto::create_private_dir(&crypto::keys_dir(&state)).unwrap();
        std::fs::copy(&backed_key, crypto::key_path(&state, &key)).unwrap();

        let restored = Gateway::open(&state, "t2").unwrap();
        assert_eq!(restored.keys.active_key_id(), key);
        assert_eq!(open_from(&restored, "mcc_A"), b"upstream-canary-restore");
        assert_eq!(status(&restored, "mcc_A"), "connected");
    }

    #[test]
    fn the_database_never_holds_a_credential_in_the_clear() {
        let dir = tempdir::TempDir::new("storm-gw-clear").unwrap();
        let gw = Gateway::open(dir.path(), "t").unwrap();
        connection(&gw, "mcc_A");
        let canary = b"upstream-canary-5f3a9c";
        seal_into(&gw, "mcc_A", canary);
        // Read while it is still open, WAL included: the leak the encryption
        // is for is exactly these files being copied.
        for suffix in ["", "-wal"] {
            let path = PathBuf::from(format!("{}{suffix}", db_path(dir.path()).display()));
            if let Ok(bytes) = std::fs::read(&path) {
                assert!(
                    !bytes.windows(canary.len()).any(|w| w == canary),
                    "{} holds the credential in the clear",
                    path.display()
                );
            }
        }
        drop(gw);
    }
}
