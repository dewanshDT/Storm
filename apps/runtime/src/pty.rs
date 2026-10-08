//! The PTY: the runtime's carrier for a terminal interaction (decision 77a).
//!
//! Built directly on `rustix` rather than a PTY crate, for two reasons. The
//! session's ending needs the *number* of the signal that ended it
//! (`failed (signal N)`), and ending a session has to reach everything the
//! agent spawned. `setsid` in the child makes it a session and process-group
//! leader, so its pid is the group id, and one `kill(-pgid)` reaches the whole
//! tree.

use std::ffi::OsStr;
use std::io;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};

use rustix::fs::{Mode, OFlags};
use rustix::pty::OpenptFlags;
use rustix::termios::Winsize;

use crate::provider::TerminalSize;

/// A new PTY pair at `size`: the master the runtime keeps, and the slave the
/// agent gets as its terminal.
pub(crate) fn open(size: TerminalSize) -> io::Result<(OwnedFd, OwnedFd)> {
    let master = rustix::pty::openpt(OpenptFlags::RDWR | OpenptFlags::NOCTTY)?;
    // `openpt` has no close-on-exec flag on every platform; set it here so no
    // agent inherits another session's master.
    rustix::io::fcntl_setfd(&master, rustix::io::FdFlags::CLOEXEC)?;
    rustix::pty::grantpt(&master)?;
    rustix::pty::unlockpt(&master)?;
    let name = rustix::pty::ptsname(&master, Vec::new())?;
    let slave = rustix::fs::open(
        name.as_c_str(),
        OFlags::RDWR | OFlags::NOCTTY | OFlags::CLOEXEC,
        Mode::empty(),
    )?;
    resize(&master, size)?;
    Ok((master, slave))
}

/// Sets the terminal's size. The kernel sends the foreground process group a
/// `SIGWINCH`, which is how the agent learns of it.
pub(crate) fn resize(master: impl AsFd, size: TerminalSize) -> io::Result<()> {
    rustix::termios::tcsetwinsize(
        master,
        Winsize {
            ws_row: size.rows(),
            ws_col: size.cols(),
            ws_xpixel: 0,
            ws_ypixel: 0,
        },
    )?;
    Ok(())
}

/// Spawns `command` on the slave, as the leader of a new session whose
/// controlling terminal is the slave.
pub(crate) fn spawn(command: &mut Command, slave: &OwnedFd) -> io::Result<Child> {
    command
        .stdin(Stdio::from(slave.try_clone()?))
        .stdout(Stdio::from(slave.try_clone()?))
        .stderr(Stdio::from(slave.try_clone()?));
    // SAFETY: the closure runs in the forked child before exec, where only
    // async-signal-safe calls are allowed. It makes two raw syscalls — setsid
    // and the TIOCSCTTY ioctl — and allocates nothing. By then std has already
    // dup'd the slave onto fd 0, which is what the ioctl is given.
    unsafe {
        command.pre_exec(|| {
            rustix::process::setsid()?;
            rustix::process::ioctl_tiocsctty(BorrowedFd::borrow_raw(0))?;
            Ok(())
        });
    }
    command.spawn()
}

/// Resolves a command the way a shell would: as given when it contains a `/`,
/// otherwise the first executable file of that name on `path` (AM38), or on
/// the host's own `PATH` when there is none.
pub(crate) fn resolve(command: &OsStr, path: Option<&OsStr>) -> Option<std::path::PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    let executable = |path: &std::path::Path| {
        std::fs::metadata(path)
            .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    };
    let as_path = std::path::Path::new(command);
    if command.as_encoded_bytes().contains(&b'/') {
        return executable(as_path).then(|| as_path.to_path_buf());
    }
    path.map(OsStr::to_os_string)
        .or_else(|| std::env::var_os("PATH"))
        .iter()
        .flat_map(std::env::split_paths)
        .map(|dir| dir.join(command))
        .find(|candidate| executable(candidate))
}
