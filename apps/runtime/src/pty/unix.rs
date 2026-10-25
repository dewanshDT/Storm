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
use std::io::{self, Read, Write};
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

/// Starts `launch` on a new PTY at `size`: as a child of this process, or,
/// on an isolated host, through the isolation helper (decision 136).
pub(crate) fn spawn(launch: &Launch, size: TerminalSize) -> io::Result<Spawned> {
    if let Some(isolated) = &launch.isolated {
        return spawn_isolated(launch, size, isolated);
    }
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
    // Nothing waits on the child yet, so its number is still its own.
    let process = crate::platform::peer::Process::open(leader.as_raw_nonzero().get()).ok();
    // Non-blocking, so an input write takes what fits and never sleeps
    // until all of it does: macOS's `ptcwrite` would (decision 83), and
    // writing a byte at a time instead split escape sequences, which Codex
    // then read as Esc and typed text (decision 132, fix b). The reader
    // shares the flag and waits for readiness instead.
    rustix::fs::fcntl_setfl(
        &master,
        rustix::fs::fcntl_getfl(&master)? | OFlags::NONBLOCK,
    )?;
    Ok(Spawned {
        child: Child(ChildKind::Local(child)),
        output: Output(File::from(master.try_clone()?)),
        input: Input {
            writer: File::from(master),
        },
        group: Group(GroupKind::Local {
            leader,
            tag: launch.tag.clone(),
        }),
        leader: process,
    })
}

/// A session in its own unit (136d). The host keeps the master; the slave
/// goes to the helper and is closed here once the unit has it, so the
/// master sees end-of-file when the session's last process lets go.
/// Linux only in practice: elsewhere no host has a link (136k).
fn spawn_isolated(
    launch: &Launch,
    size: TerminalSize,
    isolated: &crate::isolation::Isolated,
) -> io::Result<Spawned> {
    let link = &isolated.link;
    let session = launch.tag.as_str();
    let (master, slave) = open(size)?;
    let user = link.lease(session)?;
    let started = (|| -> io::Result<(i32, Option<OwnedFd>)> {
        link.prepare(session, &user)?;
        crate::platform::create_private_dir(&crate::mcp::session_dir(&link.state_dir, session))?;
        let dir = link.launch_dir();
        crate::platform::tighten_private_dir(&dir)?;
        let file = dir.join(format!("{session}.json"));
        let written = write_launch(&file, launch);
        let result = written.and_then(|()| {
            link.start(
                session,
                &user,
                &isolated.provider,
                &launch.cwd.canonicalize()?,
                slave.as_fd(),
            )
        });
        // It holds the session's secrets; the helper has read it.
        let _ = std::fs::remove_file(&file);
        result
    })();
    drop(slave);
    let (pid, pidfd) = match started {
        Ok(started) => started,
        Err(e) => {
            if let Err(release) = link.release(session, &user) {
                tracing::warn!(session, error = %release, "isolation: could not release a lease after a failed start");
            }
            return Err(e);
        }
    };
    rustix::fs::fcntl_setfl(
        &master,
        rustix::fs::fcntl_getfl(&master)? | OFlags::NONBLOCK,
    )?;
    #[cfg(target_os = "linux")]
    let leader = match &pidfd {
        Some(fd) => Some(crate::platform::peer::Process::from_pidfd(
            pid,
            fd.try_clone()?,
        )),
        None => None,
    };
    #[cfg(not(target_os = "linux"))]
    let leader = {
        let _ = pid;
        None
    };
    let unit = std::sync::Arc::new(Unit {
        link: link.clone(),
        session: session.to_string(),
        user,
        pidfd,
        ended: std::sync::atomic::AtomicBool::new(false),
    });
    Ok(Spawned {
        child: Child(ChildKind::Unit(unit.clone())),
        output: Output(File::from(master.try_clone()?)),
        input: Input {
            writer: File::from(master),
        },
        group: Group(GroupKind::Unit(unit)),
        leader,
    })
}

/// The session's arguments and environment for the helper, `0600`: the
/// program is the operator's, so it is not here. The variables are what the
/// session would have been given on top of the host's own: the helper's
/// unit starts from systemd's environment, not this process's.
fn write_launch(path: &std::path::Path, launch: &Launch) -> io::Result<()> {
    let text = |s: &std::ffi::OsStr| {
        s.to_str()
            .map(str::to_string)
            .ok_or_else(|| io::Error::other("a launch value is not UTF-8"))
    };
    let mut env: Vec<(String, String)> = Vec::new();
    let mut set = |k: String, v: String| {
        env.retain(|(name, _)| *name != k);
        env.push((k, v));
    };
    for (k, v) in &launch.env {
        set(text(k)?, text(v)?);
    }
    for k in &launch.env_remove {
        let k = text(k)?;
        env.retain(|(name, _)| *name != k);
    }
    for (k, v) in &launch.secret_env {
        let mut env_v = text(v.expose())?;
        env.retain(|(name, _)| *name != text(k).unwrap_or_default());
        env.push((text(k)?, std::mem::take(&mut env_v)));
    }
    let args = launch
        .args
        .iter()
        .map(|a| text(a))
        .collect::<io::Result<Vec<_>>>()?;
    let body = serde_json::json!({"args": args, "env": env}).to_string();
    let mut f = crate::platform::open_private(
        path,
        std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true),
    )?;
    f.write_all(body.as_bytes())
}

/// A session the helper started, shared by its child and its group.
pub(crate) struct Unit {
    link: crate::isolation::Link,
    session: String,
    user: String,
    pidfd: Option<OwnedFd>,
    /// The unit has been stopped: nothing of the session is left.
    ended: std::sync::atomic::AtomicBool,
}

impl Unit {
    fn signal(&self, signal: &str) {
        if self.ended.load(std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        if let Err(e) = self.link.signal(&self.session, signal) {
            tracing::debug!(session = %self.session, error = %e, "isolation: signal not delivered");
        }
    }

    /// Waits for the leader to exit, then ends the unit and the lease.
    fn wait(&self) -> io::Result<Exit> {
        if let Some(fd) = &self.pidfd {
            loop {
                let mut fds = [rustix::event::PollFd::new(fd, rustix::event::PollFlags::IN)];
                match rustix::event::poll(&mut fds, None) {
                    Ok(n) if n > 0 => break,
                    Ok(_) | Err(rustix::io::Errno::INTR) => continue,
                    Err(e) => return Err(e.into()),
                }
            }
        }
        let (code, signal) = self.link.end(&self.session)?;
        self.ended.store(true, std::sync::atomic::Ordering::SeqCst);
        if let Err(e) = self.link.release(&self.session, &self.user) {
            tracing::warn!(session = %self.session, error = %e, "isolation: the lease was not released");
        }
        Ok(Exit { code, signal })
    }
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

pub(crate) struct Child(ChildKind);

enum ChildKind {
    Local(std::process::Child),
    Unit(std::sync::Arc<Unit>),
}

impl Child {
    pub fn wait(self) -> io::Result<Exit> {
        match self.0 {
            ChildKind::Local(mut child) => {
                let status = child.wait()?;
                Ok(Exit {
                    code: status.code(),
                    signal: status.signal(),
                })
            }
            ChildKind::Unit(unit) => unit.wait(),
        }
    }
}

/// The terminal's output. The master is non-blocking, so a read with
/// nothing to read waits for some.
pub(crate) struct Output(File);

impl Read for Output {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            match self.0.read(buf) {
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    crate::platform::wait_readable(self.0.as_fd())?
                }
                other => return other,
            }
        }
    }
}

pub(crate) struct Input {
    writer: File,
}

impl Input {
    /// Writes as much as the terminal takes, waiting for room only when it is
    /// full, until `deadline`: a keystroke's escape sequence goes in one
    /// write. Past the deadline the write fails `TimedOut`; a write racing
    /// the agent's exit fails `BrokenPipe`.
    pub fn write(&mut self, mut bytes: &[u8], deadline: Instant) -> io::Result<()> {
        while !bytes.is_empty() {
            match self.writer.write(bytes) {
                Ok(n) => bytes = &bytes[n..],
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    let left = deadline.saturating_duration_since(Instant::now());
                    if left.is_zero() || !crate::platform::wait_writable(self.writer.as_fd(), left)?
                    {
                        return Err(io::Error::new(
                            io::ErrorKind::TimedOut,
                            "the agent is not reading its input",
                        ));
                    }
                }
                // A write racing the agent's exit fails with EIO; to the
                // caller it is the same thing as writing after the end.
                Err(e) if e.raw_os_error() == Some(rustix::io::Errno::IO.raw_os_error()) => {
                    return Err(io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        "the session has ended",
                    ));
                }
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    pub fn resize(&self, size: TerminalSize) -> io::Result<()> {
        resize(&self.writer, size)
    }
}

/// Everything a session started: the agent's process group and session,
/// and what carries its tag; or, on an isolated host, its unit, whose
/// control group holds all of it (136d).
pub(crate) struct Group(GroupKind);

enum GroupKind {
    Local {
        /// The agent's pid: its process group's id and its session's.
        leader: Pid,
        /// The session id, which every process it started carries in
        /// [`crate::platform::SESSION_TAG`].
        tag: String,
    },
    Unit(std::sync::Arc<Unit>),
}

impl Group {
    /// SIGHUP to the agent's process group; to the whole unit when isolated.
    pub fn hang_up(&self) {
        match &self.0 {
            // ESRCH just means the group is already gone.
            GroupKind::Local { leader, .. } => {
                let _ = rustix::process::kill_process_group(*leader, Signal::HUP);
            }
            GroupKind::Unit(unit) => unit.signal("hup"),
        }
    }

    /// SIGHUP to everything still in the session, or carrying its tag.
    pub fn hang_up_rest(&self) {
        match &self.0 {
            GroupKind::Local { leader, tag } => signal_each(&stragglers(*leader, tag), Signal::HUP),
            GroupKind::Unit(unit) => unit.signal("hup"),
        }
    }

    /// SIGKILL to the group and to every other member of the session the
    /// agent leads (its pid is both ids): a job-control shell's jobs have
    /// groups of their own. To the whole unit when isolated.
    pub fn kill(&self) {
        match &self.0 {
            GroupKind::Local { leader, tag } => {
                let _ = rustix::process::kill_process_group(*leader, Signal::KILL);
                signal_each(&stragglers(*leader, tag), Signal::KILL);
            }
            GroupKind::Unit(unit) => unit.signal("kill"),
        }
    }

    pub fn is_empty(&self) -> bool {
        match &self.0 {
            GroupKind::Local { leader, tag } => stragglers(*leader, tag).is_empty(),
            GroupKind::Unit(unit) => unit.ended.load(std::sync::atomic::Ordering::SeqCst),
        }
    }
}

/// Everything the session started that is still alive: the members of
/// the session the leader leads, and whatever left it but carries the
/// session's tag. If they cannot be listed, none: the group signals still
/// went out.
fn stragglers(leader: Pid, tag: &str) -> Vec<Pid> {
    let mut all: Vec<Pid> = crate::platform::session_members(leader.as_raw_nonzero().get())
        .unwrap_or_default()
        .into_iter()
        .filter_map(Pid::from_raw)
        .collect();
    for pid in crate::platform::tagged_processes(tag)
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

fn signal_each(pids: &[Pid], signal: Signal) {
    for &pid in pids {
        // ESRCH just means it is already gone.
        let _ = rustix::process::kill_process(pid, signal);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::time::Duration;

    /// A keystroke's escape sequence is one write (decision 132, fix b): a
    /// reader waiting on the other end gets `ESC [ A` whole, never a lone
    /// ESC that Codex would take for the Esc key.
    #[test]
    fn an_escape_sequence_reaches_a_waiting_reader_whole() {
        let (mut reader, writer) = std::io::pipe().unwrap();
        let read = std::thread::spawn(move || {
            let mut buf = [0u8; 64];
            let n = reader.read(&mut buf).unwrap();
            buf[..n].to_vec()
        });
        std::thread::sleep(Duration::from_millis(100));
        let mut input = Input {
            writer: File::from(OwnedFd::from(writer)),
        };
        input
            .write(b"\x1b[A", Instant::now() + Duration::from_secs(5))
            .unwrap();
        assert_eq!(read.join().unwrap(), b"\x1b[A");
    }
}
