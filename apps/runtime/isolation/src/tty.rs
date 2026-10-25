//! The session's terminal (136d): the host keeps the master and hands over
//! the slave as a descriptor. The helper takes it only if it is a
//! pseudo-terminal slave on the system's devpts, the same device its
//! `/dev/pts` path names, and then gives it to the session's user alone.
//! systemd opens it by that path for the unit (`TTYPath=`), which makes it
//! the session's controlling terminal; the number cannot be reused while
//! the helper holds the descriptor.

use std::os::fd::{AsFd, BorrowedFd};
use std::path::PathBuf;

use rustix::fs::{FileType, Mode};

use crate::protocol::Refusal;

const DEVPTS_SUPER_MAGIC: i64 = 0x1cd1;
/// `UNIX98_PTY_SLAVE_MAJOR` and the majors after it.
const PTS_MAJORS: std::ops::RangeInclusive<u32> = 136..=143;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Terminal {
    pub path: PathBuf,
    pub rows: u16,
    pub cols: u16,
}

// `Stat`'s and `StatFs`'s field types differ between architectures.
#[allow(clippy::unnecessary_cast)]
pub fn verify(fd: BorrowedFd<'_>) -> Result<Terminal, Refusal> {
    let st = rustix::fs::fstat(fd).map_err(|_| Refusal::NotATerminal)?;
    if FileType::from_raw_mode(st.st_mode as _) != FileType::CharacterDevice {
        return Err(Refusal::NotATerminal);
    }
    let rdev = st.st_rdev as u64;
    let (major, minor) = (rustix::fs::major(rdev as _), rustix::fs::minor(rdev as _));
    if !PTS_MAJORS.contains(&major) {
        return Err(Refusal::NotATerminal);
    }
    let fs = rustix::fs::fstatfs(fd).map_err(|_| Refusal::NotATerminal)?;
    if fs.f_type as i64 != DEVPTS_SUPER_MAGIC {
        return Err(Refusal::NotATerminal);
    }
    let n = (major - PTS_MAJORS.start()) * 256 + minor;
    let path = PathBuf::from(format!("/dev/pts/{n}"));
    let named = rustix::fs::lstat(&path).map_err(|_| Refusal::NotATerminal)?;
    if named.st_rdev as u64 != rdev || named.st_dev as u64 != st.st_dev as u64 {
        return Err(Refusal::NotATerminal);
    }
    let size = rustix::termios::tcgetwinsize(fd).map_err(|_| Refusal::NotATerminal)?;
    Ok(Terminal {
        path,
        rows: size.ws_row.max(1),
        cols: size.ws_col.max(1),
    })
}

/// Gives the terminal to the session's user: owner it, mode `0600`.
pub trait TtyOwner: Send + Sync {
    fn give(&self, fd: BorrowedFd<'_>, uid: u32) -> std::io::Result<()>;
}

pub struct Real;

impl TtyOwner for Real {
    fn give(&self, fd: BorrowedFd<'_>, uid: u32) -> std::io::Result<()> {
        rustix::fs::fchown(fd, Some(rustix::fs::Uid::from_raw(uid)), None)?;
        rustix::fs::fchmod(fd, Mode::from_raw_mode(0o600))?;
        Ok(())
    }
}

/// Records who it was given to.
#[derive(Default)]
pub struct Recorded(pub std::sync::Mutex<Vec<u32>>);

impl TtyOwner for Recorded {
    fn give(&self, fd: BorrowedFd<'_>, uid: u32) -> std::io::Result<()> {
        let _ = fd.as_fd();
        self.0.lock().unwrap().push(uid);
        Ok(())
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use rustix::fs::OFlags;
    use rustix::pty::OpenptFlags;
    use std::os::fd::OwnedFd;

    /// A new PTY pair, as the host opens it.
    pub fn pty() -> (OwnedFd, OwnedFd) {
        let master = rustix::pty::openpt(OpenptFlags::RDWR | OpenptFlags::NOCTTY).unwrap();
        rustix::pty::grantpt(&master).unwrap();
        rustix::pty::unlockpt(&master).unwrap();
        let name = rustix::pty::ptsname(&master, Vec::new()).unwrap();
        let slave = rustix::fs::open(
            name.as_c_str(),
            OFlags::RDWR | OFlags::NOCTTY | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .unwrap();
        rustix::termios::tcsetwinsize(
            &master,
            rustix::termios::Winsize {
                ws_row: 33,
                ws_col: 101,
                ws_xpixel: 0,
                ws_ypixel: 0,
            },
        )
        .unwrap();
        (master, slave)
    }

    #[test]
    fn a_pty_slave_is_taken_with_its_path_and_size() {
        let (master, slave) = pty();
        let name = rustix::pty::ptsname(&master, Vec::new()).unwrap();
        let t = verify(slave.as_fd()).unwrap();
        assert_eq!(t.path.to_str().unwrap(), name.to_str().unwrap());
        assert_eq!((t.rows, t.cols), (33, 101));
    }

    #[test]
    fn anything_else_is_not_a_terminal() {
        let (master, _slave) = pty();
        let (r, _w) = std::io::pipe().unwrap();
        let file = tempfile::tempfile().unwrap();
        let null = std::fs::File::open("/dev/null").unwrap();
        let dir = std::fs::File::open("/").unwrap();
        for fd in [
            master.as_fd(),
            r.as_fd(),
            file.as_fd(),
            null.as_fd(),
            dir.as_fd(),
        ] {
            assert_eq!(verify(fd), Err(Refusal::NotATerminal));
        }
    }
}
