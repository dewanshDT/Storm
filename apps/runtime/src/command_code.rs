//! Command Code's session-only MCP servers (decision 131k).
//!
//! Command Code reads, in order, its settings' `mcp.servers`,
//! `~/.commandcode/mcp.json`, `~/.commandcode/.mcp.json`, the repository's
//! `.mcp.json` (with no approval), and last the workspace's local file
//! `~/.commandcode/projects/<slug>/mcp.json` — each later one replacing an
//! earlier server of the same name. So Storm writes that local file: the
//! session's bridges, and a disabled stub (`enabled: false`) under every
//! other server's name, which Command Code never starts. A stub carries no
//! part of the server it replaces.
//!
//! Without `agent_mcp = "storm-only"` (decision 132d) there are no stubs:
//! the account's entries in the local file are kept as they are, and a
//! bridge whose slug any source already uses is named `storm_<slug>`.
//!
//! The local file is per workspace, not per session, so a bridge entry holds
//! no handle: `STORM_MCP_HANDLE` and `STORM_MCP_SOCK` go in the session's
//! environment, which Command Code passes to every stdio server, and grants
//! are checked on the server per call. A person can re-enable a server from
//! inside the session; Storm disables it again at the next launch (accepted
//! risk 10). Command Code's profile keeps `mcp: None` until a real-account
//! gate (G-CC1) has shown all of this in a live session.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::{Map, Value, json};

use crate::mcp::LaunchExtras;
use crate::mcp::McpGrant;
use crate::plugins::{Plan, Skipped};

/// Writes the workspace's local file for a session of `grants`; `home` is
/// the account's home.
pub fn write_session(
    workspace: &Path,
    home: &Path,
    grants: &[&McpGrant],
    handle: &str,
    socket: &Path,
    exe: &Path,
    storm_only: bool,
) -> Result<(LaunchExtras, Plan)> {
    let dir = home.join(".commandcode");
    let local = local_file(home, workspace)?;
    let mut plan = Plan::default();
    let mut servers = Map::new();
    let mut taken = std::collections::HashSet::new();
    let previous = crate::plugins::read_json(&local);
    if !storm_only {
        // The account's own local servers, kept as they are; Storm's
        // bridges and stubs from an earlier launch are not.
        for (name, entry) in previous["mcpServers"].as_object().into_iter().flatten() {
            if !is_bridge(entry, exe) && entry["command"] != "storm-disabled" {
                servers.insert(name.clone(), entry.clone());
            }
        }
    }
    let mut stub = |name: String, source: &str, reason: &'static str, plan: &mut Plan| {
        taken.insert(name.clone());
        if !storm_only {
            return;
        }
        if grants.iter().any(|g| g.slug == name) || servers.contains_key(&name) {
            return;
        }
        servers.insert(
            name.clone(),
            json!({"command": "storm-disabled", "enabled": false}),
        );
        plan.skipped.push(Skipped {
            name,
            source: source.into(),
            reason,
        });
    };
    let settings = crate::plugins::read_json(&dir.join("settings.json"));
    if let Some(list) = settings["mcp"]["servers"].as_array() {
        for name in list.iter().filter_map(|s| s["name"].as_str()) {
            stub(name.into(), "your own", crate::plugins::OWN, &mut plan);
        }
    }
    for file in ["mcp.json", ".mcp.json"] {
        for name in crate::plugins::server_names(&crate::plugins::read_json(&dir.join(file))) {
            stub(name, "your own", crate::plugins::OWN, &mut plan);
        }
    }
    for name in
        crate::plugins::server_names(&crate::plugins::read_json(&workspace.join(".mcp.json")))
    {
        stub(
            name,
            "this workspace's .mcp.json",
            crate::plugins::PROJECT,
            &mut plan,
        );
    }
    // The account's own local servers, but not the bridges Storm wrote here
    // for an earlier session.
    for (name, entry) in previous["mcpServers"].as_object().into_iter().flatten() {
        if !is_bridge(entry, exe) && entry["command"] != "storm-disabled" {
            stub(name.clone(), "your own", crate::plugins::OWN, &mut plan);
        }
    }
    for g in grants {
        let key = if !storm_only && taken.contains(&g.slug) {
            format!("storm_{}", g.slug)
        } else {
            g.slug.clone()
        };
        servers.insert(
            key,
            json!({"command": exe.display().to_string(), "args": ["mcp-bridge", g.slug]}),
        );
    }
    std::fs::create_dir_all(local.parent().context("a parent")?)?;
    crate::fsutil::write_atomic(
        &local,
        &serde_json::to_vec_pretty(&json!({ "mcpServers": servers }))?,
        crate::fsutil::Access::Private,
    )
    .with_context(|| format!("writing {}", local.display()))?;
    Ok((
        LaunchExtras {
            env: vec![
                (crate::mcp::HANDLE_ENV.into(), handle.into()),
                (crate::mcp::SOCKET_ENV.into(), socket.as_os_str().to_owned()),
            ],
            ..Default::default()
        },
        plan,
    ))
}

fn is_bridge(entry: &Value, exe: &Path) -> bool {
    entry["command"].as_str() == Some(&exe.display().to_string())
        && entry["args"][0].as_str() == Some("mcp-bridge")
}

/// `~/.commandcode/projects/<slug>/mcp.json`.
fn local_file(home: &Path, workspace: &Path) -> Result<PathBuf> {
    let slug = slug(&workspace.display().to_string())
        .with_context(|| format!("Command Code's name for {}", workspace.display()))?;
    Ok(home
        .join(".commandcode")
        .join("projects")
        .join(slug)
        .join("mcp.json"))
}

/// Command Code's name for a workspace: `@sindresorhus/slugify` 2.2.1 of the
/// path, as 1.86.3 calls it. Its transliteration of non-ASCII text is not
/// reproduced, so such a path has no name here and the session fails to
/// start rather than leave the repository's servers on.
fn slug(path: &str) -> Option<String> {
    if !path.is_ascii() {
        return None;
    }
    let s: Vec<u8> = path.replace('&', " and ").into_bytes();
    let up = |c: u8| c.is_ascii_uppercase();
    let low_digit = |c: u8| c.is_ascii_lowercase() || c.is_ascii_digit();
    // slugify's `decamelize`, its four replacements in order.
    let s = split(&s, |s, i| {
        i >= 2 && s[i].is_ascii_digit() && up(s[i - 1]) && up(s[i - 2])
    });
    let s = split(&s, |s, i| {
        i >= 1 && low_digit(s[i - 1]) && up(s[i]) && s.get(i + 1).is_some_and(|&c| up(c))
    });
    let s = split(&s, |s, i| i >= 1 && low_digit(s[i - 1]) && up(s[i]));
    let s = split(&s, |s, i| {
        i >= 1
            && up(s[i - 1])
            && up(s[i])
            && s.get(i + 1).is_some_and(|&c| (low_digit(c)) && c != b's')
    });
    let s = s.to_ascii_lowercase();
    // A contraction's apostrophe goes: `it's` is `its`.
    let mut kept = Vec::with_capacity(s.len());
    for (i, &c) in s.iter().enumerate() {
        let contraction = c == b'\''
            && i > 0
            && s[i - 1].is_ascii_alphanumeric()
            && matches!(s.get(i + 1), Some(b't' | b's'))
            && s.get(i + 2).is_none_or(|c| c.is_ascii_whitespace());
        if !contraction {
            kept.push(c);
        }
    }
    let mut out = String::new();
    for c in kept {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            out.push(c as char);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_matches('-');
    (!out.is_empty()).then(|| out.to_string())
}

/// A space before every byte `at` picks, judged on the input.
fn split(s: &[u8], at: impl Fn(&[u8], usize) -> bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len() + 8);
    for i in 0..s.len() {
        if at(s, i) {
            out.push(b' ');
        }
        out.push(s[i]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grant(slug: &str) -> McpGrant {
        McpGrant {
            id: format!("mcc_{slug}"),
            slug: slug.into(),
            plugin_origin: None,
        }
    }

    #[test]
    fn the_slug_is_command_codes() {
        for (path, want) in [
            (
                "/tmp/claude-960/-var-lib/my_proj.v2 x",
                "tmp-claude-960-var-lib-my-proj-v2-x",
            ),
            ("/tmp/.tmpAbC/My Repo.v2", "tmp-tmp-ab-c-my-repo-v2"),
            (
                "/a/XMLHttpRequest/APIs/ABC123/fooBAR/it's x/a&b/x\\y/_u",
                "a-xml-http-request-apis-abc-123-foo-bar-its-x-a-and-b-x-y-u",
            ),
            ("/x/aBC/AB/v2B/AbcDEFghi", "x-a-bc-ab-v2-b-abc-de-fghi"),
        ] {
            assert_eq!(slug(path).as_deref(), Some(want), "{path}");
        }
        assert_eq!(slug("/home/Ünï"), None, "never a guess");
        assert_eq!(slug("/"), None);
    }

    /// 131k: bridges with no handle, a stub under every other name (never a
    /// copy of it), the handle in the environment; an earlier session's
    /// bridges are replaced.
    #[test]
    fn the_local_file_holds_bridges_and_disabled_stubs() {
        let t = tempfile::tempdir().unwrap();
        let ws = t.path().join("ws");
        std::fs::create_dir_all(&ws).unwrap();
        let canary = "CANARY-77";
        std::fs::write(
            ws.join(".mcp.json"),
            format!(r#"{{"mcpServers":{{"evil":{{"command":"{canary}"}}}}}}"#),
        )
        .unwrap();
        let home = t.path().join("home");
        std::fs::create_dir_all(home.join(".commandcode")).unwrap();
        std::fs::write(
            home.join(".commandcode/mcp.json"),
            r#"{"mcpServers":{"mine":{"command":"z"}}}"#,
        )
        .unwrap();
        std::fs::write(
            home.join(".commandcode/settings.json"),
            r#"{"mcp":{"servers":[{"name":"set","command":"q"}]}}"#,
        )
        .unwrap();
        let local = local_file(&home, &ws).unwrap();
        std::fs::create_dir_all(local.parent().unwrap()).unwrap();
        std::fs::write(
            &local,
            r#"{"mcpServers":{"old":{"command":"/x","args":["mcp-bridge","old"]},"loc":{"command":"l"}}}"#,
        )
        .unwrap();
        let storm = grant("storm");
        let (extras, plan) = write_session(
            &ws,
            &home,
            &[&storm],
            "h",
            Path::new("/s"),
            Path::new("/x"),
            true,
        )
        .unwrap();
        let v: Value = serde_json::from_slice(&std::fs::read(&local).unwrap()).unwrap();
        let s = v["mcpServers"].as_object().unwrap();
        assert_eq!(s["storm"]["args"][1], "storm");
        assert!(
            s["storm"].get("env").is_none(),
            "no handle in a shared file"
        );
        for name in ["evil", "mine", "set", "loc"] {
            assert_eq!(s[name]["enabled"], false, "{name}");
        }
        assert!(!s.contains_key("old"), "an earlier session's bridge goes");
        assert!(!v.to_string().contains(canary), "a stub copies nothing");
        assert!(crate::platform::is_private(&local).unwrap());
        assert!(
            extras
                .env
                .contains(&(crate::mcp::HANDLE_ENV.into(), "h".into()))
        );
        let names: Vec<_> = plan.skipped.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["set", "mine", "evil", "loc"]);
    }

    /// Decision 132d: without `storm-only`, the account's local entries are
    /// kept as they are, Storm's earlier stubs and bridges go, and a bridge
    /// whose name a source uses steps aside.
    #[test]
    fn without_storm_only_the_accounts_servers_are_kept() {
        let t = tempfile::tempdir().unwrap();
        let home = t.path().join("home");
        let ws = t.path().join("ws");
        std::fs::create_dir_all(&ws).unwrap();
        std::fs::write(
            ws.join(".mcp.json"),
            r#"{"mcpServers":{"storm":{"command":"p"}}}"#,
        )
        .unwrap();
        let local = local_file(&home, &ws).unwrap();
        std::fs::create_dir_all(local.parent().unwrap()).unwrap();
        std::fs::write(
            &local,
            r#"{"mcpServers":{"loc":{"command":"l","args":["a"]},"gone":{"command":"storm-disabled","enabled":false},"old":{"command":"/x","args":["mcp-bridge","old"]}}}"#,
        )
        .unwrap();
        let storm = grant("storm");
        let (_, plan) = write_session(
            &ws,
            &home,
            &[&storm],
            "h",
            Path::new("/s"),
            Path::new("/x"),
            false,
        )
        .unwrap();
        assert!(plan.skipped.is_empty());
        let v: Value = serde_json::from_slice(&std::fs::read(&local).unwrap()).unwrap();
        let s = v["mcpServers"].as_object().unwrap();
        assert_eq!(
            s["loc"],
            json!({"command": "l", "args": ["a"]}),
            "kept as it was"
        );
        assert!(!s.contains_key("gone") && !s.contains_key("old"));
        assert!(
            !s.contains_key("storm"),
            "the project's server is not shadowed"
        );
        assert_eq!(s["storm_storm"]["args"][1], "storm");
    }
}
