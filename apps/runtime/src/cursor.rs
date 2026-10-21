//! Cursor's session-only MCP servers (decision 131j).
//!
//! Cursor has no way to load only a given set of servers, but two levers:
//!
//! - a **plugin directory** passed with `--plugin-dir`, whose servers load
//!   with no approval prompt (only a repository's need one): the session's
//!   bridges go there;
//! - its own **disabled list**, `mcp-disabled.json` in the project's data
//!   directory (what `cursor-agent mcp disable` writes, a JSON array of server
//!   names): every server from the repository's `.cursor/mcp.json` and the
//!   account's `~/.cursor/mcp.json` goes there, so none of them loads.
//!
//! A person can re-enable a disabled server from inside the session; Storm
//! disables it again at the next launch (accepted risk 10). Cursor's profile
//! keeps `mcp: None` until a real-account gate (G-C1) has shown all of this
//! in a live session.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::{Map, json};

use crate::mcp::LaunchExtras;
use crate::mcp::{McpGrant, private_dir, write_private};
use crate::plugins::{Plan, Skipped};

/// Writes `grants` into a plugin in `dir` and disables every other server
/// Cursor would load for `workspace`. `home` is the account's home and
/// `data_dir` Cursor's (`CURSOR_DATA_DIR`, else `~/.cursor`).
#[allow(clippy::too_many_arguments)]
pub fn write_session(
    dir: &Path,
    workspace: &Path,
    home: &Path,
    data_dir: &Path,
    grants: &[&McpGrant],
    handle: &str,
    socket: &Path,
    exe: &Path,
) -> Result<(LaunchExtras, Plan)> {
    let plugin = dir.join("cursor-plugin");
    private_dir(dir)?;
    private_dir(&plugin)?;
    private_dir(&plugin.join(".cursor-plugin"))?;
    write_private(
        &plugin.join(".cursor-plugin").join("plugin.json"),
        json!({"name": "storm-session", "version": "1.0.0"})
            .to_string()
            .as_bytes(),
    )?;
    let mut servers = Map::new();
    for g in grants {
        servers.insert(
            g.slug.clone(),
            json!({
                "command": exe.display().to_string(),
                "args": ["mcp-bridge", g.slug],
                "env": {
                    crate::mcp::HANDLE_ENV: handle,
                    crate::mcp::SOCKET_ENV: socket.display().to_string(),
                },
            }),
        );
    }
    write_private(
        &plugin.join(".mcp.json"),
        json!({ "mcpServers": servers }).to_string().as_bytes(),
    )?;

    let mut plan = Plan::default();
    let root = project_root(workspace);
    let mut off: Vec<String> = Vec::new();
    for (path, source, reason) in [
        (
            root.join(".cursor").join("mcp.json"),
            "this workspace's .cursor/mcp.json",
            crate::plugins::PROJECT,
        ),
        (
            home.join(".cursor").join("mcp.json"),
            "your own",
            crate::plugins::OWN,
        ),
    ] {
        for name in crate::plugins::server_names(&crate::plugins::read_json(&path)) {
            plan.skipped.push(Skipped {
                name: name.clone(),
                source: source.into(),
                reason,
            });
            off.push(name);
        }
    }
    let list = data_dir
        .join("projects")
        .join(slug(&root.display().to_string()))
        .join("mcp-disabled.json");
    let mut disabled: Vec<String> = std::fs::read(&list)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    for name in off {
        if !disabled.contains(&name) {
            disabled.push(name);
        }
    }
    // A bridge is never on the list, whatever an earlier launch left there.
    disabled.retain(|n| !grants.iter().any(|g| &g.slug == n));
    std::fs::create_dir_all(list.parent().context("a parent")?)?;
    std::fs::write(&list, serde_json::to_vec_pretty(&disabled)?)
        .with_context(|| format!("writing {}", list.display()))?;

    Ok((
        LaunchExtras {
            args: vec!["--plugin-dir".into(), plugin.into_os_string()],
            ..Default::default()
        },
        plan,
    ))
}

/// Cursor's project: the workspace's git root, else the workspace.
fn project_root(workspace: &Path) -> PathBuf {
    workspace
        .ancestors()
        .find(|d| d.join(".git").exists())
        .unwrap_or(workspace)
        .to_path_buf()
}

/// Cursor's name for a project's data directory: every run of characters
/// outside `[A-Za-z0-9]` is one `-`, none at either end.
fn slug(path: &str) -> String {
    let mut out = String::new();
    for c in path.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').to_string()
}

/// Cursor's data directory: `CURSOR_DATA_DIR`, else `~/.cursor`.
pub fn data_dir(home: &Path) -> PathBuf {
    match std::env::var_os("CURSOR_DATA_DIR").filter(|v| !v.is_empty()) {
        Some(dir) => PathBuf::from(dir),
        None => home.join(".cursor"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn grant(slug: &str) -> McpGrant {
        McpGrant {
            id: format!("mcc_{slug}"),
            slug: slug.into(),
            plugin_origin: None,
        }
    }

    #[test]
    fn the_slug_is_cursors() {
        assert_eq!(
            slug("/tmp/claude-960/-var-lib/My_Proj.v2 x"),
            "tmp-claude-960-var-lib-My-Proj-v2-x"
        );
    }

    /// 131j: the bridges are a session plugin, and every repository and
    /// account server is on Cursor's own disabled list — never a bridge.
    #[test]
    fn bridges_go_in_a_plugin_and_everything_else_is_disabled() {
        let t = tempfile::tempdir().unwrap();
        let ws = t.path().join("repo/sub");
        std::fs::create_dir_all(t.path().join("repo/.git")).unwrap();
        std::fs::create_dir_all(t.path().join("repo/.cursor")).unwrap();
        std::fs::create_dir_all(&ws).unwrap();
        std::fs::write(
            t.path().join("repo/.cursor/mcp.json"),
            r#"{"mcpServers":{"evil":{"command":"x"},"storm":{"command":"y"}}}"#,
        )
        .unwrap();
        let home = t.path().join("home");
        std::fs::create_dir_all(home.join(".cursor")).unwrap();
        std::fs::write(
            home.join(".cursor/mcp.json"),
            r#"{"mcpServers":{"mine":{"command":"z"}}}"#,
        )
        .unwrap();
        let data = home.join(".cursor");
        let list = data
            .join("projects")
            .join(slug(&t.path().join("repo").display().to_string()))
            .join("mcp-disabled.json");
        std::fs::create_dir_all(list.parent().unwrap()).unwrap();
        std::fs::write(&list, r#"["kept"]"#).unwrap();
        let session = t.path().join("ags_C");
        let storm = grant("storm");
        let (extras, plan) = write_session(
            &session,
            &ws,
            &home,
            &data,
            &[&storm],
            "h",
            Path::new("/s"),
            Path::new("/x"),
        )
        .unwrap();
        assert_eq!(extras.args[0], "--plugin-dir");
        let plugin = PathBuf::from(&extras.args[1]);
        let mcp: Value =
            serde_json::from_slice(&std::fs::read(plugin.join(".mcp.json")).unwrap()).unwrap();
        assert_eq!(mcp["mcpServers"]["storm"]["args"][1], "storm");
        assert_eq!(
            mcp["mcpServers"]["storm"]["env"][crate::mcp::HANDLE_ENV],
            "h"
        );
        assert!(crate::platform::is_private(&plugin.join(".mcp.json")).unwrap());
        let disabled: Vec<String> = serde_json::from_slice(&std::fs::read(&list).unwrap()).unwrap();
        assert_eq!(disabled, ["kept", "evil", "mine"], "never the storm bridge");
        let names: Vec<_> = plan.skipped.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["evil", "storm", "mine"]);
    }
}
