//! What differs between the platforms a Runtime Host runs on (D14, AM33).
//!
//! macOS is a second platform of the **same** Runtime Host: one crate, one
//! wire protocol, one session model. What genuinely differs lives here and
//! nowhere else — default paths, the service account, and the few syscalls
//! whose behaviour differs (a PTY write's wait for room). Business logic asks
//! this module; it never matches on `target_os` itself.

use std::io;
use std::os::fd::BorrowedFd;
use std::time::Duration;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
use linux as os;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
mod macos_service;
#[cfg(target_os = "macos")]
use macos as os;

pub mod launchd;

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
compile_error!("storm-runtime runs on Linux and macOS (D14)");

/// The platform's name, as the host reports it and the docs use it.
pub const NAME: &str = os::NAME;

/// The host's state directory: its key, `host.json`, `sessions.json`, the
/// MCP socket. Linux: the systemd unit's `StateDirectory`. macOS: AM33.
pub const DEFAULT_STATE: &str = os::DEFAULT_STATE;

/// The operator's `runtime.toml` (freeze §5.7).
pub const DEFAULT_CONFIG: &str = os::DEFAULT_CONFIG;

/// The dedicated account the packaged service runs as (77e, AM34).
pub const SERVICE_ACCOUNT: &str = os::SERVICE_ACCOUNT;

/// The `PATH` agents get, and providers are looked for on, when
/// `runtime.toml` names none (AM38). `None` keeps the inherited one: Linux,
/// where systemd's is already fixed. macOS: launchd's is bare, so the host
/// supplies its own, rooted at the account's `HOME`.
pub fn default_path() -> Option<std::ffi::OsString> {
    os::default_path()
}

/// The variable that carries a session's `PATH` past a login shell's
/// profile (F3). `/etc/zprofile` on macOS runs `path_helper`, which moves
/// the system directories ahead of the ones the host chose, so a login
/// `zsh -l` would find `/usr/bin/git` before Homebrew's. The managed
/// `.zprofile` puts this back.
pub const SESSION_PATH: &str = "STORM_RUNTIME_PATH";

/// The first line of the `.zprofile` the host manages. A `.zprofile` without
/// it is the operator's, and is left alone.
const ZPROFILE_MARKER: &str = "# Managed by storm-runtime";

const ZPROFILE: &str = "# Managed by storm-runtime: rewritten when the host starts. Delete this\n\
# line to keep your own changes, and the host will leave the file alone.\n\
#\n\
# /etc/zprofile's path_helper reorders PATH in a login shell; the host's\n\
# order (its own bin, then Homebrew, then the system) is restored here.\n\
[ -n \"$STORM_RUNTIME_PATH\" ] && export PATH=\"$STORM_RUNTIME_PATH\"\n";

/// Writes the managed `.zprofile` into `home` (F3), unless a `.zprofile` the
/// operator wrote is there. Returns whether it wrote one.
pub fn write_managed_zprofile(home: &std::path::Path) -> io::Result<bool> {
    let path = home.join(".zprofile");
    match std::fs::read_to_string(&path) {
        Ok(existing) if existing == ZPROFILE => return Ok(false),
        Ok(existing) if !existing.starts_with(ZPROFILE_MARKER) => return Ok(false),
        Ok(_) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    let tmp = home.join(".zprofile.tmp");
    std::fs::write(&tmp, ZPROFILE)?;
    std::fs::rename(&tmp, &path)?;
    Ok(true)
}

/// Prepares the service account's home before sessions start. macOS: the
/// managed `.zprofile` (F3), only in the account's own home, never in a
/// person's who runs `serve` by hand. Linux: nothing; systemd starts no
/// login profile with a reordering `path_helper`.
pub fn prepare_home() {
    #[cfg(target_os = "macos")]
    {
        let home = launchd::Layout::standard().home;
        if std::env::var_os("HOME").is_some_and(|h| std::path::Path::new(&h) == home) {
            match write_managed_zprofile(&home) {
                Ok(true) => tracing::info!(home = %home.display(), "wrote the managed .zprofile"),
                Ok(false) => {}
                Err(e) => tracing::warn!(error = %e, "could not write the managed .zprofile"),
            }
        }
    }
}

/// `storm-runtime install` (AM35).
#[derive(Debug, Default)]
pub struct InstallOptions {
    /// Who the workspace root is shared with. Defaults to `$SUDO_USER`.
    pub operator: Option<String>,
}

/// `storm-runtime uninstall` (AM35).
#[derive(Debug, Default)]
pub struct UninstallOptions {
    /// Also remove the identity, state, logs, config and the account.
    /// Workspaces are never removed.
    pub purge: bool,
}

/// Installs this binary as the platform's system service. macOS: the
/// `_stormruntime` account, `/Library/StormRuntime` and the LaunchDaemon.
/// Linux: the `.deb` does this, so it refuses with a pointer to it.
pub fn install(options: &InstallOptions) -> anyhow::Result<()> {
    os::install(options)
}

/// Removes the system service `install` set up.
pub fn uninstall(options: &UninstallOptions) -> anyhow::Result<()> {
    os::uninstall(options)
}

/// How this host appears in the app when enrollment is given no `--name`:
/// the kernel's node name, without macOS's `.local` suffix (AM40).
pub fn host_name() -> String {
    let uname = rustix::system::uname();
    let node = uname.nodename().to_string_lossy();
    let node = node.trim();
    let node = node.strip_suffix(".local").unwrap_or(node);
    let name: String = node.chars().take(64).collect();
    if name.is_empty() {
        "runtime-host".into()
    } else {
        name
    }
}

/// The most a PTY write may carry after [`wait_writable`] said yes, so that
/// the write itself never blocks. Linux: 256 bytes, what 77c verified. macOS:
/// one byte, because its "writable" means room for at least one, and a
/// blocking write of more sleeps until all of it fits — the hang the
/// deadline exists to prevent (found on a real Mac, decision 83).
pub(crate) const PTY_WRITE_CHUNK: usize = os::PTY_WRITE_CHUNK;

/// Waits up to `timeout` for `fd` (a PTY master) to accept a write. Returns
/// whether it can. The bounded input write of decision 77c rests on this:
/// without it, an agent that stops reading holds its session forever.
pub(crate) fn wait_writable(fd: BorrowedFd<'_>, timeout: Duration) -> io::Result<bool> {
    os::wait_writable(fd, timeout)
}

fn timespec(timeout: Duration) -> rustix::event::Timespec {
    rustix::event::Timespec {
        tv_sec: timeout.as_secs() as _,
        tv_nsec: timeout.subsec_nanos() as _,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// F3: the managed `.zprofile` is written, kept up to date, and never
    /// written over an operator's own.
    #[test]
    fn the_managed_zprofile_never_replaces_the_operators() {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join(".zprofile");
        assert!(write_managed_zprofile(home.path()).unwrap());
        let written = std::fs::read_to_string(&path).unwrap();
        assert!(written.starts_with(ZPROFILE_MARKER));
        assert!(written.contains("export PATH=\"$STORM_RUNTIME_PATH\""));
        assert!(!write_managed_zprofile(home.path()).unwrap(), "unchanged");

        std::fs::write(&path, format!("{ZPROFILE_MARKER}: an older one\n")).unwrap();
        assert!(write_managed_zprofile(home.path()).unwrap(), "updated");

        std::fs::write(&path, "export PATH=/mine\n").unwrap();
        assert!(!write_managed_zprofile(home.path()).unwrap());
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "export PATH=/mine\n"
        );
    }

    /// F3: a login zsh — whose system profile may reorder PATH, as macOS's
    /// `path_helper` does — ends up with the session's PATH. Skipped where
    /// there is no zsh.
    #[test]
    fn a_login_zsh_keeps_the_sessions_path() {
        let Some(zsh) = ["/bin/zsh", "/usr/bin/zsh"]
            .into_iter()
            .find(|p| std::path::Path::new(p).exists())
        else {
            return;
        };
        let home = tempfile::tempdir().unwrap();
        write_managed_zprofile(home.path()).unwrap();
        let out = std::process::Command::new(zsh)
            .args(["-l", "-c", "echo $PATH"])
            .env_clear()
            .env("HOME", home.path())
            .env("ZDOTDIR", home.path())
            .env("PATH", "/usr/bin:/bin:/first")
            .env(SESSION_PATH, "/first:/usr/bin:/bin")
            .output()
            .unwrap();
        assert_eq!(
            String::from_utf8_lossy(&out.stdout).trim(),
            "/first:/usr/bin:/bin"
        );
    }
}
