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
use serde_json::{Value, json};

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
fn claude_key(g: &McpGrant, written: &serde_json::Map<String, Value>, claude: &Claude) -> String {
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
    // project's of the same name and drops theirs (gate G-132a), so Storm's
    // steps aside instead (decision 132) — except the session's own vault
    // server, which the opening prompt names.
    if g.id != STORM_CONNECTION && claude.taken.contains(&g.slug) {
        format!("storm_{}", g.slug)
    } else {
        g.slug.clone()
    }
}

/// How a Claude Code session's `mcp.json` sits beside the agent's own
/// servers (decision 132).
pub struct Claude<'a> {
    /// `--strict-mcp-config`: Storm's servers alone (`agent_mcp =
    /// "storm-only"`).
    pub strict: bool,
    /// Names the account's and the workspace's configs already use.
    pub taken: &'a std::collections::HashSet<String>,
}

impl Claude<'_> {
    pub fn storm_only() -> Claude<'static> {
        static NONE: std::sync::LazyLock<std::collections::HashSet<String>> =
            std::sync::LazyLock::new(Default::default);
        Claude {
            strict: true,
            taken: &NONE,
        }
    }
}

/// The built-in connection to the session's own Storm (the gateway's
/// Phase 1 vault server).
pub const STORM_CONNECTION: &str = "storm";

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
///   [`Claude::strict`], `--strict-mcp-config` too, and only the session's
///   load.
/// - **`opencode`**: `XDG_CONFIG_HOME=<dir>/xdg` with the session config in
///   `<dir>/xdg/opencode/opencode.json`, and `permission: {"<slug>_*": "ask"}`
///   for every connection. Both are required, with
///   [`provider_baseline`]'s `OPENCODE_DISABLE_PROJECT_CONFIG=1`: OpenCode
///   merges every other config source with the global one, and runs MCP
///   tools without asking by default (G2).
/// - anything else, `shell` included: nothing (G-D9).
///
/// `opencode_settings` is the `runtime.toml` provider entry's `settings`:
/// merged in, minus any `mcp` key — a setting can never add an MCP server.
/// `plugin_servers` are a Claude Code session's plugins' local servers
/// (decision 131c, `plugins.rs`), keyed `plugin_…`, which no slug can be.
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
    claude: &Claude,
) -> Result<LaunchExtras> {
    let grants: Vec<&McpGrant> = grants.iter().filter(|g| slug_ok(&g.slug)).collect();
    if grants.is_empty() {
        return Ok(LaunchExtras::default());
    }
    match provider {
        "claude-code" => {
            let mut servers = serde_json::Map::new();
            for g in &grants {
                let (command, env) = bridge_entry(exe, &g.slug, handle, socket);
                servers.insert(
                    claude_key(g, &servers, claude),
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
            if claude.strict {
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
                let (command, env) = bridge_entry(exe, &g.slug, handle, socket);
                mcp.insert(
                    g.slug.clone(),
                    json!({ "type": "local", "command": command, "environment": env, "enabled": true }),
                );
                // Last, so no setting can relax it.
                permission.insert(format!("{}_*", g.slug), json!("ask"));
            }
            config.insert("$schema".into(), json!("https://opencode.ai/config.json"));
            // Replaces any `mcp` key the settings had: a host setting can
            // never add an MCP server.
            config.insert("mcp".into(), Value::Object(mcp));
            config.insert("permission".into(), Value::Object(permission));
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

/// The opening prompt of a session started from a note. A constant with
/// nothing interpolated: the agent reads the note through `session_context`,
/// so no note content, title or id is ever on a command line.
pub const OPENING_PROMPT: &str =
    "Read your context note with the storm session_context tool, then wait for my instructions.";

/// Adds the opening prompt, as each CLI takes one, to a session started from
/// a note. Nothing for any other provider, `shell` and `fake` included.
pub fn with_opening_prompt(
    mut extras: LaunchExtras,
    provider: &str,
    context: bool,
) -> LaunchExtras {
    use crate::profile::Prompt;
    if context {
        match crate::profile::get(provider).and_then(|p| p.prompt) {
            Some(Prompt::Positional) => extras.args.push(OPENING_PROMPT.into()),
            Some(Prompt::Flag(flag)) => extras.args.extend([flag.into(), OPENING_PROMPT.into()]),
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
            &Claude::storm_only(),
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
            &Claude {
                strict: false,
                taken: &taken,
            },
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
            &Claude::storm_only(),
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
            &Claude::storm_only(),
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
            &Claude::storm_only(),
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
                    &Claude::storm_only(),
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
                &Claude::storm_only(),
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
            &Claude::storm_only(),
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
                    &Claude::storm_only(),
                )
                .unwrap()
            };
            let plain = config("plain");
            assert_eq!(with_opening_prompt(config("plain"), provider, false), plain);
            let with = with_opening_prompt(config("context"), provider, true);
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
        // Nothing can ever be interpolated into it.
        for c in ['{', '}', '$', '%', '`', '\n'] {
            assert!(!OPENING_PROMPT.contains(c), "{c:?}");
        }
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
