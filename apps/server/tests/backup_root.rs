//! The nightly backup copies the storage root the server uses (decision 79).
//!
//! `STORM_VAULT_ROOT` in `storm.env` only seeds a first run; a root changed in
//! the app lives in `state/vaults.json` and wins. The backup used to rsync the
//! env file's directory, which on prod was empty, so every backup verified and
//! held no notes. These drive the real binary and the real script, because the
//! failure was in how the two meet, not in either alone.

use std::fs;
use std::path::Path;
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_storm-server");
const SCRIPT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../deploy/storm-backup.sh");

fn storage_root(state: &Path) -> std::process::Output {
    Command::new(BIN)
        .args(["storage-root", "--state"])
        .arg(state)
        .output()
        .expect("running storm-server")
}

fn write_registry(state: &Path, root: &Path) {
    fs::create_dir_all(state).unwrap();
    fs::write(
        state.join("vaults.json"),
        format!(
            "{{\n  \"root\": {},\n  \"vaults\": []\n}}\n",
            serde_json::to_string(&root.display().to_string()).unwrap()
        ),
    )
    .unwrap();
}

#[test]
fn prints_the_root_the_registry_records() {
    let tmp = tempdir::TempDir::new("storm-backup-root").unwrap();
    let state = tmp.path().join("state");
    write_registry(&state, Path::new("/mnt/media/Docs/storm"));

    let out = storage_root(&state);
    assert!(out.status.success(), "{out:?}");
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        "/mnt/media/Docs/storm"
    );
}

#[test]
fn says_nothing_and_exits_3_without_a_registry() {
    let tmp = tempdir::TempDir::new("storm-backup-root").unwrap();
    let out = storage_root(&tmp.path().join("state"));
    assert_eq!(out.status.code(), Some(3), "{out:?}");
    assert!(out.stdout.is_empty());
}

#[test]
fn an_unreadable_registry_is_an_error_not_a_fallback() {
    // Exit 3 sends the script to the env file. A registry that will not parse
    // must not: that is a server that will not start either, and a backup
    // quietly copying some other directory would hide it.
    let tmp = tempdir::TempDir::new("storm-backup-root").unwrap();
    let state = tmp.path().join("state");
    fs::create_dir_all(&state).unwrap();
    fs::write(state.join("vaults.json"), "{ not json").unwrap();

    let out = storage_root(&state);
    assert!(!out.status.success());
    assert_ne!(out.status.code(), Some(3), "{out:?}");
}

#[test]
fn the_backup_copies_the_stored_root_when_the_env_file_disagrees() {
    // Prod's shape: the env file names an empty directory, the registry the
    // directory the notes are actually in.
    let tmp = tempdir::TempDir::new("storm-backup-root").unwrap();
    let stale = tmp.path().join("env-root");
    let real = tmp.path().join("nas");
    let state = tmp.path().join("state");
    let dest = tmp.path().join("backups");
    fs::create_dir_all(&stale).unwrap();
    fs::create_dir_all(real.join("personal")).unwrap();
    fs::write(real.join("personal/Note.md"), "# kept\n").unwrap();
    write_registry(&state, &real);

    let out = Command::new("bash")
        .arg(SCRIPT)
        .env("STORM_VAULT_ROOT", &stale)
        .env("STORM_STATE", &state)
        .env("STORM_BACKUP_DEST", &dest)
        .env("STORM_SERVER_BIN", BIN)
        .output()
        .expect("running storm-backup.sh");
    let log = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{log}{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let day = fs::read_dir(&dest).unwrap().next().unwrap().unwrap().path();
    assert_eq!(
        fs::read_to_string(day.join("vaults/personal/Note.md")).unwrap(),
        "# kept\n",
        "the backup must hold the notes the server serves"
    );
    // The drift is in the log, not hidden by it.
    assert!(log.contains("stored; storm.env says"), "{log}");
}
