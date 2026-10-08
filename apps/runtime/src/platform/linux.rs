//! Linux: systemd, the `.deb` and its paths (decision 77e).

use std::io;
use std::os::fd::BorrowedFd;
use std::time::Duration;

pub const NAME: &str = "linux";
pub const DEFAULT_STATE: &str = "/var/lib/storm-runtime";
pub const DEFAULT_CONFIG: &str = "/etc/storm-runtime/runtime.toml";
pub const SERVICE_ACCOUNT: &str = "storm-runtime";

/// `poll(2)` supports a PTY master on Linux, and is what 77c verified.
pub fn wait_writable(fd: BorrowedFd<'_>, timeout: Duration) -> io::Result<bool> {
    use rustix::event::{PollFd, PollFlags, poll};
    let mut fds = [PollFd::new(&fd, PollFlags::OUT)];
    Ok(poll(&mut fds, Some(&super::timespec(timeout)))? > 0)
}
