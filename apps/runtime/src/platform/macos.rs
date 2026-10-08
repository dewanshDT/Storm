//! macOS: launchd, the `_stormruntime` account and `/Library/StormRuntime`
//! (D14, AM33–AM35).

use std::io;
use std::os::fd::{AsRawFd, BorrowedFd};
use std::time::Duration;

pub const NAME: &str = "macos";
pub const DEFAULT_STATE: &str = "/Library/StormRuntime/state";
pub const DEFAULT_CONFIG: &str = "/Library/StormRuntime/runtime.toml";
pub const SERVICE_ACCOUNT: &str = "_stormruntime";
/// xnu's `ptcselect` reports a master writable while the slave's input
/// queue is below `TTYHOG - 2`, which may be room for a single byte, and
/// `ptcwrite` then sleeps until the whole write fits. One byte per wait
/// never blocks.
pub const PTY_WRITE_CHUNK: usize = 1;

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
