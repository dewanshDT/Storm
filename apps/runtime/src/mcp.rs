//! The host's half of the MCP Gateway (spec §11; `PLAN.md` decision 81f).
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
//! - the **daemon socket** (`run/mcp.sock`, in a `0700` directory): one line
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
}

pub use crate::provider::LaunchExtras;

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
fn slug_ok(slug: &str) -> bool {
    (1..=32).contains(&slug.len())
        && slug
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// Creates a directory `0700`, created with that mode (the A2 pattern).
pub fn private_dir(dir: &Path) -> Result<()> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    if !dir.exists() {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)
            .with_context(|| format!("creating {}", dir.display()))?;
    }
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
        .with_context(|| format!("tightening {}", dir.display()))?;
    Ok(())
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
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

/// Writes the provider's MCP config into `dir` and returns what its launch
/// gains (AM32, exactly):
/// - **`claude-code`**: `--mcp-config <dir>/mcp.json --strict-mcp-config`,
///   one stdio server per slug. Only the session's servers load.
/// - **`opencode`**: `XDG_CONFIG_HOME=<dir>/xdg` with the session config in
///   `<dir>/xdg/opencode/opencode.json`, `OPENCODE_DISABLE_PROJECT_CONFIG=1`,
///   and `permission: {"<slug>_*": "ask"}` for every connection. All three are
///   required: OpenCode merges every other config source with the global one,
///   and runs MCP tools without asking by default (G2).
/// - anything else, `shell` included: nothing (G-D9).
///
/// `opencode_settings` is the `runtime.toml` provider entry's `settings`:
/// merged in, minus any `mcp` key — a setting can never add an MCP server.
pub fn write_provider_config(
    provider: &str,
    dir: &Path,
    grants: &[McpGrant],
    handle: &str,
    socket: &Path,
    exe: &Path,
    opencode_settings: Option<&Value>,
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
                    g.slug.clone(),
                    json!({
                        "type": "stdio",
                        "command": command[0],
                        "args": command[1..],
                        "env": env,
                    }),
                );
            }
            private_dir(dir)?;
            let path = dir.join("mcp.json");
            write_private(
                &path,
                &serde_json::to_vec_pretty(&json!({ "mcpServers": servers }))?,
            )?;
            Ok(LaunchExtras {
                args: vec![
                    "--mcp-config".into(),
                    path.into_os_string(),
                    "--strict-mcp-config".into(),
                ],
                env: Vec::new(),
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
                env: vec![
                    ("XDG_CONFIG_HOME".into(), xdg.into_os_string()),
                    ("OPENCODE_DISABLE_PROJECT_CONFIG".into(), "1".into()),
                ],
            })
        }
        _ => Ok(LaunchExtras::default()),
    }
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
    #[serde(default)]
    pub message: Option<Value>,
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
            },
            McpGrant {
                id: "mcc_GH".into(),
                slug: "github".into(),
            },
        ]
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
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&session).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[test]
    fn opencode_gets_its_own_config_home_no_project_config_and_ask_for_every_slug() {
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
        )
        .unwrap();
        assert!(extras.args.is_empty());
        assert_eq!(
            extras.env,
            vec![
                (
                    OsString::from("XDG_CONFIG_HOME"),
                    session.join("xdg").into_os_string()
                ),
                (
                    OsString::from("OPENCODE_DISABLE_PROJECT_CONFIG"),
                    OsString::from("1")
                ),
            ]
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
        )
        .unwrap();
        assert_eq!(extras, LaunchExtras::default());
        assert!(!session.exists());
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
