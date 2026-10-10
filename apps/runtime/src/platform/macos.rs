//! macOS: launchd, the `_stormruntime` account and `/Library/StormRuntime`
//! (D14, AM33–AM35).

use std::io;
use std::os::fd::{AsRawFd, BorrowedFd};
use std::time::Duration;

pub const NAME: &str = "macos";
pub const DEFAULT_STATE: &str = super::launchd::STATE;
pub const DEFAULT_CONFIG: &str = super::launchd::CONFIG;
pub const SERVICE_ACCOUNT: &str = "_stormruntime";
/// xnu's `ptcselect` reports a master writable while the slave's input
/// queue is below `TTYHOG - 2`, which may be room for a single byte, and
/// `ptcwrite` then sleeps until the whole write fits. One byte per wait
/// never blocks.
pub const PTY_WRITE_CHUNK: usize = 1;
pub const PROTECTED_ROOTS: &[&str] = super::launchd::PROTECTED_ROOTS;

pub use super::macos_service::{install, uninstall};

/// launchd's `PATH` is bare, so the host supplies AM38's, rooted at the
/// account's `HOME` (which the plist sets).
pub fn default_path() -> Option<std::ffi::OsString> {
    let home = std::env::var("HOME")
        .ok()
        .filter(|h| !h.is_empty())
        .unwrap_or_else(|| {
            super::launchd::Layout::standard()
                .home
                .display()
                .to_string()
        });
    Some(super::launchd::path_for_home(&home).into())
}

/// `select(2)`, not `poll(2)`: macOS's `poll` "does not support devices"
/// (its man page, BUGS), and answers a PTY master with `POLLNVAL` at once. A
/// wait built on it never waits, and the write after it blocks for as long as
/// the agent ignores its input — the hang 77c's deadline exists to prevent.
pub fn wait_writable(fd: BorrowedFd<'_>, timeout: Duration) -> io::Result<bool> {
    use rustix::event::{fd_set_insert, fd_set_num_elements, select};
    let raw = fd.as_raw_fd();
    let mut set = vec![Default::default(); fd_set_num_elements(1, raw + 1)];
    fd_set_insert(&mut set, raw);
    // SAFETY: the only fd in the set is `fd`, borrowed and therefore open for
    // the duration of the call.
    let ready = unsafe {
        select(
            raw + 1,
            None,
            Some(&mut set),
            None,
            Some(&super::timespec(timeout)),
        )?
    };
    // The set holds one fd, so any readiness is its.
    Ok(ready > 0)
}

/// `proc_listallpids`, then `getsid` for each. A pid that exited in between
/// (`ESRCH`) or is not ours to ask about (`EPERM`) is skipped; a zombie
/// (`SZOMB`) is already gone for our purposes.
pub fn session_members(sid: i32) -> io::Result<Vec<i32>> {
    Ok(all_pids()?
        .into_iter()
        // SAFETY: getsid has no memory-safety preconditions.
        .filter(|&pid| unsafe { libc::getsid(pid) } == sid)
        .filter(|&pid| !zombie(pid))
        .collect())
}

/// Every live process whose real or effective uid is `uid`.
pub fn account_processes(uid: u32) -> io::Result<Vec<i32>> {
    Ok(all_pids()?
        .into_iter()
        .filter(|&pid| {
            bsdinfo(pid).is_some_and(|i| {
                i.pbi_status != libc::SZOMB && (i.pbi_uid == uid || i.pbi_ruid == uid)
            })
        })
        .collect())
}

/// The environment `pid` was started with, NUL-separated, from
/// `KERN_PROCARGS2`: readable for the processes of one's own account. The
/// buffer is `argc`, the executable path, padding NULs, `argc` arguments, then
/// the environment, which ends at the first empty string.
pub fn environment(pid: i32) -> Option<Vec<u8>> {
    let mut argmax: libc::c_int = 0;
    let mut size = std::mem::size_of::<libc::c_int>();
    let mut mib = [libc::CTL_KERN, libc::KERN_ARGMAX];
    // SAFETY: `argmax` is a live c_int and `size` is its size.
    let rc = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            2,
            (&mut argmax as *mut libc::c_int).cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if rc != 0 || argmax <= 0 {
        return None;
    }
    let mut buf = vec![0u8; argmax as usize];
    let mut size = buf.len();
    let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid];
    // SAFETY: `buf` is a live buffer of `size` bytes.
    let rc = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            3,
            buf.as_mut_ptr().cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if rc != 0 || size < std::mem::size_of::<libc::c_int>() {
        return None;
    }
    buf.truncate(size);
    let argc = i32::from_ne_bytes(buf[..4].try_into().ok()?);
    let mut rest = &buf[4..];
    // The executable path, then the NULs that pad it.
    let path_end = rest.iter().position(|&b| b == 0)?;
    rest = &rest[path_end..];
    let first = rest.iter().position(|&b| b != 0)?;
    rest = &rest[first..];
    for _ in 0..argc {
        let end = rest.iter().position(|&b| b == 0)?;
        rest = &rest[end + 1..];
    }
    // The environment ends at the first empty string.
    let end = rest
        .windows(2)
        .position(|w| w == [0, 0])
        .map_or(rest.len(), |at| at + 1);
    Some(rest[..end].to_vec())
}

fn all_pids() -> io::Result<Vec<i32>> {
    // SAFETY: a null buffer asks for the count only.
    let count = unsafe { libc::proc_listallpids(std::ptr::null_mut(), 0) };
    if count < 0 {
        return Err(io::Error::last_os_error());
    }
    // Room for processes started since the count.
    let mut pids = vec![0 as libc::pid_t; count as usize + 64];
    let bytes = (pids.len() * std::mem::size_of::<libc::pid_t>()) as libc::c_int;
    // SAFETY: `pids` is a live buffer of exactly `bytes` bytes.
    let n = unsafe { libc::proc_listallpids(pids.as_mut_ptr().cast(), bytes) };
    if n < 0 {
        return Err(io::Error::last_os_error());
    }
    pids.truncate(n as usize);
    pids.retain(|&pid| pid > 0);
    Ok(pids)
}

fn bsdinfo(pid: i32) -> Option<libc::proc_bsdinfo> {
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
    // SAFETY: `info` is a live `proc_bsdinfo` of `size` bytes.
    let got = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDTBSDINFO,
            0,
            (&mut info as *mut libc::proc_bsdinfo).cast(),
            size,
        )
    };
    (got == size).then_some(info)
}

fn zombie(pid: i32) -> bool {
    bsdinfo(pid).is_some_and(|i| i.pbi_status == libc::SZOMB)
}
