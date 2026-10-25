//! 134-8: the pinned asks and the A6 exemptions in each adapter's config.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::*;

const VERIFIED: SpawnAsks = SpawnAsks {
    exempt: true,
    spawned: false,
};
const UNVERIFIED: SpawnAsks = SpawnAsks {
    exempt: false,
    spawned: false,
};
const CHILD: SpawnAsks = SpawnAsks {
    exempt: true,
    spawned: true,
};

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
fn opencode(
    dir: &Path,
    grants: &[McpGrant],
    settings: Option<&Value>,
    storm_only: bool,
    spawn: Option<(&SpawnAsks, &OpencodeAccount)>,
) -> String {
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
        spawn,
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

fn claude_settings(dir: &Path, spawn: Option<&SpawnAsks>) -> String {
    crate::harness::launch(
        "claude-code",
        dir,
        "h",
        Path::new("/s"),
        Path::new("/x"),
        Some("0b8f1c2e-3d4a-4b5c-8d6e-7f8091a2b3c4"),
        None,
        spawn,
        false,
    )
    .unwrap();
    std::fs::read_to_string(dir.join("claude-settings.json")).unwrap()
}

/// With spawn off every config is byte for byte what 134-7 wrote: these
/// are its outputs, taken from `feat/134-7-end-cascades`.
#[test]
fn with_spawn_off_every_config_is_byte_identical() {
    let t = tempfile::tempdir().unwrap();
    assert_eq!(
        opencode(
            &t.path().join("a"),
            &grants(),
            Some(&settings()),
            false,
            None
        ),
        r#"{"$schema":"https://opencode.ai/config.json","agent":{"build":{"permission":{"*":"allow"}}},"mcp":{"github":{"command":["/x","mcp-bridge","github"],"enabled":true,"environment":{"STORM_MCP_HANDLE":"h","STORM_MCP_SOCK":"/s"},"type":"local"},"storm":{"command":["/x","mcp-bridge","storm"],"enabled":true,"environment":{"STORM_MCP_HANDLE":"h","STORM_MCP_SOCK":"/s"},"type":"local"}},"permission":{"*":"allow","bash":"ask","github_*":"ask","storm_*":"ask"}}"#
    );
    let xdg = opencode(
        &t.path().join("b"),
        &grants(),
        Some(&settings()),
        true,
        None,
    );
    let golden = "{\n  \"$schema\": \"https://opencode.ai/config.json\",\n  \"agent\": {\n    \"build\": {\n      \"permission\": {\n        \"*\": \"allow\"\n      }\n    }\n  },\n  \"mcp\": {\n    \"github\": {\n      \"command\": [\n        \"/x\",\n        \"mcp-bridge\",\n        \"github\"\n      ],\n      \"enabled\": true,\n      \"environment\": {\n        \"STORM_MCP_HANDLE\": \"h\",\n        \"STORM_MCP_SOCK\": \"/s\"\n      },\n      \"type\": \"local\"\n    },\n    \"storm\": {\n      \"command\": [\n        \"/x\",\n        \"mcp-bridge\",\n        \"storm\"\n      ],\n      \"enabled\": true,\n      \"environment\": {\n        \"STORM_MCP_HANDLE\": \"h\",\n        \"STORM_MCP_SOCK\": \"/s\"\n      },\n      \"type\": \"local\"\n    }\n  },\n  \"permission\": {\n    \"*\": \"allow\",\n    \"bash\": \"ask\",\n    \"github_*\": \"ask\",\n    \"storm_*\": \"ask\"\n  }\n}";
    assert_eq!(xdg, golden);
    assert_eq!(
        claude_settings(&t.path().join("c"), None),
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
        Some((&VERIFIED, &OpencodeAccount::default())),
    )
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(t.path().join("d/mcp.json")).unwrap(),
        "{\n  \"mcpServers\": {\n    \"github\": {\n      \"args\": [\n        \"mcp-bridge\",\n        \"github\"\n      ],\n      \"command\": \"/x\",\n      \"env\": {\n        \"STORM_MCP_HANDLE\": \"h\",\n        \"STORM_MCP_SOCK\": \"/s\"\n      },\n      \"type\": \"stdio\"\n    },\n    \"storm\": {\n      \"args\": [\n        \"mcp-bridge\",\n        \"storm\"\n      ],\n      \"command\": \"/x\",\n      \"env\": {\n        \"STORM_MCP_HANDLE\": \"h\",\n        \"STORM_MCP_SOCK\": \"/s\"\n      },\n      \"type\": \"stdio\"\n    }\n  }\n}",
        "Claude Code's asks are in its settings, never its mcp.json"
    );
}

/// A session that cannot reach Storm's own tools gets nothing new, even if
/// a start asked.
#[test]
fn without_the_storm_connection_the_asks_change_nothing() {
    let t = tempfile::tempdir().unwrap();
    let github = vec![grants().remove(1)];
    let account = OpencodeAccount::default();
    assert_eq!(
        opencode(&t.path().join("a"), &github, Some(&settings()), false, None),
        opencode(
            &t.path().join("b"),
            &github,
            Some(&settings()),
            false,
            Some((&VERIFIED, &account))
        ),
    );
}

#[test]
fn claude_code_pins_three_asks_and_allows_the_exemptions_only_when_verified() {
    let t = tempfile::tempdir().unwrap();
    let ask = json!([
        "mcp__storm__start_agent",
        "mcp__storm__send_message",
        "mcp__storm__end_agent"
    ]);
    let v: Value =
        serde_json::from_str(&claude_settings(&t.path().join("a"), Some(&VERIFIED))).unwrap();
    assert_eq!(v["permissions"]["ask"], ask);
    assert_eq!(
        v["permissions"]["allow"],
        json!([
            "mcp__storm__list_agents",
            "mcp__storm__agent_status",
            "mcp__storm__list_children",
            "mcp__storm__wait_agent",
            "mcp__storm__read_messages",
            "mcp__storm__report_result"
        ])
    );
    assert!(v["hooks"]["SessionStart"].is_array(), "the hook stays");
    let v: Value =
        serde_json::from_str(&claude_settings(&t.path().join("b"), Some(&UNVERIFIED))).unwrap();
    assert_eq!(
        v["permissions"],
        json!({ "ask": ask }),
        "no exemption off a verified host"
    );
    let v: Value =
        serde_json::from_str(&claude_settings(&t.path().join("c"), Some(&CHILD))).unwrap();
    let allow = v["permissions"]["allow"].as_array().unwrap();
    assert_eq!(allow.len(), 7);
    assert!(allow.contains(&json!("mcp__storm__session_context")));
    let v: Value =
        serde_json::from_str(&claude_settings(&t.path().join("d"), Some(&VERIFIED))).unwrap();
    assert!(
        !v["permissions"]["allow"]
            .as_array()
            .unwrap()
            .contains(&json!("mcp__storm__session_context")),
        "a parent's session_context still asks"
    );
}

#[test]
fn claude_code_settings_that_lost_the_asks_fail_the_start() {
    let good = json!({"permissions": claude_permissions(&VERIFIED)}).to_string();
    verify_claude_permissions(good.as_bytes(), &VERIFIED).unwrap();
    for bad in [
        json!({}),
        json!({"permissions": {"ask": ["mcp__storm__start_agent"]}}),
        json!({"permissions": claude_permissions(&CHILD)}),
    ] {
        assert!(verify_claude_permissions(bad.to_string().as_bytes(), &VERIFIED).is_err());
    }
}

/// Claude Code's documented order (A4, checked on the real CLI by
/// `tests/spawn_asks_gate.rs`): any matching deny, then any matching ask,
/// then any matching allow, whatever source each came from. A rule names a
/// server (`mcp__storm`, `mcp__storm__*`) or one tool.
fn claude_decides(sources: &[&Value], tool: &str) -> &'static str {
    let matches = |rule: &str| {
        let full = format!("mcp__storm__{tool}");
        rule == full || rule == "mcp__storm" || rule == "mcp__storm__*"
    };
    let any = |kind: &str| {
        sources.iter().any(|s| {
            s["permissions"][kind]
                .as_array()
                .into_iter()
                .flatten()
                .any(|r| r.as_str().is_some_and(matches))
        })
    };
    if any("deny") {
        "deny"
    } else if any("ask") {
        "ask"
    } else if any("allow") {
        "allow"
    } else {
        "default"
    }
}

#[test]
fn the_accounts_allow_and_dont_ask_again_never_skip_a_pinned_ask_on_claude_code() {
    let storm: Value = json!({"permissions": claude_permissions(&VERIFIED)});
    let accounts = [
        json!({"permissions": {"allow": ["mcp__storm"]}}),
        json!({"permissions": {"allow": ["mcp__storm__*"]}}),
        json!({"permissions": {"allow": PINNED_ASK_TOOLS.map(|t| format!("mcp__storm__{t}"))}}),
    ];
    for account in &accounts {
        for tool in PINNED_ASK_TOOLS {
            assert_eq!(
                claude_decides(&[account, &storm], tool),
                "ask",
                "{account} {tool}"
            );
        }
        for tool in EXEMPT_TOOLS {
            assert_eq!(claude_decides(&[account, &storm], tool), "allow");
        }
    }
    let asks_all = json!({"permissions": {"ask": ["mcp__storm"]}});
    for tool in EXEMPT_TOOLS {
        assert_eq!(
            claude_decides(&[&asks_all, &storm], tool),
            "ask",
            "the account may ask for more"
        );
    }
}

/// An ordered rule list, as OpenCode reads one config's `permission`.
type Rules = Vec<(String, String)>;

fn rules(pairs: &[(&str, &str)]) -> Rules {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

/// A written config's rules in the order OpenCode parses them, which is
/// the order they are serialized in.
fn written(v: &Value) -> Rules {
    v.as_object()
        .into_iter()
        .flatten()
        .map(|(k, a)| (k.clone(), a.as_str().unwrap().to_string()))
        .collect()
}

/// OpenCode 1.18.31's merge: a key already present keeps its position and
/// takes the new value; a new key goes last.
fn merge(into: &mut Rules, layer: &Rules) {
    for (k, v) in layer {
        match into.iter_mut().find(|(have, _)| have == k) {
            Some(e) => e.1 = v.clone(),
            None => into.push((k.clone(), v.clone())),
        }
    }
}

/// OpenCode's answer for a tool: the global config, then Storm's content,
/// at the top level and then in the agent; the last match wins.
fn opencode_decides(
    global: &Rules,
    global_agent: &Rules,
    content: &Value,
    agent: &str,
    tool: &str,
) -> String {
    let mut top = global.clone();
    merge(&mut top, &written(&content["permission"]));
    let mut own = global_agent.clone();
    merge(&mut own, &written(&content["agent"][agent]["permission"]));
    top.iter()
        .chain(own.iter())
        .rev()
        .find(|(k, _)| opencode_matches(k, tool))
        .map(|(_, v)| v.clone())
        .unwrap_or_else(|| "allow".into())
}

fn account_of(global: &Rules, global_agent: &Rules, agents: &[&str]) -> OpencodeAccount {
    OpencodeAccount {
        text: serde_json::to_string(&(global, global_agent)).unwrap(),
        agents: agents.iter().map(|a| a.to_string()).collect(),
    }
}

/// The ways an account's own config could end up after Storm's asks: a
/// global `"*": "allow"`, the pinned tools' own names earlier (kept in
/// place by the merge), the same names with a `*` already, and an agent of
/// its own allowing everything.
#[test]
fn no_global_or_agent_allow_cancels_a_pinned_ask_on_opencode() {
    let cases: Vec<(Rules, Rules)> = vec![
        (rules(&[("*", "allow")]), rules(&[])),
        (
            rules(&[
                ("storm_*", "allow"),
                ("storm_start_agent", "allow"),
                ("storm_send_message", "allow"),
                ("storm_end_agent", "allow"),
                ("*", "allow"),
            ]),
            rules(&[]),
        ),
        (
            rules(&[
                ("storm_start_agent", "allow"),
                ("storm_start_agent*", "allow"),
                ("storm_start_agent**", "allow"),
                ("storm_s*", "allow"),
                ("*", "allow"),
            ]),
            rules(&[("storm_start_agent*", "allow"), ("*", "allow")]),
        ),
        (rules(&[]), rules(&[("*", "allow")])),
    ];
    let t = tempfile::tempdir().unwrap();
    for (i, (global, global_agent)) in cases.iter().enumerate() {
        let account = account_of(global, global_agent, &["mine"]);
        for storm_only in [false, true] {
            let v: Value = serde_json::from_str(&opencode(
                &t.path().join(format!("{i}{storm_only}")),
                &grants(),
                Some(&settings()),
                storm_only,
                Some((&VERIFIED, &account)),
            ))
            .unwrap();
            for agent in ["mine", "build"] {
                for tool in PINNED_ASK_TOOLS {
                    let tool = format!("storm_{tool}");
                    assert_eq!(
                        opencode_decides(global, global_agent, &v, agent, &tool),
                        "ask",
                        "case {i}, {agent}, {tool}: {v}"
                    );
                }
            }
            // 132b's `storm_*` is not moved: a global config that names it
            // before `"*": "allow"` keeps it there (recorded in decision 134).
            if !global.iter().any(|(k, _)| k == "storm_*") {
                assert_eq!(
                    opencode_decides(global, &rules(&[]), &v, "plan", "storm_search"),
                    "ask",
                    "every other Storm tool still asks"
                );
            }
        }
    }
}

/// A `runtime.toml` setting naming a pinned tool, at the top level or in an
/// agent, cannot relax it either; the setting itself stays.
#[test]
fn a_hosts_setting_cannot_relax_a_pinned_ask_on_opencode() {
    let t = tempfile::tempdir().unwrap();
    let settings = json!({
        "permission": {
            "storm_start_agent": "allow",
            "storm_start_agent*": "allow",
            "storm_start_agent *": "allow",
            "storm_?tart_agent": "allow",
            "*": "allow",
        },
        "agent": {"build": {"permission": {"storm_end_agent*": "allow", "*": "allow"}}},
    });
    let v: Value = serde_json::from_str(&opencode(
        t.path(),
        &grants(),
        Some(&settings),
        false,
        Some((&VERIFIED, &OpencodeAccount::default())),
    ))
    .unwrap();
    for tool in PINNED_ASK_TOOLS {
        let tool = format!("storm_{tool}");
        assert_eq!(
            opencode_decides(&rules(&[]), &rules(&[]), &v, "build", &tool),
            "ask",
            "{tool}: {v}"
        );
    }
    assert_eq!(
        v["permission"]["storm_start_agent*"], "allow",
        "the setting is kept"
    );
    assert_eq!(v["permission"]["storm_start_agent**"], "ask");
    assert_eq!(v["permission"]["*"], "allow");
}

#[test]
fn the_exemptions_skip_the_prompt_on_opencode_only_where_storm_may_say_so() {
    let t = tempfile::tempdir().unwrap();
    let global = rules(&[("*", "ask")]);
    let account = account_of(&global, &rules(&[]), &[]);
    let content = |name: &str, asks: &SpawnAsks, account: &OpencodeAccount| -> Value {
        serde_json::from_str(&opencode(
            &t.path().join(name),
            &grants(),
            None,
            false,
            Some((asks, account)),
        ))
        .unwrap()
    };
    let v = content("verified", &VERIFIED, &account);
    for tool in EXEMPT_TOOLS {
        let tool = format!("storm_{tool}");
        assert_eq!(
            opencode_decides(&global, &rules(&[]), &v, "build", &tool),
            "allow",
            "{tool}"
        );
    }
    assert_eq!(
        opencode_decides(&global, &rules(&[]), &v, "build", "storm_session_context"),
        "ask"
    );
    let v = content("child", &CHILD, &account);
    assert_eq!(
        opencode_decides(&global, &rules(&[]), &v, "build", "storm_session_context"),
        "allow"
    );
    let v = content("unverified", &UNVERIFIED, &account);
    for tool in EXEMPT_TOOLS {
        let tool = format!("storm_{tool}");
        assert_eq!(
            opencode_decides(&global, &rules(&[]), &v, "build", &tool),
            "ask"
        );
        assert!(v["permission"].get(&tool).is_none());
    }
    // An exempt tool the account names itself is not exempted.
    let global = rules(&[("storm_wait_agent", "deny")]);
    let v = content("named", &VERIFIED, &account_of(&global, &rules(&[]), &[]));
    assert!(v["permission"].get("storm_wait_agent").is_none());
    assert_eq!(
        opencode_decides(&global, &rules(&[]), &v, "build", "storm_wait_agent"),
        "ask"
    );
}

#[test]
fn opencode_wildcards_match_as_opencode_does() {
    for (p, n, want) in [
        ("*", "storm_start_agent", true),
        ("storm_*", "storm_start_agent", true),
        ("storm_start_agent", "storm_start_agent", true),
        ("storm_start_agent***", "storm_start_agent", true),
        ("storm_?tart_agent", "storm_start_agent", true),
        ("storm_start_agent *", "storm_start_agent", true),
        ("storm_start", "storm_start_agent", false),
        ("storm_start_agent?", "storm_start_agent", false),
        ("Storm_*", "storm_start_agent", false),
        ("storm.start_agent", "storm_start_agent", false),
    ] {
        assert_eq!(opencode_matches(p, n), want, "{p} {n}");
    }
}

#[test]
fn the_accounts_opencode_config_is_read_with_its_agents() {
    let t = tempfile::tempdir().unwrap();
    let global = t.path().join("opencode");
    let dot = t.path().join(".opencode");
    std::fs::create_dir_all(global.join("agent")).unwrap();
    std::fs::create_dir_all(dot.join("agents")).unwrap();
    std::fs::write(
        global.join("opencode.jsonc"),
        "{\n  // mine\n  \"agent\": {\"review\": {}, /* x */ \"build\": {},},\n  \"permission\": {\"storm_start_agent\": \"allow\",},\n}",
    )
    .unwrap();
    std::fs::write(
        global.join("agent/docs.md"),
        "---\npermission:\n  \"*\": allow\n---\n",
    )
    .unwrap();
    std::fs::write(dot.join("agents/ops.md"), "---\n---\n").unwrap();
    std::fs::write(dot.join("agents/notes.txt"), "").unwrap();
    let extra = t.path().join("extra.json");
    std::fs::write(&extra, r#"{"agent": {"extra": {}}}"#).unwrap();
    let a = opencode_account_in(&[global.clone(), dot.clone()], Some(extra)).unwrap();
    assert_eq!(
        a.agents.iter().map(String::as_str).collect::<Vec<_>>(),
        ["build", "docs", "extra", "ops", "review"]
    );
    assert!(a.text.contains("storm_start_agent"));
    assert!(a.text.contains("\"*\": allow"), "agent files are read too");

    std::fs::create_dir_all(global.join("agent/nested")).unwrap();
    assert!(opencode_account_in(std::slice::from_ref(&global), None).is_err());
    std::fs::remove_dir(global.join("agent/nested")).unwrap();
    std::fs::write(global.join("opencode.json"), "{ not json").unwrap();
    assert!(opencode_account_in(std::slice::from_ref(&global), None).is_err());
    assert_eq!(
        opencode_account_in(&[t.path().join("none")], None).unwrap(),
        OpencodeAccount::default()
    );
}

#[test]
fn jsonc_comments_and_trailing_commas_are_dropped_but_strings_kept() {
    let v: Value = serde_json::from_str(&strip_jsonc(
        "{\"a\": \"//not a comment\", // one\n \"b\": [1, 2,], /* two */ \"c\": \"/*x*/\",}",
    ))
    .unwrap();
    assert_eq!(
        v,
        json!({"a": "//not a comment", "b": [1, 2], "c": "/*x*/"})
    );
}

/// The host learns no tool names from the server (C0 keeps them unlisted),
/// so its pinned lists must be the server's: every one of decision 134's
/// tools is either pinned or exempt, and no other.
#[test]
fn the_pinned_tool_names_are_the_servers() {
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
    let mut server: Vec<String> = list("SPAWN_TOOLS");
    server.extend(list("SPAWNED_TOOLS"));
    server.sort();
    let mut host: Vec<String> = PINNED_ASK_TOOLS
        .iter()
        .chain(EXEMPT_TOOLS.iter())
        .map(|t| t.to_string())
        .collect();
    host.sort();
    assert_eq!(server, host);
    assert!(src.contains("pub const ONLY_FOR_AGENTS: &[&str] = &[\"session_context\"];"));
}
