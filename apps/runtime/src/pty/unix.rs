//! The PTY on Linux and macOS (decision 77a).
//!
//! Built directly on `rustix` rather than a PTY crate, for two reasons. The
//! session's ending needs the *number* of the signal that ended it
//! (`failed (signal N)`), and ending a session has to reach everything the
//! agent spawned. `setsid` in the child makes it a session and process-group
//! leader, so its pid is the group id, and one `kill(-pgid)` reaches the whole
//! group. The rest of the session — a job-control shell's jobs, in groups of
//! their own — is found by its session id, and what left it with `setsid`
//! by the session's tag (F1).

use std::ffi::OsStr;
use std::fs::File;
use std::io::{self, Write};
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::process::{Command, Stdio};
use std::time::Instant;

use rustix::fs::{Mode, OFlags};
use rustix::process::{Pid, Signal};
use rustix::pty::OpenptFlags;
use rustix::termios::Winsize;

use super::{Exit, Launch, Spawned};
use crate::provider::TerminalSize;

/// Starts `launch` on a new PTY at `size`.
pub(crate) fn spawn(launch: &Launch, size: TerminalSize) -> io::Result<Spawned> {
    let (master, slave) = open(size)?;
    let mut command = Command::new(&launch.program);
    command
        .args(&launch.args)
        .current_dir(&launch.cwd)
        .envs(launch.env.iter().map(|(k, v)| (k, v)));
    for name in &launch.env_remove {
        command.env_remove(name);
    }
    command.envs(launch.secret_env.iter().map(|(k, v)| (k, v.expose())));
    let child = spawn_on(&mut command, &slave)?;
    // The parent's copy must go, or the master never sees end-of-file.
    drop(slave);
    let leader = Pid::from_child(&child);
    Ok(Spawned {
        child: Child(child),
        output: File::from(master.try_clone()?),
        input: Input {
            writer: File::from(master),
        },
        group: Group {
            leader,
            tag: launch.tag.clone(),
        },
    })
}

/// A new PTY pair at `size`: the master the runtime keeps, and the slave the
/// agent gets as its terminal.
fn open(size: TerminalSize) -> io::Result<(OwnedFd, OwnedFd)> {
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
fn resize(master: impl AsFd, size: TerminalSize) -> io::Result<()> {
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
fn spawn_on(command: &mut Command, slave: &OwnedFd) -> io::Result<std::process::Child> {
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

pub(crate) struct Child(std::process::Child);

impl Child {
    pub fn wait(mut self) -> io::Result<Exit> {
        let status = self.0.wait()?;
        Ok(Exit {
            code: status.code(),
            signal: status.signal(),
        })
    }
}

pub(crate) struct Input {
    writer: File,
}

impl Input {
    /// Writes in small chunks, each only once the terminal can take it, until
    /// `deadline`. Past it the write fails `TimedOut`; a write racing the
    /// agent's exit fails `BrokenPipe`.
    pub fn write(&mut self, bytes: &[u8], deadline: Instant) -> io::Result<()> {
        for chunk in bytes.chunks(crate::platform::PTY_WRITE_CHUNK) {
            let left = deadline.saturating_duration_since(Instant::now());
            if !crate::platform::wait_writable(self.writer.as_fd(), left)? {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "the agent is not reading its input",
                ));
            }
            self.writer.write_all(chunk).map_err(|e| {
                // A write racing the agent's exit fails with EIO; to the caller
                // it is the same thing as writing after the end.
                if e.raw_os_error() == Some(rustix::io::Errno::IO.raw_os_error()) {
                    io::Error::new(io::ErrorKind::BrokenPipe, "the session has ended")
                } else {
                    e
                }
            })?;
        }
        Ok(())
    }

    pub fn resize(&self, size: TerminalSize) -> io::Result<()> {
        resize(&self.writer, size)
    }
}

/// The agent's process group and session, and what carries its tag.
pub(crate) struct Group {
    /// The agent's pid: its process group's id and its session's.
    leader: Pid,
    /// The session id, which every process it started carries in
    /// [`crate::platform::SESSION_TAG`].
    tag: String,
}

impl Group {
    /// SIGHUP to the agent's process group.
    pub fn hang_up(&self) {
        // ESRCH just means the group is already gone.
        let _ = rustix::process::kill_process_group(self.leader, Signal::HUP);
    }

    /// SIGHUP to everything still in the session, or carrying its tag.
    pub fn hang_up_rest(&self) {
        signal_each(&self.stragglers(), Signal::HUP);
    }

    /// SIGKILL to the group and to every other member of the session the
    /// agent leads (its pid is both ids): a job-control shell's jobs have
    /// groups of their own.
    pub fn kill(&self) {
        let _ = rustix::process::kill_process_group(self.leader, Signal::KILL);
        signal_each(&self.stragglers(), Signal::KILL);
    }

    pub fn is_empty(&self) -> bool {
        self.stragglers().is_empty()
    }

    /// Everything the session started that is still alive: the members of
    /// the session the leader leads, and whatever left it but carries the
    /// session's tag. If they cannot be listed, none: the group signals still
    /// went out.
    fn stragglers(&self) -> Vec<Pid> {
        let mut all: Vec<Pid> =
            crate::platform::session_members(self.leader.as_raw_nonzero().get())
                .unwrap_or_default()
                .into_iter()
                .filter_map(Pid::from_raw)
                .collect();
        for pid in crate::platform::tagged_processes(&self.tag)
            .unwrap_or_default()
            .into_iter()
            .filter_map(Pid::from_raw)
        {
            if !all.contains(&pid) {
                all.push(pid);
            }
        }
        all
    }
}

fn signal_each(pids: &[Pid], signal: Signal) {
    for &pid in pids {
        // ESRCH just means it is already gone.
        let _ = rustix::process::kill_process(pid, signal);
    }
}
