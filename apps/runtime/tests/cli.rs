//! `cli` sessions against real processes in a real PTY (decision 77a).
//!
//! Each test runs `/bin/sh` through the same `CliProvider` the built-in
//! providers use, and observes it only through the contract: output as it
//! lands, and the one ending.

use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use storm_runtime::cli::CliProvider;
use storm_runtime::provider::{
    Availability, Interaction, InteractionSpec, Provider, ProviderId, ProviderKind,
    ProviderSession, SessionEvents, SessionSpec, StartError, TerminalSize,
};
use storm_runtime::status::{EndReason, SessionEnd, SessionStatus};

const WAIT: Duration = Duration::from_secs(10);

#[derive(Default)]
struct Recorder {
    state: Mutex<(Vec<u8>, Vec<SessionEnd>)>,
    changed: Condvar,
}

impl SessionEvents for Recorder {
    fn output(&self, bytes: &[u8]) {
        let mut state = self.state.lock().unwrap();
        assert!(state.1.is_empty(), "output after the session ended");
        state.0.extend_from_slice(bytes);
        self.changed.notify_all();
    }

    fn ended(&self, end: SessionEnd) {
        self.state.lock().unwrap().1.push(end);
        self.changed.notify_all();
    }
}

impl Recorder {
    fn output(&self) -> String {
        String::from_utf8_lossy(&self.state.lock().unwrap().0).into_owned()
    }

    /// Waits until the output contains `needle`.
    fn wait_for(&self, needle: &str) -> String {
        let deadline = Instant::now() + WAIT;
        let mut state = self.state.lock().unwrap();
        loop {
            let text = String::from_utf8_lossy(&state.0).into_owned();
            if text.contains(needle) {
                return text;
            }
            let left = deadline
                .checked_duration_since(Instant::now())
                .unwrap_or_else(|| panic!("never saw {needle:?}; output was {text:?}"));
            state = self.changed.wait_timeout(state, left).unwrap().0;
        }
    }

    /// Waits for the one ending, and checks there is only one.
    fn wait_end(&self) -> SessionEnd {
        let deadline = Instant::now() + WAIT;
        let mut state = self.state.lock().unwrap();
        while state.1.is_empty() {
            let left = deadline
                .checked_duration_since(Instant::now())
                .unwrap_or_else(|| panic!("the session never ended"));
            state = self.changed.wait_timeout(state, left).unwrap().0;
        }
        drop(state);
        // Give a second ending the chance to show up, if there were one.
        std::thread::sleep(Duration::from_millis(100));
        let endings = self.state.lock().unwrap().1.clone();
        assert_eq!(endings.len(), 1, "ended more than once: {endings:?}");
        endings[0]
    }
}

fn sh(script: &str) -> CliProvider {
    CliProvider::new(ProviderId::new("shell").unwrap(), "/bin/sh", ["-c", script])
}

fn workspace() -> PathBuf {
    std::env::temp_dir()
}

fn start(provider: &CliProvider) -> (Box<dyn ProviderSession>, Arc<Recorder>) {
    let rec = Arc::new(Recorder::default());
    let spec = SessionSpec {
        session_id: "ags_CLI".into(),
        workspace: workspace(),
        interaction: InteractionSpec::Terminal(TerminalSize::new(80, 24).unwrap()),
        launch: Default::default(),
    };
    let session = provider.start(spec, rec.clone()).expect("start");
    (session, rec)
}

fn write(session: &mut dyn ProviderSession, bytes: &[u8]) {
    let Interaction::Terminal(term) = session.interaction();
    term.write(bytes).unwrap();
}

fn alive(pid: i32) -> bool {
    // Signal 0 checks for existence without sending anything.
    rustix::process::test_kill_process(rustix::process::Pid::from_raw(pid).unwrap()).is_ok()
}

#[test]
fn output_flows_and_the_exit_code_is_the_ending() {
    let (_session, rec) = start(&sh("echo hello from the pty; exit 3"));
    assert_eq!(rec.wait_end(), SessionEnd::Completed { exit_code: Some(3) });
    assert!(rec.output().contains("hello from the pty"));
}

#[test]
fn input_reaches_the_agent_through_the_terminal() {
    let (mut session, rec) = start(&sh("read line; echo got:$line"));
    write(session.as_mut(), b"abc\r");
    rec.wait_for("got:abc");
    assert_eq!(rec.wait_end(), SessionEnd::Completed { exit_code: Some(0) });
    assert_eq!(session.status(), SessionStatus::Completed);
}

#[test]
fn a_resize_reaches_the_terminal() {
    let (mut session, rec) = start(&sh("read x; stty size"));
    {
        let Interaction::Terminal(term) = session.interaction();
        term.resize(TerminalSize::new(100, 40).unwrap()).unwrap();
    }
    write(session.as_mut(), b"\r");
    // `stty size` prints rows then columns.
    rec.wait_for("40 100");
    rec.wait_end();
}

#[test]
fn the_terminal_environment_and_the_workspace_are_set() {
    let provider = sh("echo term=$TERM color=$COLORTERM extra=$STORM_X; pwd")
        .with_env([("STORM_X", "from-env-file")]);
    let (_session, rec) = start(&provider);
    rec.wait_end();
    let out = rec.output();
    assert!(out.contains("term=xterm-256color"), "{out}");
    assert!(out.contains("color=truecolor"), "{out}");
    assert!(out.contains("extra=from-env-file"), "{out}");
    let ws = workspace().canonicalize().unwrap();
    assert!(out.contains(ws.to_str().unwrap()), "{out}");
}

#[test]
// Known to fail on macOS until slice 4 (PLAN.md 83d), found on the first
// real-Mac CI run: when the session leader exits, BSD revokes the session's
// controlling terminal, so the master sees end-of-file while a job that
// ignores SIGHUP is still alive, the session reports `ended`, and the
// grace's SIGKILL is skipped. Slice 4 ends a session only once it is empty,
// and turns this test back on.
#[cfg_attr(target_os = "macos", ignore = "fixed in slice 4 (83d): BSD revokes the terminal")]
fn stopping_ends_everything_the_session_spawned() {
    // A background job in the session that ignores the hangup, so only the
    // group-wide SIGKILL after the grace can end it. Nothing the session
    // spawned may outlive it (freeze §7.3). A job that obeyed SIGHUP would die
    // with the session leader anyway — the kernel hangs up the terminal's
    // group — and would prove nothing about who stop signals.
    let (mut session, rec) = start(&sh("(trap '' HUP; exec sleep 1000) & echo bg:$!; wait"));
    let out = rec.wait_for("\n");
    let pid: i32 = out
        .split("bg:")
        .nth(1)
        .and_then(|s| s.split_whitespace().next())
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| panic!("no pid in {out:?}"));
    assert!(alive(pid));

    session.stop(Duration::from_millis(300));
    assert_eq!(rec.wait_end(), SessionEnd::Stopped);
    assert_eq!(session.status(), SessionStatus::Stopped);
    let deadline = Instant::now() + WAIT;
    while alive(pid) {
        assert!(Instant::now() < deadline, "the background job survived");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn an_agent_that_ignores_the_hangup_is_killed_after_the_grace() {
    let (mut session, rec) = start(&sh("trap '' HUP; echo ready; while :; do sleep 1; done"));
    rec.wait_for("ready");
    let asked = Instant::now();
    session.stop(Duration::from_millis(300));
    // Still `stopped`: the owner asked, whatever signal finished the job.
    assert_eq!(rec.wait_end(), SessionEnd::Stopped);
    assert!(asked.elapsed() >= Duration::from_millis(300));
}

#[test]
fn a_signal_nobody_here_sent_is_a_failure_with_its_number() {
    let (_session, rec) = start(&sh("kill -9 $$"));
    assert_eq!(rec.wait_end(), SessionEnd::Failed(EndReason::Signal(9)));
}

#[test]
fn input_after_the_end_is_refused() {
    let (mut session, rec) = start(&sh("exit 0"));
    rec.wait_end();
    let Interaction::Terminal(term) = session.interaction();
    let err = term.write(b"late").unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::BrokenPipe);
}

#[test]
fn a_missing_binary_is_not_installed_and_will_not_start() {
    let provider = CliProvider::new(
        ProviderId::new("ghost").unwrap(),
        "storm-no-such-agent-binary",
        Vec::<String>::new(),
    );
    assert_eq!(provider.available(), Availability::NotInstalled);
    let rec = Arc::new(Recorder::default());
    let spec = SessionSpec {
        session_id: "ags_GHOST".into(),
        workspace: workspace(),
        interaction: InteractionSpec::Terminal(TerminalSize::new(80, 24).unwrap()),
        launch: Default::default(),
    };
    assert!(matches!(
        provider.start(spec, rec.clone()),
        Err(StartError::NotAvailable)
    ));
    assert!(rec.output().is_empty(), "a failed start reports nothing");
}

#[test]
fn the_built_in_providers_are_what_the_freeze_says() {
    let claude = CliProvider::claude_code();
    assert_eq!(claude.id().as_str(), "claude-code");
    assert_eq!(claude.kind(), ProviderKind::Cli);
    assert_eq!(claude.command(), "claude");
    assert_eq!(CliProvider::opencode().id().as_str(), "opencode");
    assert_eq!(CliProvider::shell().id().as_str(), "shell");
}

#[test]
fn input_to_an_agent_that_never_reads_times_out_instead_of_hanging() {
    // Decision 77c: a PTY write waits for room within a deadline, so a hung
    // agent cannot hold its session against `end`. The wait is per platform
    // (D14): macOS's poll(2) does not support devices and never waits, so a
    // write built on it blocks for as long as the agent ignores its input.
    let (mut session, rec) = start(&sh("stty raw -echo; echo ready; exec sleep 60"));
    rec.wait_for("ready");
    let (tx, rx) = std::sync::mpsc::channel();
    let started = Instant::now();
    std::thread::spawn(move || {
        let result = {
            let Interaction::Terminal(term) = session.interaction();
            term.write(&vec![b'x'; 1 << 20])
        };
        let _ = tx.send((result.map_err(|e| e.kind()), started.elapsed()));
        session.stop(Duration::from_millis(100));
    });
    let deadline = storm_runtime::cli::INPUT_DEADLINE + Duration::from_secs(5);
    let (result, took) = rx
        .recv_timeout(deadline)
        .expect("the write hung past its deadline");
    assert_eq!(result, Err(std::io::ErrorKind::TimedOut));
    assert!(
        took >= storm_runtime::cli::INPUT_DEADLINE,
        "gave up early: {took:?}"
    );
}

#[test]
fn the_default_host_name_is_the_node_name_without_local() {
    let name = storm_runtime::platform::host_name();
    assert!(!name.is_empty() && name.chars().count() <= 64, "{name:?}");
    assert!(!name.ends_with(".local"), "{name:?}");
    let node = rustix::system::uname()
        .nodename()
        .to_string_lossy()
        .into_owned();
    if !node.trim().is_empty() {
        assert!(node.starts_with(&name), "{name:?} is not from {node:?}");
    }
}
