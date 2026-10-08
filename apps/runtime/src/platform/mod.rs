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
use macos as os;

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
