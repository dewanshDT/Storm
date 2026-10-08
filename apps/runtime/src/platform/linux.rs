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
    fn the_session_is_read_after_the_last_paren() {
        let line = "4242 (a (b) c) S 1 4242 4200 34816 4242 4194560 0";
        assert_eq!(super::stat_session(line), Some(4200));
        assert_eq!(super::stat_session("7 (x) Z 1 7 7 0"), None);
        assert_eq!(super::stat_session("garbage"), None);
    }
}
