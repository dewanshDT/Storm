//! What Storm knows about each built-in agent (decision 130).
//!
//! A `[[providers]]` entry names an id; when the id is a built-in one, the
//! session gets that agent's adapter even with a `command` of the entry's own
//! (a wrapper script, a pinned path), as `claude-code` always has. An id that
//! is not built in is a terminal and nothing else.
//!
//! Each adapter answers the same questions (the vault's *Agent Adapters —
//! Research*, R1–R6): how it launches safely, how a note's opening prompt is
//! passed, whether it can be given a session-only MCP config that asks before
//! each gateway tool, how its conversation id is learned for Resume, and
//! whether its terminal title names the conversation. An adapter that cannot
//! keep a repository's own MCP servers out says so (`mcp: None`), and the
//! server then grants it nothing (130a).

/// How a session's MCP servers reach the agent, or that they cannot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mcp {
    /// `--mcp-config <file> --strict-mcp-config` (AM32).
    ClaudeCode,
    /// A session `XDG_CONFIG_HOME` with `"<slug>_*": "ask"` (AM32, AM66).
    OpenCode,
    /// A per-session profile in Storm's own `CODEX_HOME` (130b).
    Codex,
    /// The bridges as a `--plugin-dir` plugin, every other server on Cursor's
    /// own disabled list (131j).
    Cursor,
    /// The workspace's local `mcp.json`: the bridges, and a disabled stub
    /// under every other server's name (131k).
    CommandCode,
}

/// How the opening prompt of a session started from a note is passed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Prompt {
    Positional,
    Flag(&'static str),
}

/// The shape of a conversation id the agent can resume (decision 89). It
/// becomes an argument, so nothing outside the shape is ever accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdShape {
    /// A lowercase, hyphenated UUID.
    Uuid,
    /// `ses_` and 1–64 ASCII letters or digits.
    OpenCode,
}

#[derive(Debug, Clone, Copy)]
pub struct Profile {
    pub id: &'static str,
    /// The command looked for on the host's `PATH`.
    pub command: &'static str,
    /// Always passed. Never a flag that skips the agent's own permission
    /// prompts (Q6).
    pub args: &'static [&'static str],
    pub mcp: Option<Mcp>,
    /// `None` for an agent with no Storm MCP: the prompt names a Storm tool.
    pub prompt: Option<Prompt>,
    /// `None`: no Resume.
    pub resume: Option<IdShape>,
    /// The agent's terminal title is the conversation's name (D15 AM43).
    /// `false` keeps the launch name: Command Code's title is its own name,
    /// the folder and the model.
    pub titles: bool,
}

/// The built-ins, in the order a host offers them when `runtime.toml` lists
/// none.
pub const BUILT_IN: [Profile; 6] = [
    Profile {
        id: "claude-code",
        command: "claude",
        // Gate Q6 found the operator's own CLI settings starting it in
        // another mode, so the mode is always passed.
        args: &["--permission-mode", "default"],
        mcp: Some(Mcp::ClaudeCode),
        prompt: Some(Prompt::Positional),
        resume: Some(IdShape::Uuid),
        titles: true,
    },
    Profile {
        id: "opencode",
        command: "opencode",
        args: &[],
        mcp: Some(Mcp::OpenCode),
        prompt: Some(Prompt::Flag("--prompt")),
        resume: Some(IdShape::OpenCode),
        titles: true,
    },
    Profile {
        id: "codex",
        command: "codex",
        // A session owns its process: 0.163 otherwise attaches to a shared
        // daemon, which holds a conversation's writer lock. No Codex
        // sandbox: it needs user namespaces, which the Runtime Host's unit
        // forbids (`RestrictNamespaces=true`), and the host's account is the
        // boundary for every agent (130c). Approvals stay on, and gateway
        // tools always ask (`codex.rs`). Storm's hook carries its own trust
        // record, so no hook runs unreviewed (132c-1).
        args: &[
            "--no-daemon",
            "--sandbox",
            "danger-full-access",
            "--ask-for-approval",
            "on-request",
        ],
        mcp: Some(Mcp::Codex),
        prompt: Some(Prompt::Positional),
        resume: Some(IdShape::Uuid),
        titles: true,
    },
    Profile {
        id: "cursor",
        command: "cursor-agent",
        // Cursor will not start in an untrusted folder, and its own dialog's
        // first choice also enables the repository's MCP servers. `--trust`
        // trusts the folder and leaves those servers waiting for approval.
        args: &["--trust"],
        // No grants until a real-account gate (G-C1) shows Storm's servers
        // reaching a session through `cursor.rs` (130d, 132d).
        mcp: None,
        prompt: None,
        resume: None,
        titles: false,
    },
    Profile {
        id: "command-code",
        // Not `cmd`, which is cmd.exe on Windows.
        command: "command-code",
        args: &[
            "--no-auto-update",
            "--skip-onboarding",
            "--permission-mode",
            "standard",
        ],
        // No grants until a real-account gate (G-CC1) shows Storm's servers
        // reaching a session through `command_code.rs` (130e, 132d).
        mcp: None,
        prompt: None,
        resume: None,
        titles: false,
    },
    Profile {
        id: "shell",
        // Replaced by the platform's own shell; see `CliProvider::shell`.
        command: "",
        args: &[],
        mcp: None,
        prompt: None,
        resume: None,
        titles: true,
    },
];

/// The built-in profile for `id`.
pub fn get(id: &str) -> Option<&'static Profile> {
    BUILT_IN.iter().find(|p| p.id == id)
}

/// The model a session runs, as its agent's own launch flag (decision 145b).
/// The rules are `docs/agent-model-vectors.json`'s, which the server applies
/// too: only ASCII letters, digits and `.` `_` `:` `/` `-`, a letter or digit
/// first, and a bounded length, so a value reaches argv (and a Windows batch
/// shim, AM53) unescaped. Anything else, and any other agent, is refused with
/// the forms it would take.
pub fn model_args(provider: &str, model: &str) -> Result<Vec<String>, String> {
    let alnum = |c: char| c.is_ascii_alphanumeric();
    let lower = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit();
    let first = |s: &str, ok: &dyn Fn(char) -> bool| s.chars().next().is_some_and(ok);
    let (ok, flag, forms) = match provider {
        "claude-code" => {
            let ok = matches!(model, "haiku" | "sonnet" | "opus")
                || (model.len() <= 64
                    && model.strip_prefix("claude-").is_some_and(|rest| {
                        first(rest, &lower)
                            && rest.chars().all(|c| lower(c) || c == '.' || c == '-')
                    }));
            (
                ok,
                "--model",
                "Claude Code takes an alias (haiku, sonnet or opus) or a full model id such as claude-sonnet-4-5",
            )
        }
        "opencode" => {
            let ok = model.len() <= 128
                && model.split_once('/').is_some_and(|(p, m)| {
                    first(p, &lower)
                        && p.chars().all(|c| lower(c) || c == '_' || c == '-')
                        && first(m, &alnum)
                        && m.chars().all(|c| alnum(c) || ".:_/-".contains(c))
                });
            (
                ok,
                "--model",
                "OpenCode takes provider/model, such as anthropic/claude-sonnet-4-5",
            )
        }
        "codex" => {
            let ok = model.len() <= 64
                && first(model, &alnum)
                && model.chars().all(|c| alnum(c) || "._:-".contains(c));
            (ok, "-m", "Codex takes a model id such as gpt-5-codex")
        }
        _ => return Err(format!("Storm passes no model to {provider}")),
    };
    if ok {
        Ok(vec![flag.to_string(), model.to_string()])
    } else {
        Err(format!("{model:?} is not a model Storm can pass: {forms}"))
    }
}

/// What a permission mode the person chose (decision 146b) adds to a
/// launch: the agent's own flag, or keys in Storm's OpenCode config layer.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PermissionLaunch {
    pub args: Vec<String>,
    pub opencode_permission: Vec<(String, String)>,
}

/// A permission mode as `provider` takes it (146b), by
/// `docs/agent-permission-mode-vectors.json`'s rule, which the server applies
/// too. `default` adds nothing. Claude Code takes `--permission-mode
/// acceptEdits|auto`, OpenCode `edit: allow` for `accept_edits`; anything
/// else, and every bypass, is refused, so the start fails rather than fall
/// back.
pub fn permission_mode(provider: &str, mode: &str) -> Result<PermissionLaunch, String> {
    let flag = |value: &str| PermissionLaunch {
        args: vec!["--permission-mode".into(), value.into()],
        ..Default::default()
    };
    match (provider, mode) {
        (_, "default") => Ok(PermissionLaunch::default()),
        ("claude-code", "accept_edits") => Ok(flag("acceptEdits")),
        ("claude-code", "auto") => Ok(flag("auto")),
        ("opencode", "accept_edits") => Ok(PermissionLaunch {
            opencode_permission: vec![("edit".into(), "allow".into())],
            ..Default::default()
        }),
        _ => Err(format!(
            "{:?} is not a permission mode Storm gives {provider}",
            mode.chars().take(40).collect::<String>()
        )),
    }
}

/// Whether Storm can give `id`'s sessions their own MCP servers: `Some` for
/// a built-in, `None` for an id it knows nothing about (the server keeps its
/// older behaviour for those).
pub fn supports_mcp(id: &str) -> Option<bool> {
    get(id).map(|p| p.mcp.is_some())
}

/// The providers whose conversations can be resumed (decision 89).
pub fn resumable(id: &str) -> bool {
    get(id).is_some_and(|p| p.resume.is_some())
}

/// Whether `id` is a conversation id `provider` could have made.
pub fn valid_id(provider: &str, id: &str) -> bool {
    match get(provider).and_then(|p| p.resume) {
        Some(IdShape::Uuid) => is_uuid(id),
        Some(IdShape::OpenCode) => id.strip_prefix("ses_").is_some_and(|rest| {
            (1..=64).contains(&rest.len()) && rest.bytes().all(|b| b.is_ascii_alphanumeric())
        }),
        None => false,
    }
}

fn is_uuid(id: &str) -> bool {
    let b = id.as_bytes();
    b.len() == 36
        && b.iter().enumerate().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => *c == b'-',
            _ => c.is_ascii_hexdigit() && !c.is_ascii_uppercase(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model_vectors() -> serde_json::Value {
        serde_json::from_str(include_str!("../../../docs/agent-model-vectors.json")).unwrap()
    }

    /// 145b: every accepted model becomes exactly its flag, every refused
    /// one is refused, as the server's copy of the rule decides too.
    #[test]
    fn a_model_is_passed_exactly_as_the_vectors_say() {
        let v = model_vectors();
        for case in v["accepted"].as_array().unwrap() {
            let (agent, model) = (
                case["agent"].as_str().unwrap(),
                case["model"].as_str().unwrap(),
            );
            let want: Vec<String> = serde_json::from_value(case["args"].clone()).unwrap();
            assert_eq!(model_args(agent, model), Ok(want), "{agent} {model}");
        }
        for case in v["refused"].as_array().unwrap() {
            let (agent, model) = (
                case["agent"].as_str().unwrap(),
                case["model"].as_str().unwrap(),
            );
            let refused = model_args(agent, model);
            assert!(refused.is_err(), "{agent} {model:?}: {refused:?}");
        }
        for (agent, max) in v["max_len"].as_object().unwrap() {
            let max = max.as_u64().unwrap() as usize;
            let at = match agent.as_str() {
                "claude-code" => format!("claude-{}", "a".repeat(max - 7)),
                "opencode" => format!("a/{}", "b".repeat(max - 2)),
                _ => "c".repeat(max),
            };
            assert!(model_args(agent, &at).is_ok(), "{agent} at {max}");
            assert!(
                model_args(agent, &format!("{at}x")).is_err(),
                "{agent} past {max}"
            );
        }
    }

    /// 145b with AM53: an accepted model needs no escaping in a Windows
    /// batch shim's command line, so `claude.cmd` takes it as it is.
    #[test]
    fn every_accepted_model_passes_the_batch_file_rules() {
        for case in model_vectors()["accepted"].as_array().unwrap() {
            let args = model_args(
                case["agent"].as_str().unwrap(),
                case["model"].as_str().unwrap(),
            )
            .unwrap();
            crate::platform::wincmd::batch_command_line("cmd.exe", "claude.cmd", &args).unwrap();
        }
    }

    fn mode_vectors() -> serde_json::Value {
        serde_json::from_str(include_str!(
            "../../../docs/agent-permission-mode-vectors.json"
        ))
        .unwrap()
    }

    /// 146b (A-146-2): each mode becomes exactly the table's flag or config
    /// key, as the server's copy of the rule offers it; every other mode,
    /// bypass included, is refused; and every flag passes the Windows
    /// batch-file rules unescaped.
    #[test]
    fn a_146_2_a_mode_is_mapped_exactly_as_the_vectors_say() {
        let v = mode_vectors();
        for case in v["accepted"].as_array().unwrap() {
            let (agent, mode) = (
                case["agent"].as_str().unwrap(),
                case["mode"].as_str().unwrap(),
            );
            let args: Vec<String> = serde_json::from_value(case["args"].clone()).unwrap();
            let opencode: Vec<(String, String)> = case["opencode_permission"]
                .as_object()
                .unwrap()
                .iter()
                .map(|(k, v)| (k.clone(), v.as_str().unwrap().to_string()))
                .collect();
            let got = permission_mode(agent, mode).unwrap();
            assert_eq!(
                got,
                PermissionLaunch {
                    args: args.clone(),
                    opencode_permission: opencode
                },
                "{agent} {mode}"
            );
            crate::platform::wincmd::batch_command_line("cmd.exe", "claude.cmd", &args).unwrap();
        }
        for case in v["refused"].as_array().unwrap() {
            let (agent, mode) = (
                case["agent"].as_str().unwrap(),
                case["mode"].as_str().unwrap(),
            );
            assert!(permission_mode(agent, mode).is_err(), "{agent} {mode:?}");
        }
        for (agent, modes) in v["offered"].as_object().unwrap() {
            for mode in v["order"].as_array().unwrap() {
                let mode = mode.as_str().unwrap();
                assert_eq!(
                    permission_mode(agent, mode).is_ok(),
                    modes.as_array().unwrap().iter().any(|m| m == mode),
                    "{agent} {mode}"
                );
            }
        }
        for agent in ["claude-code", "opencode", "codex"] {
            for mode in ["accept_edits", "auto"] {
                if let Ok(l) = permission_mode(agent, mode) {
                    for a in &l.args {
                        assert!(!a.contains("bypass") && !a.contains("skip"), "{a}");
                    }
                }
            }
        }
    }

    #[test]
    fn ids_are_unique_and_valid() {
        let mut seen = std::collections::HashSet::new();
        for p in BUILT_IN {
            assert!(seen.insert(p.id), "{} twice", p.id);
            assert!(crate::provider::ProviderId::new(p.id).is_some(), "{}", p.id);
        }
    }

    // Q6: a built-in never starts an agent with its permission prompts off.
    #[test]
    fn no_built_in_skips_permissions() {
        for p in BUILT_IN {
            for a in p.args {
                for bad in [
                    "dangerously-skip",
                    "--yolo",
                    "--force",
                    "bypass-approvals",
                    "approve-mcps",
                ] {
                    assert!(!a.contains(bad), "{} passes {a}", p.id);
                }
            }
        }
    }

    // An agent whose repository config Storm cannot keep out gets no MCP,
    // and with no Storm tool there is nothing for a note's prompt to name.
    #[test]
    fn a_prompt_needs_mcp() {
        for p in BUILT_IN {
            if p.prompt.is_some() {
                assert!(p.mcp.is_some(), "{}", p.id);
            }
        }
    }

    #[test]
    fn cursor_and_command_code_are_terminals() {
        for id in ["cursor", "command-code", "shell"] {
            assert_eq!(supports_mcp(id), Some(false), "{id}");
            assert!(!resumable(id), "{id}");
        }
        assert_eq!(supports_mcp("my-agent"), None);
        assert_eq!(supports_mcp("fake"), None);
    }

    // `cmd` is cmd.exe on Windows.
    #[test]
    fn command_code_runs_by_its_long_name() {
        assert_eq!(get("command-code").unwrap().command, "command-code");
    }
}
