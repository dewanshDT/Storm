//! What the OS says about the process at the other end of the bridge socket
//! and the processes above it (decision 135b, 135c). Linux has it:
//! `SO_PEERPIDFD`, `pidfd_open` and `/proc`. macOS has `LOCAL_PEERTOKEN` and
//! `sysctl`'s process table, built but not yet reported (G-135-4). Windows
//! has neither, and [`support`] says how the host reports each (135d).

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

/// macOS's mechanism: the peer's audit token, then the process table for each
/// ancestor, checked by start time (135c).
pub const MACOS_MECHANISM: &str = "local_peertoken";

/// What a macOS host reports. 135c: `unavailable` until G-135-4 passes with
/// real agents on a real Mac — never `verified` on an untested mechanism.
/// **G-135-4 switch:** once it has passed, this becomes
/// `Support::Available(MACOS_MECHANISM)`, and nothing else changes.
pub const MACOS: Support = Support::Unavailable("macos_not_gated");

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

#[cfg(target_os = "macos")]
mod imp {
    use std::io;
    use std::os::fd::{AsFd, AsRawFd};

    /// The unit tests, and a host built for G-135-4, run the mechanism; a
    /// release reports [`super::MACOS`].
    pub const SUPPORT: super::Support = if cfg!(any(test, feature = "macos-identity-gate")) {
        super::Support::Available(super::MACOS_MECHANISM)
    } else {
        super::MACOS
    };

    /// `PROC_PIDUNIQIDENTIFIERINFO` in `<sys/proc_info.h>`, not in `libc`.
    const PROC_PIDUNIQIDENTIFIERINFO: libc::c_int = 17;

    /// `struct proc_uniqidentifierinfo`. Only its layout matters past
    /// `p_idversion`.
    #[allow(dead_code)]
    #[repr(C)]
    struct UniqIdentifierInfo {
        p_uuid: [u8; 16],
        p_uniqueid: u64,
        p_puniqueid: u64,
        p_idversion: i32,
        p_orig_ppidversion: i32,
        p_reserve2: u64,
        p_reserve3: u64,
    }

    /// One read of a process: what the walk needs of it.
    struct Info {
        ppid: i32,
        zombie: bool,
        /// When it started, which a reused number cannot match.
        start: (u64, u64),
    }

    /// `proc_pidinfo` of `flavor` into a `T`; `None` once there is no
    /// process with that number. `PROC_PIDUNIQIDENTIFIERINFO` is one of the
    /// flavors it answers for any account's process.
    fn pidinfo<T>(pid: i32, flavor: libc::c_int) -> io::Result<Option<T>> {
        if pid <= 0 {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        let mut buf = std::mem::MaybeUninit::<T>::zeroed();
        let size = std::mem::size_of::<T>() as libc::c_int;
        // SAFETY: `buf` is valid for writes of `size` bytes.
        let n = unsafe { libc::proc_pidinfo(pid, flavor, 0, buf.as_mut_ptr().cast(), size) };
        if n <= 0 {
            let e = io::Error::last_os_error();
            return match e.raw_os_error() {
                Some(libc::ESRCH) => Ok(None),
                _ => Err(e),
            };
        }
        if n != size {
            return Err(io::ErrorKind::InvalidData.into());
        }
        // SAFETY: the kernel filled all of it, and `T` is plain data.
        Ok(Some(unsafe { buf.assume_init() }))
    }

    /// `struct kinfo_proc` on 64-bit macOS, which `libc` lacks: its size and
    /// the offsets of `kp_proc.p_starttime`, `kp_proc.p_stat`,
    /// `kp_proc.p_pid` and `kp_eproc.e_ppid`.
    const KINFO_PROC_SIZE: usize = 648;
    const P_STARTTIME: usize = 0;
    const P_STAT: usize = 36;
    const P_PID: usize = 40;
    const E_PPID: usize = 560;

    /// `sysctl` `KERN_PROC_PID`, what `ps` reads: it answers for any
    /// process, root's included, where `proc_pidinfo`'s full BSD info is
    /// refused past the account — and a walk that leaves the host's account
    /// still has to reach launchd.
    fn info(pid: i32) -> io::Result<Option<Info>> {
        if pid <= 0 {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        let mut mib = [libc::CTL_KERN, libc::KERN_PROC, libc::KERN_PROC_PID, pid];
        let mut buf = [0u8; KINFO_PROC_SIZE];
        let mut len = buf.len();
        // SAFETY: `mib`, `buf` and `len` are valid for the sizes given.
        let rc = unsafe {
            libc::sysctl(
                mib.as_mut_ptr(),
                mib.len() as libc::c_uint,
                buf.as_mut_ptr().cast(),
                &mut len,
                std::ptr::null_mut(),
                0,
            )
        };
        if rc != 0 {
            return Err(io::Error::last_os_error());
        }
        if len == 0 {
            return Ok(None);
        }
        let i32_at = |o: usize| i32::from_ne_bytes(buf[o..o + 4].try_into().unwrap());
        if len != KINFO_PROC_SIZE || i32_at(P_PID) != pid {
            return Err(io::ErrorKind::InvalidData.into());
        }
        let sec = i64::from_ne_bytes(buf[P_STARTTIME..P_STARTTIME + 8].try_into().unwrap());
        let usec = i32_at(P_STARTTIME + 8);
        Ok(Some(Info {
            ppid: i32_at(E_PPID),
            zombie: u32::from(buf[P_STAT]) == libc::SZOMB,
            start: (sec as u64, usec as u64),
        }))
    }

    /// The pid version an audit token for `pid` would carry now.
    fn idversion(pid: i32) -> io::Result<Option<i32>> {
        let u = pidinfo::<UniqIdentifierInfo>(pid, PROC_PIDUNIQIDENTIFIERINFO)?;
        Ok(u.map(|u| u.p_idversion))
    }

    /// A process as the host saw it: its number and when it started. There
    /// is no pidfd, so [`Process::alive`] asks whether that number still
    /// names a live process that started then — a reused number started
    /// later.
    #[derive(Debug)]
    pub struct Process {
        pid: i32,
        start: (u64, u64),
    }

    impl Process {
        pub fn open(pid: i32) -> io::Result<Self> {
            let i = info(pid)?.ok_or(io::ErrorKind::NotFound)?;
            Ok(Self {
                pid,
                start: i.start,
            })
        }

        /// `-1` for a peer that was gone, or no longer the process that
        /// connected, by the time it was read.
        pub fn pid(&self) -> i32 {
            self.pid
        }

        pub fn alive(&self) -> io::Result<bool> {
            if self.pid <= 0 {
                return Ok(false);
            }
            Ok(info(self.pid)?.is_some_and(|i| i.start == self.start && !i.zombie))
        }

        /// Whether this process started no later than `child`: a parent
        /// always does, and a process that took a dead parent's number did
        /// not (135c's start-time check).
        pub fn older_than(&self, child: &Process) -> bool {
            self.start <= child.start
        }
    }

    /// The process that connected `stream`, from its audit token. The token
    /// carries the pid version, so a number that has since gone to another
    /// process reads as gone.
    pub fn peer(stream: &impl AsFd) -> io::Result<Process> {
        let mut token = [0u32; 8];
        let mut len = std::mem::size_of_val(&token) as libc::socklen_t;
        // SAFETY: `token` and `len` are valid for writes of the sizes given,
        // and the descriptor is borrowed for the call.
        let rc = unsafe {
            libc::getsockopt(
                stream.as_fd().as_raw_fd(),
                libc::SOL_LOCAL,
                libc::LOCAL_PEERTOKEN,
                token.as_mut_ptr().cast(),
                &mut len,
            )
        };
        if rc != 0 {
            return Err(io::Error::last_os_error());
        }
        if len as usize != std::mem::size_of_val(&token) {
            return Err(io::ErrorKind::InvalidData.into());
        }
        // `audit_token_to_pid` and `audit_token_to_pidversion`.
        let (pid, version) = (token[5] as i32, token[7] as i32);
        if pid <= 0 {
            return Ok(Process { pid, start: (0, 0) });
        }
        // The start time is read after the version matched, so it is that
        // process's unless the number was reused in between — which the
        // second version read catches.
        let gone = Process {
            pid: -1,
            start: (0, 0),
        };
        if idversion(pid)? != Some(version) {
            return Ok(gone);
        }
        let Some(i) = info(pid)? else {
            return Ok(gone);
        };
        if idversion(pid)? != Some(version) {
            return Ok(gone);
        }
        Ok(Process {
            pid,
            start: i.start,
        })
    }

    /// `pid`'s parent as the kernel says now.
    pub fn parent(pid: i32) -> io::Result<i32> {
        Ok(info(pid)?.ok_or(io::ErrorKind::NotFound)?.ppid)
    }

    /// Yama is Linux's; macOS has nothing to read.
    pub fn ptrace_scope() -> io::Result<u32> {
        Err(io::ErrorKind::Unsupported.into())
    }

    /// `LOCAL_PEERTOKEN` on one end of a fresh socket pair: this process.
    pub fn socketpair_peer() -> io::Result<Process> {
        let (a, _b) = std::os::unix::net::UnixStream::pair()?;
        peer(&a)
    }
}

#[cfg(windows)]
mod imp {
    use std::io;

    pub const SUPPORT: super::Support = super::Support::Unsupported("windows");

    fn unsupported() -> io::Error {
        io::ErrorKind::Unsupported.into()
    }

    /// Never constructed on Windows.
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
