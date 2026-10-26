//! G-134-5: decision 134's pinned asks and A6 exemptions against the real
//! Claude Code and OpenCode (134-8).
//!
//! Runs only with `STORM_SPAWN_ASKS_GATE=1`, on a host with the CLIs
//! (`STORM_CLAUDE_BIN`, default `claude`, signed in; `STORM_OPENCODE_BIN`,
//! default `opencode`), or `make gate-spawn-asks`; never in CI. The 134
//! tools cannot be listed on any host yet (C0), so the config Storm's own
//! writers produce is put in front of each CLI with a stand-in `storm`
//! server offering the same tool names:
//!
//! - **Claude Code** runs `-p` with Storm's `--settings`, the workspace's
//!   `.claude/settings.local.json` allowing the pinned tools by name (what
//!   "don't ask again" writes), and a permission-prompt tool that records
//!   every prompt and allows it. The pinned tools must prompt; the
//!   exemptions must not; an ordinary Storm tool the local file allows must
//!   not (the control that the file was read) and one it does not must.
//!   It runs again in `bypassPermissions` with `--allowedTools mcp__storm`.
//! - **OpenCode** resolves each agent's rules itself (`opencode debug agent`)
//!   from a global config that allows everything — `"*": "allow"`, the
//!   pinned tools by name before it, an agent of its own and one in a
//!   markdown file — and the gate evaluates them as OpenCode does, last
//!   match first.
//!
//! `STORM_GATE_WORKSPACE_PARENT` is where the throwaway workspace goes (a
//! folder Claude Code already trusts); it is removed afterwards. Codex asks
//! for every Storm tool (`default_tools_approval_mode = "prompt"`, already
//! covered by `codex_gate.rs`); its interactive run is a manual step.
#![cfg(unix)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::{Value, json};
use storm_runtime::mcp::{
    self, AgentServers, EXEMPT_TOOLS, McpGrant, OpencodeAccount, PINNED_ASK_TOOLS, SpawnAsks,
};

const FAKE_SERVER: &str = r#"
import json, os, sys
log, name = os.environ["GATE_LOG"], os.environ["GATE_SERVER"]
tools = [t for t in os.environ.get("GATE_TOOLS", "").split(",") if t]
for line in sys.stdin:
    m = json.loads(line)
    mid, method = m.get("id"), m.get("method")
    if mid is None:
        continue
    if method == "initialize":
        r = {"protocolVersion": m["params"].get("protocolVersion", "2025-06-18"),
             "capabilities": {"tools": {}}, "serverInfo": {"name": name, "version": "1"}}
    elif method == "tools/list":
        if name == "gate":
            r = {"tools": [{"name": "approve", "description": "Permission prompt.",
                 "inputSchema": {"type": "object", "properties": {
                     "tool_name": {"type": "string"}, "input": {"type": "object"}}}}]}
        else:
            r = {"tools": [{"name": t, "description": "Gate tool " + t + ". Call it with no arguments.",
                 "inputSchema": {"type": "object", "properties": {}}} for t in tools]}
    elif method == "tools/call":
        args = m["params"].get("arguments") or {}
        with open(log, "a") as f:
            f.write(json.dumps({"server": name, "tool": m["params"]["name"], "args": args}) + "\n")
        text = json.dumps({"behavior": "allow", "updatedInput": args.get("input") or {}}) if name == "gate" else "ok"
        r = {"content": [{"type": "text", "text": text}]}
    else:
        r = {}
    sys.stdout.write(json.dumps({"jsonrpc": "2.0", "id": mid, "result": r}) + "\n")
    sys.stdout.flush()
"#;

fn enabled() -> bool {
    std::env::var("STORM_SPAWN_ASKS_GATE").as_deref() == Ok("1")
}

/// A throwaway workspace where Claude Code already trusts folders.
fn workspace() -> tempfile::TempDir {
    let parent = std::env::var_os("STORM_GATE_WORKSPACE_PARENT")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    tempfile::Builder::new()
        .prefix("storm-spawn-asks-gate-")
        .tempdir_in(parent)
        .unwrap()
}

/// The host's environment without the variables an enclosing Claude Code
/// session sets.
fn clean(c: &mut Command) -> &mut Command {
    for (k, _) in std::env::vars_os() {
        if k.to_string_lossy().starts_with("CLAUDE_CODE_") || k == "CLAUDECODE" {
            c.env_remove(k);
        }
    }
    c.stdin(Stdio::null())
}

fn storm_tools() -> Vec<String> {
    PINNED_ASK_TOOLS
        .iter()
        .chain(EXEMPT_TOOLS.iter())
        .map(|t| t.to_string())
        .chain(["search".to_string(), "get_note".to_string()])
        .collect()
}

fn server_entry(script: &Path, log: &Path, name: &str) -> Value {
    json!({
        "type": "stdio",
        "command": "python3",
        "args": [script],
        "env": {"GATE_LOG": log, "GATE_SERVER": name, "GATE_TOOLS": storm_tools().join(",")},
    })
}

fn log_lines(log: &Path) -> Vec<Value> {
    std::fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

struct Run {
    called: BTreeSet<String>,
    prompted: BTreeSet<String>,
}

fn claude_run(bin: &str, extra: &[&str]) -> Run {
    let ws = workspace();
    let dir = ws.path().join("session");
    let script = ws.path().join("server.py");
    let log = ws.path().join("calls.log");
    std::fs::write(&script, FAKE_SERVER).unwrap();
    let asks = SpawnAsks {
        exempt: true,
        spawned: false,
    };
    // Storm's own settings file, as a spawn session's launch writes it.
    let (extras, _) = storm_runtime::harness::launch(
        "claude-code",
        &dir,
        "h",
        Path::new("/nonexistent.sock"),
        Path::new("/bin/true"),
        None,
        None,
        Some(&asks),
        false,
    )
    .unwrap();
    let mcp_json = ws.path().join("mcp.json");
    std::fs::write(
        &mcp_json,
        json!({"mcpServers": {
            "storm": server_entry(&script, &log, "storm"),
            "gate": server_entry(&script, &log, "gate"),
        }})
        .to_string(),
    )
    .unwrap();
    std::fs::create_dir(ws.path().join(".claude")).unwrap();
    let local: Vec<String> = PINNED_ASK_TOOLS
        .iter()
        .chain(["search"].iter())
        .map(|t| format!("mcp__storm__{t}"))
        .collect();
    std::fs::write(
        ws.path().join(".claude/settings.local.json"),
        json!({"permissions": {"allow": local}}).to_string(),
    )
    .unwrap();
    let order: Vec<String> = ["list_agents", "agent_status", "report_result"]
        .iter()
        .chain(PINNED_ASK_TOOLS.iter())
        .chain(["search", "get_note"].iter())
        .map(|t| format!("mcp__storm__{t}"))
        .collect();
    let prompt = format!(
        "This is a permissions test. Call each of these MCP tools exactly once, in this order, \
         with no arguments, even if one fails: {}. Then reply DONE.",
        order.join(", ")
    );
    let mut c = Command::new(bin);
    clean(&mut c)
        .current_dir(ws.path())
        .args(extras.args)
        .args(["--mcp-config"])
        .arg(&mcp_json)
        .args([
            "--permission-prompt-tool",
            "mcp__gate__approve",
            "--output-format",
            "json",
            "--max-turns",
            "30",
            "-p",
            &prompt,
        ])
        .args(extra);
    let out = c.output().unwrap();
    let result: Value = serde_json::from_slice(&out.stdout).unwrap_or_default();
    eprintln!(
        "claude {extra:?}: exit {:?}, {} {}\n{}",
        out.status.code(),
        result["subtype"],
        result["result"],
        String::from_utf8_lossy(&out.stderr)
    );
    let lines = log_lines(&log);
    let called = lines
        .iter()
        .filter(|l| l["server"] == "storm")
        .map(|l| l["tool"].as_str().unwrap().to_string())
        .collect();
    let prompted = lines
        .iter()
        .filter(|l| l["server"] == "gate")
        .map(|l| {
            l["args"]["tool_name"]
                .as_str()
                .unwrap()
                .trim_start_matches("mcp__storm__")
                .to_string()
        })
        .collect();
    Run { called, prompted }
}

#[test]
fn claude_code_asks_for_the_pinned_tools_and_only_those() {
    if !enabled() {
        eprintln!("STORM_SPAWN_ASKS_GATE is not 1; skipping G-134-5 for Claude Code");
        return;
    }
    let bin = std::env::var("STORM_CLAUDE_BIN").unwrap_or_else(|_| "claude".into());
    let want: BTreeSet<String> = PINNED_ASK_TOOLS
        .iter()
        .map(|t| t.to_string())
        .chain(["get_note".to_string()])
        .collect();

    let run = claude_run(&bin, &[]);
    eprintln!(
        "default: called {:?}, prompted {:?}",
        run.called, run.prompted
    );
    for t in [
        "list_agents",
        "agent_status",
        "report_result",
        "search",
        "get_note",
    ]
    .iter()
    .chain(PINNED_ASK_TOOLS.iter())
    {
        assert!(run.called.contains(*t), "the model did not call {t}; rerun");
    }
    assert_eq!(
        run.prompted, want,
        "the pinned tools ask despite a local allow; the exemptions and the allowed control do not"
    );

    let run = claude_run(
        &bin,
        &[
            "--permission-mode",
            "bypassPermissions",
            "--allowedTools",
            "mcp__storm",
        ],
    );
    eprintln!(
        "bypass: called {:?}, prompted {:?}",
        run.called, run.prompted
    );
    for t in PINNED_ASK_TOOLS {
        assert!(run.called.contains(t), "the model did not call {t}; rerun");
        assert!(
            run.prompted.contains(t),
            "{t} ran without its ask under bypassPermissions"
        );
    }
}

/// One rule as `opencode debug agent` prints it.
fn rules_of(bin: &str, home: &Path, ws: &Path, content: &str, agent: &str) -> Value {
    let mut c = Command::new(bin);
    let out = clean(&mut c)
        .current_dir(ws)
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("OPENCODE_DISABLE_PROJECT_CONFIG", "1")
        .env("OPENCODE_CONFIG_CONTENT", content)
        .args(["debug", "agent", agent])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "opencode debug agent {agent}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}

/// OpenCode's own evaluation: the last rule whose name matches the tool.
fn decides(agent: &Value, tool: &str) -> String {
    agent["permission"]
        .as_array()
        .unwrap()
        .iter()
        .rev()
        .find(|r| {
            mcp::opencode_matches(r["permission"].as_str().unwrap(), tool)
                && mcp::opencode_matches(r["pattern"].as_str().unwrap_or("*"), "*")
        })
        .map(|r| r["action"].as_str().unwrap().to_string())
        .unwrap_or_else(|| "allow".into())
}

fn opencode_content(
    dir: &Path,
    settings: Option<&Value>,
    asks: &SpawnAsks,
    account: &OpencodeAccount,
) -> String {
    let grants = [McpGrant {
        id: "storm".into(),
        slug: "storm".into(),
        plugin_origin: None,
    }];
    let none = Default::default();
    let extras = mcp::write_provider_config(
        "opencode",
        dir,
        &grants,
        "h",
        Path::new("/nonexistent.sock"),
        Path::new("/bin/true"),
        settings,
        &Default::default(),
        &AgentServers {
            storm_only: false,
            taken: &none,
        },
        Some((asks, account)),
    )
    .unwrap();
    extras
        .env
        .iter()
        .find(|(k, _)| k == "OPENCODE_CONFIG_CONTENT")
        .map(|(_, v)| v.to_str().unwrap().to_string())
        .unwrap()
}

#[test]
fn opencode_resolves_the_pinned_asks_after_everything_the_account_allows() {
    if !enabled() {
        eprintln!("STORM_SPAWN_ASKS_GATE is not 1; skipping G-134-5 for OpenCode");
        return;
    }
    let bin = std::env::var("STORM_OPENCODE_BIN").unwrap_or_else(|_| "opencode".into());
    let ws = workspace();
    let home = ws.path().join("home");
    let global = home.join(".config/opencode");
    std::fs::create_dir_all(global.join("agent")).unwrap();
    std::fs::write(
        global.join("opencode.json"),
        json!({
            "permission": {
                "storm_start_agent": "allow",
                "storm_send_message": "allow",
                "storm_start_agent*": "allow",
                "*": "allow",
            },
            "agent": {"build": {"permission": {"*": "allow"}}, "mine": {"mode": "primary", "permission": {"storm_end_agent": "allow", "*": "allow"}}},
        })
        .to_string(),
    )
    .unwrap();
    std::fs::write(
        global.join("agent/helper.md"),
        "---\ndescription: A helper of the account's.\nmode: subagent\npermission:\n  \"*\": allow\n  storm_send_message: allow\n---\nHelp.\n",
    )
    .unwrap();
    let account =
        mcp::opencode_account_in(&[global.clone(), home.join(".opencode")], None).unwrap();
    assert_eq!(
        account
            .agents
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["build", "helper", "mine"]
    );
    let settings = json!({"permission": {"*": "allow"}});
    let verified = SpawnAsks {
        exempt: true,
        spawned: false,
    };
    let content = opencode_content(&ws.path().join("s1"), Some(&settings), &verified, &account);
    for agent in ["build", "plan", "general", "mine", "helper"] {
        let rules = rules_of(&bin, &home, ws.path(), &content, agent);
        if agent == "helper" {
            assert_eq!(
                rules["mode"], "subagent",
                "Storm's entry merged into the file's agent"
            );
        }
        for t in PINNED_ASK_TOOLS {
            assert_eq!(decides(&rules, &format!("storm_{t}")), "ask", "{agent} {t}");
        }
        eprintln!("{agent}: pinned tools ask");
    }
    // The controls: this account does allow the pinned tools without
    // Storm's keys after it, and the markdown agent without its own.
    let mcp_only = {
        let mut v: Value = serde_json::from_str(&content).unwrap();
        for t in PINNED_ASK_TOOLS {
            v["permission"]
                .as_object_mut()
                .unwrap()
                .retain(|k, _| !k.starts_with(&format!("storm_{t}")));
        }
        v.as_object_mut().unwrap().remove("agent");
        v.to_string()
    };
    let rules = rules_of(&bin, &home, ws.path(), &mcp_only, "build");
    assert_eq!(decides(&rules, "storm_start_agent"), "allow", "control");
    let no_agents = OpencodeAccount {
        agents: Default::default(),
        ..account.clone()
    };
    let content = opencode_content(
        &ws.path().join("s0"),
        Some(&settings),
        &verified,
        &no_agents,
    );
    let rules = rules_of(&bin, &home, ws.path(), &content, "helper");
    assert_eq!(decides(&rules, "storm_start_agent"), "allow", "control");

    // An account that asks for everything: the exemptions, and only they,
    // skip the prompt; off a verified host, nothing does.
    std::fs::write(
        global.join("opencode.json"),
        json!({"permission": {"*": "ask"}}).to_string(),
    )
    .unwrap();
    std::fs::remove_file(global.join("agent/helper.md")).unwrap();
    let account = mcp::opencode_account_in(std::slice::from_ref(&global), None).unwrap();
    let content = opencode_content(&ws.path().join("s2"), None, &verified, &account);
    let rules = rules_of(&bin, &home, ws.path(), &content, "build");
    for t in EXEMPT_TOOLS {
        assert_eq!(decides(&rules, &format!("storm_{t}")), "allow", "{t}");
    }
    for t in PINNED_ASK_TOOLS
        .iter()
        .chain(["search", "session_context"].iter())
    {
        assert_eq!(decides(&rules, &format!("storm_{t}")), "ask", "{t}");
    }
    let unverified = SpawnAsks {
        exempt: false,
        spawned: false,
    };
    let content = opencode_content(&ws.path().join("s3"), None, &unverified, &account);
    let rules = rules_of(&bin, &home, ws.path(), &content, "build");
    for t in EXEMPT_TOOLS {
        assert_eq!(decides(&rules, &format!("storm_{t}")), "ask", "{t}");
    }
    let child = SpawnAsks {
        exempt: true,
        spawned: true,
    };
    let content = opencode_content(&ws.path().join("s4"), None, &child, &account);
    let rules = rules_of(&bin, &home, ws.path(), &content, "build");
    assert_eq!(decides(&rules, "storm_session_context"), "allow");
}
