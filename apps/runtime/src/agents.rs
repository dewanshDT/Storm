//! Agent CLIs this host can report on, and install and manage itself
//! (decision 133).
//!
//! Storm's own copies live under `<state>/agents/managed`, whose `bin` comes
//! first on the host's search path, so a CLI the person installed elsewhere
//! still works and Storm only ever touches its own. What the host reports per
//! agent: its version (from `--version`, probed once per change of the
//! resolved binary, never during a `hello`), where it came from, and whether
//! this host could install it now.

use std::collections::HashMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

/// The agents Storm knows how to install (decision 133-1). `shell` is not one.
pub const MANAGEABLE: [&str; 5] = ["claude-code", "opencode", "codex", "cursor", "command-code"];

/// How long `--version` may take.
const PROBE_DEADLINE: Duration = Duration::from_secs(5);

pub fn managed_dir(state_dir: &Path) -> PathBuf {
    state_dir.join("agents").join("managed")
}

pub fn managed_bin(state_dir: &Path) -> PathBuf {
    managed_dir(state_dir).join("bin")
}

/// The Node major version an npm agent needs (decision 133, choice 4), or
/// `None` for an agent that is not installed through npm.
pub fn node_needed(agent: &str) -> Option<u32> {
    match agent {
        "codex" => Some(16),
        "command-code" => Some(22),
        _ => None,
    }
}

/// Why this host cannot install `agent` now, or `None` when it can.
pub fn install_blocked(
    agent: &str,
    enabled: bool,
    node_major: Option<u32>,
) -> Option<&'static str> {
    if !enabled {
        return Some("disabled");
    }
    if !cfg!(unix) {
        // 133-Q1: Linux and macOS first.
        return Some("unsupported_platform");
    }
    match (node_needed(agent), node_major) {
        (None, _) => None,
        (Some(_), None) => Some("no_node"),
        (Some(need), Some(have)) if have < need => Some("node_too_old"),
        _ => None,
    }
}

/// The version in a CLI's `--version` output: its first token that starts
/// with a digit and holds a dot (`2.1.306 (Claude Code)`, `codex-cli
/// 0.163.0`, `2026.10.22-27dcf5f`).
pub fn parse_version(out: &str) -> Option<String> {
    out.split(|c: char| c.is_whitespace() || c == '(' || c == ')' || c == ',')
        .map(|t| t.trim_start_matches('v'))
        .find(|t| t.starts_with(|c: char| c.is_ascii_digit()) && t.contains('.'))
        .map(|t| t.chars().take(64).collect())
}

/// `node --version` on `path`, as its major version.
pub fn node_major(path: Option<&OsStr>) -> Option<u32> {
    let node = crate::pty::resolve(OsStr::new("node"), path)?;
    let out = run_version(&node, path)?;
    parse_version(&out)?.split('.').next()?.parse().ok()
}

/// Runs `<binary> --version` on `path`, killed after [`PROBE_DEADLINE`]. It
/// runs in a process group of its own, which is killed once it exits or
/// times out: whatever it started would otherwise outlive it as the host's
/// account, and on macOS nothing else ends it (the acceptance stub starts a
/// child that ignores SIGHUP, as #270 found for `claude plugin list`).
pub(crate) fn run_version(binary: &Path, path: Option<&OsStr>) -> Option<String> {
    let mut command = Command::new(binary);
    command
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if let Some(path) = path {
        command.env("PATH", path);
    }
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    let mut child = command.spawn().ok()?;
    let deadline = Instant::now() + PROBE_DEADLINE;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                end_group(&child);
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    // Before reading: a leftover would hold the pipe open.
    end_group(&child);
    let mut out = String::new();
    use std::io::Read;
    child
        .stdout
        .take()?
        .take(4096)
        .read_to_string(&mut out)
        .ok()?;
    Some(out)
}

#[cfg(unix)]
fn end_group(child: &std::process::Child) {
    if let Some(pid) = rustix::process::Pid::from_raw(child.id() as i32) {
        let _ = rustix::process::kill_process_group(pid, rustix::process::Signal::KILL);
    }
}

#[cfg(not(unix))]
fn end_group(_: &std::process::Child) {}

/// One agent as last probed.
#[derive(Debug, Clone, PartialEq)]
struct Probe {
    binary: PathBuf,
    modified: Option<SystemTime>,
    version: Option<String>,
}

/// What the host last learned about its agents' binaries.
#[derive(Default)]
pub struct Inventory {
    probes: Mutex<HashMap<String, Probe>>,
    node: Mutex<Option<u32>>,
    /// Each managed agent's newest version, from the daily check (133-5).
    latest: Mutex<HashMap<String, String>>,
    /// Automatic updates that failed in a row, per agent.
    failures: Mutex<HashMap<String, u32>>,
}

/// After this many failed automatic updates in a row, an agent waits for one
/// started from the app to succeed (133-5).
const AUTO_FAILURES_MAX: u32 = 3;

impl Inventory {
    /// Probes every agent whose binary changed since the last time, and Node.
    /// Returns whether anything the host reports changed. Blocking.
    pub fn refresh(&self, agents: &[(String, Option<PathBuf>)], path: Option<&OsStr>) -> bool {
        let mut changed = false;
        for (id, binary) in agents {
            let current = self.probes.lock().unwrap().get(id).cloned();
            let Some(binary) = binary else {
                changed |= self.probes.lock().unwrap().remove(id).is_some();
                continue;
            };
            let modified = std::fs::metadata(binary).and_then(|m| m.modified()).ok();
            if current
                .as_ref()
                .is_some_and(|p| &p.binary == binary && p.modified == modified)
            {
                continue;
            }
            let probe = Probe {
                binary: binary.clone(),
                modified,
                version: run_version(binary, path).and_then(|o| parse_version(&o)),
            };
            changed |= current.as_ref().map(|p| &p.version) != Some(&probe.version);
            self.probes.lock().unwrap().insert(id.clone(), probe);
        }
        let node = node_major(path);
        let mut was = self.node.lock().unwrap();
        changed |= *was != node;
        *was = node;
        changed
    }

    pub fn version(&self, agent: &str) -> Option<String> {
        self.probes.lock().unwrap().get(agent)?.version.clone()
    }

    pub fn node_major(&self) -> Option<u32> {
        *self.node.lock().unwrap()
    }

    /// Records `agent`'s newest version; whether it changed.
    pub fn set_latest(&self, agent: &str, version: &str) -> bool {
        self.latest
            .lock()
            .unwrap()
            .insert(agent.to_string(), version.to_string())
            .as_deref()
            != Some(version)
    }

    pub fn latest(&self, agent: &str) -> Option<String> {
        self.latest.lock().unwrap().get(agent).cloned()
    }

    pub fn record_update(&self, agent: &str, ok: bool) {
        let mut failures = self.failures.lock().unwrap();
        if ok {
            failures.remove(agent);
        } else {
            *failures.entry(agent.to_string()).or_default() += 1;
        }
    }

    pub fn may_auto_update(&self, agent: &str) -> bool {
        self.failures
            .lock()
            .unwrap()
            .get(agent)
            .copied()
            .unwrap_or(0)
            < AUTO_FAILURES_MAX
    }
}

/// `"storm"` for a binary under the managed folder, `"system"` otherwise.
pub fn source(binary: &Path, state_dir: &Path) -> &'static str {
    let managed = managed_dir(state_dir);
    let canonical = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    if binary.starts_with(&managed) || canonical(binary).starts_with(canonical(&managed)) {
        "storm"
    } else {
        "system"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_are_read_from_each_clis_own_shape() {
        for (out, want) in [
            ("2.1.306 (Claude Code)\n", "2.1.306"),
            ("1.18.31\n", "1.18.31"),
            ("codex-cli 0.163.0\n", "0.163.0"),
            ("2026.10.22-27dcf5f\n", "2026.10.22-27dcf5f"),
            ("command-code v1.87.3\n", "1.87.3"),
            ("v22.11.0\n", "22.11.0"),
        ] {
            assert_eq!(parse_version(out).as_deref(), Some(want), "{out:?}");
        }
        assert_eq!(parse_version("no version here"), None);
    }

    #[test]
    fn an_install_is_blocked_only_for_a_reason() {
        assert_eq!(
            install_blocked("claude-code", false, None),
            Some("disabled")
        );
        if cfg!(unix) {
            assert_eq!(install_blocked("claude-code", true, None), None);
            assert_eq!(install_blocked("codex", true, None), Some("no_node"));
            assert_eq!(
                install_blocked("command-code", true, Some(20)),
                Some("node_too_old")
            );
            assert_eq!(install_blocked("command-code", true, Some(22)), None);
        } else {
            assert_eq!(
                install_blocked("claude-code", true, None),
                Some("unsupported_platform")
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_binary_is_probed_once_until_it_changes() {
        use std::os::unix::fs::PermissionsExt;
        let t = tempfile::tempdir().unwrap();
        let bin = t.path().join("agent");
        let count = t.path().join("count");
        let write = |version: &str| {
            std::fs::write(
                &bin,
                format!(
                    "#!/bin/sh\necho x >> '{}'\necho '{version}'\n",
                    count.display()
                ),
            )
            .unwrap();
            std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        };
        write("1.0.0");
        let inv = Inventory::default();
        let agents = vec![("codex".to_string(), Some(bin.clone()))];
        assert!(inv.refresh(&agents, None));
        assert_eq!(inv.version("codex").as_deref(), Some("1.0.0"));
        inv.refresh(&agents, None);
        assert_eq!(
            std::fs::read_to_string(&count).unwrap().lines().count(),
            1,
            "probed once"
        );

        std::thread::sleep(Duration::from_millis(1100));
        write("1.1.0");
        assert!(inv.refresh(&agents, None));
        assert_eq!(inv.version("codex").as_deref(), Some("1.1.0"));

        assert!(inv.refresh(&[("codex".to_string(), None)], None));
        assert_eq!(inv.version("codex"), None, "uninstalled");
    }

    #[test]
    fn a_managed_binary_is_storms() {
        let t = tempfile::tempdir().unwrap();
        let bin = managed_bin(t.path()).join("claude");
        assert_eq!(source(&bin, t.path()), "storm");
        assert_eq!(
            source(Path::new("/usr/local/bin/claude"), t.path()),
            "system"
        );
    }

    #[test]
    fn three_failed_automatic_updates_stop_them_until_one_succeeds() {
        let inv = Inventory::default();
        assert!(inv.may_auto_update("codex"));
        for _ in 0..3 {
            inv.record_update("codex", false);
        }
        assert!(!inv.may_auto_update("codex"));
        inv.record_update("codex", true);
        assert!(inv.may_auto_update("codex"));
        assert!(inv.set_latest("codex", "1.0"));
        assert!(!inv.set_latest("codex", "1.0"));
        assert_eq!(inv.latest("codex").as_deref(), Some("1.0"));
    }

    #[cfg(unix)]
    #[test]
    fn nothing_a_version_probe_started_outlives_it() {
        use std::os::unix::fs::PermissionsExt;
        let t = tempfile::tempdir().unwrap();
        let bin = t.path().join("agent");
        let left = t.path().join("left");
        // As the acceptance stub does: print, leave a HUP-ignoring child, and
        // hang until the deadline.
        std::fs::write(
            &bin,
            format!(
                "#!/bin/sh\necho 1.0.0\n(trap '' HUP; exec sleep 600) & echo $! > '{}'\nexec sleep 600\n",
                left.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(run_version(&bin, None), None, "it timed out");
        let pid: i32 = std::fs::read_to_string(&left)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        let alive = || {
            rustix::process::test_kill_process(rustix::process::Pid::from_raw(pid).unwrap()).is_ok()
        };
        let until = Instant::now() + Duration::from_secs(5);
        while alive() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(!alive(), "pid {pid} outlived the probe");
    }
}
