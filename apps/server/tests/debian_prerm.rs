//! `debian/prerm` runs on upgrade too, not only on removal (decision 70).
//!
//! It used to ignore its `$1`, so every `apt upgrade` disabled storm-server and
//! the backup timer: the server did not return at the next reboot, and nightly
//! backups stopped without a word. These run the real script against a fake
//! `systemctl` that records its arguments.

use std::path::{Path, PathBuf};
use std::process::Command;

fn script() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("debian/prerm")
}

/// Runs `prerm <args>` with a recording `systemctl` first on PATH, and returns
/// every call it made. `None` where the script's systemd guard makes the
/// question meaningless: it checks for `/run/systemd/system`, which a build
/// container may not have.
fn systemctl_calls(args: &[&str]) -> Option<Vec<String>> {
    if !Path::new("/run/systemd/system").is_dir() {
        eprintln!("no /run/systemd/system here; prerm skips systemctl entirely");
        return None;
    }
    let dir = std::env::temp_dir().join(format!(
        "storm-prerm-{}-{}",
        std::process::id(),
        args.join("-")
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let log = dir.join("calls");
    let fake = dir.join("systemctl");
    std::fs::write(
        &fake,
        format!("#!/bin/sh\necho \"$*\" >> {}\n", log.display()),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let path = format!(
        "{}:{}",
        dir.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let status = Command::new("sh")
        .arg(script())
        .args(args)
        .env("PATH", path)
        .status()
        .unwrap();
    assert!(status.success(), "prerm {args:?} failed");
    let calls = std::fs::read_to_string(&log)
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect();
    let _ = std::fs::remove_dir_all(&dir);
    Some(calls)
}

#[test]
fn an_upgrade_leaves_the_server_and_the_backup_timer_alone() {
    // dpkg's own spellings: `upgrade <new-version>` for the old package's
    // prerm, `failed-upgrade <old-version>` for the new one's after a failure.
    for args in [&["upgrade", "0.2.9"][..], &["failed-upgrade", "0.2.8"][..]] {
        if let Some(calls) = systemctl_calls(args) {
            assert!(
                calls.is_empty(),
                "prerm {args:?} called systemctl: {calls:?}"
            );
        }
    }
}

#[test]
fn a_removal_stops_and_disables_both_units() {
    let Some(calls) = systemctl_calls(&["remove"]) else {
        return;
    };
    for expected in [
        "stop storm-server",
        "disable storm-server",
        "stop storm-backup.timer",
        "disable storm-backup.timer",
    ] {
        assert!(
            calls.iter().any(|c| c == expected),
            "missing {expected:?} in {calls:?}"
        );
    }
}
