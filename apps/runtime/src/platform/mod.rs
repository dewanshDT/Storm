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
