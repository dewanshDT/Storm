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

#[cfg(test)]
mod tests {
    #[test]
    fn install_and_uninstall_point_at_the_package() {
        let e = super::install(&Default::default()).unwrap_err().to_string();
        assert!(e.contains("apt install storm-runtime"), "{e}");
        let e = super::uninstall(&Default::default())
            .unwrap_err()
            .to_string();
        assert!(e.contains("apt remove storm-runtime"), "{e}");
    }
}
