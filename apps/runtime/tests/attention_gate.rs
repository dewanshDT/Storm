//! G-134-7: `needs_attention`'s permission prompt hooks set and clear it,
//! and idle never does (134-9), against the real Claude Code and OpenCode.
//!
//! Runs only with `STORM_ATTENTION_GATE=1`, or `make gate-attention`; never
//! in CI. Each agent runs interactively in a terminal with the hooks Storm's
//! own writer produces when an adapter is on, and the `storm-runtime
//! attention` command they run reports to a socket this test listens on, as
//! the host's does. The gate types into the terminal like a person:
//!
//! 1. a prompt whose command needs permission: the hooks must say
//!    `permission_prompt`;
//! 2. Enter allows it: they must say `none`, and the command must have run;
//! 3. the agent then sits idle at its prompt (`STORM_GATE_IDLE_SECONDS`,
//!    default 75, past Claude Code's 60 s idle notification): nothing may
//!    say `permission_prompt`;
//! 4. another such prompt, refused with Esc: they must say `none`.
//!
//! **Claude Code** (`STORM_CLAUDE_BIN`, default `claude`, signed in) runs
//! with `--permission-mode default` and `STORM_GATE_CLAUDE_MODEL` (default
//! `haiku`). **OpenCode** (`STORM_OPENCODE_BIN`, default `opencode`) runs
//! with an empty `XDG_CONFIG_HOME` and a stand-in model server that always
//! asks for the `bash` tool, so its permission flow runs without a model
//! account; `bash` is set to `ask`. `STORM_GATE_WORKSPACE_PARENT` is where
//! the throwaway workspace goes (a folder Claude Code already trusts); it is
//! removed afterwards. Codex: manual (no hooks are written for it).
#![cfg(unix)]

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

/// Runs `GATE_ARGV` in a terminal, typing each line of its stdin (a JSON
/// string) and answering the cursor-position query; the output goes to
/// `GATE_SCREEN`.
const DRIVER: &str = r#"
import fcntl, json, os, pty, select, struct, termios
argv = json.loads(os.environ.pop("GATE_ARGV"))
cwd = os.environ.pop("GATE_CWD")
screen = open(os.environ.pop("GATE_SCREEN"), "ab")
pid, fd = pty.fork()
if pid == 0:
    os.chdir(cwd)
    os.execvp(argv[0], argv)
fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 40, 120, 0, 0))
buf = b""
while True:
    r, _, _ = select.select([fd, 0], [], [], 0.2)
    if fd in r:
        try:
            b = os.read(fd, 65536)
        except OSError:
            break
        if not b:
            break
        screen.write(b)
        screen.flush()
        if b"\x1b[6n" in b:
            os.write(fd, b"\x1b[1;1R")
    if 0 in r:
        d = os.read(0, 65536)
        if not d:
            break
        buf += d
        while b"\n" in buf:
            line, buf = buf.split(b"\n", 1)
            os.write(fd, json.loads(line).encode())
try:
    os.kill(pid, 9)
except ProcessLookupError:
    pass
"#;

/// An OpenAI-compatible model that asks for `bash` with the command after
/// `RUN:` in the last user message, and otherwise says it is done.
const FAKE_MODEL: &str = r#"
import http.server, json, sys
n = [0]
class H(http.server.BaseHTTPRequestHandler):
    def log_message(self, *a): pass
    def do_GET(self):
        self.send_response(200); self.send_header("content-type", "application/json"); self.end_headers()
        self.wfile.write(json.dumps({"data": [{"id": "m", "object": "model"}]}).encode())
    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers.get("content-length", 0))) or b"{}")
        msgs = body.get("messages", [])
        tools = [t["function"]["name"] for t in body.get("tools", [])]
        def chunk(delta, finish=None):
            return "data: " + json.dumps({"id": "c", "object": "chat.completion.chunk", "created": 0, "model": "m",
                "choices": [{"index": 0, "delta": delta, "finish_reason": finish}]}) + "\n\n"
        last = msgs[-1] if msgs else {}
        text = last.get("content")
        text = text if isinstance(text, str) else json.dumps(text)
        if "bash" in tools and last.get("role") == "user" and "RUN:" in text:
            cmd = text.split("RUN:", 1)[1].split("\n")[0].split('"')[0].strip()
            n[0] += 1
            out = [chunk({"role": "assistant", "tool_calls": [{"index": 0, "id": "call_%d" % n[0], "type": "function",
                "function": {"name": "bash", "arguments": json.dumps({"command": cmd, "description": "Run it"})}}]}),
                chunk({}, "tool_calls")]
        else:
            out = [chunk({"role": "assistant", "content": "Done."}), chunk({}, "stop")]
        self.send_response(200); self.send_header("content-type", "text/event-stream"); self.end_headers()
        for c in out + ["data: [DONE]\n\n"]:
            self.wfile.write(c.encode()); self.wfile.flush()
s = http.server.ThreadingHTTPServer(("127.0.0.1", 0), H)
print(s.server_address[1], flush=True)
s.serve_forever()
"#;

fn enabled() -> bool {
    std::env::var("STORM_ATTENTION_GATE").as_deref() == Ok("1")
}

fn idle_seconds() -> u64 {
    std::env::var("STORM_GATE_IDLE_SECONDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(75)
}

fn workspace() -> tempfile::TempDir {
    let parent = std::env::var_os("STORM_GATE_WORKSPACE_PARENT")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    tempfile::Builder::new()
        .prefix("storm-attention-gate-")
        .tempdir_in(parent)
        .unwrap()
}

/// What the agent's hooks and plugin sent, with when.
type Lines = Arc<Mutex<Vec<(Instant, Value)>>>;

/// The host's side of the socket: each connection is one line, then closed.
fn listen(path: &Path) -> Lines {
    let listener = UnixListener::bind(path).unwrap();
    let lines: Lines = Arc::default();
    let seen = lines.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut line = String::new();
            if BufReader::new(&stream).read_line(&mut line).is_ok()
                && let Ok(v) = serde_json::from_str::<Value>(&line)
            {
                eprintln!("  socket: {v}");
                seen.lock().unwrap().push((Instant::now(), v));
            }
        }
    });
    lines
}

fn attention(lines: &Lines, since: Instant) -> Vec<String> {
    lines
        .lock()
        .unwrap()
        .iter()
        .filter(|(at, _)| *at >= since)
        .filter_map(|(_, v)| v["attention"].as_str().map(str::to_string))
        .collect()
}

fn wait_for(lines: &Lines, since: Instant, kind: &str, secs: u64) -> bool {
    let deadline = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < deadline {
        if attention(lines, since).iter().any(|a| a == kind) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    false
}

struct Agent {
    child: Child,
    screen: PathBuf,
}

impl Agent {
    fn start(argv: Vec<String>, env: Vec<(String, String)>, cwd: &Path, screen: PathBuf) -> Self {
        let mut c = Command::new("python3");
        c.args(["-c", DRIVER]);
        for (k, _) in std::env::vars_os() {
            let k = k.to_string_lossy().to_string();
            if k.starts_with("CLAUDE") || k.starts_with("STORM_") || k.starts_with("OPENCODE") {
                c.env_remove(k);
            }
        }
        c.env("TERM", "xterm-256color")
            .env("GATE_ARGV", serde_json::to_string(&argv).unwrap())
            .env("GATE_CWD", cwd)
            .env("GATE_SCREEN", &screen)
            .envs(env)
            .stdin(Stdio::piped());
        Agent {
            child: c.spawn().unwrap(),
            screen,
        }
    }

    fn keys(&mut self, keys: &str) {
        let stdin = self.child.stdin.as_mut().unwrap();
        writeln!(stdin, "{}", serde_json::to_string(keys).unwrap()).unwrap();
        stdin.flush().unwrap();
    }

    /// Types a prompt, then Enter on its own, as a paste and a keypress.
    fn prompt(&mut self, text: &str) {
        self.keys(text);
        std::thread::sleep(Duration::from_millis(800));
        self.keys("\r");
    }

    /// The end of what the terminal showed, without escape sequences.
    fn tail(&self) -> String {
        let raw = std::fs::read(&self.screen).unwrap_or_default();
        let text = String::from_utf8_lossy(&raw);
        let mut out = String::new();
        let mut chars = text.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\x1b' {
                match chars.next() {
                    Some('[') => {
                        while chars
                            .next()
                            .is_some_and(|c| !c.is_ascii_alphabetic() && c != '~')
                        {
                        }
                    }
                    Some(']') => while chars.next().is_some_and(|c| c != '\x07') {},
                    _ => {}
                }
            } else if !c.is_control() || c == '\n' {
                out.push(c);
            }
        }
        let start = out.len().saturating_sub(2500);
        out[out.floor_char_boundary(start)..].to_string()
    }

    fn screen_has(&self, text: &str) -> bool {
        let raw = std::fs::read(&self.screen).unwrap_or_default();
        String::from_utf8_lossy(&raw).contains(text)
    }
}

impl Drop for Agent {
    fn drop(&mut self) {
        drop(self.child.stdin.take());
        let _ = self.child.wait();
    }
}

struct Checks(Vec<(String, bool)>);

impl Checks {
    fn check(&mut self, name: &str, ok: bool) -> bool {
        eprintln!("  {}  {name}", if ok { "PASS" } else { "FAIL" });
        self.0.push((name.to_string(), ok));
        ok
    }

    fn finish(self, agent: &str, screen: String) {
        let failed: Vec<_> = self
            .0
            .iter()
            .filter(|(_, ok)| !ok)
            .map(|(n, _)| n)
            .collect();
        eprintln!(
            "G-134-7 {agent}: {}",
            if failed.is_empty() {
                "passed"
            } else {
                "FAILED"
            }
        );
        if !failed.is_empty() {
            eprintln!("--- the terminal's last words ---\n{screen}");
        }
        assert!(failed.is_empty(), "G-134-7 failed for {agent}: {failed:?}");
    }
}

/// Steps 1–4 against a started agent. `ready` waits for its prompt.
fn run_steps(
    agent: &mut Agent,
    lines: &Lines,
    ws: &Path,
    prompt: &dyn Fn(&str) -> String,
) -> Checks {
    let mut c = Checks(Vec::new());
    let since = Instant::now();
    agent.prompt(&prompt("echo one > one.txt"));
    if !c.check(
        "a permission prompt sets it",
        wait_for(lines, since, "permission_prompt", 180),
    ) {
        return c;
    }
    std::thread::sleep(Duration::from_secs(2));
    let since = Instant::now();
    agent.keys("\r");
    c.check("allowing clears it", wait_for(lines, since, "none", 30));
    std::thread::sleep(Duration::from_secs(10));
    c.check("the allowed command ran", ws.join("one.txt").exists());

    let since = Instant::now();
    std::thread::sleep(Duration::from_secs(idle_seconds()));
    let idle = attention(lines, since);
    c.check(
        &format!("idle for {} s never sets it ({idle:?})", idle_seconds()),
        !idle.iter().any(|a| a == "permission_prompt"),
    );

    let since = Instant::now();
    agent.prompt(&prompt("echo two > two.txt"));
    if !c.check(
        "a second prompt sets it",
        wait_for(lines, since, "permission_prompt", 180),
    ) {
        return c;
    }
    std::thread::sleep(Duration::from_secs(2));
    let since = Instant::now();
    agent.keys("\x1b");
    c.check(
        "refusing with Esc clears it",
        wait_for(lines, since, "none", 30),
    );
    c.check(
        "the refused command did not run",
        !ws.join("two.txt").exists(),
    );
    c
}

#[test]
fn claude_codes_hooks_set_and_clear_a_permission_prompt() {
    if !enabled() {
        eprintln!("STORM_ATTENTION_GATE is not 1; skipping G-134-7 for Claude Code");
        return;
    }
    let bin = std::env::var("STORM_CLAUDE_BIN").unwrap_or_else(|_| "claude".into());
    let model = std::env::var("STORM_GATE_CLAUDE_MODEL").unwrap_or_else(|_| "haiku".into());
    let ws = workspace();
    let state = tempfile::tempdir().unwrap();
    let socket = state.path().join("mcp.sock");
    let lines = listen(&socket);
    let (extras, _) = storm_runtime::harness::launch(
        "claude-code",
        &state.path().join("session"),
        "h_gate",
        &socket,
        Path::new(env!("CARGO_BIN_EXE_storm-runtime")),
        None,
        None,
        None,
        true,
    )
    .unwrap();
    let mut argv = vec![bin];
    argv.extend(extras.args.iter().map(|a| a.to_string_lossy().to_string()));
    argv.extend(["--permission-mode", "default", "--model", &model].map(String::from));
    let env = extras
        .env
        .iter()
        .map(|(k, v)| {
            (
                k.to_string_lossy().to_string(),
                v.to_string_lossy().to_string(),
            )
        })
        .collect();
    let mut agent = Agent::start(argv, env, ws.path(), state.path().join("screen"));
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(60)
        && !lines
            .lock()
            .unwrap()
            .iter()
            .any(|(_, v)| v.get("harness_session").is_some())
    {
        if agent.screen_has("trust") && started.elapsed() > Duration::from_secs(8) {
            agent.keys("\r");
        }
        std::thread::sleep(Duration::from_secs(1));
    }
    std::thread::sleep(Duration::from_secs(3));
    let mut c = run_steps(&mut agent, &lines, ws.path(), &|cmd| {
        format!("Run this exact command with the Bash tool, nothing else: {cmd}")
    });
    // What does clear it after a refusal: the person's next prompt.
    let since = Instant::now();
    agent.prompt("Reply with the single word OK.");
    let next = wait_for(&lines, since, "none", 30);
    eprintln!("  note: the next prompt clears it: {next}");
    c.check(
        "hooks ran (SessionStart)",
        lines
            .lock()
            .unwrap()
            .iter()
            .any(|(_, v)| v.get("harness_session").is_some()),
    );
    let screen = agent.tail();
    drop(agent);
    c.finish("Claude Code", screen);
}

#[test]
fn opencodes_plugin_sets_and_clears_a_permission_prompt() {
    if !enabled() {
        eprintln!("STORM_ATTENTION_GATE is not 1; skipping G-134-7 for OpenCode");
        return;
    }
    let bin = std::env::var("STORM_OPENCODE_BIN").unwrap_or_else(|_| "opencode".into());
    let ws = workspace();
    let state = tempfile::tempdir().unwrap();
    let xdg = state.path().join("xdg");
    std::fs::create_dir(&xdg).unwrap();
    let socket = state.path().join("mcp.sock");
    let lines = listen(&socket);
    let mut model = Command::new("python3")
        .args(["-c", FAKE_MODEL])
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut port = String::new();
    BufReader::new(model.stdout.take().unwrap())
        .read_line(&mut port)
        .unwrap();
    let (extras, _) = storm_runtime::harness::launch(
        "opencode",
        &state.path().join("session"),
        "h_gate",
        &socket,
        Path::new(env!("CARGO_BIN_EXE_storm-runtime")),
        None,
        None,
        None,
        true,
    )
    .unwrap();
    let config = json!({
        "$schema": "https://opencode.ai/config.json",
        "model": "gate/m",
        "small_model": "gate/m",
        "provider": {"gate": {
            "npm": "@ai-sdk/openai-compatible",
            "name": "Gate",
            "options": {"baseURL": format!("http://127.0.0.1:{}/v1", port.trim()), "apiKey": "x"},
            "models": {"m": {"name": "m", "tool_call": true}},
        }},
        "permission": {"bash": "ask"},
    });
    let mut env: Vec<(String, String)> = extras
        .env
        .iter()
        .map(|(k, v)| {
            (
                k.to_string_lossy().to_string(),
                v.to_string_lossy().to_string(),
            )
        })
        .collect();
    env.extend([
        ("XDG_CONFIG_HOME".to_string(), xdg.display().to_string()),
        ("OPENCODE_CONFIG_CONTENT".to_string(), config.to_string()),
        (
            "OPENCODE_DISABLE_PROJECT_CONFIG".to_string(),
            "1".to_string(),
        ),
        ("OPENCODE_DISABLE_MODELS_FETCH".to_string(), "1".to_string()),
    ]);
    let mut argv = vec![bin];
    argv.extend(extras.args.iter().map(|a| a.to_string_lossy().to_string()));
    let mut agent = Agent::start(argv, env, ws.path(), state.path().join("screen"));
    // A first run fetches the model server's package before the prompt shows.
    std::thread::sleep(Duration::from_secs(30));
    let c = run_steps(&mut agent, &lines, ws.path(), &|cmd| format!("RUN: {cmd}"));
    let screen = agent.tail();
    drop(agent);
    let _ = model.kill();
    let _ = model.wait();
    c.finish("OpenCode", screen);
}
