//! The agent's own session id, so Storm can resume the conversation rather
//! than start a new one (decision 89).
//!
//! Claude Code and OpenCode each keep their conversations under an id of
//! their own, per host and per workspace. The host learns it while the
//! session runs, never by reading the terminal:
//!
//! - **`claude-code`**: a fresh session is started with `--session-id <uuid>`,
//!   and every session gets a `SessionStart` hook (through `--settings`, a
//!   file in the session's directory) that
//!   runs `storm-runtime harness-session`. The hook fires on startup, resume,
//!   `/clear` and compaction, with the id the conversation now has.
//! - **`opencode`**: no id can be chosen up front. `OPENCODE_CONFIG_DIR`
//!   points at the session's own directory, which holds a plugin that runs
//!   `storm-runtime harness-session <id>` for the top-level session a message
//!   lands in. The variable adds to the operator's config, never replaces it.
//!
//! `harness-session` sends the id to the daemon's socket with the session's
//! handle, which only that session was given. A resume starts the CLI with
//! `--resume <id>` or `-s <id>`.
//!
//! The same hooks and plugin carry, each only for an agent on its gated
//! list (all empty until the operator's real-CLI gates pass):
//!
//! - **144b's signals**: the agent's own idle, usage limit and permission
//!   prompt, reported with `harness-signal` and posted by the host as
//!   `provider_hook` evidence ([`GATED_SIGNALS`]);
//! - **145c's final message**: the turn's last assistant message and what
//!   it cost, reported with `harness-result` ([`RESULT_SOURCES`]);
//! - **144c/145d's waker**: `harness-wake`, a long-poll on the session's
//!   own socket that ends with the agent's documented wake when Storm says
//!   something is unread ([`WAKE`]): Claude Code's `asyncRewake` hook, or
//!   OpenCode's plugin calling `client.session.promptAsync`. Never terminal
//!   input (C9), and never `/tui/append-prompt` or `/tui/submit-prompt`.
//!
//! Every one of these commands prints nothing a session reads (131a): the
//! waker's fixed sentence goes to stderr, which is how Claude Code's
//! `asyncRewake` takes it.

use std::io::{Read, Write};
use std::path::Path;

use anyhow::Result;
use rand::Rng;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::mcp::{private_dir, write_private};
use crate::provider::LaunchExtras;

pub const HANDLE_ENV: &str = "STORM_HARNESS_HANDLE";

/// The longest a report waits for the host to take it: well past the host's
/// own wait for a session still starting.
const REPORT_WAIT: std::time::Duration = std::time::Duration::from_secs(10);

/// One line from `harness-session` on the daemon socket.
#[derive(Debug, Deserialize, Serialize)]
pub struct HarnessLine {
    pub handle: String,
    pub harness_session: String,
}

/// The adapters whose hooks report a permission prompt as `needs_attention`
/// (decision 134p): each joins only once G-134-7 passes for it
/// (`tests/attention_gate.rs`), and 134 lands dark, so none is on. Claude
/// Code 2.1.310 failed: an Esc at its prompt runs no hook, so nothing
/// clears it. OpenCode 1.18.31 passed. Codex has no hooks written. The
/// server keeps the same list and takes a report only from both.
pub const PERMISSION_PROMPT_ADAPTERS: &[&str] = &[];

pub const PERMISSION_PROMPT: &str = "permission_prompt";
/// What a hook says when the prompt is answered.
pub const ATTENTION_CLEAR: &str = "none";

/// One provider's signal, through one source, reported once its gate
/// (G-144-S-<provider>) passes on the real CLI. The server keeps the same
/// list (`attention::GATED_SIGNALS`), spelled the same with string
/// literals; a test holds the two equal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GatedSignal {
    pub provider: &'static str,
    /// `permission_prompt`, `idle` or `usage_limit`.
    pub signal: &'static str,
    /// `provider_hook`.
    pub source: &'static str,
}

/// None has passed its gate.
pub const GATED_SIGNALS: &[GatedSignal] = &[];

/// One agent's final message as a round's result (145c), once its gate
/// (G-145-R-<provider>) passes. The server's `round::RESULT_SOURCES` is the
/// same list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResultSource {
    pub provider: &'static str,
    pub source: &'static str,
}

/// None has passed its gate.
pub const RESULT_SOURCES: &[ResultSource] = &[];

/// One agent's documented wake (144c, 145d), once its gates
/// (G-144-W-<provider>, G-145-W-<provider>) pass. The server's
/// `events::WAKE` is the same list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GatedWake {
    pub provider: &'static str,
    /// `async_rewake` (Claude Code) or `prompt_async` (OpenCode).
    pub mechanism: &'static str,
}

/// None has passed its gate.
pub const WAKE: &[GatedWake] = &[];

pub const IDLE: &str = "idle";
pub const USAGE_LIMIT: &str = "usage_limit";
pub const PROVIDER_HOOK: &str = "provider_hook";

/// What the waker tells a parent, word for word: the agent then reads its
/// events through the tool (144c). Nothing of any event is in it.
pub const WAKE_EVENTS_TEXT: &str = "There are unread Storm events; call wait_agent.";
/// What the waker tells a child (145d): the message is read through the
/// tool, never typed.
pub const WAKE_MESSAGE_TEXT: &str = "A Storm message is waiting; call read_messages.";

/// The lists a host reports from: the compiled ones, or the tests' own.
#[derive(Debug, Clone, Copy)]
pub struct Gates {
    pub permission_prompt: &'static [&'static str],
    pub signals: &'static [GatedSignal],
    pub results: &'static [ResultSource],
    pub wakes: &'static [GatedWake],
}

impl Gates {
    pub const COMPILED: Gates = Gates {
        permission_prompt: PERMISSION_PROMPT_ADAPTERS,
        signals: GATED_SIGNALS,
        results: RESULT_SOURCES,
        wakes: WAKE,
    };

    fn signal(&self, provider: &str, signal: &str) -> bool {
        self.signals
            .iter()
            .any(|g| g.provider == provider && g.signal == signal && g.source == PROVIDER_HOOK)
    }

    /// What `provider`'s hooks report under these lists.
    pub fn hooks(&self, provider: &str) -> Hooks {
        Hooks {
            attention: self.permission_prompt.contains(&provider)
                || self.signal(provider, PERMISSION_PROMPT),
            idle: self.signal(provider, IDLE),
            usage_limit: self.signal(provider, USAGE_LIMIT),
            result: self
                .results
                .iter()
                .any(|r| r.provider == provider && r.source == PROVIDER_HOOK),
            wake: self.wakes.iter().any(|w| w.provider == provider),
        }
    }
}

/// What a session's hooks or plugin report beyond its id. Everything off
/// writes every file exactly as before any of this was built.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Hooks {
    /// 134-9: the permission prompt, set and cleared.
    pub attention: bool,
    /// 144b: the agent's turn starting and ending.
    pub idle: bool,
    /// 144b: a usage limit (a 429).
    pub usage_limit: bool,
    /// 145c, 145f: the turn's final message and its tokens.
    pub result: bool,
    /// 144c, 145d: the waker.
    pub wake: bool,
}

impl Hooks {
    /// Only 134-9's permission prompt hooks, as before 144.
    pub fn attention(on: bool) -> Self {
        Self {
            attention: on,
            ..Self::default()
        }
    }

    /// Everything on: what the gates run.
    pub fn all() -> Self {
        Self {
            attention: true,
            idle: true,
            usage_limit: true,
            result: true,
            wake: true,
        }
    }

    fn any_signal(&self) -> bool {
        self.idle || self.usage_limit
    }
}

/// One line from `harness-signal`: the agent's turn began (`working`),
/// ended (`idle`), or hit a usage limit (`usage_limit`).
#[derive(Debug, Deserialize, Serialize)]
pub struct SignalLine {
    pub handle: String,
    pub signal: String,
}

/// One line from `harness-result`: the turn's final message and, where the
/// agent counted them, its tokens.
#[derive(Debug, Deserialize, Serialize)]
pub struct ResultLine {
    pub handle: String,
    pub final_message: String,
    #[serde(default)]
    pub usage: Option<Usage>,
}

/// The tokens a turn took, as the agent itself counted them (145f).
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct Usage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write_tokens: Option<u64>,
}

/// The first line of `harness-wake`: it waits on this connection. `wake`
/// names what armed it (a hook's event, or `plugin`), for the log only.
#[derive(Debug, Deserialize, Serialize)]
pub struct WakeLine {
    pub handle: String,
    pub wake: String,
}

/// The host's answer to a waker: wake with `kind`, or stand down.
#[derive(Debug, Default, Deserialize, Serialize)]
pub struct WakeAnswer {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wake: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub release: bool,
}

/// The longest final message the hook sends; the server keeps less.
pub const MAX_FINAL_MESSAGE: usize = 64 * 1024;

/// One line from `attention` on the daemon socket: the agent's own prompt
/// is waiting for a person, or no longer is.
#[derive(Debug, Deserialize, Serialize)]
pub struct AttentionLine {
    pub handle: String,
    pub attention: String,
}

pub fn permission_prompt_on(provider: &str) -> bool {
    PERMISSION_PROMPT_ADAPTERS.contains(&provider)
}

/// Whether `id` is a session id `provider` could have made. It becomes an
/// argument, so nothing else is ever accepted. The shapes are the profiles'
/// (decision 130).
pub fn valid_id(provider: &str, id: &str) -> bool {
    crate::profile::valid_id(provider, id)
}

fn new_uuid() -> String {
    let mut b = [0u8; 16];
    rand::rng().fill_bytes(&mut b);
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h = data_encoding::HEXLOWER.encode(&b);
    format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    )
}

/// What a session's launch gains so its id reaches the host, and the id when
/// it is known before the agent starts. `resume` must already be valid for
/// `provider`. Nothing for any other provider. `synced` is merged into Claude
/// Code's settings file and checked there (token sync, AM71). `hooks` adds
/// what its gated lists switch on (134-9's permission prompt, 144b's
/// signals, 145c's final message, 144c's waker); with none of it every file
/// is as before.
#[allow(clippy::too_many_arguments)]
pub fn launch(
    provider: &str,
    dir: &Path,
    handle: &str,
    socket: &Path,
    exe: &Path,
    resume: Option<&str>,
    synced: Option<&serde_json::Map<String, serde_json::Value>>,
    hooks: &Hooks,
) -> Result<(LaunchExtras, Option<String>)> {
    let env = vec![
        (HANDLE_ENV.into(), handle.into()),
        (crate::mcp::SOCKET_ENV.into(), socket.as_os_str().to_owned()),
    ];
    match provider {
        "claude-code" => {
            let (mut args, id): (Vec<std::ffi::OsString>, String) = match resume {
                Some(id) => (vec!["--resume".into(), id.into()], id.to_string()),
                None => {
                    let id = new_uuid();
                    (vec!["--session-id".into(), id.clone().into()], id)
                }
            };
            let hook = format!(
                "{} harness-session",
                shell_quote(&exe.display().to_string())
            );
            let mut settings = json!({
                "hooks": {"SessionStart": [{"hooks": [{"type": "command", "command": hook}]}]}
            });
            if hooks.attention {
                claude_attention_hooks(&mut settings, exe);
            }
            claude_lifecycle_hooks(&mut settings, exe, hooks);
            if let Some(pins) = synced {
                settings
                    .as_object_mut()
                    .expect("an object")
                    .extend(pins.clone());
            }
            // A file, not inline JSON: npm installs Claude Code on Windows as
            // `claude.cmd`, and a batch file's arguments may hold no `"`
            // (AM53; found by the Windows acceptance run).
            private_dir(dir)?;
            let path = dir.join("claude-settings.json");
            write_private(&path, settings.to_string().as_bytes())?;
            if let Some(pins) = synced {
                crate::credential::verify_settings(&std::fs::read(&path)?, pins)?;
            }
            args.extend(["--settings".into(), path.into_os_string()]);
            Ok((
                LaunchExtras {
                    args,
                    env,
                    ..Default::default()
                },
                Some(id),
            ))
        }
        "opencode" => {
            let config = dir.join("opencode");
            let plugins = config.join("plugin");
            private_dir(dir)?;
            private_dir(&config)?;
            private_dir(&plugins)?;
            write_private(&plugins.join("storm.js"), plugin(exe, hooks).as_bytes())?;
            let mut env = env;
            env.push(("OPENCODE_CONFIG_DIR".into(), config.into_os_string()));
            let args = match resume {
                Some(id) => vec!["-s".into(), id.into()],
                None => Vec::new(),
            };
            Ok((
                LaunchExtras {
                    args,
                    env,
                    ..Default::default()
                },
                resume.map(str::to_string),
            ))
        }
        // Codex's hook is in Storm's own `CODEX_HOME` (`codex.rs`), the same
        // for every session; this session's handle reaches it through the
        // environment it inherits.
        "codex" => {
            let args = match resume {
                Some(id) => vec!["resume".into(), id.into()],
                None => Vec::new(),
            };
            Ok((
                LaunchExtras {
                    args,
                    env,
                    ..Default::default()
                },
                resume.map(str::to_string),
            ))
        }
        _ => Ok((LaunchExtras::default(), None)),
    }
}

fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// Claude Code's hooks for a permission prompt (134-9): `PermissionRequest`
/// as its dialog opens and `Notification` named `permission_prompt` set it;
/// a tool's end, a new prompt and the turn's end clear it. `idle_prompt`, the
/// notification for an agent waiting at its prompt, is never matched.
fn claude_attention_hooks(settings: &mut serde_json::Value, exe: &Path) {
    let command = |kind: &str| {
        json!([{"type": "command", "command": format!(
            "{} attention {kind}",
            shell_quote(&exe.display().to_string())
        )}])
    };
    let hooks = &mut settings["hooks"];
    hooks["PermissionRequest"] = json!([{"hooks": command(PERMISSION_PROMPT)}]);
    hooks["Notification"] =
        json!([{"matcher": PERMISSION_PROMPT, "hooks": command(PERMISSION_PROMPT)}]);
    for event in [
        "PostToolUse",
        "PostToolUseFailure",
        "UserPromptSubmit",
        "Stop",
    ] {
        hooks[event] = json!([{"hooks": command(ATTENTION_CLEAR)}]);
    }
}

/// Adds `handler` to `event`'s hooks, in the group with `matcher` (none for
/// an event that takes none), after whatever is there.
fn add_hook(
    settings: &mut serde_json::Value,
    event: &str,
    matcher: Option<&str>,
    handler: serde_json::Value,
) {
    let groups = &mut settings["hooks"][event];
    if !groups.is_array() {
        *groups = json!([]);
    }
    let groups = groups.as_array_mut().expect("an array");
    let same = |g: &serde_json::Value| g.get("matcher").and_then(|m| m.as_str()) == matcher;
    match groups.iter_mut().find(|g| same(g)) {
        Some(group) => group["hooks"]
            .as_array_mut()
            .expect("a group's hooks")
            .push(handler),
        None => {
            let mut group = json!({"hooks": [handler]});
            if let Some(m) = matcher {
                group["matcher"] = json!(m);
            }
            groups.push(group);
        }
    }
}

/// Claude Code's hooks for 144b, 145c and 144c, each only when `hooks`
/// says so (research 2026-10-27, Claude Code 2.1.310):
///
/// - idle: `UserPromptSubmit` starts a turn (`working`), `Stop` and
///   `Notification` `idle_prompt` end it (`idle`);
/// - usage limit: `StopFailure` with `error: "rate_limit"`, cleared by the
///   next turn;
/// - final message: `Stop`'s `last_assistant_message`, with the turn's
///   tokens from its transcript;
/// - wake: an `asyncRewake` waker armed on `SessionStart`,
///   `UserPromptSubmit`, `Stop` and `StopFailure` (an interrupt fires no
///   `Stop`), with a day's timeout since Claude Code enforces it.
fn claude_lifecycle_hooks(settings: &mut serde_json::Value, exe: &Path, hooks: &Hooks) {
    let exe = shell_quote(&exe.display().to_string());
    let command = |args: &str| json!({"type": "command", "command": format!("{exe} {args}")});
    if hooks.any_signal() {
        add_hook(
            settings,
            "UserPromptSubmit",
            None,
            command("harness-signal turn"),
        );
    }
    if hooks.idle {
        add_hook(settings, "Stop", None, command("harness-signal idle"));
        add_hook(
            settings,
            "Notification",
            Some("idle_prompt"),
            command("harness-signal idle"),
        );
    }
    if hooks.any_signal() {
        add_hook(
            settings,
            "StopFailure",
            None,
            command("harness-signal stop-failure"),
        );
    }
    if hooks.result {
        add_hook(settings, "Stop", None, command("harness-result"));
    }
    if hooks.wake {
        for event in ["SessionStart", "UserPromptSubmit", "Stop", "StopFailure"] {
            let mut waker = command("harness-wake");
            waker["asyncRewake"] = json!(true);
            waker["timeout"] = json!(WAKER_TIMEOUT_SECS);
            add_hook(settings, event, None, waker);
        }
    }
}

/// How long Claude Code lets a waker wait: it enforces a hook's timeout,
/// and 86400 was accepted by 2.1.310.
pub const WAKER_TIMEOUT_SECS: u64 = 86_400;

/// The OpenCode plugin. A subagent's session has a parent and is never the
/// one to resume; an id is reported once per change. With `hooks.attention`,
/// it also reports while any permission it was asked is unanswered (134-9),
/// one report at a time so they arrive in order. The rest, each only when
/// `hooks` says so (research 2026-10-27, OpenCode 1.19.4):
///
/// - idle: `session.status` busy and idle of the top-level session;
/// - usage limit: `session.error` with `statusCode` 429;
/// - final message: at idle, the session's last assistant message and the
///   turn's tokens, read with the plugin's own `client`;
/// - wake: `harness-wake --plugin` waits on the host, and once it names
///   something unread the plugin sends its fixed sentence with
///   `client.session.promptAsync` when the session is idle, never through
///   the TUI.
fn plugin(exe: &Path, hooks: &Hooks) -> String {
    let exe = serde_json::to_string(&exe.display().to_string()).expect("a string serializes");
    let lifecycle = hooks.any_signal() || hooks.result || hooks.wake;
    let client = if hooks.result || hooks.wake {
        "{ client }"
    } else {
        ""
    };
    let mut state = String::new();
    let mut handlers = String::new();
    if hooks.attention || lifecycle {
        state.push_str(&format!(
            r#"
  let told = Promise.resolve();
  const run = (args, input) => {{
    told = told.then(() => new Promise((done) => {{
      try {{
        const child = spawn({exe}, args, {{ stdio: [input === undefined ? "ignore" : "pipe", "ignore", "ignore"] }});
        if (input !== undefined) child.stdin.end(input);
        child.on("exit", done);
        child.on("error", done);
      }} catch {{
        done();
      }}
    }}));
  }};"#
        ));
    }
    if hooks.attention {
        state.push_str(
            r#"
  const asked = new Set();
  const tell = (kind) => run(["attention", kind]);"#,
        );
        handlers.push_str(&format!(
            r#"
      if (event.type === "permission.asked") {{
        const was = asked.size;
        asked.add(event.properties?.id);
        if (was === 0) tell("{PERMISSION_PROMPT}");
        return;
      }}
      if (event.type === "permission.replied") {{
        if (asked.delete(event.properties?.requestID) && asked.size === 0) tell("{ATTENTION_CLEAR}");
        return;
      }}"#
        ));
    }
    if lifecycle {
        state.push_str(
            r#"
  let busy = false;"#,
        );
    }
    if hooks.result {
        state.push_str(&format!(
            r#"
  const finish = async (sid) => {{
    try {{
      const got = await client.session.messages({{ path: {{ id: sid }} }});
      const list = Array.isArray(got?.data) ? got.data : [];
      let start = 0;
      list.forEach((m, i) => {{ if (m?.info?.role === "user") start = i + 1; }});
      const turn = list.slice(start).filter((m) => m?.info?.role === "assistant");
      const final = turn[turn.length - 1];
      const text = (final?.parts ?? [])
        .filter((p) => p?.type === "text" && !p.synthetic && typeof p.text === "string")
        .map((p) => p.text)
        .join("\n")
        .trim()
        .slice(0, {MAX_FINAL_MESSAGE});
      if (!text) return;
      const usage = {{ input_tokens: 0, output_tokens: 0, cache_read_tokens: 0, cache_write_tokens: 0 }};
      let counted = false;
      for (const m of turn) {{
        const t = m?.info?.tokens;
        if (!t) continue;
        counted = true;
        usage.input_tokens += t.input ?? 0;
        usage.output_tokens += (t.output ?? 0) + (t.reasoning ?? 0);
        usage.cache_read_tokens += t.cache?.read ?? 0;
        usage.cache_write_tokens += t.cache?.write ?? 0;
      }}
      run(["harness-result"], JSON.stringify({{ final_message: text, usage: counted ? usage : null }}));
    }} catch {{}}
  }};"#
        ));
    }
    if hooks.wake {
        let events = serde_json::to_string(WAKE_EVENTS_TEXT).expect("a string serializes");
        let message = serde_json::to_string(WAKE_MESSAGE_TEXT).expect("a string serializes");
        state.push_str(&format!(
            r#"
  const wake = () => {{
    let out = "";
    let child;
    try {{
      child = spawn({exe}, ["harness-wake", "--plugin"], {{ stdio: ["ignore", "pipe", "ignore"] }});
    }} catch {{
      return;
    }}
    child.stdout.on("data", (d) => {{ out += d; }});
    child.on("error", () => {{}});
    child.on("exit", async (code) => {{
      const kind = out.trim();
      const text = kind === "events" ? {events} : kind === "message" ? {message} : null;
      if (code !== 2 || text === null) return;
      while (!last || busy) await new Promise((r) => setTimeout(r, 500));
      try {{
        await client.session.promptAsync({{ path: {{ id: last }}, body: {{ parts: [{{ type: "text", text }}] }} }});
      }} catch {{}}
      wake();
    }});
  }};
  wake();"#
        ));
    }
    if lifecycle {
        let mut on_turn = String::new();
        if hooks.any_signal() {
            on_turn.push_str(
                r#"
        if (busy) run(["harness-signal", "turn"]);"#,
            );
        }
        if hooks.idle {
            on_turn.push_str(
                r#"
        if (!busy) run(["harness-signal", "idle"]);"#,
            );
        }
        if hooks.result {
            on_turn.push_str(
                r#"
        if (!busy) finish(sid);"#,
            );
        }
        handlers.push_str(&format!(
            r#"
      if (event.type === "session.status") {{
        const sid = event.properties?.sessionID;
        if (typeof sid !== "string" || children.has(sid)) return;
        const now = event.properties?.status?.type !== "idle";
        if (now === busy) return;
        busy = now;{on_turn}
        return;
      }}"#
        ));
    }
    if hooks.usage_limit {
        handlers.push_str(
            r#"
      if (event.type === "session.error") {
        const sid = event.properties?.sessionID;
        if (!children.has(sid) && event.properties?.error?.data?.statusCode === 429) {
          run(["harness-signal", "usage_limit"]);
        }
        return;
      }"#,
        );
    }
    format!(
        r#"import {{ spawn }} from "node:child_process";

export const StormHarness = async ({client}) => {{
  const children = new Set();
  let last;{state}
  return {{
    event: async ({{ event }}) => {{{handlers}
      const info = event.properties?.info;
      if (event.type === "session.created" || event.type === "session.updated") {{
        if (info?.parentID) children.add(info.id);
        return;
      }}
      if (event.type !== "message.updated") return;
      const id = info?.sessionID;
      if (typeof id !== "string" || id === last || children.has(id)) return;
      last = id;
      try {{
        spawn({exe}, ["harness-session", id], {{ stdio: "ignore", detached: true }}).unref();
      }} catch {{}}
    }},
  }};
}};
"#
    )
}

/// `storm-runtime harness-session [ID]`: sends the id, or the `session_id`
/// of the hook input on stdin, to the daemon. Always successful: a failure
/// here must never get in the agent's way. It prints nothing but, when Claude
/// Code starts or resumes a session whose launch skipped MCP servers, a
/// `systemMessage` naming them (decision 131a), which Claude Code shows the
/// person; a hook's plain output would reach the agent instead.
pub fn report(id: Option<String>) {
    let hook = match id {
        Some(_) => None,
        None => read_hook_input(),
    };
    let id = id.or_else(|| Some(hook.as_ref()?["session_id"].as_str()?.to_string()));
    if let Some(id) = id {
        let _ = try_report(id);
    }
    let notice_file = std::env::var_os(crate::plugins::NOTICE_ENV).map(std::path::PathBuf::from);
    if let Some(text) = hook
        .as_ref()
        .and_then(|h| notice(h, notice_file.as_deref()))
    {
        println!("{}", json!({ "systemMessage": text }));
    }
}

/// `storm-runtime attention KIND`: tells the daemon the agent's permission
/// prompt is waiting (`permission_prompt`) or answered (`none`). Run by the
/// agent's own hook or plugin. Always successful, and prints nothing: a
/// hook's output would reach the agent.
pub fn report_attention(kind: &str) {
    // A Claude Code hook is handed its input; read it so the hook never
    // blocks on a full pipe.
    let _ = read_hook_input();
    let Ok(handle) = std::env::var(HANDLE_ENV) else {
        return;
    };
    let Some(socket) = std::env::var_os(crate::mcp::SOCKET_ENV) else {
        return;
    };
    let _ = send_attention(&socket, handle, kind.to_string());
}

pub(crate) fn send_attention(
    socket: impl AsRef<Path>,
    handle: String,
    attention: String,
) -> Result<()> {
    let mut line = serde_json::to_vec(&AttentionLine { handle, attention })?;
    line.push(b'\n');
    let mut stream = crate::platform::ipc::connect_blocking(socket.as_ref().as_os_str())?;
    stream.write_all(&line)?;
    stream.wait_closed(REPORT_WAIT);
    Ok(())
}

/// `storm-runtime harness-signal KIND`: the agent's turn began (`turn`),
/// ended (`idle`), failed (`stop-failure`, read from Claude Code's hook
/// input: a usage limit when its `error` is `rate_limit`, and an end either
/// way) or hit a usage limit (`usage_limit`, OpenCode's 429). Run by the
/// agent's own hook or plugin. Always successful, and prints nothing.
pub fn report_signal(kind: &str) {
    let hook = read_hook_input();
    let signals: &[&str] = match kind {
        "turn" => &["working"],
        "idle" => &[IDLE],
        USAGE_LIMIT => &[USAGE_LIMIT],
        "stop-failure" => {
            if hook
                .as_ref()
                .is_some_and(|h| h["error"].as_str() == Some("rate_limit"))
            {
                &[USAGE_LIMIT, IDLE]
            } else {
                &[IDLE]
            }
        }
        _ => &[],
    };
    let (Ok(handle), Some(socket)) = (
        std::env::var(HANDLE_ENV),
        std::env::var_os(crate::mcp::SOCKET_ENV),
    ) else {
        return;
    };
    for signal in signals {
        let _ = send_line(
            &socket,
            &SignalLine {
                handle: handle.clone(),
                signal: signal.to_string(),
            },
        );
    }
}

/// `storm-runtime harness-result`: the turn's final message, from Claude
/// Code's `Stop` input (`last_assistant_message`, with the turn's tokens
/// read from its transcript) or from OpenCode's plugin (`final_message`,
/// `usage`). Always successful, and prints nothing.
pub fn report_result() {
    let Some(input) = read_hook_input() else {
        return;
    };
    let (text, usage) = match input["final_message"].as_str() {
        Some(text) => (
            text.to_string(),
            serde_json::from_value::<Usage>(input["usage"].clone()).ok(),
        ),
        None => match input["last_assistant_message"].as_str() {
            Some(text) => (
                text.to_string(),
                input["transcript_path"]
                    .as_str()
                    .and_then(|p| settled_transcript_usage(Path::new(p), text)),
            ),
            None => return,
        },
    };
    let mut text = text;
    if text.len() > MAX_FINAL_MESSAGE {
        let mut end = MAX_FINAL_MESSAGE;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
    let (Ok(handle), Some(socket)) = (
        std::env::var(HANDLE_ENV),
        std::env::var_os(crate::mcp::SOCKET_ENV),
    ) else {
        return;
    };
    let _ = send_line(
        &socket,
        &ResultLine {
            handle,
            final_message: text,
            usage: usage.filter(|u| *u != Usage::default()),
        },
    );
}

/// Claude Code writes the turn's last assistant entry to its transcript a
/// moment after `Stop` starts its hooks (2.1.312): the turn's tokens once
/// the entry with its final message is there, waiting at most
/// [`TRANSCRIPT_WAIT`].
fn settled_transcript_usage(path: &Path, final_message: &str) -> Option<Usage> {
    let deadline = std::time::Instant::now() + TRANSCRIPT_WAIT;
    loop {
        if let Some(usage) = transcript_usage(path, final_message) {
            return Some(usage);
        }
        if std::time::Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

const TRANSCRIPT_WAIT: std::time::Duration = std::time::Duration::from_secs(3);

/// The largest transcript `harness-result` reads.
const MAX_TRANSCRIPT: u64 = 64 << 20;

/// The tokens of the Claude Code turn that ended with `final_message`, from
/// its transcript: every assistant entry after the prompt that started it
/// (a `user` entry that is not a tool's result) up to the one that says
/// `final_message`, each message once, Claude Code's own stand-ins for an
/// error not counted. Nothing until that entry is written, and nothing when
/// the transcript says nothing of it: never guessed.
pub fn transcript_usage(path: &Path, final_message: &str) -> Option<Usage> {
    let file = std::fs::File::open(path).ok()?;
    if file.metadata().ok()?.len() > MAX_TRANSCRIPT {
        return None;
    }
    let text = std::io::read_to_string(file).ok()?;
    let entries: Vec<serde_json::Value> = text
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    let says = |e: &serde_json::Value| -> String {
        match &e["message"]["content"] {
            serde_json::Value::String(t) => t.clone(),
            serde_json::Value::Array(blocks) => blocks
                .iter()
                .filter(|b| b["type"] == "text")
                .filter_map(|b| b["text"].as_str())
                .collect::<Vec<_>>()
                .join("\n"),
            _ => String::new(),
        }
    };
    let end = entries
        .iter()
        .rposition(|e| e["type"] == "assistant" && says(e).trim() == final_message.trim())?;
    let prompt = |e: &serde_json::Value| {
        e["type"] == "user"
            && match &e["message"]["content"] {
                serde_json::Value::String(_) => true,
                serde_json::Value::Array(blocks) => {
                    blocks.iter().any(|b| b["type"] != "tool_result")
                }
                _ => false,
            }
    };
    let start = entries[..end].iter().rposition(prompt).map_or(0, |i| i + 1);
    let mut seen = std::collections::HashSet::new();
    let mut usage = Usage::default();
    let mut counted = false;
    for e in &entries[start..=end] {
        if e["type"] != "assistant" || e["message"]["model"] == "<synthetic>" {
            continue;
        }
        let u = &e["message"]["usage"];
        if !u.is_object() {
            continue;
        }
        // A streamed message is written once per block, with one usage.
        let id = e["message"]["id"].as_str().unwrap_or_default().to_string();
        if !id.is_empty() && !seen.insert(id) {
            continue;
        }
        counted = true;
        let add = |slot: &mut Option<u64>, v: &serde_json::Value| {
            if let Some(n) = v.as_u64() {
                *slot = Some(slot.unwrap_or(0).saturating_add(n));
            }
        };
        add(&mut usage.input_tokens, &u["input_tokens"]);
        add(&mut usage.output_tokens, &u["output_tokens"]);
        add(&mut usage.cache_read_tokens, &u["cache_read_input_tokens"]);
        add(
            &mut usage.cache_write_tokens,
            &u["cache_creation_input_tokens"],
        );
    }
    counted.then_some(usage)
}

/// `storm-runtime harness-wake [--plugin]`: waits on the session's own
/// socket until its host says something is unread, then wakes the agent
/// the way it documents: for Claude Code's `asyncRewake` hook, the fixed
/// sentence on stderr and exit 2; for OpenCode's plugin, the kind on stdout
/// and exit 2. Exit 0 with nothing printed when the host releases it (a
/// newer waiter took over, or the session is ending) or the connection
/// goes. Nothing ever reaches a terminal.
pub fn wake(plugin: bool) -> i32 {
    let hook = if plugin { None } else { read_hook_input() };
    let armed_by = hook
        .as_ref()
        .and_then(|h| h["hook_event_name"].as_str().map(str::to_string))
        .unwrap_or_else(|| "plugin".into());
    let (Ok(handle), Some(socket)) = (
        std::env::var(HANDLE_ENV),
        std::env::var_os(crate::mcp::SOCKET_ENV),
    ) else {
        return 0;
    };
    let answer = (|| -> Result<WakeAnswer> {
        let mut line = serde_json::to_vec(&WakeLine {
            handle,
            wake: armed_by,
        })?;
        line.push(b'\n');
        let mut stream = crate::platform::ipc::connect_blocking(socket.as_os_str())?;
        stream.write_all(&line)?;
        let answer = stream.read_line(4096)?;
        Ok(serde_json::from_str(&answer)?)
    })();
    let kind = match answer {
        Ok(WakeAnswer {
            wake: Some(kind), ..
        }) => kind,
        _ => return 0,
    };
    let text = match kind.as_str() {
        "events" => WAKE_EVENTS_TEXT,
        "message" => WAKE_MESSAGE_TEXT,
        _ => return 0,
    };
    if plugin {
        println!("{kind}");
    } else {
        eprintln!("{text}");
    }
    2
}

pub(crate) fn send_line(socket: impl AsRef<Path>, line: &impl Serialize) -> Result<()> {
    let mut bytes = serde_json::to_vec(line)?;
    bytes.push(b'\n');
    let mut stream = crate::platform::ipc::connect_blocking(socket.as_ref().as_os_str())?;
    stream.write_all(&bytes)?;
    stream.wait_closed(REPORT_WAIT);
    Ok(())
}

fn read_hook_input() -> Option<serde_json::Value> {
    let mut input = String::new();
    std::io::stdin()
        .take(1 << 20)
        .read_to_string(&mut input)
        .ok()?;
    serde_json::from_str(&input).ok()
}

/// The session's notice, on a fresh start or a resume only: `/clear` and a
/// compaction fire the same hook and would repeat it.
fn notice(hook: &serde_json::Value, file: Option<&Path>) -> Option<String> {
    if !matches!(hook["source"].as_str(), Some("startup" | "resume")) {
        return None;
    }
    let text = std::fs::read_to_string(file?).ok()?;
    (!text.trim().is_empty()).then_some(text)
}

fn try_report(id: String) -> Result<()> {
    let handle = std::env::var(HANDLE_ENV)?;
    let socket = std::env::var_os(crate::mcp::SOCKET_ENV).ok_or(anyhow::anyhow!("no socket"))?;
    send(&socket, handle, id)
}

pub(crate) fn send(socket: impl AsRef<Path>, handle: String, id: String) -> Result<()> {
    let mut line = serde_json::to_vec(&HarnessLine {
        handle,
        harness_session: id,
    })?;
    line.push(b'\n');
    let mut stream = crate::platform::ipc::connect_blocking(socket.as_ref().as_os_str())?;
    stream.write_all(&line)?;
    // Stay alive until the host has checked who sent it (decision 135).
    stream.wait_closed(REPORT_WAIT);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    #[test]
    fn only_ids_the_agent_could_have_made_are_valid() {
        let v: serde_json::Value =
            serde_json::from_str(include_str!("../../../docs/harness-session-vectors.json"))
                .unwrap();
        for (set, want) in [("valid", true), ("invalid", false)] {
            for case in v[set].as_array().unwrap() {
                let (provider, id) = (
                    case["provider"].as_str().unwrap(),
                    case["id"].as_str().unwrap(),
                );
                assert_eq!(valid_id(provider, id), want, "{provider} {id:?}");
            }
        }
    }

    /// Decision 131a: the notice on a start or a resume, never on `/clear`
    /// or a compaction, and nothing without one.
    #[test]
    fn the_notice_is_for_a_start_or_a_resume() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("mcp-notice.txt");
        std::fs::write(&file, "Storm skipped 1 MCP server").unwrap();
        let hook = |source: &str| json!({"session_id": "x", "source": source});
        for source in ["startup", "resume"] {
            assert_eq!(
                notice(&hook(source), Some(&file)).as_deref(),
                Some("Storm skipped 1 MCP server")
            );
        }
        for source in ["clear", "compact"] {
            assert_eq!(notice(&hook(source), Some(&file)), None);
        }
        assert_eq!(notice(&hook("startup"), None), None);
        assert_eq!(
            notice(&hook("startup"), Some(&dir.path().join("none"))),
            None
        );
    }

    #[test]
    fn a_new_uuid_is_a_valid_v4() {
        let id = new_uuid();
        assert!(valid_id("claude-code", &id), "{id}");
        assert_eq!(&id[14..15], "4");
        assert!(matches!(&id[19..20], "8" | "9" | "a" | "b"));
        assert_ne!(new_uuid(), id);
    }

    #[test]
    fn claude_code_starts_with_its_id_and_a_session_start_hook() {
        let dir = tempfile::tempdir().unwrap();
        let exe = Path::new("/opt/it's/storm-runtime");
        let (extras, id) = launch(
            "claude-code",
            dir.path(),
            "h",
            Path::new("/s"),
            exe,
            None,
            None,
            &Hooks::default(),
        )
        .unwrap();
        let id = id.unwrap();
        let args: Vec<String> = extras
            .args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(args[..2], ["--session-id".to_string(), id]);
        assert_eq!(args[2], "--settings");
        assert_eq!(Path::new(&args[3]), dir.path().join("claude-settings.json"));
        crate::platform::assert_private(Path::new(&args[3]), 0o600);
        let settings: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&args[3]).unwrap()).unwrap();
        assert_eq!(
            settings["hooks"]["SessionStart"][0]["hooks"][0]["command"],
            r"'/opt/it'\''s/storm-runtime' harness-session"
        );
        assert!(settings.get("env").is_none(), "unsynced: unchanged");
        assert!(settings.get("apiKeyHelper").is_none());
        assert!(extras.env.contains(&(HANDLE_ENV.into(), "h".into())));
        assert!(
            extras
                .env
                .contains(&(crate::mcp::SOCKET_ENV.into(), "/s".into()))
        );
    }

    #[test]
    fn a_synced_session_has_one_settings_file_with_every_pin_and_no_token() {
        let dir = tempfile::tempdir().unwrap();
        let odd = r#"http://u:p"@proxy\lan:3128/ü"#;
        let pins = crate::credential::settings(|name| {
            (name == "HTTPS_PROXY" || name == "SSL_CERT_DIR").then(|| odd.into())
        })
        .unwrap();
        let (extras, _) = launch(
            "claude-code",
            dir.path(),
            "h",
            Path::new("/s"),
            Path::new("/x"),
            None,
            Some(&pins),
            &Hooks::default(),
        )
        .unwrap();
        let at = extras.args.iter().position(|a| a == "--settings").unwrap();
        assert_eq!(extras.args.iter().filter(|a| *a == "--settings").count(), 1);
        let path = Path::new(&extras.args[at + 1]);
        crate::platform::assert_private(path, 0o600);
        let bytes = std::fs::read(path).unwrap();
        let settings: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(settings["apiKeyHelper"], "");
        assert_eq!(
            settings["env"]["ANTHROPIC_BASE_URL"],
            "https://api.anthropic.com"
        );
        assert_eq!(settings["env"]["HTTPS_PROXY"], odd);
        assert_eq!(settings["env"]["SSL_CERT_DIR"], odd);
        for name in crate::credential::EMPTY_PINS {
            assert_eq!(settings["env"][name], "", "{name}");
        }
        for name in crate::credential::NETWORK_PINS {
            assert!(settings["env"][name].is_string(), "{name}");
        }
        assert!(
            settings["env"]
                .as_object()
                .unwrap()
                .values()
                .all(|v| v.is_string())
        );
        assert!(
            settings["hooks"]["SessionStart"][0]["hooks"][0]["command"]
                .as_str()
                .unwrap()
                .ends_with("harness-session")
        );
        crate::credential::verify_settings(&bytes, &pins).unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("sk-ant-"));
        assert!(!String::from_utf8_lossy(&bytes).contains(crate::credential::TOKEN_ENV));
    }

    /// 134-9: without the permission prompt hooks, OpenCode's plugin is the
    /// one every session had before.
    #[test]
    fn the_plugin_without_attention_is_unchanged() {
        let before = r#"import { spawn } from "node:child_process";

export const StormHarness = async () => {
  const children = new Set();
  let last;
  return {
    event: async ({ event }) => {
      const info = event.properties?.info;
      if (event.type === "session.created" || event.type === "session.updated") {
        if (info?.parentID) children.add(info.id);
        return;
      }
      if (event.type !== "message.updated") return;
      const id = info?.sessionID;
      if (typeof id !== "string" || id === last || children.has(id)) return;
      last = id;
      try {
        spawn("/x", ["harness-session", id], { stdio: "ignore", detached: true }).unref();
      } catch {}
    },
  };
};
"#;
        assert_eq!(plugin(Path::new("/x"), &Hooks::default()), before);
    }

    /// 134-9: no adapter's permission prompt hooks are written until its
    /// G-134-7 passes, and the server keeps the same list.
    #[test]
    fn no_adapter_has_permission_prompt_hooks_yet() {
        for provider in ["claude-code", "opencode", "codex", "shell", "cursor"] {
            assert!(!permission_prompt_on(provider), "{provider}");
        }
        let server = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../server/src/agent/attention.rs"),
        )
        .unwrap();
        let list = |src: &str| {
            let head = "PERMISSION_PROMPT_ADAPTERS: &[&str] = &[";
            let rest = &src[src.find(head).unwrap() + head.len()..];
            rest[..rest.find("];").unwrap()].to_string()
        };
        assert_eq!(list(include_str!("harness.rs")), list(&server));
        assert_eq!(list(&server).trim(), "");
    }

    /// 134-9 with the hooks on: Claude Code's dialog sets it, and a tool's
    /// end, a prompt and the turn's end clear it; its idle notification is
    /// never matched. OpenCode's plugin counts unanswered permissions.
    #[test]
    fn the_permission_prompt_hooks_set_on_a_prompt_and_clear_on_its_answer() {
        let dir = tempfile::tempdir().unwrap();
        let exe = Path::new("/opt/it's/storm-runtime");
        let settings = |attention| {
            launch(
                "claude-code",
                dir.path(),
                "h",
                Path::new("/s"),
                exe,
                None,
                None,
                &Hooks::attention(attention),
            )
            .unwrap();
            let bytes = std::fs::read(dir.path().join("claude-settings.json")).unwrap();
            serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()
        };
        let off = settings(false);
        assert_eq!(
            off["hooks"].as_object().unwrap().keys().collect::<Vec<_>>(),
            ["SessionStart"]
        );
        let on = settings(true);
        let command = |event: &str| on["hooks"][event][0]["hooks"][0]["command"].clone();
        let set = json!(r"'/opt/it'\''s/storm-runtime' attention permission_prompt");
        let clear = json!(r"'/opt/it'\''s/storm-runtime' attention none");
        assert_eq!(command("PermissionRequest"), set);
        assert_eq!(command("Notification"), set);
        assert_eq!(
            on["hooks"]["Notification"][0]["matcher"],
            "permission_prompt"
        );
        assert_eq!(on["hooks"]["Notification"].as_array().unwrap().len(), 1);
        for event in [
            "PostToolUse",
            "PostToolUseFailure",
            "UserPromptSubmit",
            "Stop",
        ] {
            assert_eq!(command(event), clear, "{event}");
        }
        assert!(!on.to_string().contains("idle"), "{on}");
        assert_eq!(on["hooks"]["SessionStart"], off["hooks"]["SessionStart"]);

        let js = plugin(Path::new("/x"), &Hooks::attention(true));
        assert!(js.contains(r#"event.type === "permission.asked""#), "{js}");
        assert!(js.contains(r#"event.properties?.requestID"#), "{js}");
        assert!(js.contains(r#"tell("permission_prompt")"#), "{js}");
        assert!(js.contains(r#"tell("none")"#), "{js}");
        assert!(!js.contains("idle"), "{js}");
    }

    #[test]
    fn a_resume_names_the_conversation() {
        let dir = tempfile::tempdir().unwrap();
        let exe = Path::new("/x");
        let uuid = "80d0cf8c-8176-43b4-8206-7908d74d1387";
        let (claude, id) = launch(
            "claude-code",
            dir.path(),
            "h",
            Path::new("/s"),
            exe,
            Some(uuid),
            None,
            &Hooks::default(),
        )
        .unwrap();
        assert_eq!(
            claude.args[..2],
            [OsString::from("--resume"), OsString::from(uuid)]
        );
        assert_eq!(id.as_deref(), Some(uuid));

        let ses = "ses_ed5303b0dffeRnwXZdkDih2uxK";
        let (oc, id) = launch(
            "opencode",
            dir.path(),
            "h",
            Path::new("/s"),
            exe,
            Some(ses),
            None,
            &Hooks::default(),
        )
        .unwrap();
        assert_eq!(oc.args, ["-s", ses]);
        assert_eq!(id.as_deref(), Some(ses));
    }

    #[test]
    fn opencode_gets_a_config_dir_with_the_plugin_and_no_id_yet() {
        let dir = tempfile::tempdir().unwrap();
        let (extras, id) = launch(
            "opencode",
            dir.path(),
            "h",
            Path::new("/s"),
            Path::new("/usr/bin/storm-runtime"),
            None,
            None,
            &Hooks::default(),
        )
        .unwrap();
        assert!(extras.args.is_empty());
        assert_eq!(id, None);
        let config = dir.path().join("opencode");
        assert!(extras.env.contains(&(
            "OPENCODE_CONFIG_DIR".into(),
            config.clone().into_os_string()
        )));
        let plugin = std::fs::read_to_string(config.join("plugin/storm.js")).unwrap();
        assert!(plugin.contains(r#"spawn("/usr/bin/storm-runtime", ["harness-session", id]"#));
        crate::platform::assert_private(&config.join("plugin/storm.js"), 0o600);
    }

    #[test]
    fn other_providers_gain_nothing() {
        let dir = tempfile::tempdir().unwrap();
        for provider in ["shell", "fake", "my-agent"] {
            let (extras, id) = launch(
                provider,
                dir.path(),
                "h",
                Path::new("/s"),
                Path::new("/x"),
                None,
                None,
                &Hooks::default(),
            )
            .unwrap();
            assert_eq!(extras, LaunchExtras::default());
            assert_eq!(id, None);
        }
    }

    // ---- decisions 144b, 145c, 144c/145d -------------------------------------

    /// The text of `name`'s list in `src`: what is between `NAME: &[…] = &[`
    /// and `];`, without whitespace.
    fn list(src: &str, name: &str) -> String {
        let at = src
            .find(&format!("pub const {name}: &["))
            .unwrap_or_else(|| panic!("{name}"));
        let rest = &src[at..];
        let open = rest.find("= &[").unwrap() + 4;
        let body = &rest[open..open + rest[open..].find("];").unwrap()];
        body.split_whitespace().collect()
    }

    /// Each gated list here is the server's, spelled the same, and every
    /// one is empty until the operator's real-CLI gates pass.
    #[test]
    fn the_gated_lists_are_the_servers_and_empty() {
        let server = |file: &str| {
            std::fs::read_to_string(
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../server/src/agent")
                    .join(file),
            )
            .unwrap()
        };
        let here = include_str!("harness.rs");
        for (name, file, theirs) in [
            ("GATED_SIGNALS", "attention.rs", "GATED_SIGNALS"),
            ("RESULT_SOURCES", "round.rs", "RESULT_SOURCES"),
            ("WAKE", "events.rs", "WAKE"),
        ] {
            assert_eq!(list(here, name), list(&server(file), theirs), "{name}");
            assert_eq!(list(here, name), "", "{name} switched on");
        }
        assert!(GATED_SIGNALS.is_empty() && RESULT_SOURCES.is_empty() && WAKE.is_empty());
        for provider in ["claude-code", "opencode", "codex", "cursor", "shell"] {
            assert_eq!(
                Gates::COMPILED.hooks(provider),
                Hooks::default(),
                "{provider}"
            );
        }
    }

    fn settings_of(hooks: &Hooks) -> String {
        let dir = tempfile::tempdir().unwrap();
        launch(
            "claude-code",
            dir.path(),
            "h",
            Path::new("/s"),
            Path::new("/x"),
            None,
            None,
            hooks,
        )
        .unwrap();
        std::fs::read_to_string(dir.path().join("claude-settings.json")).unwrap()
    }

    /// With no gate passed no hook, plugin code or waker is installed: the
    /// files are byte for byte what they were before 144's runtime half.
    #[test]
    fn with_nothing_gated_the_configs_are_byte_identical() {
        let hooks = Gates::COMPILED.hooks("claude-code");
        assert_eq!(
            settings_of(&hooks),
            r#"{"hooks":{"SessionStart":[{"hooks":[{"command":"'/x' harness-session","type":"command"}]}]}}"#
        );
        let js = plugin(Path::new("/x"), &Gates::COMPILED.hooks("opencode"));
        assert_eq!(js, plugin(Path::new("/x"), &Hooks::default()));
        for absent in [
            "harness-wake",
            "promptAsync",
            "harness-signal",
            "harness-result",
            "client",
        ] {
            assert!(!js.contains(absent), "{absent}");
        }
    }

    /// 144b, 145c, 144c with every gate on: Claude Code's settings carry
    /// the turn's signals, the final message and an `asyncRewake` waker on
    /// each event that ends or starts a turn, with a day's timeout, beside
    /// the `SessionStart` hook that was there.
    #[test]
    fn claude_codes_settings_carry_each_gated_hook() {
        let on: serde_json::Value = serde_json::from_str(&settings_of(&Hooks::all())).unwrap();
        let commands = |event: &str, matcher: Option<&str>| -> Vec<serde_json::Value> {
            on["hooks"][event]
                .as_array()
                .unwrap()
                .iter()
                .filter(|g| g.get("matcher").and_then(|m| m.as_str()) == matcher)
                .flat_map(|g| g["hooks"].as_array().unwrap().clone())
                .collect()
        };
        let names = |event: &str, matcher: Option<&str>| -> Vec<String> {
            commands(event, matcher)
                .iter()
                .map(|h| {
                    h["command"]
                        .as_str()
                        .unwrap()
                        .trim_start_matches("'/x' ")
                        .to_string()
                })
                .collect()
        };
        assert_eq!(
            names("SessionStart", None),
            ["harness-session", "harness-wake"]
        );
        assert!(
            names("UserPromptSubmit", None)
                .starts_with(&["attention none".into(), "harness-signal turn".into()])
        );
        assert!(names("UserPromptSubmit", None).ends_with(&["harness-wake".into()]));
        assert_eq!(
            names("Stop", None),
            [
                "attention none",
                "harness-signal idle",
                "harness-result",
                "harness-wake"
            ]
        );
        assert_eq!(
            names("StopFailure", None),
            ["harness-signal stop-failure", "harness-wake"]
        );
        assert_eq!(
            names("Notification", Some("idle_prompt")),
            ["harness-signal idle"]
        );
        assert_eq!(
            names("Notification", Some("permission_prompt")),
            ["attention permission_prompt"]
        );
        for event in ["SessionStart", "UserPromptSubmit", "Stop", "StopFailure"] {
            let waker = commands(event, None)
                .into_iter()
                .find(|h| h["command"] == "'/x' harness-wake")
                .unwrap();
            assert_eq!(waker["asyncRewake"], true, "{event}");
            assert_eq!(waker["timeout"], WAKER_TIMEOUT_SECS, "{event}");
        }
        // Each part alone adds only its own.
        let wake_only: serde_json::Value = serde_json::from_str(&settings_of(&Hooks {
            wake: true,
            ..Hooks::default()
        }))
        .unwrap();
        assert!(!wake_only.to_string().contains("harness-signal"));
        assert!(!wake_only.to_string().contains("harness-result"));
    }

    /// OpenCode's plugin with every gate on: its turn, its 429, its final
    /// message through its own client, and its wake through
    /// `client.session.promptAsync` with the fixed sentences only.
    #[test]
    fn opencodes_plugin_carries_each_gated_part() {
        let js = plugin(Path::new("/x"), &Hooks::all());
        for part in [
            "async ({ client }) =>",
            r#"event.type === "session.status""#,
            r#"run(["harness-signal", "turn"])"#,
            r#"run(["harness-signal", "idle"])"#,
            "statusCode === 429",
            r#"run(["harness-signal", "usage_limit"])"#,
            "client.session.messages",
            r#"run(["harness-result"]"#,
            r#"spawn("/x", ["harness-wake", "--plugin"]"#,
            "client.session.promptAsync",
            WAKE_EVENTS_TEXT,
            WAKE_MESSAGE_TEXT,
            r#"event.type === "permission.asked""#,
        ] {
            assert!(js.contains(part), "{part}\n{js}");
        }
    }

    /// A-144-12, A-145-11: the waker, the hooks and the plugin reach the
    /// agent only through its own documented mechanisms. No path of theirs
    /// writes to a terminal, types into OpenCode's TUI, or prints to the
    /// agent.
    #[test]
    fn a_144_12_a_145_11_no_hook_waker_or_plugin_path_writes_to_a_terminal() {
        let harness = include_str!("harness.rs");
        let host = include_str!("host.rs");
        let mut found = Vec::new();
        for (src, name) in [
            (harness, "report_signal"),
            (harness, "report_result"),
            (harness, "transcript_usage"),
            (harness, "wake"),
            (harness, "send_line"),
            (harness, "claude_lifecycle_hooks"),
            (harness, "plugin"),
            (host, "hook_session"),
            (host, "signal_reported"),
            (host, "result_reported"),
            (host, "waiter"),
            (host, "park_child"),
        ] {
            let body = body_of(src, name);
            assert!(body.len() > 100, "{name} was read");
            found.extend(
                terminal_offences(&body)
                    .into_iter()
                    .map(|l| format!("{name}: {l}")),
            );
        }
        found.extend(terminal_offences(&plugin(Path::new("/x"), &Hooks::all())));
        found.extend(terminal_offences(&settings_of(&Hooks::all())));
        assert!(found.is_empty(), "{found:#?}");
        // The waker's words go to stderr, the only place asyncRewake reads
        // them; nothing else of its prints.
        let waker = body_of(harness, "wake");
        assert_eq!(waker.matches("eprintln!").count(), 1);
        assert_eq!(waker.matches("println!").count(), 2, "one is the eprintln");
        for name in ["report_signal", "report_result"] {
            assert!(!body_of(harness, name).contains("print"), "{name} prints");
        }
    }

    /// The lines of `text` that write to, or name, a terminal or OpenCode's
    /// TUI. Spelled so this file's own text does not match.
    fn terminal_offences(text: &str) -> Vec<String> {
        let pty = ["pt", "y"].concat();
        let forbidden = [
            ["Interaction", "::Terminal"].concat(),
            ["term", ".write"].concat(),
            [".input", ".send"].concat(),
            ["terminal", ".input"].concat(),
            ["/dev/", "tty"].concat(),
            ["append", "-prompt"].concat(),
            ["submit", "-prompt"].concat(),
            ["/t", "ui"].concat(),
            ["process.std", "out.write"].concat(),
        ];
        text.lines()
            .filter(|l| {
                // A comment, not a URL's `//`.
                let l = l.trim_start();
                let code = if l.starts_with("//") {
                    ""
                } else {
                    l.split(" //").next().unwrap_or_default()
                }
                .to_lowercase();
                let names_a_terminal =
                    code.split(|c: char| !c.is_alphanumeric() && c != '_')
                        .any(|w| {
                            w == pty
                                || w.starts_with(&format!("{pty}_"))
                                || w.ends_with(&format!("_{pty}"))
                        });
                names_a_terminal || forbidden.iter().any(|f| code.contains(&f.to_lowercase()))
            })
            .map(str::to_string)
            .collect()
    }

    /// The scan finds what it is for.
    #[test]
    fn a_144_12_a_145_11_the_hook_scan_finds_a_planted_write() {
        for planted in [
            "let _ = std::fs::write(\"/dev/tty\", b\"go\");",
            "s.input.send(b\"x\".to_vec());",
            "let Interaction::Terminal(term) = guard.interaction();",
            "await fetch(\"http://x/tui/append-prompt\")",
            "pty_write(fd, b\"wake\")",
            "process.stdout.write(\"go\")",
        ] {
            assert_eq!(terminal_offences(planted).len(), 1, "{planted}");
        }
        assert!(
            terminal_offences("if !id.is_empty() { client.session.promptAsync(x) }").is_empty()
        );
    }

    /// The source of `fn name`, from its signature to its closing brace.
    fn body_of(src: &str, name: &str) -> String {
        let at = src
            .find(&format!("fn {name}("))
            .unwrap_or_else(|| panic!("no fn {name}"));
        let open = at + src[at..].find('{').unwrap();
        let mut depth = 0;
        for (i, c) in src[open..].char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return src[at..open + i + 1].to_string();
                    }
                }
                _ => {}
            }
        }
        panic!("fn {name} does not close");
    }

    /// 145f: a turn's tokens, as Claude Code's transcript has them: every
    /// assistant message after the last prompt, each once, a tool's result
    /// not a prompt. Nothing said is nothing reported.
    #[test]
    fn a_145_9_a_transcript_gives_its_last_turns_tokens() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.jsonl");
        let lines = [
            json!({"type": "user", "message": {"role": "user", "content": "first"}}),
            json!({"type": "assistant", "message": {"id": "m0", "usage": {"input_tokens": 1000, "output_tokens": 1000}}}),
            json!({"type": "user", "message": {"role": "user", "content": [{"type": "text", "text": "second"}]}}),
            json!({"type": "assistant", "message": {"id": "m1", "usage": {"input_tokens": 10, "output_tokens": 5, "cache_read_input_tokens": 7, "cache_creation_input_tokens": 2}}}),
            json!({"type": "assistant", "message": {"id": "m1", "usage": {"input_tokens": 10, "output_tokens": 5}}}),
            json!({"type": "user", "message": {"role": "user", "content": [{"type": "tool_result", "content": "ok"}]}}),
            json!({"type": "attachment"}),
            json!({"type": "assistant", "message": {"id": "m2", "content": [{"type": "text", "text": "all done"}], "usage": {"input_tokens": 20, "output_tokens": 1}}}),
            json!({"type": "assistant", "message": {"id": "m3", "model": "<synthetic>", "content": [{"type": "text", "text": "all done"}], "usage": {"input_tokens": 0, "output_tokens": 0}}}),
        ];
        std::fs::write(
            &path,
            lines
                .iter()
                .map(|l| l.to_string() + "\n")
                .collect::<String>()
                + "not json\n",
        )
        .unwrap();
        assert_eq!(
            transcript_usage(&path, "all done"),
            Some(Usage {
                input_tokens: Some(30),
                output_tokens: Some(6),
                cache_read_tokens: Some(7),
                cache_write_tokens: Some(2),
            })
        );
        assert_eq!(
            transcript_usage(&path, "not yet written"),
            None,
            "the turn's own entry is not there yet"
        );
        std::fs::write(
            &path,
            json!({"type": "user", "message": {"content": "hi"}}).to_string(),
        )
        .unwrap();
        assert_eq!(transcript_usage(&path, "all done"), None, "never guessed");
        assert_eq!(transcript_usage(&dir.path().join("none"), "x"), None);
    }
}
