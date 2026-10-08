//! What differs between the platforms a Runtime Host runs on (D14, AM33).
//!
//! macOS is a second platform of the **same** Runtime Host: one crate, one
//! wire protocol, one session model. What genuinely differs lives here and
//! nowhere else — default paths, the service account, and the few syscalls
//! whose behaviour differs (a PTY write's wait for room). Business logic asks
//! this module; it never matches on `target_os` itself.

use std::ffi::CStr;
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

/// Locations no workspace root may be in or contain, whatever
/// `runtime.toml` says (AM39). macOS: every person's home (and with it
/// `~/Documents`, `~/Desktop`, `~/Downloads`, iCloud Drive), removable and
/// network volumes, autofs mounts, and the data volume's own spelling of
/// all of these. A LaunchDaemon cannot answer the privacy prompts guarding
/// them, and the host never asks for Full Disk Access. Linux: none beyond
/// the unit's `ProtectHome`.
pub const PROTECTED_ROOTS: &[&str] = os::PROTECTED_ROOTS;

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

/// Every live (non-zombie) process whose session id is `sid`.
///
/// A session's leader is `setsid`'s caller, so for an agent session `sid` is
/// the agent's pid. Ending the session's process group is not enough: a
/// job-control shell (`zsh -l`, `bash -i`) puts each `cmd &` in a process
/// group of its own, inside the same session. Everything still in the session
/// is found here; only a process that called `setsid` itself leaves it, and
/// the exclusive-account sweep covers those when the host stops (AM37).
pub fn session_members(sid: i32) -> io::Result<Vec<i32>> {
    os::session_members(sid)
}

fn timespec(timeout: Duration) -> rustix::event::Timespec {
    rustix::event::Timespec {
        tv_sec: timeout.as_secs() as _,
        tv_nsec: timeout.subsec_nanos() as _,
    }
}

// ---- the exclusive account (AM37) ----------------------------------------

/// Whether `serve --exclusive-account` may run as `uid`, named `name`.
///
/// The sweep signals every process the caller may signal. As root that is
/// the whole machine; as a human it is their session. So it is allowed only
/// as the platform's dedicated service account, which by construction runs
/// nothing but this host and what this host started.
pub fn exclusive_account_allowed(uid: u32, name: Option<&str>) -> Result<(), String> {
    if uid == 0 {
        return Err(
            "--exclusive-account refuses to run as root: it would signal \
                    every process on the machine"
                .into(),
        );
    }
    match name {
        Some(name) if name == SERVICE_ACCOUNT => Ok(()),
        Some(name) => Err(format!(
            "--exclusive-account runs only as the service account {SERVICE_ACCOUNT}, \
             not as {name}: it ends every other process of its account"
        )),
        None => Err(format!(
            "--exclusive-account: uid {uid} has no account name; it runs only as \
             {SERVICE_ACCOUNT}"
        )),
    }
}

/// [`exclusive_account_allowed`] for the running process.
pub fn check_exclusive_account() -> Result<(), String> {
    let uid = rustix::process::geteuid().as_raw();
    exclusive_account_allowed(uid, account_name(uid).as_deref())
}

/// The login name of `uid`, from the password database (`getpwuid_r`).
pub fn account_name(uid: u32) -> Option<String> {
    let mut buf = vec![0 as libc::c_char; 16 * 1024];
    let mut pwd: libc::passwd = unsafe { std::mem::zeroed() };
    let mut out: *mut libc::passwd = std::ptr::null_mut();
    // SAFETY: every pointer is to a live local of the right type and size;
    // `out` is either null or points at `pwd`, whose strings live in `buf`.
    let rc = unsafe { libc::getpwuid_r(uid, &mut pwd, buf.as_mut_ptr(), buf.len(), &mut out) };
    if rc != 0 || out.is_null() || pwd.pw_name.is_null() {
        return None;
    }
    // SAFETY: getpwuid_r succeeded, so `pw_name` is a NUL-terminated string
    // inside `buf`, which is still alive.
    Some(
        unsafe { CStr::from_ptr(pwd.pw_name) }
            .to_string_lossy()
            .into_owned(),
    )
}

/// Every live process of this account except this one: what the sweep
/// signals. Listed, not `kill(-1)`: on macOS a POSIX `kill(-1)` reaches the
/// caller too, so the first real-Mac run of the sweep killed the host as it
/// started (decision 83).
pub fn sweep_targets() -> io::Result<Vec<i32>> {
    let uid = rustix::process::geteuid().as_raw();
    let me = rustix::process::getpid().as_raw_nonzero().get();
    Ok(os::account_processes(uid)?
        .into_iter()
        .filter(|&pid| pid != me)
        .collect())
}

/// Ends every other process of this account: SIGHUP, then SIGKILL after
/// `grace` to whatever is left. The counterpart of systemd's control-group
/// kill, and the only one launchd has (AM37). **Call it only after
/// [`check_exclusive_account`] passed** — `serve` does, and nothing else
/// calls this.
pub fn sweep_account(grace: Duration) {
    let signal_all = |signal| {
        for pid in sweep_targets().unwrap_or_default() {
            if let Some(pid) = rustix::process::Pid::from_raw(pid) {
                // ESRCH: it is already gone.
                let _ = rustix::process::kill_process(pid, signal);
            }
        }
    };
    signal_all(rustix::process::Signal::HUP);
    std::thread::sleep(grace);
    signal_all(rustix::process::Signal::KILL);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sweep_targets_the_accounts_other_processes_never_itself() {
        // Lists without signalling, so it is safe whoever runs the suite.
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .unwrap();
        let targets = sweep_targets().unwrap();
        let _ = child.kill();
        let _ = child.wait();
        let me = rustix::process::getpid().as_raw_nonzero().get();
        assert!(
            !targets.contains(&me),
            "the sweep would kill the host itself"
        );
        assert!(
            targets.contains(&(child.id() as i32)),
            "a process of this account is missing: {targets:?}"
        );
    }

    #[test]
    fn the_sweep_runs_only_as_the_service_account() {
        assert!(exclusive_account_allowed(0, Some("root")).is_err());
        // Root is refused whatever it is called.
        assert!(exclusive_account_allowed(0, Some(SERVICE_ACCOUNT)).is_err());
        let human = exclusive_account_allowed(501, Some("alice")).unwrap_err();
        assert!(human.contains(SERVICE_ACCOUNT), "{human}");
        assert!(exclusive_account_allowed(501, None).is_err());
        assert_eq!(
            exclusive_account_allowed(250, Some(SERVICE_ACCOUNT)),
            Ok(())
        );
    }

    #[test]
    fn the_running_account_is_found_by_uid() {
        let uid = rustix::process::geteuid().as_raw();
        // Whoever runs the tests is in the password database. (It may be the
        // service account itself — a host's own agent session runs the suite
        // — which is why no test ever calls `sweep_account`.)
        let name = account_name(uid).expect("the test runner's account");
        assert!(!name.is_empty());
        assert_eq!(account_name(0).as_deref(), Some("root"));
    }
}
