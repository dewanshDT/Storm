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
#[cfg(unix)]
const SCRIPT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../deploy/storm-backup.sh");

fn storage_root(state: &Path) -> std::process::Output {
    Command::new(BIN)
        .args(["storage-root", "--state"])
        .arg(state)
        .output()
        .expect("running storm-server")
}

fn write_registry(state: &Path, root: &Path) {
    write_registry_with(state, root, &[]);
}

/// A registry listing `dirs` as vaults under `root`, written the way the
/// server writes it (pretty, one key per line).
fn write_registry_with(state: &Path, root: &Path, dirs: &[&str]) {
    fs::create_dir_all(state).unwrap();
    let vaults: Vec<_> = dirs
        .iter()
        .enumerate()
        .map(|(i, dir)| {
            serde_json::json!({
                "id": format!("00000000-0000-4000-8000-{i:012}"),
                "name": dir,
                "dir": dir,
                "created": "2026-10-20T00:00:00Z",
            })
        })
        .collect();
    let registry = serde_json::json!({ "root": root.display().to_string(), "vaults": vaults });
    fs::write(
        state.join("vaults.json"),
        serde_json::to_string_pretty(&registry).unwrap() + "\n",
    )
    .unwrap();
}

#[cfg(unix)]
fn run_script(vault_root: &Path, state: &Path, dest: &Path) -> std::process::Output {
    Command::new("bash")
        .arg(SCRIPT)
        .env("STORM_VAULT_ROOT", vault_root)
        .env("STORM_STATE", state)
        .env("STORM_BACKUP_DEST", dest)
        .env("STORM_SERVER_BIN", BIN)
        .output()
        .expect("running storm-backup.sh")
}

fn run_native(vault_root: &Path, state: &Path, dest: &Path) -> std::process::Output {
    Command::new(BIN)
        .arg("backup")
        .arg("--vault-root")
        .arg(vault_root)
        .arg("--state")
        .arg(state)
        .arg("--dest")
        .arg(dest)
        .output()
        .expect("running storm-server backup")
}

#[cfg(unix)]
fn only_day(dest: &Path) -> std::path::PathBuf {
    let days: Vec<_> = fs::read_dir(dest)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(days.len(), 1, "{days:?}");
    days.into_iter().next().unwrap()
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

// The script needs bash and rsync; Windows backs up with `storm-server backup`.
#[cfg(unix)]
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

/// `storm-server backup` (decision 99, W-S3), which Windows' nightly task
/// runs: the same rule as the script, that the stored root wins over the
/// flag, and snapshots that open.
#[test]
fn the_native_backup_copies_the_stored_root_when_the_flag_disagrees() {
    let tmp = tempdir::TempDir::new("storm-backup-native").unwrap();
    let stale = tmp.path().join("flag-root");
    let real = tmp.path().join("notes");
    let state = tmp.path().join("state");
    let dest = tmp.path().join("backups");
    fs::create_dir_all(&stale).unwrap();
    fs::create_dir_all(real.join("personal")).unwrap();
    fs::write(real.join("personal/Note.md"), "# kept\n").unwrap();
    write_registry(&state, &real);

    let out = Command::new(BIN)
        .arg("backup")
        .arg("--vault-root")
        .arg(&stale)
        .arg("--state")
        .arg(&state)
        .arg("--dest")
        .arg(&dest)
        .output()
        .expect("running storm-server backup");
    assert!(out.status.success(), "{out:?}");
    let day = fs::read_dir(&dest).unwrap().next().unwrap().unwrap().path();
    assert_eq!(
        fs::read_to_string(day.join("vaults").join("personal").join("Note.md")).unwrap(),
        "# kept\n",
        "the backup must hold the notes the server serves"
    );
    assert!(day.join("index").is_dir());
}

/// DP1 (decision 128j): each night is a full copy, so unchanged notes must be
/// hard links into the night before, not thirty full copies of the vault.
#[cfg(unix)]
#[test]
fn a_second_night_links_unchanged_files_to_the_first() {
    use std::os::unix::fs::MetadataExt;

    let tmp = tempdir::TempDir::new("storm-backup-link").unwrap();
    let root = tmp.path().join("vaults");
    let state = tmp.path().join("state");
    let dest = tmp.path().join("backups");
    fs::create_dir_all(root.join("personal")).unwrap();
    fs::write(root.join("personal/Same.md"), "# same\n").unwrap();
    fs::write(root.join("personal/Edited.md"), "# before\n").unwrap();
    write_registry_with(&state, &root, &["personal"]);

    let out = run_script(&root, &state, &dest);
    assert!(out.status.success(), "{out:?}");
    // Last night's copy, under a date that cannot be today's.
    let yesterday = dest.join("2000-01-01");
    fs::rename(only_day(&dest), &yesterday).unwrap();

    fs::write(root.join("personal/Edited.md"), "# after\n").unwrap();
    let out = run_script(&root, &state, &dest);
    assert!(out.status.success(), "{out:?}");
    let today = fs::read_dir(&dest)
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| *p != yesterday)
        .unwrap();

    let ino = |p: &Path| fs::metadata(p).unwrap().ino();
    assert_eq!(
        ino(&yesterday.join("vaults/personal/Same.md")),
        ino(&today.join("vaults/personal/Same.md")),
        "an unchanged note is a hard link to last night's copy"
    );
    assert_eq!(
        fs::read_to_string(today.join("vaults/personal/Edited.md")).unwrap(),
        "# after\n"
    );
    assert_eq!(
        fs::read_to_string(yesterday.join("vaults/personal/Edited.md")).unwrap(),
        "# before\n",
        "an edit never reaches back into an older night"
    );
}

/// DP1: an unmounted share is an empty directory. Mirroring it would write a
/// night that verifies and holds nothing, and the pruning would then age out
/// every good one; the registry listing vaults is what says it is wrong.
#[cfg(unix)]
#[test]
fn the_script_refuses_an_empty_root_the_registry_says_has_vaults() {
    let tmp = tempdir::TempDir::new("storm-backup-empty").unwrap();
    let root = tmp.path().join("nas");
    let state = tmp.path().join("state");
    let dest = tmp.path().join("backups");
    fs::create_dir_all(root.join("personal")).unwrap();
    write_registry_with(&state, &root, &["personal"]);

    let out = run_script(&root, &state, &dest);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success(), "{out:?}");
    assert!(err.contains("ERROR"), "{err}");
    assert!(
        !dest.exists() || fs::read_dir(&dest).unwrap().next().is_none(),
        "nothing is written for a refused night"
    );
}

#[test]
fn the_native_backup_refuses_an_empty_root_the_registry_says_has_vaults() {
    let tmp = tempdir::TempDir::new("storm-backup-empty-native").unwrap();
    let root = tmp.path().join("nas");
    let state = tmp.path().join("state");
    let dest = tmp.path().join("backups");
    fs::create_dir_all(root.join("personal")).unwrap();
    write_registry_with(&state, &root, &["personal"]);

    let out = run_native(&root, &state, &dest);
    assert!(!out.status.success(), "{out:?}");
    assert!(
        !dest.exists() || fs::read_dir(&dest).unwrap().next().is_none(),
        "nothing is written for a refused night"
    );
}

/// A fresh install has no vaults and no files, and that is a fine backup.
#[cfg(unix)]
#[test]
fn an_empty_root_with_no_vaults_still_backs_up() {
    let tmp = tempdir::TempDir::new("storm-backup-fresh").unwrap();
    let root = tmp.path().join("vaults");
    let state = tmp.path().join("state");
    let dest = tmp.path().join("backups");
    fs::create_dir_all(&root).unwrap();
    write_registry(&state, &root);

    let out = run_script(&root, &state, &dest);
    assert!(out.status.success(), "{out:?}");
    let out = run_native(&root, &state, &tmp.path().join("native"));
    assert!(out.status.success(), "{out:?}");
}
