//! What differs between the platforms a Runtime Host runs on (D14, AM33;
//! D16, AM48).
//!
//! macOS and Windows are further platforms of the **same** Runtime Host: one
//! crate, one wire protocol, one session model. What genuinely differs lives
//! here and nowhere else — default paths, the service account, private
//! files, the local socket, and the few syscalls whose behaviour differs.
//! Business logic asks this module; it never matches on `target_os` itself.
//! The terminal carrier is `crate::pty`, per platform the same way.

use std::io;

#[cfg(unix)]
mod posix;
#[cfg(unix)]
use posix as shared;
#[cfg(unix)]
pub use posix::{
    account_name, check_exclusive_account, exclusive_account_allowed, host_name, session_members,
    sweep_account, sweep_targets, tagged_processes,
};
#[cfg(unix)]
pub(crate) use posix::{wait_readable, wait_writable};

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

#[cfg(windows)]
mod windows;
#[cfg(windows)]
mod windows_service;
#[cfg(windows)]
use windows as os;
#[cfg(windows)]
use windows as shared;
#[cfg(windows)]
pub(crate) use windows::{OwnerOnly, wide};
#[cfg(windows)]
pub use windows::{
    check_exclusive_account, current_account, exclusive_account_allowed, host_name, sweep_account,
    sweep_targets,
};
#[cfg(windows)]
pub use windows_service::{enable_and_start, service_main};

pub mod ipc;
pub mod launchd;
pub mod wincmd;
pub mod winsvc;

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
compile_error!("storm-runtime runs on Linux, macOS and Windows (D14, D16)");

/// The platform's name, as the host reports it and the docs use it.
pub const NAME: &str = os::NAME;

/// The host's state directory: its key, `host.json`, `sessions.json`, the
/// MCP socket. Linux: the systemd unit's `StateDirectory`. macOS: AM33.
/// Windows: AM48.
pub const DEFAULT_STATE: &str = os::DEFAULT_STATE;

/// The operator's `runtime.toml` (freeze §5.7).
pub const DEFAULT_CONFIG: &str = os::DEFAULT_CONFIG;

/// The dedicated account the packaged service runs as (77e, AM34, AM49).
pub const SERVICE_ACCOUNT: &str = os::SERVICE_ACCOUNT;

/// The environment variable that marks every process a session started:
/// `STORM_RUNTIME_SESSION=<session id>`. Children inherit it, so it follows
/// what left the session with `setsid` and was reparented — a daemonized
/// agent server (F1) — where the session id no longer reaches. On Windows a
/// Job Object holds the session instead (AM52); the tag is still set.
pub const SESSION_TAG: &str = "STORM_RUNTIME_SESSION";

/// The `PATH` agents get, and providers are looked for on, when
/// `runtime.toml` names none (AM38). `None` keeps the inherited one: Linux,
/// where systemd's is already fixed. macOS: launchd's is bare, so the host
/// supplies its own, rooted at the account's `HOME`. Windows: the account's
/// own bins, then the machine `PATH` from the registry (AM53).
pub fn default_path() -> Option<std::ffi::OsString> {
    os::default_path()
}

/// Where a tool a person runs (not the service) keeps its own state, such
/// as `mcp-connect`'s relay cache. Linux: XDG state. macOS: Application
/// Support. Windows: `%LOCALAPPDATA%`. `None` without a home.
pub fn user_state_dir() -> Option<std::path::PathBuf> {
    os::user_state_dir()
}

/// Locations no workspace root may be in or contain, whatever
/// `runtime.toml` says (AM39). macOS: every person's home (and with it
/// `~/Documents`, `~/Desktop`, `~/Downloads`, iCloud Drive), removable and
/// network volumes, autofs mounts, and the data volume's own spelling of
/// all of these. A LaunchDaemon cannot answer the privacy prompts guarding
/// them, and the host never asks for Full Disk Access. Linux: none beyond
/// the unit's `ProtectHome`. Windows: none here; its locations depend on the
/// machine, so [`refused_root`] decides.
pub const PROTECTED_ROOTS: &[&str] = os::PROTECTED_ROOTS;

/// Why `root` (canonical) may not be a workspace root on this machine, beyond
/// [`PROTECTED_ROOTS`], or `None`. Windows: profiles, network shares and
/// removable drives (AM54). Elsewhere: always `None`.
pub fn refused_root(root: &std::path::Path) -> Option<String> {
    shared::refused_root(root)
}

/// The `shell` provider's command and arguments (§9.2, AM53): `$SHELL -l` on
/// Linux and macOS; PowerShell 7 if `path` has it, else Windows PowerShell,
/// on Windows.
pub fn session_shell(
    path: Option<&std::ffi::OsStr>,
) -> (std::ffi::OsString, Vec<std::ffi::OsString>) {
    shared::session_shell(path)
}

/// Creates `dir` and any missing parents, private to this host's account:
/// `0700`, created with that mode (A2) on Unix; a protected DACL of SYSTEM,
/// Administrators and the account on Windows (AM49). An existing directory
/// is left as it is.
pub fn create_private_dir(dir: &std::path::Path) -> io::Result<()> {
    shared::create_private_dir(dir)
}

/// [`create_private_dir`], then makes `dir` private even if it already
/// existed.
pub fn tighten_private_dir(dir: &std::path::Path) -> io::Result<()> {
    shared::tighten_private_dir(dir)
}

/// Opens `path` with `options`, private to this host's account: created
/// `0600` on Unix; given AM49's protected DACL on Windows.
pub fn open_private(
    path: &std::path::Path,
    options: &mut std::fs::OpenOptions,
) -> io::Result<std::fs::File> {
    shared::open_private(path, options)
}

/// Whether only this host's account (and, on Windows, SYSTEM and
/// Administrators) can read `path`: the env-file rule. Unix: no group or
/// other bit. Windows: no allow entry for `Everyone`, `Authenticated Users`
/// or `Users` (AM49).
pub fn is_private(path: &std::path::Path) -> io::Result<bool> {
    shared::is_private(path)
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
    crate::fsutil::write_atomic(&path, ZPROFILE.as_bytes(), crate::fsutil::Access::Default)?;
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
    /// Who the workspace root is shared with. Defaults to `$SUDO_USER`; on
    /// Windows, the user who opened the elevated shell.
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
/// Windows: `C:\ProgramData\StormRuntime` and the `StormRuntime` service
/// (AM50). Linux: the `.deb` does this, so it refuses with a pointer to it.
pub fn install(options: &InstallOptions) -> anyhow::Result<()> {
    os::install(options)
}

/// Removes the system service `install` set up.
pub fn uninstall(options: &UninstallOptions) -> anyhow::Result<()> {
    os::uninstall(options)
}

/// Asserts `path` is private: exactly `mode` on Unix (A2), AM49's DACL on
/// Windows.
#[cfg(test)]
#[track_caller]
pub(crate) fn assert_private(path: &std::path::Path, mode: u32) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let actual = std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(actual, mode, "{} is {actual:o}", path.display());
    }
    #[cfg(windows)]
    {
        let _ = mode;
        assert!(
            is_private(path).unwrap(),
            "{} is readable by others",
            path.display()
        );
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
