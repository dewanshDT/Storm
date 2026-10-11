//! `cli` sessions on ConPTY, in a Job Object (D16, AM51–AM53): what
//! `tests/cli.rs` proves of the PTY on Unix, proved of Windows' carrier,
//! through the same contract — output as it lands, and the one ending.
#![cfg(windows)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use storm_runtime::cli::CliProvider;
use storm_runtime::provider::{
    Availability, Interaction, InteractionSpec, Provider, ProviderId, ProviderSession,
    SessionEvents, SessionSpec, StartError, TerminalSize,
};
use storm_runtime::status::SessionEnd;

const WAIT: Duration = Duration::from_secs(20);

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
    /// The output with ConPTY's escape sequences and line breaks taken out,
    /// so a test looks for text the way a person would read it.
    fn text(&self) -> String {
        plain(&String::from_utf8_lossy(&self.state.lock().unwrap().0))
    }

    fn wait_for(&self, needle: &str) -> String {
        let deadline = Instant::now() + WAIT;
        let mut state = self.state.lock().unwrap();
        loop {
            let text = plain(&String::from_utf8_lossy(&state.0));
            if text.contains(needle) {
                return text;
            }
            let left = deadline
                .checked_duration_since(Instant::now())
                .unwrap_or_else(|| panic!("never saw {needle:?}; output was {text:?}"));
            state = self.changed.wait_timeout(state, left).unwrap().0;
        }
    }

    fn wait_end(&self) -> SessionEnd {
        let deadline = Instant::now() + WAIT;
        let mut state = self.state.lock().unwrap();
        while state.1.is_empty() {
            let left = deadline
                .checked_duration_since(Instant::now())
                .unwrap_or_else(|| panic!("the session did not end; output was {:?}", state.0));
            state = self.changed.wait_timeout(state, left).unwrap().0;
        }
        drop(state);
        std::thread::sleep(Duration::from_millis(100));
        let endings = self.state.lock().unwrap().1.clone();
        assert_eq!(endings.len(), 1, "ended more than once: {endings:?}");
        endings[0]
    }
}

/// `s` without CSI/OSC sequences, carriage returns or line feeds.
fn plain(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\x1b' => match chars.next() {
                Some('[') => {
                    for c in chars.by_ref() {
                        if ('@'..='~').contains(&c) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    while let Some(c) = chars.next() {
                        if c == '\x07' || (c == '\x1b' && chars.next_if_eq(&'\\').is_some()) {
                            break;
                        }
                    }
                }
                _ => {}
            },
            '\r' | '\n' => {}
            c => out.push(c),
        }
    }
    out
}

fn id(s: &str) -> ProviderId {
    ProviderId::new(s).unwrap()
}

fn powershell(script: &str) -> CliProvider {
    CliProvider::new(
        id("shell"),
        "powershell",
        [
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            script,
        ],
    )
}

fn cmd(script: &str) -> CliProvider {
    CliProvider::new(id("shell"), "cmd", ["/d", "/c", script])
}

fn workspace() -> PathBuf {
    std::env::temp_dir()
}

fn start(provider: &CliProvider) -> (Box<dyn ProviderSession>, Arc<Recorder>, String) {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let session_id = format!("ags_WIN{n}_{}", std::process::id());
    let rec = Arc::new(Recorder::default());
    let spec = SessionSpec {
        session_id: session_id.clone(),
        workspace: workspace(),
        interaction: InteractionSpec::Terminal(TerminalSize::new(80, 24).unwrap()),
        launch: Default::default(),
    };
    let session = provider.start(spec, rec.clone()).expect("start");
    (session, rec, session_id)
}

fn write(session: &mut dyn ProviderSession, bytes: &[u8]) {
    let Interaction::Terminal(term) = session.interaction();
    term.write(bytes).unwrap();
}

fn alive(pid: u32) -> bool {
    let out = Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/NH", "/FO", "CSV"])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).contains(&format!("\"{pid}\""))
}

fn wait_gone(pid: u32, what: &str) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while alive(pid) {
        assert!(Instant::now() < deadline, "{what} ({pid}) is still running");
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn pid_after(text: &str, tag: &str) -> u32 {
    text.split(tag)
        .nth(1)
        .and_then(|r| r.split(|c: char| !c.is_ascii_digit()).next())
        .and_then(|p| p.parse().ok())
        .unwrap_or_else(|| panic!("no {tag} in {text:?}"))
}

/// A process the agent starts in a window of its own: off the
/// pseudoconsole, so nothing but the job reaches it.
const DETACHED: &str = "$p = Start-Process -PassThru -WindowStyle Hidden -FilePath ping \
                        -ArgumentList '-n','1000','127.0.0.1'; 'child:' + $p.Id + ':'";

#[test]
fn output_flows_and_the_exit_code_is_the_ending() {
    let (_session, rec, _) = start(&cmd("echo hello from conpty & exit 3"));
    rec.wait_for("hello from conpty");
    assert_eq!(rec.wait_end(), SessionEnd::Completed { exit_code: Some(3) });
}

#[test]
fn input_reaches_the_agent_through_the_terminal() {
    let (mut session, rec, _) = start(&powershell(
        "$line = Read-Host 'ready'; Write-Output ('got:' + $line + ':')",
    ));
    rec.wait_for("ready");
    write(session.as_mut(), b"storm\r");
    rec.wait_for("got:storm:");
    assert_eq!(rec.wait_end(), SessionEnd::Completed { exit_code: Some(0) });
}

#[test]
fn a_resize_reaches_the_terminal() {
    let (mut session, rec, _) = start(&powershell(
        "$null = Read-Host 'ready'; Write-Output ('cols:' + [Console]::WindowWidth + ':')",
    ));
    rec.wait_for("ready");
    {
        let Interaction::Terminal(term) = session.interaction();
        term.resize(TerminalSize::new(132, 40).unwrap()).unwrap();
    }
    std::thread::sleep(Duration::from_millis(300));
    write(session.as_mut(), b"\r");
    rec.wait_for("cols:132:");
}

#[test]
fn the_terminal_environment_and_the_workspace_are_set() {
    let (_session, rec, session_id) = start(&cmd(
        "echo term=%TERM%; tag=%STORM_RUNTIME_SESSION%; cwd=%CD%;",
    ));
    rec.wait_for("cwd=");
    rec.wait_end();
    let text = rec.text().replace(' ', "");
    assert!(text.contains("term=xterm-256color;"), "{text}");
    assert!(text.contains(&format!("tag={session_id};")), "{text}");
    let workspace = workspace().display().to_string().replace(' ', "");
    let workspace = workspace.trim_end_matches('\\').to_lowercase();
    assert!(
        text.to_lowercase().contains(&format!("cwd={workspace};")),
        "{text}"
    );
}

/// AM52: Stop hangs up (the console's processes get CTRL_CLOSE), and after
/// the grace the job ends what is left — a process the agent started off the
/// console included.
#[test]
fn stopping_ends_everything_the_session_spawned() {
    let (mut session, rec, _) = start(&powershell(&format!("{DETACHED}; Start-Sleep 1000")));
    let child = pid_after(&rec.wait_for("child:"), "child:");
    assert!(alive(child));
    session.stop(Duration::from_millis(500));
    assert_eq!(rec.wait_end(), SessionEnd::Stopped);
    wait_gone(child, "a detached child");
}

/// AM52: `ended` means the session is empty, on Windows too. The agent
/// exits, its detached child lives on, and the ending waits for the job to
/// be emptied.
#[test]
fn ended_means_the_session_is_empty() {
    let (_session, rec, _) = start(&powershell(DETACHED));
    let child = pid_after(&rec.wait_for("child:"), "child:");
    assert_eq!(rec.wait_end(), SessionEnd::Completed { exit_code: Some(0) });
    assert!(
        !alive(child),
        "ended was reported with {child} still running"
    );
}

#[test]
fn input_after_the_end_is_refused() {
    let (mut session, rec, _) = start(&cmd("exit 0"));
    rec.wait_end();
    let Interaction::Terminal(term) = session.interaction();
    let err = term.write(b"late\r").unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::BrokenPipe);
}

#[test]
fn a_missing_binary_is_not_installed_and_will_not_start() {
    let missing = CliProvider::new(id("nope"), "storm-no-such-agent", Vec::<String>::new());
    assert_eq!(missing.available(), Availability::NotInstalled);
    let spec = SessionSpec {
        session_id: "ags_MISSING".into(),
        workspace: workspace(),
        interaction: InteractionSpec::Terminal(TerminalSize::new(80, 24).unwrap()),
        launch: Default::default(),
    };
    let rec = Arc::new(Recorder::default());
    assert!(matches!(
        missing.start(spec, rec),
        Err(StartError::NotAvailable)
    ));
}

fn batch(dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, "@echo off\r\necho args=[%*]\r\n").unwrap();
    path
}

/// AM53: `claude` finds `claude.cmd` through `PATHEXT`, on the configured
/// `PATH` only, and a batch file runs through cmd with its arguments intact.
#[test]
fn a_batch_cli_is_found_through_pathext_and_runs() {
    let bin = tempfile::tempdir().unwrap();
    batch(bin.path(), "storm-test-agent.cmd");
    let bare = CliProvider::new(id("custom"), "storm-test-agent", ["one", "two words"]);
    assert_eq!(bare.available(), Availability::NotInstalled);
    let on_path = bare.with_path(bin.path().as_os_str().to_owned());
    assert_eq!(on_path.available(), Availability::Available);
    let (_session, rec, _) = start(&on_path);
    rec.wait_for(r#"args=["one" "two words"]"#);
    assert_eq!(rec.wait_end(), SessionEnd::Completed { exit_code: Some(0) });
}

/// AM53: what cmd.exe would expand is refused for a batch file, never
/// escaped.
#[test]
fn a_batch_cli_refuses_an_argument_cmd_would_interpret() {
    let bin = tempfile::tempdir().unwrap();
    let path = batch(bin.path(), "agent.cmd");
    let provider = CliProvider::new(id("custom"), path.as_os_str(), ["%USERNAME%"]);
    let spec = SessionSpec {
        session_id: "ags_BATBAD".into(),
        workspace: workspace(),
        interaction: InteractionSpec::Terminal(TerminalSize::new(80, 24).unwrap()),
        launch: Default::default(),
    };
    let rec = Arc::new(Recorder::default());
    match provider.start(spec, rec) {
        Err(StartError::Failed(why)) => assert!(why.contains("refused"), "{why}"),
        other => panic!("started anyway: {:?}", other.err()),
    }
}

#[test]
fn the_shell_provider_is_powershell() {
    let shell = CliProvider::shell();
    let command = shell.command().to_string_lossy().to_lowercase();
    assert!(command == "pwsh" || command == "powershell", "{command}");
    assert_eq!(shell.available(), Availability::Available);
}

#[test]
fn the_default_host_name_is_the_computer_name() {
    let name = storm_runtime::platform::host_name();
    assert_eq!(name, std::env::var("COMPUTERNAME").unwrap().to_lowercase());
}
