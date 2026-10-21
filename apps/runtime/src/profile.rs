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
        // tools always ask (`codex.rs`). The hook bypass reaches only
        // Storm's own `CODEX_HOME`, whose only hook is Storm's; an untrusted
        // workspace's hooks never load (130b).
        args: &[
            "--no-daemon",
            "--sandbox",
            "danger-full-access",
            "--ask-for-approval",
            "on-request",
            "--dangerously-bypass-hook-trust",
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
        // No grants until a real-account gate shows a session-only config
        // that keeps a repository's servers out (130d).
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
        // A repository's `.mcp.json` loads with no approval and nothing turns
        // it off: a terminal and nothing more (130e).
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
