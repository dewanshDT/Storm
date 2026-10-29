//! Decision 145a: 134-8's pinned asks and A6 exemptions are retired. The
//! spawn tools ask as the agent's own permission mode decides, like every
//! other Storm tool, so no config a host writes names one of them.

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
    )
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(t.path().join("d/mcp.json")).unwrap(),
        "{\n  \"mcpServers\": {\n    \"github\": {\n      \"args\": [\n        \"mcp-bridge\",\n        \"github\"\n      ],\n      \"command\": \"/x\",\n      \"env\": {\n        \"STORM_MCP_HANDLE\": \"h\",\n        \"STORM_MCP_SOCK\": \"/s\"\n      },\n      \"type\": \"stdio\"\n    },\n    \"storm\": {\n      \"args\": [\n        \"mcp-bridge\",\n        \"storm\"\n      ],\n      \"command\": \"/x\",\n      \"env\": {\n        \"STORM_MCP_HANDLE\": \"h\",\n        \"STORM_MCP_SOCK\": \"/s\"\n      },\n      \"type\": \"stdio\"\n    }\n  }\n}",
        "nothing of spawning is in Claude Code's mcp.json"
    );
}

/// No config names a spawn tool: neither Claude Code's settings nor
/// OpenCode's permissions carry an ask or an allow for one, under any key.
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
