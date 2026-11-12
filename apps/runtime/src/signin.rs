//! Whether each agent CLI on this host is signed in (decision 153-S4), so
//! the app can say "Sign in" instead of letting a session fail.
//!
//! A read-only look at the account's own CLIs, nothing more:
//!
//! - an API key the host passes to the agent's sessions counts as signed
//!   in, by the variable's presence only;
//! - otherwise the CLI's own status command, as the host's account with its
//!   sessions' environment, in a neutral directory, stdin closed, killed
//!   with everything it started after [`DEADLINE`]: Claude Code's `claude
//!   auth status` and Codex's `codex login status` by exit code alone,
//!   Cursor's `cursor-agent status --format json` by its one
//!   `isAuthenticated` boolean;
//! - OpenCode by its credential store's size, never its contents: it runs on
//!   free models with no sign-in, so it is never `signed_out`;
//! - Command Code has no status command Storm knows, so it is `unknown`.
//!
//! What a command prints is never logged or sent; a CLI's output is read
//! only where a field is the answer, and dropped.

use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How long a status command may take.
pub const DEADLINE: Duration = Duration::from_secs(5);

/// The most of a status command's output that is read.
const OUTPUT_MAX: u64 = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignIn {
    SignedIn,
    SignedOut,
    /// Not installed, no reliable signal, timed out, or failed.
    Unknown,
}

impl SignIn {
    /// What the inventory says: nothing for `Unknown`, as an older host.
    pub fn wire(self) -> Option<bool> {
        match self {
            Self::SignedIn => Some(true),
            Self::SignedOut => Some(false),
            Self::Unknown => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::SignedIn => "signed_in",
            Self::SignedOut => "signed_out",
            Self::Unknown => "unknown",
        }
    }
}

/// How an agent's sign-in is read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Signal {
    /// Its status command: exit 0 is signed in, 1 signed out.
    Exit(&'static [&'static str]),
    /// Its status command's JSON: this boolean field.
    JsonBool(&'static [&'static str], &'static str),
    /// OpenCode's store, by size.
    OpenCodeStore,
    None,
}

fn signal(agent: &str) -> Signal {
    match agent {
        "claude-code" => Signal::Exit(&["auth", "status"]),
        "codex" => Signal::Exit(&["login", "status"]),
        "cursor" => Signal::JsonBool(&["status", "--format", "json"], "isAuthenticated"),
        "opencode" => Signal::OpenCodeStore,
        _ => Signal::None,
    }
}

/// The variables an agent signs in with, as its sessions would see them.
fn env_signed_in(agent: &str, env: &HashSet<String>) -> bool {
    match agent {
        "opencode" => env.iter().any(|k| k.ends_with("_API_KEY")),
        _ => crate::credential_paths::env_keys(agent)
            .iter()
            .any(|k| env.contains(*k)),
    }
}

/// The names of the variables set to a non-empty value, from the host's own
/// environment and the provider's env file: names only, never a value.
pub fn present(
    daemon: impl IntoIterator<Item = (OsString, OsString)>,
    provider: &[(OsString, bool)],
) -> HashSet<String> {
    daemon
        .into_iter()
        .filter(|(_, v)| !v.is_empty())
        .map(|(k, _)| k)
        .chain(
            provider
                .iter()
                .filter(|(_, set)| *set)
                .map(|(k, _)| k.clone()),
        )
        .filter_map(|k| k.into_string().ok())
        .collect()
}

/// OpenCode's credential store: `$XDG_DATA_HOME/opencode/auth.json`, else
/// `~/.local/share/opencode/auth.json`.
pub fn opencode_store() -> Option<PathBuf> {
    let set = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty());
    let data = set("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| set("HOME").map(|h| PathBuf::from(h).join(".local/share")))?;
    Some(data.join("opencode").join("auth.json"))
}

/// What the host last found, and when it last looked.
#[derive(Default)]
pub struct Ledger {
    states: Mutex<HashMap<String, SignIn>>,
    last: Mutex<Option<Instant>>,
}

impl Ledger {
    pub fn get(&self, agent: &str) -> SignIn {
        self.states
            .lock()
            .unwrap()
            .get(agent)
            .copied()
            .unwrap_or(SignIn::Unknown)
    }

    /// Whether a probe starts now: `force`, or none started in the last
    /// `min`. One that does is recorded as started.
    pub fn begin(&self, force: bool, min: Duration) -> bool {
        let mut last = self.last.lock().unwrap();
        if !force && last.is_some_and(|at| at.elapsed() < min) {
            return false;
        }
        *last = Some(Instant::now());
        true
    }

    /// Replaces what was found; whether anything changed.
    pub fn record(&self, found: HashMap<String, SignIn>) -> bool {
        let mut states = self.states.lock().unwrap();
        let changed = *states != found;
        *states = found;
        changed
    }
}

/// What one agent needs to be probed.
pub struct Probe<'a> {
    pub agent: &'a str,
    /// The agent's binary with its sessions' environment, `None` when it is
    /// not installed.
    pub command: Option<Command>,
    pub env: &'a HashSet<String>,
    pub opencode_store: Option<&'a Path>,
    pub deadline: Duration,
}

/// Blocking: at most one status command, bounded by `deadline`.
pub fn probe(p: Probe) -> SignIn {
    let Some(mut command) = p.command else {
        return SignIn::Unknown;
    };
    if env_signed_in(p.agent, p.env) {
        return SignIn::SignedIn;
    }
    match signal(p.agent) {
        Signal::Exit(args) => {
            command.args(args);
            match run(command, false, p.deadline).map(|(status, _)| status.code()) {
                Some(Some(0)) => SignIn::SignedIn,
                Some(Some(1)) => SignIn::SignedOut,
                _ => SignIn::Unknown,
            }
        }
        Signal::JsonBool(args, field) => {
            command.args(args);
            let answer = run(command, true, p.deadline)
                .filter(|(status, _)| status.success())
                .and_then(|(_, out)| serde_json::from_slice::<serde_json::Value>(&out).ok())
                .and_then(|v| v.get(field).and_then(serde_json::Value::as_bool));
            match answer {
                Some(true) => SignIn::SignedIn,
                Some(false) => SignIn::SignedOut,
                None => SignIn::Unknown,
            }
        }
        // Larger than `{}`: at least one provider is stored.
        Signal::OpenCodeStore => match p.opencode_store.and_then(|s| std::fs::metadata(s).ok()) {
            Some(m) if m.is_file() && m.len() > 4 => SignIn::SignedIn,
            _ => SignIn::Unknown,
        },
        Signal::None => SignIn::Unknown,
    }
}

/// Runs `command` in a neutral directory with stdin closed, in a process
/// group of its own that is killed once it exits or at `deadline`. Its
/// stdout is kept only when `capture`, and stderr never.
fn run(mut command: Command, capture: bool, deadline: Duration) -> Option<(ExitStatus, Vec<u8>)> {
    command
        .current_dir(std::env::temp_dir())
        .stdin(Stdio::null())
        .stdout(if capture {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stderr(Stdio::null());
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    let mut child = command.spawn().ok()?;
    let until = Instant::now() + deadline;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < until => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                crate::agents::end_group(&child);
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    };
    crate::agents::end_group(&child);
    let mut out = Vec::new();
    if let Some(stdout) = child.stdout.take() {
        use std::io::Read;
        stdout.take(OUTPUT_MAX).read_to_end(&mut out).ok()?;
    }
    Some((status, out))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn script(dir: &Path, body: &str) -> PathBuf {
        let bin = dir.join("agent");
        std::fs::write(&bin, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        bin
    }

    fn with(agent: &str, bin: Option<&Path>, env: &HashSet<String>) -> SignIn {
        probe(Probe {
            agent,
            command: bin.map(Command::new),
            env,
            opencode_store: None,
            deadline: Duration::from_secs(2),
        })
    }

    #[test]
    fn a_status_commands_exit_is_the_state() {
        let t = tempfile::tempdir().unwrap();
        let none = HashSet::new();
        for agent in ["claude-code", "codex"] {
            let ok = script(t.path(), "echo secret-token-abc; exit 0");
            assert_eq!(with(agent, Some(&ok), &none), SignIn::SignedIn, "{agent}");
            let out = script(t.path(), "echo 'Not logged in'; exit 1");
            assert_eq!(with(agent, Some(&out), &none), SignIn::SignedOut, "{agent}");
            let odd = script(t.path(), "exit 3");
            assert_eq!(with(agent, Some(&odd), &none), SignIn::Unknown, "{agent}");
        }
        assert_eq!(with("codex", None, &none), SignIn::Unknown, "not installed");
    }

    #[test]
    fn the_status_command_is_the_agents_own_with_stdin_closed() {
        let t = tempfile::tempdir().unwrap();
        let seen = t.path().join("seen");
        let bin = script(
            t.path(),
            &format!(
                "echo \"$*|$(pwd)\" > '{}'; read x && exit 0; exit 1",
                seen.display()
            ),
        );
        assert_eq!(
            with("codex", Some(&bin), &HashSet::new()),
            SignIn::SignedOut
        );
        let seen = std::fs::read_to_string(&seen).unwrap();
        let (args, cwd) = seen.trim().split_once('|').unwrap();
        assert_eq!(args, "login status");
        assert_eq!(
            Path::new(cwd).canonicalize().unwrap(),
            std::env::temp_dir().canonicalize().unwrap()
        );
        let bin = script(t.path(), "exit 0");
        assert_eq!(
            with("claude-code", Some(&bin), &HashSet::new()),
            SignIn::SignedIn
        );
    }

    #[test]
    fn a_hung_status_command_is_unknown_once_its_deadline_passes() {
        let t = tempfile::tempdir().unwrap();
        let bin = script(t.path(), "exec sleep 600");
        let started = Instant::now();
        let state = probe(Probe {
            agent: "claude-code",
            command: Some(Command::new(&bin)),
            env: &HashSet::new(),
            opencode_store: None,
            deadline: Duration::from_millis(300),
        });
        assert_eq!(state, SignIn::Unknown);
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "killed at the deadline"
        );
    }

    #[test]
    fn cursor_is_read_from_its_one_boolean() {
        let t = tempfile::tempdir().unwrap();
        let none = HashSet::new();
        let yes = script(
            t.path(),
            r#"[ "$*" = "status --format json" ] || exit 9; echo '{"isAuthenticated":true,"email":"a@b.c"}'"#,
        );
        assert_eq!(with("cursor", Some(&yes), &none), SignIn::SignedIn);
        let no = script(
            t.path(),
            r#"echo '{"isAuthenticated":false,"message":"Not logged in"}'"#,
        );
        assert_eq!(with("cursor", Some(&no), &none), SignIn::SignedOut);
        // An older Cursor without `--format`, or prose: no signal.
        let old = script(t.path(), "echo 'Not logged in'");
        assert_eq!(with("cursor", Some(&old), &none), SignIn::Unknown);
        let failed = script(t.path(), r#"echo '{"isAuthenticated":true}'; exit 1"#);
        assert_eq!(with("cursor", Some(&failed), &none), SignIn::Unknown);
    }

    #[test]
    fn an_api_key_the_sessions_get_is_signed_in_without_running_anything() {
        let t = tempfile::tempdir().unwrap();
        let ran = t.path().join("ran");
        let bin = script(t.path(), &format!("touch '{}'; exit 1", ran.display()));
        let names = present(
            [
                ("ANTHROPIC_API_KEY".into(), "sk-ant-x".into()),
                ("OPENAI_API_KEY".into(), "".into()),
            ],
            &[
                ("CURSOR_API_KEY".into(), true),
                ("CODEX_API_KEY".into(), false),
            ],
        );
        assert_eq!(
            names,
            HashSet::from([
                "ANTHROPIC_API_KEY".to_string(),
                "CURSOR_API_KEY".to_string()
            ]),
            "names only, and only non-empty ones"
        );
        assert_eq!(with("claude-code", Some(&bin), &names), SignIn::SignedIn);
        assert_eq!(with("cursor", Some(&bin), &names), SignIn::SignedIn);
        assert!(!ran.exists(), "no command ran");
        assert_eq!(
            with("codex", Some(&bin), &names),
            SignIn::SignedOut,
            "an empty key"
        );
        let groq = present([("GROQ_API_KEY".into(), "g".into())], &[]);
        assert_eq!(with("opencode", Some(&bin), &groq), SignIn::SignedIn);
        assert_eq!(with("command-code", Some(&bin), &names), SignIn::Unknown);
    }

    #[test]
    fn opencode_is_its_stores_size_and_never_signed_out() {
        let t = tempfile::tempdir().unwrap();
        let bin = script(t.path(), "exit 1");
        let store = t.path().join("auth.json");
        let at = |store: &Path| {
            probe(Probe {
                agent: "opencode",
                command: Some(Command::new(&bin)),
                env: &HashSet::new(),
                opencode_store: Some(store),
                deadline: DEADLINE,
            })
        };
        assert_eq!(at(&store), SignIn::Unknown, "no store");
        std::fs::write(&store, "{}").unwrap();
        assert_eq!(at(&store), SignIn::Unknown, "an empty store");
        std::fs::write(&store, r#"{"anthropic":{"type":"api","key":"k"}}"#).unwrap();
        assert_eq!(at(&store), SignIn::SignedIn);
    }

    #[test]
    fn the_wire_says_nothing_for_unknown() {
        assert_eq!(SignIn::SignedIn.wire(), Some(true));
        assert_eq!(SignIn::SignedOut.wire(), Some(false));
        assert_eq!(SignIn::Unknown.wire(), None);
    }
}
