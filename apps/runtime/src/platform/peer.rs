//! What the OS says about the process at the other end of the bridge socket
//! and the processes above it (decision 135b). Linux has it: `SO_PEERPIDFD`,
//! `pidfd_open` and `/proc`. Everywhere else every call is unsupported, and
//! [`support`] says how the host reports that (135c, 135d).

pub use imp::{Process, parent, peer, ptrace_scope, socketpair_peer};

/// Whether this platform can bind a connection to its session.
pub enum Support {
    /// The mechanism exists; the host still checks its preconditions and runs
    /// its self-test before it reports `verified`.
    Available(&'static str),
    /// Not built here yet: `unavailable`, with this reason.
    Unavailable(&'static str),
    /// Never on this platform: `unsupported`, with this reason.
    Unsupported(&'static str),
}

pub fn support() -> Support {
    imp::SUPPORT
}

#[cfg(target_os = "linux")]
mod imp {
    use std::io;
    use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd};

    use rustix::event::{PollFd, PollFlags, Timespec, poll};
    use rustix::process::{Pid, PidfdFlags, pidfd_open};

    pub const SUPPORT: super::Support = super::Support::Available("so_peerpidfd");

    /// `SO_PEERPIDFD` (Linux 6.5), not yet in `libc`. The asm-generic value,
    /// which every architecture Storm builds for uses.
    const SO_PEERPIDFD: libc::c_int = 77;

    /// A process the host holds a pidfd for. The pidfd always names the same
    /// process, whatever later reuses its number, so [`Process::alive`] is
    /// about that process and no other.
    #[derive(Debug)]
    pub struct Process {
        pid: i32,
        fd: OwnedFd,
    }

    impl Process {
        pub fn open(pid: i32) -> io::Result<Self> {
            let raw = Pid::from_raw(pid).ok_or(io::ErrorKind::InvalidInput)?;
            let fd = pidfd_open(raw, PidfdFlags::empty())?;
            Ok(Self { pid, fd })
        }

        /// A pidfd someone else opened: the isolation helper's for a
        /// session's leader (decision 136i).
        pub fn from_pidfd(pid: i32, fd: OwnedFd) -> Self {
            Self { pid, fd }
        }

        /// Its pid as the pidfd was opened for, or as the kernel reports it
        /// for a peer: `0` or less once it is gone or outside this pid
        /// namespace.
        pub fn pid(&self) -> i32 {
            self.pid
        }

        /// A pidfd becomes readable when its process exits, zombie or not;
        /// `/proc` still answers for a zombie, so only this tells.
        pub fn alive(&self) -> io::Result<bool> {
            let mut fds = [PollFd::new(&self.fd, PollFlags::IN)];
            let ready = poll(
                &mut fds,
                Some(&Timespec {
                    tv_sec: 0,
                    tv_nsec: 0,
                }),
            )?;
            Ok(ready == 0)
        }
    }

    /// The process that connected `stream`: who called `connect`, not who
    /// holds the socket now.
    pub fn peer(stream: &impl AsFd) -> io::Result<Process> {
        let mut raw: libc::c_int = -1;
        let mut len = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
        // SAFETY: `raw` and `len` are valid for writes of the sizes given,
        // and the descriptor is borrowed for the call.
        let rc = unsafe {
            libc::getsockopt(
                stream.as_fd().as_raw_fd(),
                libc::SOL_SOCKET,
                SO_PEERPIDFD,
                (&mut raw as *mut libc::c_int).cast(),
                &mut len,
            )
        };
        if rc != 0 {
            return Err(io::Error::last_os_error());
        }
        if raw < 0 {
            return Err(io::ErrorKind::InvalidData.into());
        }
        // SAFETY: the kernel just gave us this descriptor and nothing else
        // owns it.
        let fd = unsafe { OwnedFd::from_raw_fd(raw) };
        let pid = pidfd_pid(&fd)?;
        Ok(Process { pid, fd })
    }

    /// The pid a pidfd names, from `/proc/self/fdinfo`: `-1` once the process
    /// is gone, `0` if it is in a pid namespace this one cannot see.
    fn pidfd_pid(fd: &OwnedFd) -> io::Result<i32> {
        let info = std::fs::read_to_string(format!("/proc/self/fdinfo/{}", fd.as_raw_fd()))?;
        info.lines()
            .find_map(|l| l.strip_prefix("Pid:"))
            .and_then(|v| v.trim().parse().ok())
            .ok_or_else(|| io::ErrorKind::InvalidData.into())
    }

    /// `pid`'s parent as `/proc` says now. `0` is a parent outside this pid
    /// namespace.
    pub fn parent(pid: i32) -> io::Result<i32> {
        let status = std::fs::read_to_string(format!("/proc/{pid}/status"))?;
        status
            .lines()
            .find_map(|l| l.strip_prefix("PPid:"))
            .and_then(|v| v.trim().parse().ok())
            .ok_or_else(|| io::ErrorKind::InvalidData.into())
    }

    /// Yama's `ptrace_scope`. Without Yama there is no file, and nothing
    /// stops one process of the account from attaching to another.
    pub fn ptrace_scope() -> io::Result<u32> {
        std::fs::read_to_string("/proc/sys/kernel/yama/ptrace_scope")?
            .trim()
            .parse()
            .map_err(|_| io::ErrorKind::InvalidData.into())
    }

    /// `SO_PEERPIDFD` on one end of a fresh socket pair: this process.
    pub fn socketpair_peer() -> io::Result<Process> {
        let (a, _b) = std::os::unix::net::UnixStream::pair()?;
        peer(&a)
    }
}

#[cfg(not(target_os = "linux"))]
mod imp {
    use std::io;

    #[cfg(target_os = "macos")]
    pub const SUPPORT: super::Support = super::Support::Unavailable("macos_not_gated");
    #[cfg(windows)]
    pub const SUPPORT: super::Support = super::Support::Unsupported("windows");

    fn unsupported() -> io::Error {
        io::ErrorKind::Unsupported.into()
    }

    /// Never constructed off Linux.
    #[derive(Debug)]
    pub struct Process {
        never: std::convert::Infallible,
    }

    impl Process {
        pub fn open(_pid: i32) -> io::Result<Self> {
            Err(unsupported())
        }

        pub fn pid(&self) -> i32 {
            match self.never {}
        }

        pub fn alive(&self) -> io::Result<bool> {
            match self.never {}
        }
    }

    pub fn peer<T>(_stream: &T) -> io::Result<Process> {
        Err(unsupported())
    }

    pub fn parent(_pid: i32) -> io::Result<i32> {
        Err(unsupported())
    }

    pub fn ptrace_scope() -> io::Result<u32> {
        Err(unsupported())
    }

    pub fn socketpair_peer() -> io::Result<Process> {
        Err(unsupported())
    }
}
