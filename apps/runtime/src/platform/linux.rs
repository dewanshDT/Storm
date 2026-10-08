//! Linux: systemd, the `.deb` and its paths (decision 77e).

use std::io;
use std::os::fd::BorrowedFd;
use std::time::Duration;

pub const NAME: &str = "linux";
pub const DEFAULT_STATE: &str = "/var/lib/storm-runtime";
pub const DEFAULT_CONFIG: &str = "/etc/storm-runtime/runtime.toml";
pub const SERVICE_ACCOUNT: &str = "storm-runtime";
/// `poll` reports a master writable with room to spare; 77c's chunk.
pub const PTY_WRITE_CHUNK: usize = 256;
/// The unit's `ProtectHome` already hides `/home`, `/root` and
/// `/run/user` (77e).
pub const PROTECTED_ROOTS: &[&str] = &[];

/// systemd gives the unit a fixed `PATH`; the host keeps it (77a).
pub fn default_path() -> Option<std::ffi::OsString> {
    None
}

const USE_THE_PACKAGE: &str = "on Linux the storm-runtime package installs and removes the \
service: `sudo apt install storm-runtime` / `sudo apt remove storm-runtime` \
(deploy/README.md, Runtime Hosts)";

pub fn install(_: &super::InstallOptions) -> anyhow::Result<()> {
    anyhow::bail!("{USE_THE_PACKAGE}")
}

pub fn uninstall(_: &super::UninstallOptions) -> anyhow::Result<()> {
    anyhow::bail!("{USE_THE_PACKAGE}")
}

/// `poll(2)` supports a PTY master on Linux, and is what 77c verified.
pub fn wait_writable(fd: BorrowedFd<'_>, timeout: Duration) -> io::Result<bool> {
    use rustix::event::{PollFd, PollFlags, poll};
    let mut fds = [PollFd::new(&fd, PollFlags::OUT)];
    Ok(poll(&mut fds, Some(&super::timespec(timeout)))? > 0)
}

/// `/proc/<pid>/stat`: field 6 is the session. The command name (field 2)
/// may contain spaces and parentheses, so fields are counted after its last
/// `)`. Zombies (`Z`, `X`) are already gone for our purposes.
pub fn session_members(sid: i32) -> io::Result<Vec<i32>> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir("/proc")? {
        let Ok(entry) = entry else { continue };
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|n| n.parse::<i32>().ok())
        else {
            continue;
        };
        // A process may exit between the listing and the read.
        let Ok(stat) = std::fs::read_to_string(entry.path().join("stat")) else {
            continue;
        };
        if stat_session(&stat) == Some(sid) {
            out.push(pid);
        }
    }
    Ok(out)
}

/// Every live process whose real or effective uid is `uid`: the ones a
/// process of that account may signal. `/proc/<pid>/status` has
/// `Uid: real effective saved fs`.
pub fn account_processes(uid: u32) -> io::Result<Vec<i32>> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir("/proc")? {
        let Ok(entry) = entry else { continue };
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|n| n.parse::<i32>().ok())
        else {
            continue;
        };
        let Ok(status) = std::fs::read_to_string(entry.path().join("status")) else {
            continue;
        };
        if status_owned_by(&status, uid) {
            out.push(pid);
        }
    }
    Ok(out)
}

fn status_owned_by(status: &str, uid: u32) -> bool {
    let field = |name: &str| {
        status
            .lines()
            .find_map(|l| l.strip_prefix(name))
            .map(str::trim)
    };
    if field("State:").is_some_and(|s| s.starts_with('Z') || s.starts_with('X')) {
        return false;
    }
    field("Uid:").is_some_and(|ids| {
        ids.split_whitespace()
            .take(2)
            .any(|id| id.parse() == Ok(uid))
    })
}

/// The session of a live process, from its `stat` line; `None` for a zombie.
fn stat_session(stat: &str) -> Option<i32> {
    let rest = &stat[stat.rfind(')')? + 1..];
    let mut fields = rest.split_whitespace();
    let state = fields.next()?;
    if state == "Z" || state == "X" {
        return None;
    }
    // ppid, pgrp, then session.
    fields.nth(2)?.parse().ok()
}

#[cfg(test)]
mod tests {
    #[test]
    fn linux_keeps_the_units_path() {
        assert_eq!(super::default_path(), None);
    }

    #[test]
    fn install_and_uninstall_point_at_the_package() {
        let e = super::install(&Default::default()).unwrap_err().to_string();
        assert!(e.contains("apt install storm-runtime"), "{e}");
        let e = super::uninstall(&Default::default())
            .unwrap_err()
            .to_string();
        assert!(e.contains("apt remove storm-runtime"), "{e}");
    }

    #[test]
    fn an_owner_is_the_real_or_effective_uid_of_a_live_process() {
        let s = |state: &str, ids: &str| format!("Name:\tx\nState:\t{state}\nUid:\t{ids}\n");
        assert!(super::status_owned_by(
            &s("S (sleeping)", "999\t999\t999\t999"),
            999
        ));
        assert!(super::status_owned_by(
            &s("R (running)", "0\t999\t0\t0"),
            999
        ));
        assert!(!super::status_owned_by(
            &s("S (sleeping)", "0\t0\t999\t0"),
            999
        ));
        assert!(!super::status_owned_by(
            &s("Z (zombie)", "999\t999\t999\t999"),
            999
        ));
    }

    #[test]
    fn the_session_is_read_after_the_last_paren() {
        let line = "4242 (a (b) c) S 1 4242 4200 34816 4242 4194560 0";
        assert_eq!(super::stat_session(line), Some(4200));
        assert_eq!(super::stat_session("7 (x) Z 1 7 7 0"), None);
        assert_eq!(super::stat_session("garbage"), None);
    }
}
