//! A Claude Code session's plugin MCP servers (decision 131).
//!
//! `--strict-mcp-config` (G-D12) drops every MCP server that is not in the
//! session's own `mcp.json`, plugins' included, and says nothing. So the host
//! finds the session's enabled plugins itself and:
//!
//! - writes each plugin's **local (stdio)** server into `mcp.json` under the
//!   name Claude Code would give it, so its tools keep the names the plugin's
//!   own commands and hooks use (131c);
//! - never writes a **remote** one: those reach a session only through the
//!   gateway (131);
//! - names every server the session will not have, and why (131a): the
//!   remote ones, the ones it cannot resolve (131d), and the account's and
//!   the workspace's own servers, by name only.
//!
//! Nothing here ever copies a command, URL, header or env value out of the
//! account's or the workspace's own files.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use serde::Serialize;
use serde_json::{Map, Value, json};

/// How long `claude plugin list` may take before the session starts without
/// plugin servers (about a second on a Runtime Host).
const LIST_DEADLINE: Duration = Duration::from_secs(5);
/// The session's environment variable naming its notice file, which
/// `harness-session` prints.
pub const NOTICE_ENV: &str = "STORM_MCP_NOTICE";
pub const NOTICE_FILE: &str = "mcp-notice.txt";

/// An enabled plugin, as `claude plugin list --json` reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    /// `<name>@<marketplace>`.
    pub id: String,
    pub install_path: PathBuf,
    /// The servers the list itself reports (Claude Code 2.1.306 does, after
    /// resolving every form a manifest may use); `None` from one that does
    /// not, and then the plugin's own files are read.
    pub servers: Option<Map<String, Value>>,
    /// Enabled for the account, not only for the workspace's project.
    pub enabled: bool,
    pub version: Option<String>,
}

/// A server the session will not have.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Skipped {
    pub name: String,
    pub source: String,
    pub reason: &'static str,
}

pub(crate) const REMOTE: &str = "remote: add it in Settings ▸ Integrations";
pub(crate) const OFF: &str = "plugin servers are off on this host";
const UNRESOLVED: &str = "needs plugin options Storm can't read";
pub(crate) const UNREADABLE: &str = "its plugin's MCP config could not be read";
const UNLISTED: &str = "Storm couldn't list this host's plugins";
pub(crate) const OWN: &str = "your own MCP servers don't load in Storm sessions";
pub(crate) const PROJECT: &str = "project MCP servers don't load in Storm sessions";
pub(crate) const HOOKED: &str = "turned off: it has hooks, or Storm couldn't read it";
pub(crate) const REMOVED: &str =
    "removed from Storm's Codex home: your own MCP servers don't load in Storm sessions";
pub(crate) const APPS: &str = "ChatGPT apps don't load in Storm sessions";

/// What a session gets from its plugins, and what it does not.
#[derive(Debug, Default, PartialEq)]
pub struct Plan {
    /// `mcp.json` entries, by server key.
    pub local: Map<String, Value>,
    /// Plugin servers the agent loads itself (Codex), by name.
    pub native: Vec<String>,
    pub skipped: Vec<Skipped>,
    /// The remote servers found, as [`remote_entry`]s for the host's
    /// inventory (131g).
    pub remote: Vec<Value>,
}

impl Plan {
    /// The first `running` report's extra fields (131a): the app shows them
    /// beside the egress line. A server too old to know them ignores them.
    pub fn report(&self) -> Value {
        json!({
            "mcp_skipped": self.skipped,
            "mcp_plugin_local": self.local.keys().chain(&self.native).collect::<Vec<_>>(),
        })
    }

    /// The terminal's line, or nothing to say.
    pub fn notice(&self) -> Option<String> {
        if self.skipped.is_empty() {
            return None;
        }
        let n = self.skipped.len();
        let mut text = format!(
            "Storm skipped {n} MCP server{} in this session:",
            if n == 1 { "" } else { "s" }
        );
        for s in &self.skipped {
            text.push_str(&format!("\n  • {} ({}) — {}", s.name, s.source, s.reason));
        }
        Some(text)
    }
}

/// The plan for a Claude Code session in `workspace`: its enabled plugins,
/// listed with the provider's own program (`helper`), and the account's
/// Claude Code files — `CLAUDE_CONFIG_DIR` when set, else the home's.
pub fn session_plan(helper: Option<Command>, allow_local: bool, workspace: &Path) -> Plan {
    let listed = helper.and_then(|c| match list(c, workspace) {
        Ok(listed) => Some(listed),
        Err(e) => {
            tracing::warn!(error = %format!("{e:#}"), "could not list Claude Code's plugins");
            None
        }
    });
    let (claude_json, config_dir) = match std::env::var_os("CLAUDE_CONFIG_DIR") {
        Some(dir) => {
            let dir = PathBuf::from(dir);
            (Some(dir.join(".claude.json")), Some(dir))
        }
        None => match std::env::var_os("HOME") {
            Some(home) => {
                let home = PathBuf::from(home);
                (Some(home.join(".claude.json")), Some(home.join(".claude")))
            }
            None => (None, None),
        },
    };
    let data_root = config_dir
        .unwrap_or_else(|| PathBuf::from(".claude"))
        .join("plugins")
        .join("data");
    plan(
        listed.as_deref(),
        allow_local,
        claude_json.as_deref(),
        &data_root,
        workspace,
    )
}

/// The remote servers of the plugins the account has enabled for Claude Code,
/// listed with `helper` from `dir`, for the host's inventory (131g).
pub fn inventory(helper: Command, dir: &Path) -> Vec<Value> {
    let listed = match list(helper, dir) {
        Ok(listed) => listed,
        Err(e) => {
            tracing::warn!(error = %format!("{e:#}"), "could not list Claude Code's plugins");
            return Vec::new();
        }
    };
    let mut out = Vec::new();
    for p in listed.iter().filter(|p| p.enabled) {
        let name = p.id.rsplit_once('@').map_or(p.id.as_str(), |(n, _)| n);
        let servers = match &p.servers {
            Some(servers) => Some(servers.clone().into_iter().collect()),
            None => declared(&p.install_path),
        };
        for (server, entry) in servers.into_iter().flatten() {
            if let Value::Object(entry) = entry
                && is_remote(&entry)
            {
                out.extend(remote_entry(
                    "claude-code",
                    name,
                    p.version.as_deref(),
                    &server,
                    &entry,
                ));
            }
        }
    }
    out
}

/// A plugin's remote server as the host reports it (131g): its URL as
/// declared, unexpanded, and at most the *name* of a header it fills from the
/// environment — never a header value, an env value or a vendor's client id.
pub(crate) fn remote_entry(
    agent: &str,
    plugin: &str,
    version: Option<&str>,
    server: &str,
    entry: &Map<String, Value>,
) -> Option<Value> {
    let url = entry.get("url")?.as_str()?;
    if url.len() > 2048 || plugin.len() > 128 || server.len() > 128 {
        return None;
    }
    let transport = entry
        .get("type")
        .and_then(Value::as_str)
        .filter(|t| matches!(*t, "sse" | "ws"))
        .unwrap_or("http");
    let templated_header = entry
        .get("headers")
        .and_then(Value::as_object)
        .and_then(|h| {
            h.iter()
                .find(|(_, v)| v.as_str().is_some_and(|v| v.contains("${")))
        })
        .map(|(k, _)| k.as_str())
        .filter(|k| k.len() <= 128);
    let auth_hint = match templated_header {
        Some(name) => Some(format!("header:{name}")),
        None if entry.get("oauth").and_then(|o| o.get("clientId")).is_some() => {
            Some("vendor_client".to_string())
        }
        None => None,
    };
    Some(json!({
        "agent": agent,
        "plugin": plugin,
        "plugin_version": version.filter(|v| v.len() <= 64),
        "server": server,
        "transport": transport,
        "url": url,
        "templated": url.contains("${"),
        "auth_hint": auth_hint,
    }))
}

/// Runs `claude plugin list --json` in `workspace` (so a project-enabled
/// plugin counts) and returns the enabled plugins.
pub fn list(mut command: Command, workspace: &Path) -> Result<Vec<Listed>> {
    command
        .args(["plugin", "list", "--json"])
        .current_dir(workspace)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    let mut child = command.spawn().context("starting claude plugin list")?;
    let mut stdout = child.stdout.take().context("no stdout")?;
    // Read on a thread: a full pipe would hold the child past the deadline.
    let reader = std::thread::spawn(move || {
        let mut out = Vec::new();
        let _ = stdout.by_ref().take(4 << 20).read_to_end(&mut out);
        out
    });
    let deadline = Instant::now() + LIST_DEADLINE;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            end_group(&child);
            let _ = child.kill();
            let _ = child.wait();
            bail!("claude plugin list took longer than {LIST_DEADLINE:?}");
        }
        std::thread::sleep(Duration::from_millis(25));
    };
    // Whatever it left behind would outlive it as the host's account and
    // hold the pipe open.
    end_group(&child);
    let out = reader.join().unwrap_or_default();
    if !status.success() {
        bail!("claude plugin list exited with {status}");
    }
    parse_list(&out)
}

/// SIGKILL to the listing's process group: on macOS nothing else ends a
/// child it started.
#[cfg(unix)]
fn end_group(child: &std::process::Child) {
    if let Some(pid) = rustix::process::Pid::from_raw(child.id() as i32) {
        let _ = rustix::process::kill_process_group(pid, rustix::process::Signal::KILL);
    }
}

#[cfg(not(unix))]
fn end_group(_: &std::process::Child) {}

fn parse_list(out: &[u8]) -> Result<Vec<Listed>> {
    let entries: Vec<Value> = serde_json::from_slice(out).context("parsing claude plugin list")?;
    Ok(entries
        .iter()
        .filter(|e| e["enabled"] == true || e["projectEnabled"] == true)
        .filter_map(|e| {
            Some(Listed {
                id: e["id"].as_str()?.to_string(),
                install_path: PathBuf::from(e["installPath"].as_str()?),
                servers: e["mcpServers"].as_object().cloned(),
                enabled: e["enabled"] == true,
                version: e["version"].as_str().map(str::to_owned),
            })
        })
        .collect())
}

/// The session's plan. `listed` is `None` when the plugins could not be
/// listed; `claude_json` is the account's `.claude.json` and `data_root`
/// Claude Code's `plugins/data`.
pub fn plan(
    listed: Option<&[Listed]>,
    allow_local: bool,
    claude_json: Option<&Path>,
    data_root: &Path,
    workspace: &Path,
) -> Plan {
    let mut plan = Plan::default();
    match listed {
        None => plan.skipped.push(Skipped {
            name: "plugin servers".into(),
            source: "plugins".into(),
            reason: UNLISTED,
        }),
        Some(listed) => {
            for p in listed {
                plugin(&mut plan, p, allow_local, data_root, workspace);
            }
        }
    }
    if let Some(path) = claude_json {
        for name in own_servers(path, workspace) {
            plan.skipped.push(Skipped {
                name,
                source: "your own".into(),
                reason: OWN,
            });
        }
    }
    for name in server_names(&read_json(&workspace.join(".mcp.json"))) {
        plan.skipped.push(Skipped {
            name,
            source: "this workspace's .mcp.json".into(),
            reason: PROJECT,
        });
    }
    plan
}

fn plugin(plan: &mut Plan, p: &Listed, allow_local: bool, data_root: &Path, workspace: &Path) {
    let name = p.id.rsplit_once('@').map_or(p.id.as_str(), |(n, _)| n);
    let source = format!("plugin {name}");
    let servers = match &p.servers {
        Some(servers) => Some(servers.clone().into_iter().collect()),
        None => declared(&p.install_path),
    };
    let Some(servers) = servers else {
        plan.skipped.push(Skipped {
            name: name.into(),
            source,
            reason: UNREADABLE,
        });
        return;
    };
    let data = data_root.join(data_dir_name(&p.id));
    for (server, entry) in servers {
        let skip = |reason| Skipped {
            name: server.clone(),
            source: source.clone(),
            reason,
        };
        let Value::Object(entry) = entry else {
            plan.skipped.push(skip(UNREADABLE));
            continue;
        };
        if is_remote(&entry) {
            plan.skipped.push(skip(REMOTE));
            plan.remote.extend(remote_entry(
                "claude-code",
                name,
                p.version.as_deref(),
                &server,
                &entry,
            ));
            continue;
        }
        if !entry.get("command").is_some_and(Value::is_string) {
            plan.skipped.push(skip(UNREADABLE));
            continue;
        }
        if !allow_local {
            plan.skipped.push(skip(OFF));
            continue;
        }
        let root = p.install_path.display().to_string();
        let data_s = data.display().to_string();
        let ws = workspace.display().to_string();
        let mut value = Value::Object(entry);
        substitute(
            &mut value,
            &[
                ("${CLAUDE_PLUGIN_ROOT}", &root),
                ("${CLAUDE_PLUGIN_DATA}", &data_s),
                ("${CLAUDE_PROJECT_DIR}", &ws),
            ],
        );
        if mentions_unresolved(&value) {
            plan.skipped.push(skip(UNRESOLVED));
            continue;
        }
        // Claude Code gives a plugin's server these two in its environment
        // as well; a server may read them there rather than from its config.
        let Value::Object(mut entry) = value else {
            unreachable!()
        };
        let env = entry
            .entry("env")
            .or_insert_with(|| Value::Object(Map::new()));
        if let Value::Object(env) = env {
            env.entry("CLAUDE_PLUGIN_ROOT").or_insert(json!(root));
            env.entry("CLAUDE_PLUGIN_DATA").or_insert(json!(data_s));
        }
        entry.entry("type").or_insert(json!("stdio"));
        let key = server_key(name, &server);
        if plan.local.contains_key(&key) {
            plan.skipped.push(skip(UNREADABLE));
            continue;
        }
        let _ = std::fs::create_dir_all(&data);
        plan.local.insert(key, Value::Object(entry));
    }
}

/// Where a plugin's manifest may be, Claude Code's first (Codex also reads
/// Claude- and Cursor-format plugins).
const MANIFESTS: [&str; 4] = [
    ".claude-plugin/plugin.json",
    ".codex-plugin/plugin.json",
    ".cursor-plugin/plugin.json",
    "plugin.json",
];

/// The servers a plugin declares: its `.mcp.json`, or `mcpServers` in its
/// manifest — an object, a path to such a file, or a list of either.
pub(crate) fn declared(root: &Path) -> Option<Vec<(String, Value)>> {
    let manifest = MANIFESTS
        .iter()
        .map(|m| read_json(&root.join(m)))
        .find(|v| !v.is_null())
        .unwrap_or(Value::Null);
    let mut out = Vec::new();
    let mut sources = Vec::new();
    match manifest.get("mcpServers") {
        Some(Value::Array(items)) => sources.extend(items.iter().cloned()),
        Some(v) => sources.push(v.clone()),
        None => {
            let path = root.join(".mcp.json");
            if !path.exists() {
                return Some(out);
            }
            sources.push(Value::String(".mcp.json".into()));
        }
    }
    for source in sources {
        let servers = match source {
            Value::String(rel) => {
                let rel = rel.trim_start_matches("./");
                if Path::new(rel).components().any(|c| {
                    matches!(
                        c,
                        std::path::Component::ParentDir
                            | std::path::Component::RootDir
                            | std::path::Component::Prefix(_)
                    )
                }) {
                    return None;
                }
                let text = std::fs::read(root.join(rel)).ok()?;
                let v: Value = serde_json::from_slice(&text).ok()?;
                unwrap_servers(v)?
            }
            Value::Object(_) => unwrap_servers(source)?,
            _ => return None,
        };
        out.extend(servers);
    }
    Some(out)
}

/// `{"mcpServers": {...}}`, or the map itself.
fn unwrap_servers(v: Value) -> Option<Vec<(String, Value)>> {
    let Value::Object(mut map) = v else {
        return None;
    };
    let map = match map.remove("mcpServers") {
        Some(Value::Object(inner)) => inner,
        Some(_) => return None,
        None => map,
    };
    Some(map.into_iter().collect())
}

pub(crate) fn is_remote(entry: &Map<String, Value>) -> bool {
    entry.contains_key("url")
        || matches!(
            entry.get("type").and_then(Value::as_str),
            Some("http" | "sse" | "ws" | "streamable-http")
        )
}

fn substitute(v: &mut Value, pairs: &[(&str, &str)]) {
    match v {
        Value::String(s) => {
            for (from, to) in pairs {
                if s.contains(from) {
                    *s = s.replace(from, to);
                }
            }
        }
        Value::Array(items) => items.iter_mut().for_each(|i| substitute(i, pairs)),
        Value::Object(map) => map.values_mut().for_each(|i| substitute(i, pairs)),
        _ => {}
    }
}

/// A variable only Claude Code's own plugin loader fills in: a plugin's
/// options, or any other `CLAUDE_` one.
fn mentions_unresolved(v: &Value) -> bool {
    match v {
        Value::String(s) => s.contains("${user_config.") || s.contains("${CLAUDE_"),
        Value::Array(items) => items.iter().any(mentions_unresolved),
        Value::Object(map) => map.values().any(mentions_unresolved),
        _ => false,
    }
}

/// The key Claude Code's own name for the server (`plugin:<name>:<server>`)
/// becomes in tool names: every character outside `[A-Za-z0-9_-]` is `_`.
pub fn server_key(plugin: &str, server: &str) -> String {
    format!("plugin_{plugin}_{server}")
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Claude Code's `plugins/data/<id>`: the id with every character outside
/// `[A-Za-z0-9_-]` as `-` (`slack@claude-plugins-official` →
/// `slack-claude-plugins-official`).
fn data_dir_name(id: &str) -> String {
    id.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

pub(crate) fn read_json(path: &Path) -> Value {
    std::fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(Value::Null)
}

pub(crate) fn server_names(v: &Value) -> Vec<String> {
    v.get("mcpServers")
        .and_then(Value::as_object)
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default()
}

/// The account's own servers in `.claude.json`: its top level, and this
/// workspace's project entry. Names only.
fn own_servers(path: &Path, workspace: &Path) -> Vec<String> {
    let v = read_json(path);
    let mut names = server_names(&v);
    let mut keys = vec![workspace.display().to_string()];
    if let Ok(c) = workspace.canonicalize() {
        keys.push(c.display().to_string());
    }
    for key in keys {
        for name in server_names(&v["projects"][&key]) {
            if !names.contains(&name) {
                names.push(name);
            }
        }
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, body: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    }

    fn listed(id: &str, root: &Path) -> Listed {
        Listed {
            id: id.into(),
            install_path: root.into(),
            servers: None,
            enabled: true,
            version: None,
        }
    }

    #[test]
    fn the_list_counts_enabled_and_project_enabled_plugins() {
        let out = br#"[
            {"id":"a@m","enabled":true,"installPath":"/p/a","projectEnabled":false},
            {"id":"b@m","enabled":false,"installPath":"/p/b","projectEnabled":true},
            {"id":"c@m","enabled":false,"installPath":"/p/c","projectEnabled":false}
        ]"#;
        let got = parse_list(out).unwrap();
        assert_eq!(
            got.iter().map(|l| l.id.as_str()).collect::<Vec<_>>(),
            ["a@m", "b@m"]
        );
        assert_eq!(got[0].servers, None);
        let with = parse_list(
            br#"[{"id":"a@m","enabled":true,"installPath":"/p/a","mcpServers":{"s":{"command":"x"}}}]"#,
        )
        .unwrap();
        assert_eq!(with[0].servers.as_ref().unwrap()["s"]["command"], "x");
        assert!(parse_list(b"not json").is_err());
    }

    /// The list's own `mcpServers` win over the plugin's files.
    #[test]
    fn the_listed_servers_are_used_when_reported() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("p");
        write(&root.join(".mcp.json"), r#"{"stale":{"command":"old"}}"#);
        let mut l = listed("p@m", &root);
        l.servers = Some(
            serde_json::from_str(r#"{"fresh":{"command":"${CLAUDE_PLUGIN_ROOT}/new"}}"#).unwrap(),
        );
        let p = plan(Some(&[l]), true, None, t.path(), t.path());
        assert_eq!(p.local.keys().collect::<Vec<_>>(), ["plugin_p_fresh"]);
        assert_eq!(
            p.local["plugin_p_fresh"]["command"],
            format!("{}/new", root.display())
        );
    }

    /// 131c, 131d: a stdio server is merged under Claude Code's own name with
    /// its variables filled in; a remote one and an unresolvable one are
    /// named instead.
    #[test]
    fn local_servers_are_merged_and_the_rest_named() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("slack");
        write(
            &root.join(".mcp.json"),
            r#"{"mcpServers":{
                "local":{"command":"${CLAUDE_PLUGIN_ROOT}/bin/srv","args":["--data","${CLAUDE_PLUGIN_DATA}","${API_KEY}"]},
                "remote":{"type":"http","url":"https://mcp.slack.com/mcp"},
                "opts":{"command":"x","env":{"K":"${user_config.api_key}"}}
            }}"#,
        );
        let data = t.path().join("data");
        let ws = t.path().join("ws");
        std::fs::create_dir_all(&ws).unwrap();
        let p = plan(
            Some(&[listed("slack@official", &root)]),
            true,
            None,
            &data,
            &ws,
        );
        let entry = &p.local["plugin_slack_local"];
        assert_eq!(entry["command"], format!("{}/bin/srv", root.display()));
        let data_dir = data.join("slack-official").display().to_string();
        assert_eq!(entry["args"][1], data_dir);
        assert_eq!(
            entry["args"][2], "${API_KEY}",
            "left for Claude Code to expand"
        );
        assert_eq!(entry["env"]["CLAUDE_PLUGIN_DATA"], data_dir);
        assert_eq!(
            entry["env"]["CLAUDE_PLUGIN_ROOT"],
            root.display().to_string()
        );
        assert_eq!(entry["type"], "stdio");
        assert_eq!(p.local.len(), 1);
        let reasons: Vec<_> = p
            .skipped
            .iter()
            .map(|s| (s.name.as_str(), s.reason))
            .collect();
        assert!(reasons.contains(&("remote", REMOTE)));
        assert!(reasons.contains(&("opts", UNRESOLVED)));
        assert_eq!(p.remote.len(), 1);
        assert_eq!(p.remote[0]["agent"], "claude-code");
        assert_eq!(p.remote[0]["plugin"], "slack");
        assert_eq!(p.remote[0]["server"], "remote");
        assert!(
            p.notice()
                .unwrap()
                .contains("remote (plugin slack) — remote:")
        );
    }

    /// 131g: a remote entry carries the URL as declared and at most a
    /// header's name — never a header value, an env value or a client id.
    #[test]
    fn a_remote_entry_carries_no_secret() {
        let canary = "CANARY-131G";
        let entry = |v: Value| v.as_object().unwrap().clone();
        let header = remote_entry(
            "claude-code",
            "gh",
            Some("1.2.0"),
            "github",
            &entry(json!({"type": "http", "url": "https://api.example.com/mcp",
                "headers": {"X-Static": canary, "Authorization": "Bearer ${GH_TOKEN}"},
                "env": {"K": canary}})),
        )
        .unwrap();
        assert_eq!(header["auth_hint"], "header:Authorization");
        assert_eq!(header["plugin_version"], "1.2.0");
        assert_eq!(header["templated"], false);
        let vendor = remote_entry(
            "codex",
            "slack",
            None,
            "slack",
            &entry(json!({"type": "sse", "url": "${SLACK_URL}/mcp",
                "oauth": {"clientId": canary, "callbackPort": 3118}})),
        )
        .unwrap();
        assert_eq!(vendor["auth_hint"], "vendor_client");
        assert_eq!(vendor["transport"], "sse");
        assert_eq!(vendor["templated"], true);
        for e in [&header, &vendor] {
            assert!(!e.to_string().contains(canary), "{e}");
            assert!(!e.to_string().contains("Bearer"), "{e}");
        }
        assert!(remote_entry("x", "p", None, "s", &entry(json!({"command": "srv"}))).is_none());
    }

    #[test]
    fn the_off_switch_names_local_servers_instead() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("pw");
        write(
            &root.join(".mcp.json"),
            r#"{"playwright":{"command":"npx"}}"#,
        );
        let p = plan(
            Some(&[listed("playwright@m", &root)]),
            false,
            None,
            t.path(),
            t.path(),
        );
        assert!(p.local.is_empty());
        assert_eq!(p.skipped[0].reason, OFF);
        assert_eq!(p.skipped[0].name, "playwright");
    }

    #[test]
    fn a_manifest_may_declare_servers_inline_or_by_path() {
        let t = tempfile::tempdir().unwrap();
        let a = t.path().join("a");
        write(
            &a.join(".claude-plugin/plugin.json"),
            r#"{"name":"a","mcpServers":{"s1":{"command":"one"}}}"#,
        );
        let b = t.path().join("b");
        write(
            &b.join(".claude-plugin/plugin.json"),
            r#"{"name":"b","mcpServers":["./conf/mcp.json"]}"#,
        );
        write(
            &b.join("conf/mcp.json"),
            r#"{"mcpServers":{"s2":{"command":"two"}}}"#,
        );
        let c = t.path().join("c");
        write(
            &c.join(".claude-plugin/plugin.json"),
            r#"{"name":"c","mcpServers":"../../etc/x.json"}"#,
        );
        let p = plan(
            Some(&[listed("a@m", &a), listed("b@m", &b), listed("c@m", &c)]),
            true,
            None,
            t.path(),
            t.path(),
        );
        assert_eq!(p.local["plugin_a_s1"]["command"], "one");
        assert_eq!(p.local["plugin_b_s2"]["command"], "two");
        assert_eq!(
            p.skipped,
            [Skipped {
                name: "c".into(),
                source: "plugin c".into(),
                reason: UNREADABLE
            }]
        );
    }

    /// The account's and the workspace's servers are named, and nothing but
    /// their names is read out (a planted canary never appears).
    #[test]
    fn own_and_project_servers_are_named_by_name_only() {
        let t = tempfile::tempdir().unwrap();
        let ws = t.path().join("ws");
        let canary = "CANARY-9f2c";
        write(
            &ws.join(".mcp.json"),
            &format!(r#"{{"mcpServers":{{"repo_srv":{{"command":"{canary}"}}}}}}"#),
        );
        let claude_json = t.path().join(".claude.json");
        // Built with `json!`: a Windows path in a hand-written string would
        // be an invalid JSON escape.
        let mut projects = Map::new();
        projects.insert(
            ws.display().to_string(),
            json!({"mcpServers": {"here": {"command": canary}}}),
        );
        write(
            &claude_json,
            &json!({
                "mcpServers": {"proxmox": {
                    "type": "http",
                    "url": format!("https://x/{canary}"),
                    "headers": {"Authorization": format!("Bearer {canary}")},
                }},
                "projects": projects,
            })
            .to_string(),
        );
        let p = plan(Some(&[]), true, Some(&claude_json), t.path(), &ws);
        let names: Vec<_> = p
            .skipped
            .iter()
            .map(|s| (s.name.as_str(), s.reason))
            .collect();
        assert_eq!(
            names,
            [("proxmox", OWN), ("here", OWN), ("repo_srv", PROJECT)]
        );
        let all = format!("{} {}", p.report(), p.notice().unwrap());
        assert!(!all.contains(canary), "{all}");
    }

    #[test]
    fn a_failed_listing_is_named_and_nothing_merged() {
        let t = tempfile::tempdir().unwrap();
        let p = plan(None, true, None, t.path(), t.path());
        assert!(p.local.is_empty());
        assert_eq!(p.skipped[0].reason, UNLISTED);
        assert!(
            plan(Some(&[]), true, None, t.path(), t.path())
                .notice()
                .is_none()
        );
    }

    /// Gateway slugs are `[a-z0-9-]`, so a plugin key — always with `_` — can
    /// never take one's place.
    #[test]
    fn keys_are_claude_codes_tool_prefix_and_never_a_slug() {
        assert_eq!(server_key("probe", "localsrv"), "plugin_probe_localsrv");
        assert_eq!(server_key("my.plugin", "a:b"), "plugin_my_plugin_a_b");
        assert!(server_key("x", "y").contains('_'));
        assert_eq!(
            data_dir_name("slack@claude-plugins-official"),
            "slack-claude-plugins-official"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_slow_or_failing_listing_is_an_error_not_a_hang() {
        let t = tempfile::tempdir().unwrap();
        let mut ok = Command::new("sh");
        ok.args([
            "-c",
            r#"echo '[{"id":"a@m","enabled":true,"installPath":"/p"}]'; :"#,
            "sh",
        ]);
        assert_eq!(list(ok, t.path()).unwrap().len(), 1);
        let mut bad = Command::new("sh");
        bad.args(["-c", "exit 3", "sh"]);
        assert!(list(bad, t.path()).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn nothing_the_listing_started_outlives_it() {
        let t = tempfile::tempdir().unwrap();
        let alive = |pid: i32| {
            rustix::process::test_kill_process(rustix::process::Pid::from_raw(pid).unwrap()).is_ok()
        };
        let gone_soon = |pid: i32| {
            let until = Instant::now() + Duration::from_secs(5);
            while alive(pid) && Instant::now() < until {
                std::thread::sleep(Duration::from_millis(50));
            }
            !alive(pid)
        };

        // It exits, leaving a child that ignores HUP and holds stdout.
        let pidfile = t.path().join("left");
        let mut leaves = Command::new("sh");
        leaves.args([
            "-c",
            r#"(trap '' HUP; exec sleep 600) & echo $! > "$1"; echo '[]'"#,
            "sh",
            pidfile.to_str().unwrap(),
        ]);
        let dir = t.path().to_path_buf();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || tx.send(list(leaves, &dir).map(|l| l.len())));
        let got = rx
            .recv_timeout(LIST_DEADLINE)
            .expect("the listing waited on its child");
        assert_eq!(got.unwrap(), 0);
        let left: i32 = std::fs::read_to_string(&pidfile)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert!(gone_soon(left), "pid {left} outlived the listing");

        // It hangs past the deadline with a child of its own.
        let pidfile = t.path().join("hung");
        let mut hangs = Command::new("sh");
        hangs.args([
            "-c",
            r#"(trap '' HUP; exec sleep 600) & echo $! > "$1"; exec sleep 600"#,
            "sh",
            pidfile.to_str().unwrap(),
        ]);
        assert!(list(hangs, t.path()).is_err());
        let hung: i32 = std::fs::read_to_string(&pidfile)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert!(gone_soon(hung), "pid {hung} outlived the deadline");
    }
}
