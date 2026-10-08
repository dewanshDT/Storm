//! `cli` providers: a command on the host, run inside a PTY the runtime owns
//! (freeze §9.2, decision 77a).
//!
//! The provider contributes only a launch description — command, args,
//! environment — and the runtime builds the terminal around it. Every CLI
//! agent therefore gets the same terminal behaviour: interactive programs,
//! resize and control sequences work alike for all of them.
//!
//! **How a session ends** (freeze §7.3):
//! - `stop` sends SIGHUP to the session's process group, then SIGKILL after
//!   the grace. It is reported `stopped` whatever signal finished the job.
//! - An agent that exits by itself is `completed` with its exit code; one
//!   killed by a signal nobody here sent is `failed (signal N)`.
//! - Either way, once the agent is gone its process group gets SIGHUP and then
//!   SIGKILL, so nothing the session spawned outlives it, and a background job
//!   holding the terminal cannot keep the session open forever.
//! - `ended` is reported only after the output has drained, so nothing arrives
//!   after it.

use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::AsFd;
use std::os::unix::process::ExitStatusExt;
use std::process::{Command, ExitStatus};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::Duration;

use rustix::process::{Pid, Signal};

use crate::provider::{
    Availability, Interaction, InteractionKind, InteractionSpec, Provider, ProviderId,
    ProviderKind, ProviderSession, SessionEvents, SessionSpec, StartError, TerminalChannel,
    TerminalSize,
};
use crate::pty;
use crate::status::{EndReason, SessionEnd, SessionStatus};

/// How long a process group gets between SIGHUP and SIGKILL (freeze §7.3).
pub const DEFAULT_GRACE: Duration = Duration::from_secs(5);

/// The longest one input write may wait for the agent to read.
pub const INPUT_DEADLINE: Duration = Duration::from_secs(5);

const INTERACTIONS: [InteractionKind; 1] = [InteractionKind::Terminal];

/// A provider that runs a command on the host.
#[derive(Debug, Clone)]
pub struct CliProvider {
    id: ProviderId,
    command: OsString,
    args: Vec<OsString>,
    env: Vec<(OsString, OsString)>,
    /// The `PATH` it is looked for on and runs with (AM38). `None`: the
    /// host's own.
    path: Option<OsString>,
}

impl CliProvider {
    pub fn new(
        id: ProviderId,
        command: impl Into<OsString>,
        args: impl IntoIterator<Item = impl Into<OsString>>,
    ) -> Self {
        Self {
            id,
            command: command.into(),
            args: args.into_iter().map(Into::into).collect(),
            env: Vec::new(),
            path: None,
        }
    }

    /// Looks the command up on `path`, and gives every session that `PATH`,
    /// so "installed" and "runs" cannot disagree (AM38). It is set after a
    /// provider env file, so an env file cannot make them disagree either.
    pub fn with_path(mut self, path: impl Into<OsString>) -> Self {
        self.path = Some(path.into());
        self
    }

    /// Variables added after the terminal's own, so a provider may override
    /// them. Provider secrets arrive this way, from an env file the host's
    /// config names; Storm never stores or transmits them.
    pub fn with_env(
        mut self,
        env: impl IntoIterator<Item = (impl Into<OsString>, impl Into<OsString>)>,
    ) -> Self {
        self.env = env.into_iter().map(|(k, v)| (k.into(), v.into())).collect();
        self
    }

    /// Claude Code, the default provider. The permission mode is always passed:
    /// gate Q6 found the operator's own CLI settings starting it in another
    /// mode. Never `--dangerously-skip-permissions`.
    pub fn claude_code() -> Self {
        Self::new(
            id("claude-code"),
            "claude",
            ["--permission-mode", "default"],
        )
    }

    pub fn opencode() -> Self {
        Self::new(id("opencode"), "opencode", Vec::<OsString>::new())
    }

    /// A login shell in the workspace, as the runtime's own user. Not the
    /// server shell, which stays Phase 4.
    pub fn shell() -> Self {
        let shell = std::env::var_os("SHELL")
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "/bin/sh".into());
        Self::new(id("shell"), shell, ["-l"])
    }

    pub fn command(&self) -> &OsStr {
        &self.command
    }
}

fn id(s: &str) -> ProviderId {
    ProviderId::new(s).expect("a built-in id is valid")
}

impl Provider for CliProvider {
    fn id(&self) -> &ProviderId {
        &self.id
    }

    fn kind(&self) -> ProviderKind {
        ProviderKind::Cli
    }

    fn interactions(&self) -> &[InteractionKind] {
        &INTERACTIONS
    }

    fn available(&self) -> Availability {
        match pty::resolve(&self.command, self.path.as_deref()) {
            Some(_) => Availability::Available,
            None => Availability::NotInstalled,
        }
    }

    fn start(
        &self,
        spec: SessionSpec,
        events: Arc<dyn SessionEvents>,
    ) -> Result<Box<dyn ProviderSession>, StartError> {
        let program =
            pty::resolve(&self.command, self.path.as_deref()).ok_or(StartError::NotAvailable)?;
        let InteractionSpec::Terminal(size) = spec.interaction;
        let failed = |what: &str, e: io::Error| StartError::Failed(format!("{what}: {e}"));

        let (master, slave) = pty::open(size).map_err(|e| failed("opening a pty", e))?;
        let mut command = Command::new(&program);
        command
            .args(&self.args)
            .args(&spec.launch.args)
            .current_dir(&spec.workspace)
            .env("TERM", "xterm-256color")
            .env("COLORTERM", "truecolor")
            .envs(self.env.iter().map(|(k, v)| (k, v)))
            .envs(self.path.iter().map(|p| ("PATH", p)))
            // The session's own values last (AM32): its MCP config must win
            // over anything a provider env file says.
            .envs(spec.launch.env.iter().map(|(k, v)| (k, v)));
        let child = pty::spawn(&mut command, &slave)
            .map_err(|e| failed(&format!("starting {}", program.display()), e))?;
        // The parent's copy must go, or the master never sees end-of-file.
        drop(slave);

        let group = Pid::from_child(&child);
        let reader = File::from(
            master
                .try_clone()
                .map_err(|e| failed("cloning the pty", e))?,
        );
        let shared = Arc::new(Shared {
            status: Mutex::new(SessionStatus::Running),
            stop_requested: AtomicBool::new(false),
            exit: (Mutex::new(None), Condvar::new()),
        });

        let waiter = shared.clone();
        thread::Builder::new()
            .name(format!("{}-wait", spec.session_id))
            .spawn(move || {
                let mut child = child;
                let status = child.wait();
                // Whatever the agent left behind in its group goes with it.
                hang_up(group);
                waiter.set_exit(status);
                if !waiter.sleep_until_ended(DEFAULT_GRACE) {
                    kill(group);
                }
            })
            .map_err(|e| failed("starting the wait thread", e))?;

        let drain = shared.clone();
        thread::Builder::new()
            .name(format!("{}-read", spec.session_id))
            .spawn(move || drain.read_until_closed(reader, events))
            .map_err(|e| failed("starting the read thread", e))?;

        Ok(Box::new(CliSession {
            terminal: CliTerminal {
                writer: File::from(master),
                shared,
            },
            group,
        }))
    }
}

/// State the session, its reader and its waiter share.
struct Shared {
    status: Mutex<SessionStatus>,
    stop_requested: AtomicBool,
    /// The agent's exit, once it has one, and a wake-up for whoever waits.
    exit: (Mutex<Option<io::Result<ExitStatus>>>, Condvar),
}

impl Shared {
    fn status(&self) -> SessionStatus {
        *self.status.lock().unwrap()
    }

    fn set_exit(&self, status: io::Result<ExitStatus>) {
        let (lock, cvar) = &self.exit;
        *lock.lock().unwrap() = Some(status);
        cvar.notify_all();
    }

    /// Waits up to `limit` for the session to be reported ended. Returns
    /// whether it was.
    fn sleep_until_ended(&self, limit: Duration) -> bool {
        let step = Duration::from_millis(50);
        let mut waited = Duration::ZERO;
        while waited < limit {
            if self.status().is_ended() {
                return true;
            }
            thread::sleep(step);
            waited += step;
        }
        self.status().is_ended()
    }

    /// Forwards output until the terminal closes, then reports the ending —
    /// in that order, so nothing is reported after `ended`.
    fn read_until_closed(&self, mut reader: File, events: Arc<dyn SessionEvents>) {
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => events.output(&buf[..n]),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                // EIO is how Linux reports that every slave fd is closed.
                Err(_) => break,
            }
        }
        let (lock, cvar) = &self.exit;
        let mut exit = lock.lock().unwrap();
        while exit.is_none() {
            exit = cvar.wait(exit).unwrap();
        }
        let end = match (self.stop_requested.load(Ordering::SeqCst), exit.as_ref()) {
            (true, _) => SessionEnd::Stopped,
            (false, Some(Ok(status))) => match status.signal() {
                Some(signal) => SessionEnd::Failed(EndReason::Signal(signal)),
                None => SessionEnd::Completed {
                    exit_code: status.code(),
                },
            },
            (false, _) => SessionEnd::Completed { exit_code: None },
        };
        drop(exit);
        *self.status.lock().unwrap() = end.status();
        events.ended(end);
    }
}

fn hang_up(group: Pid) {
    // ESRCH just means the group is already gone.
    let _ = rustix::process::kill_process_group(group, Signal::HUP);
}

fn kill(group: Pid) {
    let _ = rustix::process::kill_process_group(group, Signal::KILL);
}

struct CliSession {
    terminal: CliTerminal,
    group: Pid,
}

struct CliTerminal {
    writer: File,
    shared: Arc<Shared>,
}

impl CliTerminal {
    fn ensure_running(&self) -> io::Result<()> {
        if self.shared.status().is_ended() {
            Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "the session has ended",
            ))
        } else {
            Ok(())
        }
    }
}

impl TerminalChannel for CliTerminal {
    /// Writes in small chunks, each only once the terminal can take it, within
    /// [`INPUT_DEADLINE`] overall.
    ///
    /// A PTY write blocks while the agent is not reading its input, and the
    /// caller holds the session across this call. An unbounded write would let
    /// a hung agent hold it forever, so that even the owner's `end` could not
    /// get in. Input is at-most-once (freeze §11.3): a write that cannot land
    /// in time fails with `TimedOut` rather than waiting.
    fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.ensure_running()?;
        let deadline = std::time::Instant::now() + INPUT_DEADLINE;
        for chunk in bytes.chunks(crate::platform::PTY_WRITE_CHUNK) {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
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

    fn resize(&mut self, size: TerminalSize) -> io::Result<()> {
        self.ensure_running()?;
        pty::resize(&self.writer, size)
    }
}

impl ProviderSession for CliSession {
    fn interaction(&mut self) -> Interaction<'_> {
        Interaction::Terminal(&mut self.terminal)
    }

    fn stop(&mut self, grace: Duration) {
        let shared = &self.terminal.shared;
        if shared.status().is_ended() || shared.stop_requested.swap(true, Ordering::SeqCst) {
            return;
        }
        hang_up(self.group);
        let (shared, group) = (shared.clone(), self.group);
        // Non-blocking: the caller is the host link, which must not stall for
        // the grace period. The thread exits as soon as the session ends.
        let _ = thread::Builder::new()
            .name("stop-grace".into())
            .spawn(move || {
                if !shared.sleep_until_ended(grace) {
                    kill(group);
                }
            });
    }

    fn status(&self) -> SessionStatus {
        self.terminal.shared.status()
    }
}
