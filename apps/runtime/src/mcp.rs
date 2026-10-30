//! The host's half of the MCP Gateway (spec §11; decision 81f).
//!
//! **The host forwards; it never authorizes and never holds a credential.**
//! The server tells it which connections a session may use — ids and slugs,
//! nothing else (AM23) — and every call is decided by the server, per call.
//! What lives here:
//!
//! - the **per-provider config writers** (AM20, AM32): what Claude Code and
//!   OpenCode are given so that a session loads *only* its own servers, and
//!   asks before each gateway tool;
//! - the **session handles**: a random value per session that pairs a bridge
//!   with its session on the daemon's socket, never accepted off the host;
//! - the **daemon socket** (`run/mcp.sock`, in a `0700` directory; a named
//!   pipe on Windows, AM55): one line
//!   in from a bridge, the server's answer streamed back, **at most once** —
//!   while the link is down a call fails at once with `storm_unreachable`.
//!
//! A bridge's config holds the handle and the socket path, never a credential
//! (C5): there is no credential on this host to put there.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Context, Result};
use rand::Rng;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

/// A granted connection, as the server's `start` names it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpGrant {
    pub id: String,
    pub slug: String,
    /// The plugin server this connection was added from (131h). An older
    /// side leaves it out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_origin: Option<PluginOrigin>,
}

/// A plugin's server, by the plugin's name and the server's (131h).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginOrigin {
    pub plugin: String,
    pub server: String,
}

pub use crate::provider::LaunchExtras;

/// A bridge's server name in a Claude Code `mcp.json`: the plugin's own
/// `plugin_<plugin>_<server>` for a connection added from a plugin, so its
/// commands and hooks match the tools (131h); else, or when that name is
/// taken or unusable, the slug. The bridge itself is always addressed by
/// slug.
fn claude_key(
    g: &McpGrant,
    written: &serde_json::Map<String, Value>,
    own: &AgentServers,
) -> String {
    let usable = |s: &str| !s.is_empty() && s.len() <= 64;
    // A bridge for a plugin's own server keeps the plugin's name, and
    // replaces that server on purpose (131h).
    if let Some(o) = &g.plugin_origin
        && usable(&o.plugin)
        && usable(&o.server)
    {
        let key = crate::plugins::server_key(&o.plugin, &o.server);
        if !written.contains_key(&key) {
            return key;
        }
    }
    // Claude Code loads `--mcp-config`'s entry over the account's or a
    // project's of the same name and drops theirs (gate G-132a).
    own.key(g)
}

/// The server names OpenCode's global config already uses (decision 132b):
/// `opencode.json`, `opencode.jsonc` or `config.json` under
/// `$XDG_CONFIG_HOME/opencode`, else `~/.config/opencode`. A file that is not
/// plain JSON (a `.jsonc` with comments) gives no names.
pub fn opencode_names() -> std::collections::HashSet<String> {
    let dir = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")));
    let Some(dir) = dir.map(|d| d.join("opencode")) else {
        return Default::default();
    };
    ["opencode.json", "opencode.jsonc", "config.json"]
        .iter()
        .filter_map(|f| std::fs::read(dir.join(f)).ok())
        .filter_map(|b| serde_json::from_slice::<Value>(&b).ok())
        .filter_map(|v| {
            v["mcp"]
                .as_object()
                .map(|m| m.keys().cloned().collect::<Vec<_>>())
        })
        .flatten()
        .collect()
}

/// How a session's servers sit beside the agent's own (decision 132).
pub struct AgentServers<'a> {
    /// Storm's servers alone (`agent_mcp = "storm-only"`), even when the
    /// session has none.
    pub storm_only: bool,
    /// Server names the agent's own config already uses, so Storm's step
    /// aside.
    pub taken: &'a std::collections::HashSet<String>,
}

impl AgentServers<'_> {
    pub fn storm_only() -> AgentServers<'static> {
        static NONE: std::sync::LazyLock<std::collections::HashSet<String>> =
            std::sync::LazyLock::new(Default::default);
        AgentServers {
            storm_only: true,
            taken: &NONE,
        }
    }

    /// The name a grant's server gets: its slug, or `storm_<slug>` when the
    /// agent's own config has that name — except the built-in `storm`, which
    /// the opening prompt names.
    fn key(&self, g: &McpGrant) -> String {
        if g.id != STORM_CONNECTION && self.taken.contains(&g.slug) {
            format!("storm_{}", g.slug)
        } else {
            g.slug.clone()
        }
    }
}

/// The built-in connection to the session's own Storm (the gateway's
/// Phase 1 vault server).
pub const STORM_CONNECTION: &str = "storm";

/// The Storm tools a child calls on itself, allowed outright in its CLI's
/// config (decision 146a): each is scoped by the server to the calling
/// session and has no workspace or host effect. The host learns no tool
/// names from the server, so they are pinned here and held to the server's
/// lists by a test.
pub const CHILD_ALLOWED_TOOLS: [&str; 3] = ["session_context", "report_result", "read_messages"];

/// The spawn tools, allowed outright for a parent launched with the spawn
/// switch on (146a): the person's standing consent (145a).
pub const SPAWN_ALLOWED_TOOLS: [&str; 8] = [
    "list_agents",
    "start_agent",
    "agent_status",
    "list_children",
    "wait_agent",
    "send_message",
    "end_agent",
    "read_output",
];

/// The Storm tool a top-level Claude Code session asks for auto with
/// (147c), allowed outright so the person sees only Storm's card, never
/// the CLI's own prompt for the call first.
pub const MODE_ALLOWED_TOOLS: [&str; 1] = ["request_permission_mode"];

/// The Storm tools a top-level session deletes with (149b), allowed
/// outright so the person sees only Storm's dialog, which every deletion
/// waits on whatever the mode, never the CLI's prompt first. The server
/// offers them only to a session with vault writes.
pub const DELETE_ALLOWED_TOOLS: [&str; 2] = ["delete_note", "delete_folder"];

/// What Storm's own layer of a session's CLI config says about permissions
/// (decisions 146, 148, 149): the Storm tools allowed outright, the keys a
/// permission mode sets in OpenCode's config, and whether every other
/// OpenCode ask becomes a deny (a child's, 148a). Empty for a session that
/// does not read Storm and runs in `default`, whose config is then
/// byte-identical to before.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StormLayer {
    pub allow: Vec<&'static str>,
    pub opencode_permission: Vec<(String, String)>,
    pub deny_asks: bool,
}

impl StormLayer {
    /// `child`: a session another agent started, reading Storm. `spawn`: one
    /// launched with the switch on, reading Storm.
    pub fn new(child: bool, spawn: bool, opencode_permission: Vec<(String, String)>) -> Self {
        let mut allow = Vec::new();
        if child {
            allow.extend(CHILD_ALLOWED_TOOLS);
        }
        if spawn {
            allow.extend(SPAWN_ALLOWED_TOOLS);
        }
        Self {
            allow,
            opencode_permission,
            deny_asks: false,
        }
    }

    /// With `request_permission_mode` allowed too (147c).
    pub fn asking_for_modes(mut self) -> Self {
        self.allow.extend(MODE_ALLOWED_TOOLS);
        self
    }

    /// With the delete tools allowed too (149b).
    pub fn deleting(mut self) -> Self {
        self.allow.extend(DELETE_ALLOWED_TOOLS);
        self
    }

    /// A child's OpenCode layer (148a): every ask a deny.
    pub fn denying_asks(mut self) -> Self {
        self.deny_asks = true;
        self
    }

    /// Claude Code's rules for [`Self::allow`], for `permissions.allow` in
    /// Storm's `--settings` file. The built-in connection is always
    /// `storm` there.
    pub fn claude_allow(&self) -> Vec<String> {
        self.allow
            .iter()
            .map(|t| format!("mcp__{STORM_CONNECTION}__{t}"))
            .collect()
    }
}

/// The text of every OpenCode config file the account has where OpenCode
/// finds it, one entry per file: the global directory
/// (`$XDG_CONFIG_HOME/opencode`, else `~/.config/opencode`; not under
/// `storm-only`, which replaces it), `~/.opencode`, and `$OPENCODE_CONFIG`.
/// A file that exists but cannot be read is an error: Storm could not tell
/// which keys it already uses.
pub fn opencode_account_files(storm_only: bool) -> Result<Vec<String>> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let mut dirs = Vec::new();
    if !storm_only {
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| home.as_ref().map(|h| h.join(".config")));
        dirs.extend(config.map(|c| c.join("opencode")));
    }
    dirs.extend(home.map(|h| h.join(".opencode")));
    let mut files: Vec<PathBuf> = dirs
        .iter()
        .flat_map(|d| {
            ["config.json", "opencode.json", "opencode.jsonc"]
                .iter()
                .map(move |f| d.join(f))
        })
        .collect();
    files.extend(std::env::var_os("OPENCODE_CONFIG").map(PathBuf::from));
    files
        .iter()
        .filter(|f| f.is_file())
        .map(|file| {
            std::fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))
        })
        .collect()
}

/// An OpenCode config file's JSON: JSONC's comments and trailing commas
/// dropped first, outside strings.
fn parse_jsonc(text: &str) -> Result<Value> {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    let mut in_string = false;
    while let Some(c) = chars.next() {
        if in_string {
            out.push(c);
            match c {
                '\\' => out.extend(chars.next()),
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match (c, chars.peek()) {
            ('"', _) => {
                in_string = true;
                out.push(c);
            }
            ('/', Some('/')) => {
                for c in chars.by_ref() {
                    if c == '\n' {
                        out.push('\n');
                        break;
                    }
                }
            }
            ('/', Some('*')) => {
                chars.next();
                let mut last = ' ';
                for c in chars.by_ref() {
                    if last == '*' && c == '/' {
                        break;
                    }
                    last = c;
                }
                out.push(' ');
            }
            (',', _) => {
                let rest: String = chars.clone().collect();
                let next = rest.trim_start().chars().next();
                if !matches!(next, Some('}') | Some(']')) {
                    out.push(c);
                }
            }
            _ => out.push(c),
        }
    }
    Ok(serde_json::from_str(&out)?)
}

/// OpenCode's own asks, from its defaults (1.19.4): a loop, a path outside
/// the workspace, a question for the person, entering plan mode, and
/// reading an env file. A child's layer denies each (148a).
const OPENCODE_DEFAULT_ASKS: [&str; 3] = ["doom_loop", "question", "plan_enter"];

/// Turns every OpenCode ask a child's config would hold into a deny
/// (148a): Storm's own keys in place, each the account's configs give
/// (top level and per agent) under the same key, so its merged position is
/// kept, and OpenCode's defaults. An explicit allow or deny is left as it
/// is. Nothing here allows anything.
fn deny_asks(
    config: &mut Map<String, Value>,
    permission: &mut Map<String, Value>,
    account: &[Value],
) {
    fn deny_in(v: &mut Value) {
        match v {
            Value::String(s) if s == "ask" => *v = json!("deny"),
            Value::Object(m) => m.values_mut().for_each(deny_in),
            _ => {}
        }
    }
    fn merge_asks(into: &mut Map<String, Value>, theirs: &Map<String, Value>) {
        for (key, value) in theirs {
            match value {
                Value::String(s) if s == "ask" => {
                    into.insert(key.clone(), json!("deny"));
                }
                Value::Object(patterns) => {
                    let asks: Vec<&String> = patterns
                        .iter()
                        .filter(|(_, a)| *a == "ask")
                        .map(|(p, _)| p)
                        .collect();
                    if asks.is_empty() {
                        continue;
                    }
                    let entry = into.entry(key.clone()).or_insert_with(|| json!({}));
                    if let Value::Object(own) = entry {
                        for p in asks {
                            own.insert(p.clone(), json!("deny"));
                        }
                    }
                }
                _ => {}
            }
        }
    }
    permission.values_mut().for_each(deny_in);
    let mut agents = Map::new();
    for file in account {
        if let Some(theirs) = file.get("permission").and_then(Value::as_object) {
            merge_asks(permission, theirs);
        }
        for (name, agent) in file
            .get("agent")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
        {
            if let Some(theirs) = agent.get("permission").and_then(Value::as_object) {
                let own = agents
                    .entry(name.clone())
                    .or_insert_with(|| json!({"permission": {}}));
                merge_asks(
                    own["permission"].as_object_mut().expect("an object"),
                    theirs,
                );
            }
        }
    }
    for key in OPENCODE_DEFAULT_ASKS {
        if !matches!(permission.get(key), Some(Value::String(s)) if s != "ask") {
            permission.insert(key.into(), json!("deny"));
        }
    }
    for (key, patterns) in [
        ("external_directory", json!({"*": "deny"})),
        (
            "read",
            json!({"*.env": "deny", "*.env.*": "deny", "*.env.example": "allow"}),
        ),
    ] {
        let entry = permission.entry(key).or_insert_with(|| json!({}));
        if let Value::Object(own) = entry {
            for (p, a) in patterns.as_object().expect("an object") {
                own.entry(p.clone()).or_insert_with(|| a.clone());
            }
        }
    }
    if let Some(Value::Object(own)) = config.get_mut("agent") {
        for agent in own.values_mut() {
            if let Some(p) = agent.get_mut("permission") {
                deny_in(p);
            }
        }
    }
    for (name, theirs) in agents {
        let own = config
            .entry("agent")
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .and_then(|a| a.entry(name).or_insert_with(|| json!({})).as_object_mut());
        if let Some(own) = own {
            let into = own.entry("permission").or_insert_with(|| json!({}));
            if let (Some(into), Some(theirs)) =
                (into.as_object_mut(), theirs["permission"].as_object())
            {
                for (k, v) in theirs {
                    into.entry(k.clone()).or_insert_with(|| v.clone());
                }
            }
        }
    }
}

/// The key an allow is written under (146a): the tool's own name, or with as
/// many `*` after it as it takes to be a key no config of the account's
/// already has. OpenCode keeps a merged key where it first appeared, so only
/// a new key is sure to follow the connection's `<slug>_*` ask; it sorts
/// after that ask too, since `*` sorts before the letters these names have.
/// A trailing `*` still names only this tool: no other Storm tool's name
/// begins with another's.
fn opencode_allow_key(tool: &str, seen: &str) -> String {
    let mut key = tool.to_string();
    while seen.contains(&key) {
        key.push('*');
    }
    key
}

pub const SESSIONS_DIR: &str = "sessions";
pub const RUN_DIR: &str = "run";
pub const SOCKET: &str = "mcp.sock";
pub const HANDLE_ENV: &str = "STORM_MCP_HANDLE";
pub const SOCKET_ENV: &str = "STORM_MCP_SOCK";

pub fn socket_path(state_dir: &Path) -> PathBuf {
    state_dir.join(RUN_DIR).join(SOCKET)
}

pub fn session_dir(state_dir: &Path, session: &str) -> PathBuf {
    state_dir.join(SESSIONS_DIR).join(session)
}

/// A slug is `[a-z0-9-]` (the server validates it); refused here too, since
/// it becomes a config key and an argument.
pub(crate) fn slug_ok(slug: &str) -> bool {
    (1..=32).contains(&slug.len())
        && slug
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// Creates a directory private to this account (`0700`, created with that
/// mode: the A2 pattern), and makes it so if it already existed.
pub fn private_dir(dir: &Path) -> Result<()> {
    crate::platform::tighten_private_dir(dir).with_context(|| format!("creating {}", dir.display()))
}

pub(crate) fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let mut f = crate::platform::open_private(
        path,
        std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true),
    )
    .with_context(|| format!("creating {}", path.display()))?;
    f.write_all(bytes)?;
    Ok(())
}

/// The stdio server entry for one connection: this binary's `mcp-bridge`,
/// with the handle and socket in its environment. Nothing else.
fn bridge_entry(exe: &Path, slug: &str, handle: &str, socket: &Path) -> (Vec<String>, Value) {
    let command = vec![
        exe.display().to_string(),
        "mcp-bridge".to_string(),
        slug.to_string(),
    ];
    let env = json!({
        HANDLE_ENV: handle,
        SOCKET_ENV: socket.display().to_string(),
    });
    (command, env)
}

/// OpenCode's own order for a project's instruction file.
pub const OPENCODE_INSTRUCTION_FILES: [&str; 3] = ["AGENTS.md", "CLAUDE.md", "CONTEXT.md"];
pub const OPENCODE_INSTRUCTIONS: &str = "opencode-instructions.json";

/// What every session of `provider` gains, with or without grants (AM66):
/// `opencode` gets `OPENCODE_DISABLE_PROJECT_CONFIG=1`, so a repository's
/// `opencode.json` cannot redirect a provider. The flag also drops the
/// workspace's instruction file, so the first of
/// [`OPENCODE_INSTRUCTION_FILES`] in `workspace` (not above it) is named by
/// absolute path in `<dir>/opencode-instructions.json`, passed as
/// `OPENCODE_CONFIG`. OpenCode concatenates that `instructions` array with
/// the operator's, so neither replaces the other. Everything else: nothing.
pub fn provider_baseline(provider: &str, dir: &Path, workspace: &Path) -> Result<LaunchExtras> {
    if provider != "opencode" {
        return Ok(LaunchExtras::default());
    }
    let mut env = vec![("OPENCODE_DISABLE_PROJECT_CONFIG".into(), "1".into())];
    if let Some(file) = opencode_instruction_file(workspace) {
        let path = dir.join(OPENCODE_INSTRUCTIONS);
        private_dir(dir)?;
        write_private(
            &path,
            &serde_json::to_vec_pretty(&json!({
                "$schema": "https://opencode.ai/config.json",
                "instructions": [file],
            }))?,
        )?;
        env.push(("OPENCODE_CONFIG".into(), path.into_os_string()));
    }
    Ok(LaunchExtras {
        args: Vec::new(),
        env,
        ..Default::default()
    })
}

fn opencode_instruction_file(workspace: &Path) -> Option<String> {
    let found = OPENCODE_INSTRUCTION_FILES
        .iter()
        .map(|name| workspace.join(name))
        .find(|p| p.is_file())?;
    let found = found.to_string_lossy();
    // A canonical Windows path is `\\?\…`; OpenCode wants the plain form.
    let found = if cfg!(windows) {
        crate::platform::winsvc::normalize(&found)
    } else {
        found.into_owned()
    };
    // OpenCode globs an absolute `instructions` entry, so such a path could
    // match some other file, or none.
    if found.contains(['*', '?', '[', ']', '{', '}']) {
        tracing::warn!(path = %found, "not restoring an instruction file whose path holds a glob character");
        return None;
    }
    Some(found)
}

/// Writes the provider's MCP config into `dir` and returns what its launch
/// gains (AM32 as amended by AM66):
/// - **`claude-code`**: `--mcp-config <dir>/mcp.json`, one stdio server per
///   slug, beside the agent's own servers (decision 132); with
///   [`AgentServers::storm_only`], `--strict-mcp-config` too, and only the
///   session's load.
/// - **`opencode`**: the session config, with `permission: {"<slug>_*":
///   "ask"}` for every connection, in `OPENCODE_CONFIG_CONTENT` on top of
///   OpenCode's global config (decision 132b). With
///   [`AgentServers::storm_only`], `XDG_CONFIG_HOME=<dir>/xdg` with it in
///   `<dir>/xdg/opencode/opencode.json` instead. Either needs, with
///   [`provider_baseline`]'s `OPENCODE_DISABLE_PROJECT_CONFIG=1`: OpenCode
///   merges every other config source with the global one, and runs MCP
///   tools without asking by default (G2).
/// - anything else, `shell` included: nothing (G-D9).
///
/// `opencode_settings` is the `runtime.toml` provider entry's `settings`:
/// merged in, minus any `mcp` key — a setting can never add an MCP server.
/// `plugin_servers` are a Claude Code session's plugins' local servers
/// (decision 131c, `plugins.rs`), keyed `plugin_…`, which no slug can be.
/// `layer` is Storm's own word on permissions (decision 146): OpenCode's
/// allows for Storm's tools follow the built-in connection's ask, and a
/// permission mode's keys join the config, which an OpenCode session then
/// gets even without grants. Claude Code's allows are in its `--settings`
/// file (`harness.rs`); Codex asks for every Storm tool (132c). An empty
/// layer writes exactly what it always did. `account` is the text of the
/// account's own OpenCode configs, read only when there is an allow to
/// place.
#[allow(clippy::too_many_arguments)]
pub fn write_provider_config(
    provider: &str,
    dir: &Path,
    grants: &[McpGrant],
    handle: &str,
    socket: &Path,
    exe: &Path,
    opencode_settings: Option<&Value>,
    plugin_servers: &serde_json::Map<String, Value>,
    own: &AgentServers,
    layer: &StormLayer,
    account: &dyn Fn() -> Result<Vec<String>>,
) -> Result<LaunchExtras> {
    let grants: Vec<&McpGrant> = grants.iter().filter(|g| slug_ok(&g.slug)).collect();
    let opencode_mode = provider == "opencode" && !layer.opencode_permission.is_empty();
    if grants.is_empty() && !own.storm_only && !opencode_mode {
        return Ok(LaunchExtras::default());
    }
    match provider {
        "claude-code" => {
            let mut servers = serde_json::Map::new();
            for g in &grants {
                let (command, env) = bridge_entry(exe, &g.slug, handle, socket);
                servers.insert(
                    claude_key(g, &servers, own),
                    json!({
                        "type": "stdio",
                        "command": command[0],
                        "args": command[1..],
                        "env": env,
                    }),
                );
            }
            for (key, entry) in plugin_servers {
                if !servers.contains_key(key) {
                    servers.insert(key.clone(), entry.clone());
                }
            }
            private_dir(dir)?;
            let path = dir.join("mcp.json");
            write_private(
                &path,
                &serde_json::to_vec_pretty(&json!({ "mcpServers": servers }))?,
            )?;
            let mut args: Vec<std::ffi::OsString> =
                vec!["--mcp-config".into(), path.into_os_string()];
            if own.storm_only {
                args.push("--strict-mcp-config".into());
            }
            Ok(LaunchExtras {
                args,
                env: Vec::new(),
                ..Default::default()
            })
        }
        "opencode" => {
            let mut config = opencode_settings
                .and_then(|v| v.as_object().cloned())
                .unwrap_or_default();
            let mut mcp = serde_json::Map::new();
            let mut permission = config
                .remove("permission")
                .and_then(|p| p.as_object().cloned())
                .unwrap_or_default();
            for g in &grants {
                let key = own.key(g);
                let (command, env) = bridge_entry(exe, &g.slug, handle, socket);
                mcp.insert(
                    key.clone(),
                    json!({ "type": "local", "command": command, "environment": env, "enabled": true }),
                );
                // Last, so no setting can relax it; a child is never asked.
                let action = if layer.deny_asks { "deny" } else { "ask" };
                permission.insert(format!("{key}_*"), json!(action));
            }
            let files = if layer.deny_asks || !layer.allow.is_empty() {
                account()?
            } else {
                Vec::new()
            };
            if layer.deny_asks {
                let parsed = files
                    .iter()
                    .map(|f| parse_jsonc(f))
                    .collect::<Result<Vec<_>>>()
                    .context("an OpenCode config Storm cannot read could make a child ask")?;
                deny_asks(&mut config, &mut permission, &parsed);
            }
            let seen = format!(
                "{}{}",
                files.concat(),
                serde_json::to_string(&permission).unwrap_or_default()
            );
            let storm = grants.iter().find(|g| g.id == STORM_CONNECTION);
            if let Some(g) = storm.filter(|_| !layer.allow.is_empty()) {
                let key = own.key(g);
                for tool in &layer.allow {
                    permission.insert(
                        opencode_allow_key(&format!("{key}_{tool}"), &seen),
                        json!("allow"),
                    );
                }
            }
            // A child's allows follow the denies above under keys of their
            // own, as Storm's tool allows do (148a).
            for (k, v) in &layer.opencode_permission {
                let k = if layer.deny_asks {
                    opencode_allow_key(k, &seen)
                } else {
                    k.clone()
                };
                permission.insert(k, json!(v));
            }
            config.insert("$schema".into(), json!("https://opencode.ai/config.json"));
            // Replaces any `mcp` key the settings had: a host setting can
            // never add an MCP server.
            config.insert("mcp".into(), Value::Object(mcp));
            config.insert("permission".into(), Value::Object(permission));
            if !own.storm_only {
                // On top of OpenCode's global config, after its keys: its
                // permissions are evaluated last match first, so Storm's
                // asks follow everything the global config says (decision
                // 132b; a project's config stays off, AM66, since its keys
                // could precede them).
                return Ok(LaunchExtras {
                    args: Vec::new(),
                    env: vec![(
                        "OPENCODE_CONFIG_CONTENT".into(),
                        serde_json::to_string(&Value::Object(config))?.into(),
                    )],
                    ..Default::default()
                });
            }
            let xdg = dir.join("xdg");
            let opencode_dir = xdg.join("opencode");
            private_dir(dir)?;
            private_dir(&xdg)?;
            private_dir(&opencode_dir)?;
            write_private(
                &opencode_dir.join("opencode.json"),
                &serde_json::to_vec_pretty(&Value::Object(config))?,
            )?;
            Ok(LaunchExtras {
                args: Vec::new(),
                env: vec![("XDG_CONFIG_HOME".into(), xdg.into_os_string())],
                ..Default::default()
            })
        }
        _ => Ok(LaunchExtras::default()),
    }
}

/// The opening prompt of a session started from a note or with a role
/// (decision 138f). A constant with nothing interpolated: the agent reads
/// both notes through `session_context`, so no note content, title or id is
/// ever on a command line.
pub const OPENING_PROMPT: &str =
    "Read your launch context with the storm session_context tool, then wait for my instructions.";

/// The opening prompt of a session another agent started (decisions 134f,
/// 145): as constant as [`OPENING_PROMPT`]. Its task, and any note, come only
/// through `session_context`; nothing ever reaches its terminal, so it is
/// told to report each round and then wait in `read_messages`, where each
/// message from its parent is its next round's task, and that a refusal is
/// reported `blocked` (148b), since nobody answers a child. Word for word
/// `docs/spawned-start-vectors.json`'s `opening_prompt`.
pub const SPAWNED_OPENING_PROMPT: &str = "Read your task with the storm session_context tool and do it. At the end of each round, report your result with the storm report_result tool. Then call the storm read_messages tool with wait set, and keep calling it until you are ended: each message from the agent that started you is the task for your next round. Nobody can answer a prompt of yours: if something you need is refused, say so in report_result with outcome blocked and stop that path.";

/// Which opening prompt a session gets: a spawned child's, else a note's.
/// A child resumed from its parked conversation (145e) already knows its
/// task and its loop: it is sent to `read_messages`, where its next round
/// waits, in the waker's own fixed words.
pub fn opening_prompt(context: bool, spawned: bool, resumed: bool) -> Option<&'static str> {
    if spawned && resumed {
        Some(crate::harness::WAKE_MESSAGE_TEXT)
    } else if spawned {
        Some(SPAWNED_OPENING_PROMPT)
    } else if context {
        Some(OPENING_PROMPT)
    } else {
        None
    }
}

/// Adds an opening prompt, as each CLI takes one. Nothing for any other
/// provider, `shell` and `fake` included.
pub fn with_opening_prompt(
    mut extras: LaunchExtras,
    provider: &str,
    prompt: Option<&'static str>,
) -> LaunchExtras {
    use crate::profile::Prompt;
    if let Some(prompt) = prompt {
        match crate::profile::get(provider).and_then(|p| p.prompt) {
            Some(Prompt::Positional) => extras.args.push(prompt.into()),
            Some(Prompt::Flag(flag)) => extras.args.extend([flag.into(), prompt.into()]),
            None => {}
        }
    }
    extras
}

/// A session's bridge registration on this host.
#[derive(Debug, Clone)]
pub struct Registration {
    pub session: String,
    /// slug → connection id.
    pub connections: HashMap<String, String>,
}

/// Session handles: random, host-local, and the only thing a bridge presents.
#[derive(Default)]
pub struct Handles {
    by_handle: Mutex<HashMap<String, Registration>>,
}

pub fn new_handle() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    data_encoding::BASE64URL_NOPAD.encode(&bytes)
}

impl Handles {
    pub fn register(&self, handle: &str, session: &str, grants: &[McpGrant]) {
        self.by_handle.lock().unwrap().insert(
            handle.to_string(),
            Registration {
                session: session.to_string(),
                connections: grants
                    .iter()
                    .map(|g| (g.slug.clone(), g.id.clone()))
                    .collect(),
            },
        );
    }

    pub fn forget_session(&self, session: &str) {
        self.by_handle
            .lock()
            .unwrap()
            .retain(|_, r| r.session != session);
    }

    /// `(session, connection id)` for a bridge's handle and slug.
    pub fn resolve(&self, handle: &str, slug: &str) -> Option<(String, String)> {
        let map = self.by_handle.lock().unwrap();
        let r = map.get(handle)?;
        Some((r.session.clone(), r.connections.get(slug)?.clone()))
    }
}

/// One line from a bridge on the daemon socket.
#[derive(Debug, Deserialize, Serialize)]
pub struct BridgeLine {
    pub handle: String,
    pub connection: String,
    /// Forwarded byte for byte: the host never needs to look inside it.
    #[serde(default)]
    pub message: Option<Box<serde_json::value::RawValue>>,
    /// Keep this connection open for the session's unsolicited messages.
    #[serde(default)]
    pub subscribe: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    fn grants() -> Vec<McpGrant> {
        vec![
            McpGrant {
                id: "storm".into(),
                slug: "storm".into(),
                plugin_origin: None,
            },
            McpGrant {
                id: "mcc_GH".into(),
                slug: "github".into(),
                plugin_origin: None,
            },
        ]
    }

    #[test]
    fn claude_code_merges_plugin_servers_beside_the_bridges() {
        let dir = tempfile::tempdir().unwrap();
        let session = dir.path().join("ags_P");
        let mut plugins = serde_json::Map::new();
        plugins.insert(
            "plugin_pw_playwright".into(),
            json!({"type": "stdio", "command": "npx"}),
        );
        plugins.insert("storm".into(), json!({"command": "/evil"}));
        let extras = write_provider_config(
            "claude-code",
            &session,
            &grants(),
            "h",
            Path::new("/s"),
            Path::new("/x"),
            None,
            &plugins,
            &AgentServers::storm_only(),
            &StormLayer::default(),
            &|| Ok(Vec::new()),
        )
        .unwrap();
        assert!(extras.args.contains(&OsString::from("--strict-mcp-config")));
        let v: Value =
            serde_json::from_slice(&std::fs::read(session.join("mcp.json")).unwrap()).unwrap();
        assert_eq!(v["mcpServers"]["plugin_pw_playwright"]["command"], "npx");
        assert_eq!(
            v["mcpServers"]["storm"]["command"], "/x",
            "a bridge is never replaced"
        );
    }

    /// Decision 132: the agent's own servers load beside Storm's, and where
    /// one already has a slug's name Storm's steps aside, since Claude Code
    /// would load `--mcp-config`'s and drop the account's (gate G-132a).
    #[test]
    fn claude_code_keeps_its_own_servers_and_storm_steps_aside_on_a_clash() {
        let dir = tempfile::tempdir().unwrap();
        let session = dir.path().join("ags_N");
        let taken: std::collections::HashSet<String> =
            ["storm".to_string(), "github".to_string()].into();
        let extras = write_provider_config(
            "claude-code",
            &session,
            &grants(),
            "h",
            Path::new("/s"),
            Path::new("/x"),
            None,
            &Default::default(),
            &AgentServers {
                storm_only: false,
                taken: &taken,
            },
            &StormLayer::default(),
            &|| Ok(Vec::new()),
        )
        .unwrap();
        assert!(!extras.args.contains(&OsString::from("--strict-mcp-config")));
        assert!(extras.args.contains(&OsString::from("--mcp-config")));
        let v: Value =
            serde_json::from_slice(&std::fs::read(session.join("mcp.json")).unwrap()).unwrap();
        let keys: Vec<&String> = v["mcpServers"].as_object().unwrap().keys().collect();
        assert!(keys.contains(&&"storm_github".to_string()), "{keys:?}");
        assert!(!keys.contains(&&"github".to_string()), "{keys:?}");
        assert!(
            keys.contains(&&"storm".to_string()),
            "the opening prompt names it: {keys:?}"
        );
        assert_eq!(
            v["mcpServers"]["storm_github"]["args"],
            json!(["mcp-bridge", "github"]),
            "still addressed by its slug"
        );
    }

    /// Decision 132b: OpenCode keeps its global config and gets Storm's
    /// servers and asks on top of it, in the environment; a slug the global
    /// config uses steps aside.
    #[test]
    fn opencode_gets_storms_servers_on_top_of_its_global_config() {
        let dir = tempfile::tempdir().unwrap();
        let session = dir.path().join("ags_OC");
        let taken: std::collections::HashSet<String> =
            ["storm".to_string(), "github".to_string()].into();
        let extras = write_provider_config(
            "opencode",
            &session,
            &grants(),
            "h",
            Path::new("/s"),
            Path::new("/x"),
            Some(&json!({"mcp": {"evil": {}}, "permission": {"bash": "allow"}})),
            &Default::default(),
            &AgentServers {
                storm_only: false,
                taken: &taken,
            },
            &StormLayer::default(),
            &|| Ok(Vec::new()),
        )
        .unwrap();
        assert!(
            !extras.env.iter().any(|(k, _)| k == "XDG_CONFIG_HOME"),
            "the global config stays"
        );
        let content = extras
            .env
            .iter()
            .find(|(k, _)| k == "OPENCODE_CONFIG_CONTENT")
            .map(|(_, v)| v.to_str().unwrap().to_string())
            .unwrap();
        let v: Value = serde_json::from_str(&content).unwrap();
        let keys: Vec<&String> = v["mcp"].as_object().unwrap().keys().collect();
        assert_eq!(keys, ["storm", "storm_github"], "a setting adds no server");
        assert_eq!(
            v["mcp"]["storm_github"]["command"],
            json!(["/x", "mcp-bridge", "github"])
        );
        assert_eq!(v["permission"]["storm_github_*"], "ask");
        assert_eq!(v["permission"]["storm_*"], "ask");
        assert_eq!(v["permission"]["bash"], "allow");
        assert!(!session.exists(), "nothing written to disk");
    }

    #[test]
    fn storm_only_holds_even_without_grants() {
        let dir = tempfile::tempdir().unwrap();
        for provider in ["claude-code", "opencode"] {
            let session = dir.path().join(provider);
            let extras = write_provider_config(
                provider,
                &session,
                &[],
                "h",
                Path::new("/s"),
                Path::new("/x"),
                None,
                &Default::default(),
                &AgentServers::storm_only(),
                &StormLayer::default(),
                &|| Ok(Vec::new()),
            )
            .unwrap();
            assert_ne!(extras, LaunchExtras::default(), "{provider}");
        }
        let v: Value = serde_json::from_slice(
            &std::fs::read(dir.path().join("claude-code").join("mcp.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(v["mcpServers"], json!({}));
    }

    /// 131h: a connection added from a plugin keeps the plugin's server name
    /// in Claude Code, unless another bridge has it; the bridge is still
    /// addressed by slug, and a local plugin server never takes its place.
    #[test]
    fn a_plugin_connection_keeps_the_plugins_name_in_claude_code() {
        let dir = tempfile::tempdir().unwrap();
        let session = dir.path().join("ags_O");
        let origin = Some(PluginOrigin {
            plugin: "slack".into(),
            server: "slack".into(),
        });
        let grants = [
            McpGrant {
                id: "mcc_A".into(),
                slug: "slack".into(),
                plugin_origin: origin.clone(),
            },
            McpGrant {
                id: "mcc_B".into(),
                slug: "slack-2".into(),
                plugin_origin: origin,
            },
            McpGrant {
                id: "mcc_C".into(),
                slug: "odd".into(),
                plugin_origin: Some(PluginOrigin {
                    plugin: String::new(),
                    server: "x".into(),
                }),
            },
        ];
        let mut plugins = serde_json::Map::new();
        plugins.insert("plugin_slack_slack".into(), json!({"command": "/evil"}));
        write_provider_config(
            "claude-code",
            &session,
            &grants,
            "h",
            Path::new("/s"),
            Path::new("/x"),
            None,
            &plugins,
            &AgentServers::storm_only(),
            &StormLayer::default(),
            &|| Ok(Vec::new()),
        )
        .unwrap();
        let v: Value =
            serde_json::from_slice(&std::fs::read(session.join("mcp.json")).unwrap()).unwrap();
        let s = v["mcpServers"].as_object().unwrap();
        assert_eq!(
            s["plugin_slack_slack"]["args"],
            json!(["mcp-bridge", "slack"])
        );
        assert_eq!(s["slack-2"]["args"], json!(["mcp-bridge", "slack-2"]));
        assert_eq!(s["odd"]["args"], json!(["mcp-bridge", "odd"]));
        assert_eq!(s.len(), 3);
    }

    #[test]
    fn claude_code_gets_a_strict_config_of_bridges_only() {
        let dir = tempfile::tempdir().unwrap();
        let session = dir.path().join("ags_X");
        let extras = write_provider_config(
            "claude-code",
            &session,
            &grants(),
            "handle-1",
            Path::new("/var/lib/storm-runtime/run/mcp.sock"),
            Path::new("/usr/bin/storm-runtime"),
            None,
            &serde_json::Map::new(),
            &AgentServers::storm_only(),
            &StormLayer::default(),
            &|| Ok(Vec::new()),
        )
        .unwrap();
        let path = session.join("mcp.json");
        assert_eq!(
            extras.args,
            vec![
                OsString::from("--mcp-config"),
                path.clone().into_os_string(),
                OsString::from("--strict-mcp-config")
            ]
        );
        assert!(extras.env.is_empty());
        let config: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        let gh = &config["mcpServers"]["github"];
        assert_eq!(gh["command"], "/usr/bin/storm-runtime");
        assert_eq!(gh["args"], json!(["mcp-bridge", "github"]));
        assert_eq!(gh["env"][HANDLE_ENV], "handle-1");
        assert_eq!(config["mcpServers"].as_object().unwrap().len(), 2);
        crate::platform::assert_private(&session, 0o700);
        crate::platform::assert_private(&path, 0o600);
    }

    #[test]
    fn opencode_gets_its_own_config_home_and_ask_for_every_slug() {
        let dir = tempfile::tempdir().unwrap();
        let session = dir.path().join("ags_X");
        // A host setting may set a model; it may not add an MCP server, nor
        // relax a gateway connection's prompt.
        let settings = json!({
            "model": "opencode/big-pickle",
            "mcp": {"proxmox": {"type": "remote", "url": "https://evil"}},
            "permission": {"github_*": "allow", "bash": "ask"},
        });
        let extras = write_provider_config(
            "opencode",
            &session,
            &grants(),
            "h",
            Path::new("/s"),
            Path::new("/usr/bin/storm-runtime"),
            Some(&settings),
            &serde_json::Map::new(),
            &AgentServers::storm_only(),
            &StormLayer::default(),
            &|| Ok(Vec::new()),
        )
        .unwrap();
        assert!(extras.args.is_empty());
        assert_eq!(
            extras.env,
            vec![(
                OsString::from("XDG_CONFIG_HOME"),
                session.join("xdg").into_os_string()
            )]
        );
        let config: Value = serde_json::from_slice(
            &std::fs::read(session.join("xdg/opencode/opencode.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(config["model"], "opencode/big-pickle");
        let servers: Vec<&String> = config["mcp"].as_object().unwrap().keys().collect();
        assert_eq!(servers, vec!["github", "storm"]);
        assert_eq!(config["permission"]["github_*"], "ask");
        assert_eq!(config["permission"]["storm_*"], "ask");
        assert_eq!(config["permission"]["bash"], "ask");
        assert_eq!(
            config["mcp"]["github"]["command"],
            json!(["/usr/bin/storm-runtime", "mcp-bridge", "github"])
        );
    }

    fn flag_count(extras: &LaunchExtras) -> usize {
        extras
            .env
            .iter()
            .filter(|(k, v)| k == "OPENCODE_DISABLE_PROJECT_CONFIG" && v == "1")
            .count()
    }

    #[test]
    fn every_opencode_session_skips_project_config_once_with_or_without_grants() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        for provider in ["opencode", "claude-code", "shell", "fake"] {
            for grants in [vec![], grants()] {
                let session = dir.path().join(format!("{provider}-{}", grants.len()));
                let mut launch = provider_baseline(provider, &session, workspace.path()).unwrap();
                let mcp = write_provider_config(
                    provider,
                    &session,
                    &grants,
                    "h",
                    Path::new("/s"),
                    Path::new("/x"),
                    None,
                    &serde_json::Map::new(),
                    &AgentServers::storm_only(),
                    &StormLayer::default(),
                    &|| Ok(Vec::new()),
                )
                .unwrap();
                launch.env.extend(mcp.env);
                let expected = usize::from(provider == "opencode");
                assert_eq!(flag_count(&launch), expected, "{provider}, {grants:?}");
            }
        }
    }

    fn instructions(workspace: &Path, session: &Path) -> Option<Value> {
        let extras = provider_baseline("opencode", session, workspace).unwrap();
        assert_eq!(flag_count(&extras), 1);
        let path = session.join(OPENCODE_INSTRUCTIONS);
        let named = extras
            .env
            .iter()
            .find(|(k, _)| k == "OPENCODE_CONFIG")
            .map(|(_, v)| v.clone());
        match named {
            Some(v) => {
                assert_eq!(v, path.clone().into_os_string());
                crate::platform::assert_private(&path, 0o600);
                Some(serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap())
            }
            None => {
                assert!(!path.exists());
                None
            }
        }
    }

    #[test]
    fn the_workspaces_instruction_file_comes_back_by_absolute_path_in_opencodes_order() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().join("app");
        std::fs::create_dir(&workspace).unwrap();
        // Only the workspace itself is looked in, never above it.
        std::fs::write(dir.path().join("AGENTS.md"), "parent").unwrap();
        assert_eq!(instructions(&workspace, &dir.path().join("ags_0")), None);
        let mut expected = None;
        for name in ["CONTEXT.md", "CLAUDE.md", "AGENTS.md"] {
            std::fs::write(workspace.join(name), "x").unwrap();
            expected = Some(workspace.join(name).display().to_string());
            let config =
                instructions(&workspace, &dir.path().join(format!("ags_{name}"))).expect(name);
            assert_eq!(config["instructions"], json!([expected]), "{name}");
            assert_eq!(config.as_object().unwrap().len(), 2);
        }
        assert!(Path::new(expected.as_deref().unwrap()).is_absolute());
        // A directory named like the file is not one.
        let other = dir.path().join("other");
        std::fs::create_dir_all(other.join("AGENTS.md")).unwrap();
        assert_eq!(instructions(&other, &dir.path().join("ags_dir")), None);
    }

    #[test]
    fn a_path_with_a_glob_character_is_not_restored() {
        let dir = tempfile::tempdir().unwrap();
        for (i, name) in ["a*b", "a?b", "a[1]", "a{b,c}"].into_iter().enumerate() {
            if cfg!(windows) && name.contains(['*', '?']) {
                continue;
            }
            let workspace = dir.path().join(name);
            std::fs::create_dir(&workspace).unwrap();
            std::fs::write(workspace.join("AGENTS.md"), "x").unwrap();
            assert_eq!(
                instructions(&workspace, &dir.path().join(format!("ags_{i}"))),
                None,
                "{name}"
            );
        }
    }

    #[test]
    fn shell_and_unknown_providers_get_nothing_and_no_grants_write_nothing() {
        let dir = tempfile::tempdir().unwrap();
        for provider in ["shell", "fake", "my-agent"] {
            let session = dir.path().join(provider);
            let extras = write_provider_config(
                provider,
                &session,
                &grants(),
                "h",
                Path::new("/s"),
                Path::new("/x"),
                None,
                &serde_json::Map::new(),
                &AgentServers::storm_only(),
                &StormLayer::default(),
                &|| Ok(Vec::new()),
            )
            .unwrap();
            assert_eq!(extras, LaunchExtras::default());
            assert!(!session.exists());
        }
        let session = dir.path().join("none");
        let extras = write_provider_config(
            "claude-code",
            &session,
            &[],
            "h",
            Path::new("/s"),
            Path::new("/x"),
            None,
            &serde_json::Map::new(),
            &AgentServers {
                storm_only: false,
                taken: &Default::default(),
            },
            &StormLayer::default(),
            &|| Ok(Vec::new()),
        )
        .unwrap();
        assert_eq!(extras, LaunchExtras::default());
        assert!(!session.exists());
    }

    #[test]
    fn a_session_from_a_note_gets_exactly_the_constant_prompt_and_nothing_else() {
        let dir = tempfile::tempdir().unwrap();
        for provider in ["claude-code", "opencode", "shell", "fake", "my-agent"] {
            let config = |name: &str| {
                write_provider_config(
                    provider,
                    &dir.path().join(name).join(provider),
                    &grants(),
                    "h",
                    Path::new("/s"),
                    Path::new("/x"),
                    None,
                    &serde_json::Map::new(),
                    &AgentServers::storm_only(),
                    &StormLayer::default(),
                    &|| Ok(Vec::new()),
                )
                .unwrap()
            };
            let plain = config("plain");
            assert_eq!(with_opening_prompt(config("plain"), provider, None), plain);
            let with = with_opening_prompt(config("context"), provider, Some(OPENING_PROMPT));
            assert_eq!(
                with.env.len(),
                plain.env.len(),
                "{provider}: the env is unchanged"
            );
            let added: Vec<_> = with.args[plain.args.len()..].to_vec();
            let expected: Vec<std::ffi::OsString> = match provider {
                "claude-code" => vec![OPENING_PROMPT.into()],
                "opencode" => vec!["--prompt".into(), OPENING_PROMPT.into()],
                _ => vec![],
            };
            assert_eq!(added, expected, "{provider}");
        }
        // Nothing can ever be interpolated into either.
        for c in ['{', '}', '$', '%', '`', '\n', '"', '\''] {
            assert!(!OPENING_PROMPT.contains(c), "{c:?}");
            assert!(!SPAWNED_OPENING_PROMPT.contains(c), "{c:?}");
        }
    }

    /// 134f: a spawned child gets its own prompt, whatever else it was
    /// started with; a note's session the note's; any other none.
    #[test]
    fn a_spawned_child_gets_the_spawned_opening_prompt() {
        assert_eq!(opening_prompt(false, false, false), None);
        assert_eq!(opening_prompt(true, false, false), Some(OPENING_PROMPT));
        assert_eq!(
            opening_prompt(false, true, false),
            Some(SPAWNED_OPENING_PROMPT)
        );
        assert_eq!(
            opening_prompt(true, true, false),
            Some(SPAWNED_OPENING_PROMPT)
        );
        // 145e: a parked child resumed is sent to its next task.
        assert_eq!(
            opening_prompt(true, true, true),
            Some(crate::harness::WAKE_MESSAGE_TEXT)
        );
        assert_eq!(opening_prompt(false, false, true), None);
        for tool in ["session_context", "read_messages", "report_result"] {
            assert!(SPAWNED_OPENING_PROMPT.contains(tool), "{tool}");
        }
    }

    /// 145: the child's prompt is the one the vectors name, so the server's
    /// test of the tools it names reads the same words.
    #[test]
    fn the_spawned_opening_prompt_is_the_vectors() {
        let v: Value =
            serde_json::from_str(include_str!("../../../docs/spawned-start-vectors.json")).unwrap();
        assert_eq!(v["opening_prompt"], SPAWNED_OPENING_PROMPT);
    }

    #[test]
    fn the_opening_prompt_asks_for_the_whole_launch_context() {
        assert_eq!(
            OPENING_PROMPT,
            "Read your launch context with the storm session_context tool, then wait for my instructions."
        );
    }

    #[test]
    fn a_handle_resolves_only_its_own_sessions_connections() {
        let h = Handles::default();
        let handle = new_handle();
        assert_eq!(handle.len(), 43);
        h.register(&handle, "ags_X", &grants());
        assert_eq!(
            h.resolve(&handle, "github"),
            Some(("ags_X".into(), "mcc_GH".into()))
        );
        assert_eq!(h.resolve(&handle, "linear"), None);
        assert_eq!(h.resolve("forged", "github"), None);
        h.forget_session("ags_X");
        assert_eq!(h.resolve(&handle, "github"), None);
    }
}

#[cfg(test)]
#[path = "mcp_spawn_tests.rs"]
mod spawn_tests;
