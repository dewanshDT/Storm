//! The real-CLI gates of decisions 144, 145, 146 and 147's runtime half, for Claude
//! Code and OpenCode: each runs the real agent in a terminal that is only
//! typed into like a person would, with the hooks or plugin Storm's own
//! writer produces when every list is on, reporting to a socket this test
//! serves as the host does.
//!
//! - **G-144-S-<provider>**: the agent's own signals arrive: its turn
//!   starting (`working`) and ending (`idle`), and a usage limit on a 429.
//!   Positive control: a turn that hits no limit reports none.
//! - **G-145-R-<provider>**: the turn's final message arrives, word for
//!   word, with the tokens the agent counted. Positive control: a turn that
//!   fails on a 429 reports no final message.
//! - **G-144-W-<provider>** and **G-145-W-<provider>**: an idle agent is
//!   woken by its documented mechanism (Claude Code's `asyncRewake` hook,
//!   OpenCode's plugin `client.session.promptAsync`), and the model then
//!   receives exactly Storm's fixed sentence (a parent's for 144, a
//!   child's for 145). Positive control: without the wake it stays idle.
//! - **G-146-A-claude-code**: a child's Storm tools allowed in Storm's
//!   `--settings` run with no prompt, and a note write still asks.
//!   Positive control: without the allow, `report_result` asks.
//! - **G-146-M-<provider>**: a permission mode reaches the agent and works:
//!   Claude Code's `--permission-mode acceptEdits` and `auto` run an edit
//!   with no prompt over an account whose own mode asks, OpenCode's
//!   `edit: allow` in Storm's layer over the account's `edit: ask`.
//!   Positive control: the account's mode alone asks.
//! - **G-147-claude-code**: Storm's manual flag starts in Manual and an edit
//!   prompts; Storm's `disableAutoMode` keeps auto out of the Shift+Tab
//!   cycle while the test plays the person (positive control: without it,
//!   auto is reached); and whether an edit to the `--settings` file while
//!   the CLI runs is picked up (positive control: the user settings are).
//! - **G-146-P-<provider>**: a permission prompt sets the flag and the
//!   person's answer clears it (Claude Code: `PermissionRequest`, then
//!   `PostToolUse`; after an Esc, the next turn; OpenCode:
//!   `permission.asked`, then `permission.replied`). The test plays the
//!   person, typing the answer into the terminal; Storm never does.
//!   Positive control: a turn with no tool sets nothing.
//!
//! Runs only with `STORM_LIFECYCLE_GATE=1` (`make gate-lifecycle`); never in
//! CI. By default each agent talks to a stand-in model server started here
//! (no account is used): Claude Code (`STORM_CLAUDE_BIN`, default `claude`)
//! with a throwaway home and an API key the stand-in accepts, OpenCode
//! (`STORM_OPENCODE_BIN`, default `opencode`) with a throwaway config and
//! data directory. With `STORM_GATE_REAL=1` they use the account's own
//! login and model instead (Claude Code `STORM_GATE_CLAUDE_MODEL`, default
//! `haiku`; OpenCode `STORM_GATE_OPENCODE_MODEL`, required), and the checks
//! that need the stand-in (a 429, an exact reply) say so and are skipped.
//! `STORM_GATE_WORKSPACE_PARENT` is where the throwaway workspace goes in
//! real mode (a folder Claude Code already trusts). Every result is printed
//! as `G-… PASS|FAIL|SKIP`, and a failed check fails its test.
#![cfg(unix)]

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use storm_runtime::harness::{Hooks, WAKE_EVENTS_TEXT, WAKE_MESSAGE_TEXT};

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

/// A stand-in for Anthropic's Messages API, streaming: `ECHO:<word>` in
/// the last user message is answered with the word, `LIMIT` with a 429 as
/// a usage limit gives it, anything else with `Done.`. Every request is
/// logged, one JSON line each, with the last user message's text.
const FAKE_ANTHROPIC: &str = r#"
import http.server, json, sys, time
LOG = open(sys.argv[1], "a")
def text_of(msg):
    c = msg.get("content")
    if isinstance(c, str):
        return c
    out = []
    for b in c or []:
        if b.get("type") == "text":
            out.append(b.get("text", ""))
        elif b.get("type") == "tool_result":
            out.append("[tool_result]")
    return "\n".join(out)
class H(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    def log_message(self, *a): pass
    def send_json(self, code, obj, headers=()):
        b = json.dumps(obj).encode()
        self.send_response(code)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(b)))
        for k, v in headers: self.send_header(k, v)
        self.end_headers(); self.wfile.write(b)
    def do_GET(self):
        self.send_json(200, {})
    def do_HEAD(self):
        self.send_response(200); self.send_header("content-length", "0"); self.end_headers()
    def do_POST(self):
        raw = self.rfile.read(int(self.headers.get("content-length", 0) or 0))
        try: body = json.loads(raw or b"{}")
        except Exception: body = {}
        if "count_tokens" in self.path:
            return self.send_json(200, {"input_tokens": 10})
        if not self.path.startswith("/v1/messages"):
            return self.send_json(200, {})
        msgs = body.get("messages", [])
        users = [m for m in msgs if m.get("role") == "user"]
        last = text_of(users[-1]) if users else ""
        # The newest prompt only: a turn after a failed one is sent with it.
        newest = last.strip().split("\n")[-1] if last.strip() else ""
        LOG.write(json.dumps({"t": time.time(), "model": body.get("model"), "last": last[-4000:], "tools": [t.get("name") for t in body.get("tools") or []]}) + "\n"); LOG.flush()
        if "LIMIT" in newest and "Storm" not in newest:
            hdr = [("anthropic-ratelimit-unified-status", "rejected"), ("retry-after", "3600"), ("x-should-retry", "false")]
            return self.send_json(429, {"type": "error", "error": {"type": "rate_limit_error", "message": "This request would exceed your account's rate limit. Please try again later."}}, hdr)
        reply = "Done."
        if "ECHO:" in newest:
            reply = newest.split("ECHO:", 1)[1].split()[0]
        # A tool call the turn asks for, only where the request offers it.
        tools = [t.get("name") for t in body.get("tools") or []]
        call = None
        if "WRITE:" in newest and "Write" in tools:
            call = ("Write", {"file_path": newest.split("WRITE:", 1)[1].split()[0], "content": "storm gate\n"})
        elif "CALL:" in newest and newest.split("CALL:", 1)[1].split()[0] in tools:
            call = (newest.split("CALL:", 1)[1].split()[0], {})
        model = body.get("model", "m")
        tid = "toolu_gate%d" % int(time.time() * 1000)
        if not body.get("stream"):
            content = [{"type": "tool_use", "id": tid, "name": call[0], "input": call[1]}] if call else [{"type": "text", "text": reply}]
            return self.send_json(200, {"id": "msg_x", "type": "message", "role": "assistant", "model": model, "content": content, "stop_reason": "tool_use" if call else "end_turn", "stop_sequence": None, "usage": {"input_tokens": 10, "output_tokens": 5}})
        ev = []
        def e(t, d): ev.append("event: %s\ndata: %s\n\n" % (t, json.dumps(d)))
        e("message_start", {"type": "message_start", "message": {"id": "msg_%d" % int(time.time() * 1000), "type": "message", "role": "assistant", "model": model, "content": [], "stop_reason": None, "stop_sequence": None, "usage": {"input_tokens": 10, "output_tokens": 1}}})
        if call:
            e("content_block_start", {"type": "content_block_start", "index": 0, "content_block": {"type": "tool_use", "id": tid, "name": call[0], "input": {}}})
            e("content_block_delta", {"type": "content_block_delta", "index": 0, "delta": {"type": "input_json_delta", "partial_json": json.dumps(call[1])}})
        else:
            e("content_block_start", {"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}})
            e("content_block_delta", {"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": reply}})
        e("content_block_stop", {"type": "content_block_stop", "index": 0})
        e("message_delta", {"type": "message_delta", "delta": {"stop_reason": "tool_use" if call else "end_turn", "stop_sequence": None}, "usage": {"output_tokens": 5}})
        e("message_stop", {"type": "message_stop"})
        data = "".join(ev).encode()
        self.send_response(200)
        self.send_header("content-type", "text/event-stream")
        self.send_header("content-length", str(len(data)))
        self.end_headers(); self.wfile.write(data); self.wfile.flush()
s = http.server.ThreadingHTTPServer(("127.0.0.1", 0), H)
print(s.server_address[1], flush=True)
s.serve_forever()
"#;

/// A stand-in OpenAI-compatible model with the same three answers, and
/// the usage of each answer in its last chunk.
const FAKE_OPENAI: &str = r#"
import http.server, json, sys, time
LOG = open(sys.argv[1], "a")
class H(http.server.BaseHTTPRequestHandler):
    def log_message(self, *a): pass
    def do_GET(self):
        self.send_response(200); self.send_header("content-type", "application/json"); self.end_headers()
        self.wfile.write(json.dumps({"data": [{"id": "m", "object": "model"}]}).encode())
    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers.get("content-length", 0))) or b"{}")
        msgs = body.get("messages", [])
        users = [m for m in msgs if m.get("role") == "user"]
        last = users[-1].get("content") if users else ""
        last = last if isinstance(last, str) else json.dumps(last)
        LOG.write(json.dumps({"t": time.time(), "last": last[-4000:]}) + "\n"); LOG.flush()
        if "LIMIT" in last and "Storm" not in last:
            b = json.dumps({"error": {"message": "Rate limit reached for requests", "type": "rate_limit_error", "code": "rate_limit_exceeded"}}).encode()
            self.send_response(429); self.send_header("content-type", "application/json"); self.send_header("retry-after", "1"); self.send_header("content-length", str(len(b))); self.end_headers(); self.wfile.write(b); return
        reply = "Done."
        if "ECHO:" in last:
            reply = last.split("ECHO:", 1)[1].split()[0].strip('"\\')
        def chunk(delta, finish=None, usage=None):
            c = {"id": "c", "object": "chat.completion.chunk", "created": 0, "model": "m", "choices": [{"index": 0, "delta": delta, "finish_reason": finish}]}
            if usage: c["usage"] = usage
            return "data: " + json.dumps(c) + "\n\n"
        usage = {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}
        tools = [(t.get("function") or {}).get("name") for t in body.get("tools") or []]
        answered = bool(msgs) and msgs[-1].get("role") == "tool"
        if "WRITE:" in last and "write" in tools and not answered:
            path = last.split("WRITE:", 1)[1].split()[0].strip('"\\')
            args = json.dumps({"filePath": path, "content": "storm gate\n"})
            call = {"index": 0, "id": "call_gate%d" % int(time.time() * 1000), "type": "function", "function": {"name": "write", "arguments": args}}
            out = [chunk({"role": "assistant", "tool_calls": [call]}), chunk({}, "tool_calls", usage)]
        else:
            out = [chunk({"role": "assistant", "content": reply}), chunk({}, "stop", usage)]
        self.send_response(200); self.send_header("content-type", "text/event-stream"); self.end_headers()
        for c in out + ["data: [DONE]\n\n"]:
            self.wfile.write(c.encode()); self.wfile.flush()
s = http.server.ThreadingHTTPServer(("127.0.0.1", 0), H)
print(s.server_address[1], flush=True)
s.serve_forever()
"#;

fn enabled(gate: &str) -> bool {
    let on = std::env::var("STORM_LIFECYCLE_GATE").as_deref() == Ok("1");
    if !on {
        eprintln!("STORM_LIFECYCLE_GATE is not 1; skipping {gate}");
    }
    on
}

fn real() -> bool {
    std::env::var("STORM_GATE_REAL").as_deref() == Ok("1")
}

/// What the agent's hooks and plugin sent, with when.
type Lines = Arc<Mutex<Vec<(Instant, Value)>>>;

/// The host's side of the socket: a report is one line, then closed; a
/// waker stays, and the newest one is the session's (older ones are
/// released at once, as the host does).
struct FakeHost {
    lines: Lines,
    waiters: Arc<Mutex<Vec<UnixStream>>>,
}

impl FakeHost {
    fn listen(path: &Path) -> Self {
        let listener = UnixListener::bind(path).unwrap();
        let lines: Lines = Arc::default();
        let waiters: Arc<Mutex<Vec<UnixStream>>> = Arc::default();
        let (seen, held) = (lines.clone(), waiters.clone());
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut line = String::new();
                if BufReader::new(&stream).read_line(&mut line).is_err() {
                    continue;
                }
                let Ok(v) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                eprintln!("  socket: {}", short(&v));
                seen.lock().unwrap().push((Instant::now(), v.clone()));
                if v.get("wake").is_some() {
                    let mut held = held.lock().unwrap();
                    for mut old in held.drain(..) {
                        let _ = old.write_all(b"{\"release\":true}\n");
                    }
                    held.push(stream);
                }
            }
        });
        Self { lines, waiters }
    }

    fn since(&self, at: Instant) -> Vec<Value> {
        self.lines
            .lock()
            .unwrap()
            .iter()
            .filter(|(t, _)| *t >= at)
            .map(|(_, v)| v.clone())
            .collect()
    }

    fn signals(&self, at: Instant) -> Vec<String> {
        self.since(at)
            .iter()
            .filter_map(|v| v["signal"].as_str().map(str::to_string))
            .collect()
    }

    /// What the permission prompt hooks said: `permission_prompt` or
    /// `none`.
    fn attention(&self, at: Instant) -> Vec<String> {
        self.since(at)
            .iter()
            .filter_map(|v| v["attention"].as_str().map(str::to_string))
            .collect()
    }

    fn results(&self, at: Instant) -> Vec<Value> {
        self.since(at)
            .into_iter()
            .filter(|v| v.get("final_message").is_some())
            .collect()
    }

    fn waiting(&self) -> bool {
        !self.waiters.lock().unwrap().is_empty()
    }

    /// Tells the session's waiter `kind` is unread, as the host does.
    fn wake(&self, kind: &str) -> bool {
        let Some(mut w) = self.waiters.lock().unwrap().pop() else {
            return false;
        };
        w.write_all(format!("{}\n", json!({"wake": kind})).as_bytes())
            .is_ok()
    }
}

fn short(v: &Value) -> String {
    let mut v = v.clone();
    if let Some(h) = v.get_mut("handle") {
        *h = json!("…");
    }
    let s = v.to_string();
    s.chars().take(200).collect()
}

/// A stand-in model server and its request log.
struct Model {
    child: Child,
    log: PathBuf,
    port: String,
}

impl Model {
    fn start(script: &str, dir: &Path) -> Self {
        let log = dir.join("model.log");
        let mut child = Command::new("python3")
            .args(["-c", script])
            .arg(&log)
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut port = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut port)
            .unwrap();
        Self {
            child,
            log,
            port: port.trim().to_string(),
        }
    }

    /// The last user message of every request so far.
    fn requests(&self) -> Vec<String> {
        std::fs::read_to_string(&self.log)
            .unwrap_or_default()
            .lines()
            .filter_map(|l| serde_json::from_str::<Value>(l).ok())
            .map(|v| v["last"].as_str().unwrap_or_default().to_string())
            .collect()
    }
}

impl Drop for Model {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
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
            if k.starts_with("CLAUDE")
                || k.starts_with("STORM_")
                || k.starts_with("OPENCODE")
                || k.starts_with("ANTHROPIC")
            {
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

    fn screen_has(&self, text: &str) -> bool {
        let raw = std::fs::read(&self.screen).unwrap_or_default();
        String::from_utf8_lossy(&raw).contains(text)
    }

    /// How much the terminal has shown so far.
    fn shown(&self) -> usize {
        std::fs::metadata(&self.screen).map_or(0, |m| m.len() as usize)
    }

    /// The permission mode Claude Code's footer last said after `from`.
    fn mode_since(&self, from: usize) -> Option<&'static str> {
        let raw = std::fs::read(&self.screen).unwrap_or_default();
        let text = plain(&String::from_utf8_lossy(&raw[from.min(raw.len())..])).to_lowercase();
        MODES
            .iter()
            .filter_map(|(word, mode)| text.rfind(word).map(|at| (at, *mode)))
            .max_by_key(|(at, _)| *at)
            .map(|(_, mode)| mode)
    }

    /// Plays the person: Shift+Tab `n` times, the footer's mode after each.
    fn cycle(&mut self, n: usize) -> Vec<&'static str> {
        (0..n)
            .map(|_| {
                let at = self.shown();
                self.keys("\x1b[Z");
                std::thread::sleep(Duration::from_millis(1500));
                self.mode_since(at).unwrap_or("?")
            })
            .collect()
    }

    /// The end of what the terminal showed, without escape sequences.
    fn tail(&self) -> String {
        let raw = std::fs::read(&self.screen).unwrap_or_default();
        let out = plain(&String::from_utf8_lossy(&raw));
        let start = out.len().saturating_sub(2500);
        out[out.floor_char_boundary(start)..].to_string()
    }
}

/// Claude Code's footer word for each mode it cycles through.
const MODES: [(&str, &str); 4] = [
    ("manual mode on", "manual"),
    ("accept edits on", "acceptEdits"),
    ("plan mode on", "plan"),
    ("auto mode on", "auto"),
];

/// Terminal output without escape sequences.
fn plain(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            match chars.next() {
                Some('[') => {
                    while chars
                        .next()
                        .is_some_and(|c| !c.is_ascii_alphabetic() && c != '~')
                    {}
                }
                Some(']') => while chars.next().is_some_and(|c| c != '\x07') {},
                _ => {}
            }
        } else if !c.is_control() || c == '\n' {
            out.push(c);
        }
    }
    out
}

impl Drop for Agent {
    fn drop(&mut self) {
        drop(self.child.stdin.take());
        let _ = self.child.wait();
    }
}

fn wait_until(secs: u64, mut ok: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < deadline {
        if ok() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    ok()
}

/// One gate's checks, printed as they run.
struct Checks {
    gate: String,
    results: Vec<(String, Option<bool>)>,
}

impl Checks {
    fn new(gate: &str) -> Self {
        eprintln!("{gate}: running");
        Self {
            gate: gate.to_string(),
            results: Vec::new(),
        }
    }

    fn check(&mut self, name: &str, ok: bool) -> bool {
        eprintln!("  {}  {name}", if ok { "PASS" } else { "FAIL" });
        self.results.push((name.to_string(), Some(ok)));
        ok
    }

    fn skip(&mut self, name: &str, why: &str) {
        eprintln!("  SKIP  {name} ({why})");
        self.results.push((name.to_string(), None));
    }

    /// `finish` for a gate whose sessions are already gone.
    fn finish_quiet(self) {
        let failed: Vec<&String> = self
            .results
            .iter()
            .filter(|(_, ok)| *ok == Some(false))
            .map(|(n, _)| n)
            .collect();
        let skipped = self.results.iter().any(|(_, ok)| ok.is_none());
        let word = match (failed.is_empty(), skipped) {
            (false, _) => "FAIL",
            (true, true) => "PASS (with skips)",
            (true, false) => "PASS",
        };
        eprintln!("{} {word}", self.gate);
        assert!(failed.is_empty(), "{} failed: {failed:?}", self.gate);
    }

    fn finish(self, agent: &Agent) {
        let failed: Vec<&String> = self
            .results
            .iter()
            .filter(|(_, ok)| *ok == Some(false))
            .map(|(n, _)| n)
            .collect();
        let skipped = self.results.iter().any(|(_, ok)| ok.is_none());
        let word = match (failed.is_empty(), skipped) {
            (false, _) => "FAIL",
            (true, true) => "PASS (with skips)",
            (true, false) => "PASS",
        };
        eprintln!("{} {word}", self.gate);
        if !failed.is_empty() {
            eprintln!("--- the terminal's last words ---\n{}", agent.tail());
        }
        assert!(failed.is_empty(), "{} failed: {failed:?}", self.gate);
    }
}

/// A running agent with Storm's hooks, all gated parts on, and its host.
struct Session {
    agent: Agent,
    host: FakeHost,
    model: Option<Model>,
    /// Its workspace.
    ws: PathBuf,
    /// The stand-in Storm MCP server's log of tool calls, when it has one.
    mcp_log: Option<PathBuf>,
    /// Claude Code's: Storm's `--settings` file and its throwaway home.
    settings: Option<PathBuf>,
    home: Option<PathBuf>,
    _dirs: Vec<tempfile::TempDir>,
}

impl Session {
    fn requests(&self) -> Vec<String> {
        self.model.as_ref().map(Model::requests).unwrap_or_default()
    }

    /// The tools the stand-in Storm server ran.
    fn storm_calls(&self) -> Vec<String> {
        self.mcp_log
            .as_ref()
            .map(|p| std::fs::read_to_string(p).unwrap_or_default())
            .unwrap_or_default()
            .lines()
            .filter_map(|l| serde_json::from_str::<Value>(l).ok())
            .filter_map(|v| v["name"].as_str().map(str::to_string))
            .collect()
    }
}

/// A stand-in for Storm's MCP server, named `storm` as the host names it:
/// it lists a child's tools and a note write, and logs every call.
const FAKE_STORM_MCP: &str = r#"
import json, sys
LOG = open(sys.argv[1], "a")
TOOLS = ["session_context", "report_result", "read_messages", "create_note"]
for line in sys.stdin:
    try:
        m = json.loads(line)
    except Exception:
        continue
    if m.get("id") is None:
        continue
    method = m.get("method")
    if method == "initialize":
        res = {"protocolVersion": (m.get("params") or {}).get("protocolVersion", "2025-06-18"), "capabilities": {"tools": {}}, "serverInfo": {"name": "storm", "version": "0"}}
    elif method == "tools/list":
        res = {"tools": [{"name": t, "description": "Storm gate stand-in: " + t, "inputSchema": {"type": "object", "properties": {}}} for t in TOOLS]}
    elif method == "tools/call":
        LOG.write(json.dumps(m.get("params")) + "\n"); LOG.flush()
        res = {"content": [{"type": "text", "text": "ok"}]}
    else:
        res = {}
    sys.stdout.write(json.dumps({"jsonrpc": "2.0", "id": m["id"], "result": res}) + "\n")
    sys.stdout.flush()
"#;

/// How a gate's Claude Code is launched beyond Storm's hooks: extra flags
/// (a permission mode), Storm's allows in its `--settings` (146a), and the
/// stand-in Storm server.
#[derive(Default)]
struct ClaudeOpts {
    args: Vec<String>,
    allow: Vec<String>,
    /// Storm's `permissions.disableAutoMode` in its `--settings` (147c).
    auto_off: bool,
    storm_mcp: bool,
    /// The account's own `permissions.defaultMode` (its user settings).
    /// Claude Code 2.1.312's own default is `auto`; `manual` is what asks.
    account_mode: Option<&'static str>,
}

fn workspace() -> tempfile::TempDir {
    let parent = std::env::var_os("STORM_GATE_WORKSPACE_PARENT")
        .map(PathBuf::from)
        .filter(|_| real())
        .unwrap_or_else(std::env::temp_dir);
    tempfile::Builder::new()
        .prefix("storm-lifecycle-gate-")
        .tempdir_in(parent)
        .unwrap()
}

fn env_of(extras: &storm_runtime::provider::LaunchExtras) -> Vec<(String, String)> {
    extras
        .env
        .iter()
        .map(|(k, v)| {
            (
                k.to_string_lossy().to_string(),
                v.to_string_lossy().to_string(),
            )
        })
        .collect()
}

const FAKE_KEY: &str = "sk-ant-api03-STORMGATE000000000000000000000000000000000000000000000000000000000000000000000000-AAAAAAAA";

fn claude() -> Session {
    claude_with(ClaudeOpts::default())
}

fn claude_with(opts: ClaudeOpts) -> Session {
    let bin = std::env::var("STORM_CLAUDE_BIN").unwrap_or_else(|_| "claude".into());
    let ws = workspace();
    let state = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let socket = state.path().join("mcp.sock");
    let host = FakeHost::listen(&socket);
    let (extras, _) = storm_runtime::harness::launch_with(
        "claude-code",
        &state.path().join("session"),
        "h_gate",
        &socket,
        Path::new(env!("CARGO_BIN_EXE_storm-runtime")),
        None,
        None,
        &Hooks::all(),
        storm_runtime::harness::Permissions {
            allow: &opts.allow,
            auto_off: opts.auto_off,
        },
    )
    .unwrap();
    let mut argv = vec![bin];
    argv.extend(extras.args.iter().map(|a| a.to_string_lossy().to_string()));
    argv.extend(opts.args);
    let mcp_log = opts.storm_mcp.then(|| state.path().join("storm-mcp.log"));
    if let Some(log) = &mcp_log {
        let config = state.path().join("mcp.json");
        std::fs::write(
            &config,
            json!({"mcpServers": {"storm": {
                "type": "stdio", "command": "python3",
                "args": ["-c", FAKE_STORM_MCP, log.display().to_string()],
            }}})
            .to_string(),
        )
        .unwrap();
        argv.extend(["--mcp-config".to_string(), config.display().to_string()]);
    }
    let mut env = env_of(&extras);
    let model = if real() {
        let m = std::env::var("STORM_GATE_CLAUDE_MODEL").unwrap_or_else(|_| "haiku".into());
        argv.extend(["--model".to_string(), m]);
        None
    } else {
        let model = Model::start(FAKE_ANTHROPIC, state.path());
        let ws_path = ws.path().canonicalize().unwrap();
        std::fs::write(
            home.path().join(".claude.json"),
            json!({
                "hasCompletedOnboarding": true,
                "theme": "dark",
                "numStartups": 5,
                "customApiKeyResponses": {"approved": [&FAKE_KEY[FAKE_KEY.len() - 20..]], "rejected": []},
                "projects": {ws_path.display().to_string(): {
                    "hasTrustDialogAccepted": true,
                    "hasCompletedProjectOnboarding": true,
                    "allowedTools": [],
                }},
            })
            .to_string(),
        )
        .unwrap();
        if let Some(mode) = opts.account_mode {
            let dir = home.path().join(".claude");
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(
                dir.join("settings.json"),
                json!({"permissions": {"defaultMode": mode}}).to_string(),
            )
            .unwrap();
        }
        env.extend([
            ("HOME".to_string(), home.path().display().to_string()),
            (
                "ANTHROPIC_BASE_URL".to_string(),
                format!("http://127.0.0.1:{}", model.port),
            ),
            ("ANTHROPIC_API_KEY".to_string(), FAKE_KEY.to_string()),
        ]);
        Some(model)
    };
    env.push(("DISABLE_AUTOUPDATER".to_string(), "1".to_string()));
    if opts.storm_mcp {
        // So the stand-in model is offered the MCP tools themselves, not
        // Claude Code's tool search in front of them.
        env.push(("ENABLE_TOOL_SEARCH".to_string(), "false".to_string()));
    }
    let mut agent = Agent::start(argv, env, ws.path(), state.path().join("screen"));
    let started = Instant::now();
    // Ready: its SessionStart hook reported the conversation and armed the
    // waker. A real account's first look at a folder asks for trust.
    while started.elapsed() < Duration::from_secs(90)
        && !(host
            .since(started)
            .iter()
            .any(|v| v.get("harness_session").is_some())
            && host.waiting())
    {
        if real() && agent.screen_has("trust") && started.elapsed() > Duration::from_secs(8) {
            agent.keys("\r");
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    std::thread::sleep(Duration::from_secs(3));
    Session {
        agent,
        host,
        model,
        ws: ws.path().canonicalize().unwrap(),
        mcp_log,
        settings: Some(state.path().join("session").join("claude-settings.json")),
        home: Some(home.path().to_path_buf()),
        _dirs: vec![ws, state, home],
    }
}

/// How a gate's OpenCode is launched: the account's own permission rules
/// (its global config) and Storm's layer on top (`OPENCODE_CONFIG_CONTENT`).
#[derive(Default)]
struct OpencodeOpts {
    account_permission: Option<Value>,
    storm_permission: Option<Value>,
}

fn opencode() -> Session {
    opencode_with(OpencodeOpts::default())
}

fn opencode_with(opts: OpencodeOpts) -> Session {
    let bin = std::env::var("STORM_OPENCODE_BIN").unwrap_or_else(|_| "opencode".into());
    let ws = workspace();
    let state = tempfile::tempdir().unwrap();
    let socket = state.path().join("mcp.sock");
    let host = FakeHost::listen(&socket);
    let (extras, _) = storm_runtime::harness::launch(
        "opencode",
        &state.path().join("session"),
        "h_gate",
        &socket,
        Path::new(env!("CARGO_BIN_EXE_storm-runtime")),
        None,
        None,
        &Hooks::all(),
        &[],
    )
    .unwrap();
    let mut env = env_of(&extras);
    let model = if real() {
        let m = std::env::var("STORM_GATE_OPENCODE_MODEL")
            .expect("STORM_GATE_OPENCODE_MODEL names the model, as provider/model");
        let mut config = json!({"model": m, "small_model": m});
        if let Some(p) = &opts.storm_permission {
            config["permission"] = p.clone();
        }
        env.push(("OPENCODE_CONFIG_CONTENT".to_string(), config.to_string()));
        // The account's own rule, as a config OpenCode merges under Storm's.
        if let Some(p) = &opts.account_permission {
            let file = state.path().join("account-opencode.json");
            std::fs::write(&file, json!({"permission": p}).to_string()).unwrap();
            env.push(("OPENCODE_CONFIG".to_string(), file.display().to_string()));
        }
        None
    } else {
        let model = Model::start(FAKE_OPENAI, state.path());
        let config = json!({
            "$schema": "https://opencode.ai/config.json",
            "model": "gate/m",
            "small_model": "gate/m",
            "autoupdate": false,
            "share": "disabled",
            "provider": {"gate": {
                "npm": "@ai-sdk/openai-compatible",
                "name": "Gate",
                "options": {"baseURL": format!("http://127.0.0.1:{}/v1", model.port), "apiKey": "x"},
                "models": {"m": {"name": "m", "tool_call": true}},
            }},
        });
        let mut config = config;
        if let Some(p) = &opts.storm_permission {
            config["permission"] = p.clone();
        }
        let xdg = |name: &str| {
            let dir = state.path().join(name);
            std::fs::create_dir_all(&dir).unwrap();
            dir.display().to_string()
        };
        if let Some(p) = &opts.account_permission {
            let dir = state.path().join("config").join("opencode");
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(
                dir.join("opencode.json"),
                json!({"$schema": "https://opencode.ai/config.json", "permission": p}).to_string(),
            )
            .unwrap();
        }
        env.extend([
            ("XDG_CONFIG_HOME".to_string(), xdg("config")),
            ("XDG_DATA_HOME".to_string(), xdg("data")),
            ("XDG_STATE_HOME".to_string(), xdg("state")),
            ("OPENCODE_CONFIG_CONTENT".to_string(), config.to_string()),
            ("OPENCODE_DISABLE_MODELS_FETCH".to_string(), "1".to_string()),
        ]);
        Some(model)
    };
    env.push((
        "OPENCODE_DISABLE_PROJECT_CONFIG".to_string(),
        "1".to_string(),
    ));
    let mut argv = vec![bin];
    argv.extend(extras.args.iter().map(|a| a.to_string_lossy().to_string()));
    let agent = Agent::start(argv, env, ws.path(), state.path().join("screen"));
    // Ready: the plugin is loaded (its waker is waiting), and the TUI has
    // drawn its prompt. A first run fetches the plugin's package.
    let started = Instant::now();
    wait_until(120, || host.waiting());
    std::thread::sleep(Duration::from_secs(8));
    eprintln!(
        "  opencode ready after {} s (waker: {})",
        started.elapsed().as_secs(),
        host.waiting()
    );
    Session {
        agent,
        host,
        model,
        ws: ws.path().canonicalize().unwrap(),
        mcp_log: None,
        settings: None,
        home: None,
        _dirs: vec![ws, state],
    }
}

fn session(provider: &str) -> Session {
    match provider {
        "claude-code" => claude(),
        _ => opencode(),
    }
}

/// A turn that ends: its prompt typed, and the hooks' `idle` after it.
fn turn(s: &mut Session, prompt: &str, secs: u64) -> (Instant, bool) {
    let at = Instant::now();
    s.agent.prompt(prompt);
    let ended = wait_until(secs, || s.host.signals(at).iter().any(|k| k == "idle"));
    (at, ended)
}

/// G-144-S-<provider>.
fn signals(provider: &str) {
    let gate = format!("G-144-S-{provider}");
    if !enabled(&gate) {
        return;
    }
    let mut s = session(provider);
    let mut c = Checks::new(&gate);
    let (at, ended) = turn(&mut s, "Say hello", 120);
    let seen = s.host.signals(at);
    c.check(&format!("a turn says working then idle ({seen:?})"), {
        let w = seen.iter().position(|k| k == "working");
        let i = seen.iter().rposition(|k| k == "idle");
        ended && w.is_some_and(|w| i.is_some_and(|i| w < i))
    });
    c.check(
        "control: a turn with no limit says no usage_limit",
        !seen.iter().any(|k| k == "usage_limit"),
    );
    if real() {
        c.skip("a 429 says usage_limit", "needs the stand-in model");
    } else {
        let at = Instant::now();
        s.agent.prompt("LIMIT now");
        let limited = wait_until(120, || {
            s.host.signals(at).iter().any(|k| k == "usage_limit")
        });
        c.check(
            &format!("a 429 says usage_limit ({:?})", s.host.signals(at)),
            limited,
        );
        let at = Instant::now();
        let (_, ended) = turn(&mut s, "Say hello again", 120);
        c.check(
            "the next turn clears it (working again)",
            ended && s.host.signals(at).iter().any(|k| k == "working"),
        );
    }
    c.finish(&s.agent);
}

/// G-145-R-<provider>.
fn final_message(provider: &str) {
    let gate = format!("G-145-R-{provider}");
    if !enabled(&gate) {
        return;
    }
    let mut s = session(provider);
    let mut c = Checks::new(&gate);
    let canary = format!("canary{}", std::process::id());
    if real() {
        c.skip(
            "control: a failed turn gives no final message",
            "needs the stand-in model",
        );
    } else {
        let at = Instant::now();
        s.agent.prompt("LIMIT now");
        wait_until(120, || s.host.signals(at).iter().any(|k| k == "idle"));
        std::thread::sleep(Duration::from_secs(3));
        c.check(
            "control: a failed turn gives no final message",
            s.host.results(at).is_empty(),
        );
    }
    let prompt = if real() {
        format!("Reply with exactly this one word and nothing else: {canary}")
    } else {
        format!("ECHO:{canary}")
    };
    let at = Instant::now();
    s.agent.prompt(&prompt);
    let got = wait_until(120, || {
        s.host.results(at).iter().any(|r| {
            r["final_message"]
                .as_str()
                .is_some_and(|t| t.contains(&canary))
        })
    });
    let results = s.host.results(at);
    c.check(
        &format!(
            "the turn's final message arrives ({:?})",
            results
                .iter()
                .map(|r| r["final_message"].clone())
                .collect::<Vec<_>>()
        ),
        got,
    );
    if !real() {
        c.check(
            "word for word",
            results
                .iter()
                .any(|r| r["final_message"].as_str() == Some(canary.as_str())),
        );
    }
    let usage = results
        .iter()
        .find(|r| {
            r["final_message"]
                .as_str()
                .is_some_and(|t| t.contains(&canary))
        })
        .map(|r| r["usage"].clone())
        .unwrap_or(Value::Null);
    c.check(
        &format!("with the agent's own token count ({usage})"),
        usage["output_tokens"].as_u64().is_some_and(|n| n > 0),
    );
    c.finish(&s.agent);
}

/// G-144-W-<provider> (`events`, a parent's) and G-145-W-<provider>
/// (`message`, a child's).
fn wake(provider: &str, kind: &str) {
    let (gate, sentence) = match kind {
        "events" => (format!("G-144-W-{provider}"), WAKE_EVENTS_TEXT),
        _ => (format!("G-145-W-{provider}"), WAKE_MESSAGE_TEXT),
    };
    if !enabled(&gate) {
        return;
    }
    let mut s = session(provider);
    let mut c = Checks::new(&gate);
    let (_, ended) = turn(&mut s, "Say hello", 120);
    c.check("the agent went idle after its turn", ended);
    c.check(
        "a waker waits on the session's socket",
        wait_until(30, || s.host.waiting()),
    );
    // The control: nothing is pushed, so nothing happens.
    let quiet = Instant::now();
    let before = s.requests().len();
    std::thread::sleep(Duration::from_secs(
        std::env::var("STORM_GATE_QUIET_SECONDS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(20),
    ));
    if real() {
        c.check(
            "control: idle without a wake, no turn starts",
            !s.host.signals(quiet).iter().any(|k| k == "working"),
        );
    } else {
        c.check(
            "control: idle without a wake, the model hears nothing",
            s.requests().len() == before && !s.host.signals(quiet).iter().any(|k| k == "working"),
        );
    }
    let at = Instant::now();
    c.check("the host's wake reached the waker", s.host.wake(kind));
    let woke = wait_until(60, || s.host.signals(at).iter().any(|k| k == "working"));
    c.check("the agent woke: a turn started", woke);
    if !real() {
        let heard = wait_until(30, || {
            s.requests()[before..].iter().any(|r| r.contains(sentence))
        });
        let new: Vec<String> = s.requests()[before..]
            .iter()
            .map(|r| {
                r.chars()
                    .rev()
                    .take(160)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect()
            })
            .collect();
        c.check(
            &format!("the model received Storm's fixed sentence ({new:?})"),
            heard,
        );
        c.check(
            "and nothing else of Storm's",
            s.requests()[before..]
                .iter()
                .all(|r| !r.contains("unread") || r.contains(sentence)),
        );
    }
    c.check(
        "the agent re-armed its waker",
        wait_until(60, || s.host.waiting()),
    );
    c.finish(&s.agent);
}

#[test]
fn g_144_s_claude_code() {
    signals("claude-code");
}

#[test]
fn g_144_s_opencode() {
    signals("opencode");
}

#[test]
fn g_145_r_claude_code() {
    final_message("claude-code");
}

#[test]
fn g_145_r_opencode() {
    final_message("opencode");
}

#[test]
fn g_144_w_claude_code() {
    wake("claude-code", "events");
}

#[test]
fn g_144_w_opencode() {
    wake("opencode", "events");
}

#[test]
fn g_145_w_claude_code() {
    wake("claude-code", "message");
}

#[test]
fn g_145_w_opencode() {
    wake("opencode", "message");
}

/// Waits for a turn to end: the hooks' `idle` since `at`.
fn idle_since(s: &Session, at: Instant, secs: u64) -> bool {
    wait_until(secs, || s.host.signals(at).iter().any(|k| k == "idle"))
}

fn asked(s: &Session, at: Instant, secs: u64) -> bool {
    wait_until(secs, || {
        s.host
            .attention(at)
            .iter()
            .any(|k| k == "permission_prompt")
    })
}

/// G-146-A: Storm's own tools are pre-allowed (146a, A-146-1) on the real
/// Claude Code: a child whose `--settings` allows `mcp__storm__<tool>` runs
/// `report_result`, `session_context` and `read_messages` with no prompt,
/// while a note write still asks. Positive control: the same call without
/// the allow stops on Claude Code's own prompt, as VM 113's child did.
#[test]
fn g_146_a_claude_code() {
    let gate = "G-146-A-claude-code";
    if !enabled(gate) {
        return;
    }
    if real() {
        eprintln!("{gate} SKIP (it needs the stand-in model to call a tool)");
        return;
    }
    let mut c = Checks::new(gate);
    let mut control = claude_with(ClaudeOpts {
        storm_mcp: true,
        account_mode: Some("manual"),
        ..Default::default()
    });
    let at = Instant::now();
    control.agent.prompt("CALL:mcp__storm__report_result");
    c.check(
        "control: without the allow, report_result stops on the CLI's prompt",
        asked(&control, at, 90) && control.storm_calls().is_empty(),
    );
    drop(control);

    let allow: Vec<String> = storm_runtime::mcp::CHILD_ALLOWED_TOOLS
        .iter()
        .map(|t| format!("mcp__storm__{t}"))
        .collect();
    let mut s = claude_with(ClaudeOpts {
        allow,
        storm_mcp: true,
        account_mode: Some("manual"),
        ..Default::default()
    });
    for tool in storm_runtime::mcp::CHILD_ALLOWED_TOOLS {
        let at = Instant::now();
        s.agent.prompt(&format!("CALL:mcp__storm__{tool}"));
        let ran = wait_until(90, || s.storm_calls().iter().any(|t| t == tool));
        idle_since(&s, at, 60);
        c.check(
            &format!("{tool} runs with no prompt ({:?})", s.host.attention(at)),
            ran && !s
                .host
                .attention(at)
                .iter()
                .any(|k| k == "permission_prompt"),
        );
    }
    let at = Instant::now();
    s.agent.prompt("CALL:mcp__storm__create_note");
    c.check(
        "a note write still asks",
        asked(&s, at, 90) && !s.storm_calls().iter().any(|t| t == "create_note"),
    );
    c.finish(&s.agent);
}

/// G-146-M-claude-code: the host's Claude Code takes `--permission-mode
/// acceptEdits` and `auto` (146b) and they behave as documented: an edit in
/// the workspace runs with no prompt. Positive control: `default` (no flag)
/// prompts for the same edit and does not write it.
#[test]
fn g_146_m_claude_code() {
    let gate = "G-146-M-claude-code";
    if !enabled(gate) {
        return;
    }
    if real() {
        eprintln!("{gate} SKIP (it needs the stand-in model to call a tool)");
        return;
    }
    let mut c = Checks::new(gate);
    let edit = |s: &mut Session| -> (bool, bool, PathBuf) {
        let file = s.ws.join("gate-edit.txt");
        let at = Instant::now();
        s.agent.prompt(&format!("WRITE:{}", file.display()));
        let written = wait_until(60, || file.exists());
        let asked = !s
            .host
            .attention(at)
            .iter()
            .all(|k| k != "permission_prompt")
            || asked(s, at, if written { 3 } else { 30 });
        (written, asked, file)
    };
    // 147a: `default` is the CLI's asking mode by name, never no flag.
    c.check(
        "default is --permission-mode default (147a)",
        storm_runtime::profile::permission_mode("claude-code", "default")
            .unwrap()
            .args
            == ["--permission-mode", "default"],
    );
    let mut own = claude();
    let (written, prompted, _) = edit(&mut own);
    let says_auto = own.agent.screen_has("auto mode on");
    drop(own);
    if prompted && !written {
        c.check("default (no flag) prompts for an edit", true);
    } else {
        c.skip(
            "default (no flag) prompts for an edit",
            &format!(
                "this CLI's own default did not ask (written {written}, prompted {prompted}, \
                 footer says auto mode: {says_auto}); the operator decides what default means"
            ),
        );
    }
    let mut control = claude_with(ClaudeOpts {
        account_mode: Some("manual"),
        ..Default::default()
    });
    let (written, prompted, _) = edit(&mut control);
    c.check(
        "control: with the account's manual mode and no flag, an edit prompts and is not written",
        prompted && !written,
    );
    drop(control);

    let mode_args = |m: &str| {
        storm_runtime::profile::permission_mode("claude-code", m)
            .unwrap()
            .args
    };
    for (mode, flag) in [("accept_edits", "acceptEdits"), ("auto", "auto")] {
        let args = mode_args(mode);
        c.check(
            &format!("{mode} is --permission-mode {flag}"),
            args == ["--permission-mode", flag],
        );
        let mut s = claude_with(ClaudeOpts {
            args,
            account_mode: Some("manual"),
            ..Default::default()
        });
        let accepted = s
            .host
            .lines
            .lock()
            .unwrap()
            .iter()
            .any(|(_, v)| v.get("harness_session").is_some());
        c.check(
            &format!("the CLI accepts --permission-mode {flag} and starts"),
            accepted && s.agent.child.try_wait().ok().flatten().is_none(),
        );
        let (written, prompted, _) = edit(&mut s);
        c.check(
            &format!(
                "{flag}: the edit runs with no prompt (written {written}, prompted {prompted})"
            ),
            written && !prompted,
        );
    }
    c.finish_quiet();
}

/// G-146-M-opencode: `accept_edits` is `edit: allow` in Storm's config
/// layer, and it holds over the account's own `edit: ask`. Positive
/// control: with the account's rule alone, the edit asks.
#[test]
fn g_146_m_opencode() {
    let gate = "G-146-M-opencode";
    if !enabled(gate) {
        return;
    }
    if real() {
        eprintln!("{gate} SKIP (it needs the stand-in model to call a tool)");
        return;
    }
    let mut c = Checks::new(gate);
    let layer = storm_runtime::profile::permission_mode("opencode", "accept_edits").unwrap();
    let storm: serde_json::Map<String, Value> = layer
        .opencode_permission
        .iter()
        .map(|(k, v)| (k.clone(), json!(v)))
        .collect();
    c.check(
        "accept_edits is edit: allow",
        Value::Object(storm.clone()) == json!({"edit": "allow"}),
    );
    for (storm, want_prompt) in [(None, true), (Some(Value::Object(storm)), false)] {
        let mut s = opencode_with(OpencodeOpts {
            account_permission: Some(json!({"edit": "ask"})),
            storm_permission: storm,
        });
        let file = s.ws.join("gate-edit.txt");
        let at = Instant::now();
        s.agent.prompt(&format!("WRITE:{}", file.display()));
        let written = wait_until(60, || file.exists());
        let prompted = asked(&s, at, if written { 3 } else { 30 });
        if want_prompt {
            c.check(
                &format!(
                    "control: the account's edit: ask asks (prompted {prompted}, written {written})"
                ),
                prompted && !written,
            );
        } else {
            c.check(
                &format!("Storm's edit: allow runs it with no prompt (prompted {prompted}, written {written})"),
                written && !prompted,
            );
        }
    }
    c.finish_quiet();
}

/// G-146-P-<provider>: the prompt flag is set and cleared by the real
/// CLI's own hooks or plugin (146c). The test plays the person at the
/// terminal and answers the prompt; Storm's code never does. Positive
/// control: a turn with no tool call sets nothing.
fn prompt_flag(provider: &str) {
    let gate = format!("G-146-P-{provider}");
    if !enabled(&gate) {
        return;
    }
    if real() {
        eprintln!("{gate} SKIP (it needs the stand-in model to call a tool)");
        return;
    }
    let mut c = Checks::new(&gate);
    let mut s = match provider {
        "claude-code" => claude_with(ClaudeOpts {
            account_mode: Some("manual"),
            ..Default::default()
        }),
        _ => opencode_with(OpencodeOpts {
            account_permission: Some(json!({"edit": "ask"})),
            ..Default::default()
        }),
    };
    let (at, ended) = turn(&mut s, "Say hello", 120);
    c.check(
        &format!(
            "control: a turn with no tool sets nothing ({:?})",
            s.host.attention(at)
        ),
        ended
            && !s
                .host
                .attention(at)
                .iter()
                .any(|k| k == "permission_prompt"),
    );
    let file = s.ws.join("gate-prompt.txt");
    let at = Instant::now();
    s.agent.prompt(&format!("WRITE:{}", file.display()));
    let set = asked(&s, at, 90);
    c.check(
        &format!(
            "the prompt sets permission_prompt ({:?})",
            s.host.attention(at)
        ),
        set && !file.exists(),
    );
    std::thread::sleep(Duration::from_secs(2));
    c.check(
        "it stays set while nobody answers",
        !file.exists()
            && s.host.attention(at).last().map(String::as_str) == Some("permission_prompt"),
    );
    // The person, at the terminal: Enter takes the highlighted "allow once".
    let answered = Instant::now();
    s.agent.keys("\r");
    let written = wait_until(60, || file.exists());
    let cleared = wait_until(60, || {
        s.host.attention(answered).iter().any(|k| k == "none")
    });
    c.check(
        &format!(
            "the person's answer runs the tool and clears it (written {written}, {:?})",
            s.host.attention(answered)
        ),
        written && cleared,
    );
    if provider == "claude-code" {
        // Esc runs no hook: the flag clears on the next turn boundary.
        let file = s.ws.join("gate-esc.txt");
        let at = Instant::now();
        s.agent.prompt(&format!("WRITE:{}", file.display()));
        let set = asked(&s, at, 90);
        let esc = Instant::now();
        s.agent.keys("\x1b");
        std::thread::sleep(Duration::from_secs(4));
        let held = !s.host.attention(esc).iter().any(|k| k == "none");
        let next = Instant::now();
        s.agent.prompt("Say hello");
        let cleared = wait_until(60, || s.host.attention(next).iter().any(|k| k == "none"));
        c.check(
            &format!(
                "after Esc it clears at the next turn (set {set}, held {held}, cleared {cleared})"
            ),
            set && cleared && !file.exists(),
        );
    }
    c.finish(&s.agent);
}

#[test]
fn g_146_p_claude_code() {
    prompt_flag("claude-code");
}

#[test]
fn g_146_p_opencode() {
    prompt_flag("opencode");
}

/// G-147-claude-code (decision 147): on the real CLI,
/// - 147a: Storm's `default` is `--permission-mode default`, which starts in
///   Manual over the CLI's own default (auto) and makes an edit prompt;
///   `manual`, the newer spelling, is the same mode. Positive control: with
///   no flag and no account mode the CLI starts in auto.
/// - 147c: with Storm's `permissions.disableAutoMode` in its `--settings`,
///   the person's Shift+Tab (the test plays the person) never reaches auto
///   and cycles manual, accept edits and plan. Positive control: without
///   it, auto is reached. An auto launch, which gets no such setting, keeps
///   auto in the cycle.
/// - Reload: whether an edit to the `--settings` file while the CLI runs is
///   picked up (taking the setting out puts auto back in the cycle). It
///   must match `harness::SETTINGS_FLAG_RELOADS`, which Storm's switch is
///   built on. Positive control: an edit to the account's own user
///   settings is picked up (adding the setting takes auto out).
#[test]
fn g_147_claude_code() {
    let gate = "G-147-claude-code";
    if !enabled(gate) {
        return;
    }
    if real() {
        eprintln!("{gate} SKIP (it needs the stand-in model and a throwaway home)");
        return;
    }
    let mut c = Checks::new(gate);
    let manual = storm_runtime::profile::permission_mode("claude-code", "default").unwrap();
    c.check(
        "default is --permission-mode default, with auto out of the cycle",
        manual.args == ["--permission-mode", "default"] && manual.auto_off,
    );
    let auto = storm_runtime::profile::permission_mode("claude-code", "auto").unwrap();
    c.check(
        "auto is --permission-mode auto, with auto kept in the cycle",
        auto.args == ["--permission-mode", "auto"] && !auto.auto_off,
    );

    // Positive control: the CLI's own default, no flag, no setting.
    let mut own = claude();
    let starts = own.agent.mode_since(0);
    let seen = own.agent.cycle(5);
    c.check(
        &format!("control: with no flag it starts in auto ({starts:?}), and Shift+Tab reaches auto ({seen:?})"),
        starts == Some("auto") && seen.contains(&"auto"),
    );
    drop(own);

    // Storm's manual session, as the host launches it.
    let mut s = claude_with(ClaudeOpts {
        args: manual.args.clone(),
        auto_off: true,
        ..Default::default()
    });
    let starts = s.agent.mode_since(0);
    c.check(
        &format!("--permission-mode default starts in Manual ({starts:?})"),
        starts == Some("manual"),
    );
    let file = s.ws.join("gate-147.txt");
    let at = Instant::now();
    s.agent.prompt(&format!("WRITE:{}", file.display()));
    let prompted = asked(&s, at, 60);
    c.check(
        &format!(
            "an edit prompts in Manual (prompted {prompted}, written {})",
            file.exists()
        ),
        prompted && !file.exists(),
    );
    s.agent.keys("\x1b");
    std::thread::sleep(Duration::from_secs(2));
    let seen = s.agent.cycle(8);
    c.check(
        &format!("with disableAutoMode, Shift+Tab never reaches auto ({seen:?})"),
        !seen.contains(&"auto")
            && ["manual", "acceptEdits", "plan"]
                .iter()
                .all(|m| seen.contains(m)),
    );

    // Reload: Storm's file loses the setting while the CLI runs.
    let path = s.settings.clone().unwrap();
    let mut settings: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    settings["permissions"]
        .as_object_mut()
        .unwrap()
        .remove("disableAutoMode");
    std::fs::write(&path, settings.to_string()).unwrap();
    std::thread::sleep(Duration::from_secs(12));
    let seen = s.agent.cycle(6);
    let reloads = seen.contains(&"auto");
    eprintln!("  finding: an edit to the --settings file while running is picked up: {reloads}");
    c.check(
        &format!(
            "the --settings file is re-read while running: {reloads} ({seen:?}), as \
             SETTINGS_FLAG_RELOADS ({}) says",
            storm_runtime::harness::SETTINGS_FLAG_RELOADS
        ),
        reloads == storm_runtime::harness::SETTINGS_FLAG_RELOADS,
    );
    drop(s);

    // Control for the reload: the account's own user settings are watched.
    let mut watched = claude_with(ClaudeOpts {
        args: manual.args.clone(),
        ..Default::default()
    });
    let before = watched.agent.cycle(5);
    let dir = watched.home.clone().unwrap().join(".claude");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("settings.json"),
        json!({"permissions": {"disableAutoMode": "disable"}}).to_string(),
    )
    .unwrap();
    std::thread::sleep(Duration::from_secs(12));
    let after = watched.agent.cycle(6);
    c.check(
        &format!(
            "control: an edit to the user settings is picked up (auto before {before:?}, none after {after:?})"
        ),
        before.contains(&"auto") && !after.contains(&"auto"),
    );
    drop(watched);

    // An auto launch keeps auto in the cycle; the newer spelling is manual.
    let mut a = claude_with(ClaudeOpts {
        args: auto.args.clone(),
        auto_off: auto.auto_off,
        ..Default::default()
    });
    let starts = a.agent.mode_since(0);
    let seen = a.agent.cycle(5);
    c.check(
        &format!("an auto launch starts in auto ({starts:?}) and keeps it in the cycle ({seen:?})"),
        starts == Some("auto") && seen.contains(&"auto"),
    );
    drop(a);
    let m = claude_with(ClaudeOpts {
        args: vec!["--permission-mode".into(), "manual".into()],
        auto_off: true,
        ..Default::default()
    });
    let starts = m.agent.mode_since(0);
    c.check(
        &format!("--permission-mode manual is the same mode ({starts:?})"),
        starts == Some("manual"),
    );
    c.finish(&m.agent);
}
