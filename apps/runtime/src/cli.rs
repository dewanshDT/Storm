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
//! - **The SIGKILL reaches the whole session, not only the group.** A
//!   job-control shell (`zsh -l`, `bash -i`) puts each `cmd &` in a process
//!   group of its own, which `kill(-pgid)` never reaches. Every process whose
//!   session id is the agent's pid is found by `platform::session_members`.
//! - **`ended` means the session is empty.** Before reporting it, whatever is
//!   still in the session (a job that ignored SIGHUP and let go of the
//!   terminal) gets SIGHUP, then SIGKILL after the grace. A host that waits
//!   for `ended` (its shutdown, AM37) therefore leaves nothing behind. Only a
//!   process that called `setsid` itself has left the session; the
//!   exclusive-account sweep ends those when the host stops.
//! - **What left the session is found by its tag** (F1). Every process a
//!   session starts inherits `STORM_RUNTIME_SESSION=<id>`
//!   ([`crate::platform::SESSION_TAG`]). A server an agent daemonized with
//!   `setsid` (OpenCode's `serve --service`) has left the session and been
//!   reparented, but still carries the tag, so it is a straggler like any
//!   other: the session's ending ends it too. So does any per-account
//!   daemon a session happened to start on demand (a tmux server, an
//!   ssh-agent, a build daemon): it is the session's, and goes with it, even
//!   if a later session was using it. That is the design — nothing a session
//!   started outlives it — and the price of finding what left the session.
//! - `ended` is reported only after the output has drained, so nothing arrives
//!   after it.
//! - **On Windows** (AM51, AM52) the same policy runs on other means: the
//!   hang-up is closing the pseudoconsole (`CTRL_CLOSE_EVENT` to its console
//!   processes), the kill is terminating the session's Job Object, and
//!   "anything left" is the job's process count. Every process the agent
//!   starts is in the job and cannot leave it, so the tag is not searched.

use std::ffi::{OsStr, OsString};
use std::io::{self, Read};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::Duration;

use crate::provider::{
    Availability, Interaction, InteractionKind, InteractionSpec, Provider, ProviderId,
    ProviderKind, ProviderSession, SessionEvents, SessionSpec, StartError, TerminalChannel,
    TerminalSize,
};
use crate::pty::{self, Exit, Group, Launch};
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

    /// A built-in agent as its profile launches it (decision 130). `shell`
    /// is the platform's own shell instead.
    pub fn built_in(profile: &crate::profile::Profile) -> Self {
        if profile.id == "shell" {
            return Self::shell();
        }
        Self::new(
            id(profile.id),
            profile.command,
            profile.args.iter().copied(),
        )
    }

    /// Claude Code, the default provider.
    pub fn claude_code() -> Self {
        Self::built_in(crate::profile::get("claude-code").expect("a built-in"))
    }

    pub fn opencode() -> Self {
        Self::built_in(crate::profile::get("opencode").expect("a built-in"))
    }

    /// A login shell in the workspace, as the runtime's own user. Not the
    /// server shell, which stays Phase 4. PowerShell on Windows (AM53).
    pub fn shell() -> Self {
        let (shell, args) =
            crate::platform::session_shell(crate::platform::default_path().as_deref());
        Self::new(id("shell"), shell, args)
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

    fn resolved(&self) -> Option<std::path::PathBuf> {
        pty::resolve(&self.command, self.path.as_deref())
    }

    fn helper_command(&self) -> Option<std::process::Command> {
        let program = pty::resolve(&self.command, self.path.as_deref())?;
        let mut command = std::process::Command::new(program);
        command.envs(self.env.iter().map(|(k, v)| (k, v)));
        if let Some(path) = &self.path {
            command.env("PATH", path);
        }
        Some(command)
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

        let mut env: Vec<(OsString, OsString)> = vec![
            ("TERM".into(), "xterm-256color".into()),
            ("COLORTERM".into(), "truecolor".into()),
            (
                crate::platform::SESSION_TAG.into(),
                spec.session_id.clone().into(),
            ),
        ];
        env.extend(self.env.iter().cloned());
        env.extend(self.path.iter().map(|p| ("PATH".into(), p.clone())));
        // A login shell's profile may reorder PATH; this is what the
        // managed `.zprofile` restores (F3).
        env.extend(
            self.path
                .iter()
                .map(|p| (crate::platform::SESSION_PATH.into(), p.clone())),
        );
        // The session's own values last (AM32): its MCP config must win
        // over anything a provider env file says.
        env.extend(spec.launch.env.iter().cloned());
        // Then a synced login's removals and its token (AM71).
        for name in &spec.launch.env_remove {
            if self.env.iter().any(|(k, _)| k == name) {
                tracing::info!(session = %spec.session_id, variable = %name.to_string_lossy(), "synced login: ignoring it from the provider env file");
            }
            if std::env::var_os(name).is_some() {
                tracing::info!(session = %spec.session_id, variable = %name.to_string_lossy(), "synced login: ignoring it from the daemon environment");
            }
        }
        let launch = Launch {
            program: program.clone(),
            args: self.args.iter().chain(&spec.launch.args).cloned().collect(),
            cwd: spec.workspace.clone(),
            env,
            env_remove: spec.launch.env_remove,
            secret_env: spec.launch.secret_env,
            tag: spec.session_id.clone(),
        };
        let spawned = pty::spawn(&launch, size)
            .map_err(|e| failed(&format!("starting {}", program.display()), e))?;

        let shared = Arc::new(Shared {
            status: Mutex::new(SessionStatus::Running),
            stop_requested: AtomicBool::new(false),
            exit: (Mutex::new(None), Condvar::new()),
            group: spawned.group,
            grace: Mutex::new(DEFAULT_GRACE),
        });

        let waiter = shared.clone();
        let child = spawned.child;
        thread::Builder::new()
            .name(format!("{}-wait", spec.session_id))
            .spawn(move || {
                let status = child.wait();
                // Whatever the agent left behind in its group goes with it.
                waiter.group.hang_up();
                waiter.set_exit(status);
                if !waiter.sleep_until_ended(DEFAULT_GRACE) {
                    waiter.group.kill();
                }
            })
            .map_err(|e| failed("starting the wait thread", e))?;

        let drain = shared.clone();
        let reader = spawned.output;
        thread::Builder::new()
            .name(format!("{}-read", spec.session_id))
            .spawn(move || drain.read_until_closed(reader, events))
            .map_err(|e| failed("starting the read thread", e))?;

        Ok(Box::new(CliSession {
            terminal: CliTerminal {
                input: spawned.input,
                shared,
            },
        }))
    }
}

/// State the session, its reader and its waiter share.
struct Shared {
    status: Mutex<SessionStatus>,
    stop_requested: AtomicBool,
    /// The agent's exit, once it has one, and a wake-up for whoever waits.
    exit: (Mutex<Option<io::Result<Exit>>>, Condvar),
    /// Everything the session started.
    group: Group,
    /// How long stragglers get between SIGHUP and SIGKILL: the stop's grace
    /// when the owner asked, [`DEFAULT_GRACE`] otherwise.
    grace: Mutex<Duration>,
}

impl Shared {
    fn status(&self) -> SessionStatus {
        *self.status.lock().unwrap()
    }

    fn set_exit(&self, status: io::Result<Exit>) {
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

    /// Ends whatever is still in the session once the terminal has closed:
    /// SIGHUP, then SIGKILL to what remains after the grace. Returns once the
    /// session is empty, or a second after the SIGKILL.
    fn end_stragglers(&self) {
        if self.group.is_empty() {
            return;
        }
        self.group.hang_up_rest();
        let grace = *self.grace.lock().unwrap();
        let step = Duration::from_millis(50);
        let mut waited = Duration::ZERO;
        while waited < grace {
            if self.group.is_empty() {
                return;
            }
            thread::sleep(step);
            waited += step;
        }
        self.group.kill();
        for _ in 0..20 {
            if self.group.is_empty() {
                return;
            }
            thread::sleep(step);
        }
    }

    /// Forwards output until the terminal closes, then reports the ending —
    /// in that order, so nothing is reported after `ended`.
    fn read_until_closed(&self, mut reader: impl Read, events: Arc<dyn SessionEvents>) {
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
            (false, Some(Ok(exit))) => match exit.signal {
                Some(signal) => SessionEnd::Failed(EndReason::Signal(signal)),
                None => SessionEnd::Completed {
                    exit_code: exit.code,
                },
            },
            (false, _) => SessionEnd::Completed { exit_code: None },
        };
        drop(exit);
        self.end_stragglers();
        *self.status.lock().unwrap() = end.status();
        events.ended(end);
    }
}

struct CliSession {
    terminal: CliTerminal,
}

struct CliTerminal {
    input: pty::Input,
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
        self.input
            .write(bytes, std::time::Instant::now() + INPUT_DEADLINE)
    }

    fn resize(&mut self, size: TerminalSize) -> io::Result<()> {
        self.ensure_running()?;
        self.input.resize(size)
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
        *shared.grace.lock().unwrap() = grace;
        shared.group.hang_up();
        let shared = shared.clone();
        // Non-blocking: the caller is the host link, which must not stall for
        // the grace period. The thread exits as soon as the session ends.
        let _ = thread::Builder::new()
            .name("stop-grace".into())
            .spawn(move || {
                if !shared.sleep_until_ended(grace) {
                    shared.group.kill();
                }
            });
    }

    fn status(&self) -> SessionStatus {
        self.terminal.shared.status()
    }
}
