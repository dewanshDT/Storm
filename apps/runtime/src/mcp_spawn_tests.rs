//! Decision 145a retired 134-8's pinned asks; decision 146a gives back the
//! allow half for the sessions that need it: a child's own Storm tools and a
//! spawning parent's spawn tools are allowed outright. A session that cannot
//! spawn and is not a child gets a config byte-identical to before.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::*;

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

fn settings() -> Value {
    json!({
        "permission": {"*": "allow", "bash": "ask"},
        "agent": {"build": {"permission": {"*": "allow"}}},
    })
}

/// What a session's OpenCode launch gets: `OPENCODE_CONFIG_CONTENT`, or under
/// `storm-only` its own config file.
fn opencode(dir: &Path, grants: &[McpGrant], settings: Option<&Value>, storm_only: bool) -> String {
    let none = Default::default();
    let own = if storm_only {
        AgentServers::storm_only()
    } else {
        AgentServers {
            storm_only: false,
            taken: &none,
        }
    };
    let extras = write_provider_config(
        "opencode",
        dir,
        grants,
        "h",
        Path::new("/s"),
        Path::new("/x"),
        settings,
        &Default::default(),
        &own,
        &crate::mcp::StormLayer::default(),
        &|| Ok(String::new()),
    )
    .unwrap();
    if storm_only {
        return std::fs::read_to_string(dir.join("xdg/opencode/opencode.json")).unwrap();
    }
    extras
        .env
        .iter()
        .find(|(k, _)| k == "OPENCODE_CONFIG_CONTENT")
        .map(|(_, v)| v.to_str().unwrap().to_string())
        .unwrap()
}

fn claude_settings(dir: &Path) -> String {
    crate::harness::launch(
        "claude-code",
        dir,
        "h",
        Path::new("/s"),
        Path::new("/x"),
        Some("0b8f1c2e-3d4a-4b5c-8d6e-7f8091a2b3c4"),
        None,
        &crate::harness::Hooks::default(),
        &[],
    )
    .unwrap();
    std::fs::read_to_string(dir.join("claude-settings.json")).unwrap()
}

/// 145a: every config is byte for byte what 134-7 wrote for a session
/// without spawning (its outputs, taken from `feat/134-7-end-cascades`),
/// whatever the session may start: the writers take no spawn input at all.
#[test]
fn every_config_is_byte_identical_to_a_session_without_spawning() {
    let t = tempfile::tempdir().unwrap();
    assert_eq!(
        opencode(&t.path().join("a"), &grants(), Some(&settings()), false),
        r#"{"$schema":"https://opencode.ai/config.json","agent":{"build":{"permission":{"*":"allow"}}},"mcp":{"github":{"command":["/x","mcp-bridge","github"],"enabled":true,"environment":{"STORM_MCP_HANDLE":"h","STORM_MCP_SOCK":"/s"},"type":"local"},"storm":{"command":["/x","mcp-bridge","storm"],"enabled":true,"environment":{"STORM_MCP_HANDLE":"h","STORM_MCP_SOCK":"/s"},"type":"local"}},"permission":{"*":"allow","bash":"ask","github_*":"ask","storm_*":"ask"}}"#
    );
    let xdg = opencode(&t.path().join("b"), &grants(), Some(&settings()), true);
    let golden = "{\n  \"$schema\": \"https://opencode.ai/config.json\",\n  \"agent\": {\n    \"build\": {\n      \"permission\": {\n        \"*\": \"allow\"\n      }\n    }\n  },\n  \"mcp\": {\n    \"github\": {\n      \"command\": [\n        \"/x\",\n        \"mcp-bridge\",\n        \"github\"\n      ],\n      \"enabled\": true,\n      \"environment\": {\n        \"STORM_MCP_HANDLE\": \"h\",\n        \"STORM_MCP_SOCK\": \"/s\"\n      },\n      \"type\": \"local\"\n    },\n    \"storm\": {\n      \"command\": [\n        \"/x\",\n        \"mcp-bridge\",\n        \"storm\"\n      ],\n      \"enabled\": true,\n      \"environment\": {\n        \"STORM_MCP_HANDLE\": \"h\",\n        \"STORM_MCP_SOCK\": \"/s\"\n      },\n      \"type\": \"local\"\n    }\n  },\n  \"permission\": {\n    \"*\": \"allow\",\n    \"bash\": \"ask\",\n    \"github_*\": \"ask\",\n    \"storm_*\": \"ask\"\n  }\n}";
    assert_eq!(xdg, golden);
    assert_eq!(
        claude_settings(&t.path().join("c")),
        r#"{"hooks":{"SessionStart":[{"hooks":[{"command":"'/x' harness-session","type":"command"}]}]}}"#
    );
    let none = Default::default();
    write_provider_config(
        "claude-code",
        &t.path().join("d"),
        &grants(),
        "h",
        Path::new("/s"),
        Path::new("/x"),
        None,
        &Default::default(),
        &AgentServers {
            storm_only: false,
            taken: &none,
        },
        &crate::mcp::StormLayer::default(),
        &|| Ok(String::new()),
    )
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(t.path().join("d/mcp.json")).unwrap(),
        "{\n  \"mcpServers\": {\n    \"github\": {\n      \"args\": [\n        \"mcp-bridge\",\n        \"github\"\n      ],\n      \"command\": \"/x\",\n      \"env\": {\n        \"STORM_MCP_HANDLE\": \"h\",\n        \"STORM_MCP_SOCK\": \"/s\"\n      },\n      \"type\": \"stdio\"\n    },\n    \"storm\": {\n      \"args\": [\n        \"mcp-bridge\",\n        \"storm\"\n      ],\n      \"command\": \"/x\",\n      \"env\": {\n        \"STORM_MCP_HANDLE\": \"h\",\n        \"STORM_MCP_SOCK\": \"/s\"\n      },\n      \"type\": \"stdio\"\n    }\n  }\n}",
        "nothing of spawning is in Claude Code's mcp.json"
    );
}

/// With an empty layer no config names a spawn tool: neither Claude Code's
/// settings nor OpenCode's permissions carry an ask or an allow for one,
/// under any key.
#[test]
fn no_config_names_a_spawn_tool() {
    let t = tempfile::tempdir().unwrap();
    let written = [
        opencode(&t.path().join("a"), &grants(), Some(&settings()), false),
        opencode(&t.path().join("b"), &grants(), Some(&settings()), true),
        claude_settings(&t.path().join("c")),
    ];
    let v: Value = serde_json::from_str(&written[2]).unwrap();
    assert!(v.get("permissions").is_none(), "{v}");
    for tool in spawn_tools() {
        for config in &written {
            assert!(!config.contains(&tool), "{tool} in {config}");
        }
    }
}

/// Every one of the server's spawn tools, read from its source (the host
/// learns no tool names from the server).
fn spawn_tools() -> Vec<String> {
    let src = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../server/src/mcp.rs"),
    )
    .unwrap();
    let list = |name: &str| -> Vec<String> {
        let at = src
            .find(&format!("pub const {name}: &[&str] = &["))
            .unwrap();
        let body = &src[at..at + src[at..].find("];").unwrap()];
        body.split('"')
            .skip(1)
            .step_by(2)
            .map(str::to_string)
            .collect()
    };
    let mut tools = list("SPAWN_TOOLS");
    tools.extend(list("SPAWNED_TOOLS"));
    assert!(tools.contains(&"start_agent".to_string()));
    tools
}

#[test]
fn the_spawned_prompt_names_only_tools_a_child_has() {
    let mut offered = spawn_tools();
    offered.push("session_context".into());
    for word in SPAWNED_OPENING_PROMPT.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')) {
        if word.contains('_') {
            assert!(offered.iter().any(|t| t == word), "{word}");
        }
    }
}

fn opencode_with(dir: &Path, layer: &StormLayer, account: &str, storm_only: bool) -> Value {
    let none = Default::default();
    let own = if storm_only {
        AgentServers::storm_only()
    } else {
        AgentServers {
            storm_only: false,
            taken: &none,
        }
    };
    let account = account.to_string();
    let extras = write_provider_config(
        "opencode",
        dir,
        &grants(),
        "h",
        Path::new("/s"),
        Path::new("/x"),
        Some(&settings()),
        &Default::default(),
        &own,
        layer,
        &move || Ok(account.clone()),
    )
    .unwrap();
    let text = if storm_only {
        std::fs::read_to_string(dir.join("xdg/opencode/opencode.json")).unwrap()
    } else {
        extras
            .env
            .iter()
            .find(|(k, _)| k == "OPENCODE_CONFIG_CONTENT")
            .map(|(_, v)| v.to_str().unwrap().to_string())
            .unwrap()
    };
    serde_json::from_str(&text).unwrap()
}

/// A-146-1: a child's `session_context`, `report_result` and
/// `read_messages`, and a spawning parent's spawn tools, are allowed
/// outright: in Claude Code's `--settings` as `mcp__storm__<tool>`, and in
/// OpenCode's config as allow keys that follow (and sort after) the
/// connection's `storm_*` ask, under a key no account config uses. Nothing
/// else is allowed: note writes keep the agent's own behaviour.
#[test]
fn a_146_1_storms_own_tools_are_allowed_for_a_child_and_a_spawning_parent() {
    let t = tempfile::tempdir().unwrap();
    let child = StormLayer::new(true, false, Vec::new());
    let parent = StormLayer::new(false, true, Vec::new());
    assert_eq!(
        StormLayer::new(false, false, Vec::new()),
        StormLayer::default()
    );

    let allow = child.claude_allow();
    assert_eq!(
        allow,
        [
            "mcp__storm__session_context",
            "mcp__storm__report_result",
            "mcp__storm__read_messages"
        ]
    );
    crate::harness::launch(
        "claude-code",
        &t.path().join("c"),
        "h",
        Path::new("/s"),
        Path::new("/x"),
        None,
        None,
        &crate::harness::Hooks::default(),
        &allow,
    )
    .unwrap();
    let written: Value = serde_json::from_str(
        &std::fs::read_to_string(t.path().join("c/claude-settings.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(written["permissions"], json!({ "allow": allow }));
    assert!(written["permissions"].get("ask").is_none());
    assert!(written["permissions"].get("deny").is_none());
    assert!(
        crate::harness::verify_allow(br#"{"permissions":{"allow":[]}}"#, &allow).is_err(),
        "a file that lost them fails the start"
    );
    assert_eq!(parent.claude_allow().len(), SPAWN_ALLOWED_TOOLS.len());

    for storm_only in [false, true] {
        let config = opencode_with(
            &t.path().join(format!("o{storm_only}")),
            &child,
            "",
            storm_only,
        );
        let permission = config["permission"].as_object().unwrap();
        let keys: Vec<&String> = permission.keys().collect();
        let ask = keys.iter().position(|k| *k == "storm_*").unwrap();
        for tool in CHILD_ALLOWED_TOOLS {
            let key = format!("storm_{tool}");
            assert_eq!(permission[&key], "allow", "{key}");
            assert!(
                keys.iter().position(|k| **k == key).unwrap() > ask,
                "{key} follows the ask"
            );
        }
        assert_eq!(permission.len(), 4 + CHILD_ALLOWED_TOOLS.len());
    }
    let config = opencode_with(&t.path().join("p"), &parent, "", false);
    for tool in SPAWN_ALLOWED_TOOLS {
        assert_eq!(config["permission"][format!("storm_{tool}")], "allow");
    }

    // The account's own key keeps its first position when merged, before
    // Storm's ask: Storm's allow goes under a key it has not used.
    let account =
        r#"{"permission": {"storm_report_result": "ask", "storm_report_result*": "deny"}}"#;
    let config = opencode_with(&t.path().join("a"), &child, account, false);
    assert_eq!(config["permission"]["storm_report_result**"], "allow");
    assert!(config["permission"].get("storm_report_result").is_none());

    for layer in [&child, &parent] {
        let text = opencode_with(&t.path().join("w"), layer, "", false).to_string()
            + &layer.claude_allow().join(",");
        for tool in ["create_note", "update_note", "delete_note", "create_script"] {
            assert!(!text.contains(tool), "{tool}");
        }
    }
}

/// 146a: the host's lists are the server's own: the spawn tools are
/// exactly its `SPAWN_TOOLS`, a child's are its `SPAWNED_TOOLS` and
/// `session_context`.
#[test]
fn the_allowed_tools_are_the_servers() {
    let spawn: Vec<String> = SPAWN_ALLOWED_TOOLS.iter().map(|t| t.to_string()).collect();
    let mut server = spawn_tools();
    let spawned = server.split_off(spawn.len());
    assert_eq!(server, spawn);
    let mut child: Vec<String> = spawned;
    child.insert(0, "session_context".into());
    assert_eq!(child, CHILD_ALLOWED_TOOLS);
}

/// A-146-2: OpenCode's `accept_edits` is `edit: allow` in Storm's layer,
/// and an OpenCode session gets that layer even without grants; Claude
/// Code's mode is a flag, never in its MCP config.
#[test]
fn a_146_2_opencodes_accept_edits_is_storms_edit_allow() {
    let t = tempfile::tempdir().unwrap();
    let layer = StormLayer::new(false, false, vec![("edit".into(), "allow".into())]);
    let config = opencode_with(&t.path().join("a"), &layer, "", false);
    assert_eq!(config["permission"]["edit"], "allow");
    assert_eq!(config["permission"]["storm_*"], "ask");
    let none = Default::default();
    let own = AgentServers {
        storm_only: false,
        taken: &none,
    };
    let extras = write_provider_config(
        "opencode",
        &t.path().join("b"),
        &[],
        "h",
        Path::new("/s"),
        Path::new("/x"),
        None,
        &Default::default(),
        &own,
        &layer,
        &|| Ok(String::new()),
    )
    .unwrap();
    let content = extras
        .env
        .iter()
        .find(|(k, _)| k == "OPENCODE_CONFIG_CONTENT")
        .map(|(_, v)| v.to_str().unwrap().to_string())
        .unwrap();
    let v: Value = serde_json::from_str(&content).unwrap();
    assert_eq!(v["permission"], json!({"edit": "allow"}));
    assert_eq!(v["mcp"], json!({}));
    let claude = write_provider_config(
        "claude-code",
        &t.path().join("c"),
        &[],
        "h",
        Path::new("/s"),
        Path::new("/x"),
        None,
        &Default::default(),
        &own,
        &layer,
        &|| Ok(String::new()),
    )
    .unwrap();
    assert_eq!(claude, LaunchExtras::default());
}
