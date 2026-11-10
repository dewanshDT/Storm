//! The permission mode a person chooses at launch (decisions 146b, 151 N5):
//! one of the agent's own modes, passed through as its own flag, or
//! `default`, which passes nothing so the agent's own config decides. A
//! child runs its parent's mode, or `default`. The host's profile checks a
//! mode again and turns it into the agent's own flag or config;
//! `docs/agent-permission-mode-vectors.json` holds both crates to one rule.

pub const DEFAULT: &str = "default";
pub const ACCEPT_EDITS: &str = "accept_edits";
pub const PLAN: &str = "plan";
pub const AUTO: &str = "auto";
pub const DONT_ASK: &str = "dont_ask";
pub const BYPASS: &str = "bypass";

/// Every mode Storm passes through, in the launcher's order.
pub const MODES: [&str; 6] = [DEFAULT, ACCEPT_EDITS, PLAN, AUTO, DONT_ASK, BYPASS];

/// The agent id a start names: an id as given, or a name that can mean
/// only one agent ("claude", "Claude Code", "open code").
pub fn agent_id(asked: &str) -> String {
    let folded: String = asked
        .trim()
        .chars()
        .filter(|c| !matches!(c, ' ' | '-' | '_'))
        .flat_map(char::to_lowercase)
        .collect();
    match folded.as_str() {
        "claude" | "claudecode" => "claude-code".into(),
        "opencode" => "opencode".into(),
        "codex" => "codex".into(),
        _ => asked.trim().to_string(),
    }
}

/// The modes `agent` offers, in [`MODES`]' order.
pub fn offered(agent: &str) -> &'static [&'static str] {
    match agent {
        "claude-code" => &MODES,
        "opencode" => &MODES[..2],
        _ => &MODES[..1],
    }
}

pub fn label(agent: &str) -> &str {
    match agent {
        "claude-code" => "Claude Code",
        "opencode" => "OpenCode",
        "codex" => "Codex",
        "cursor" => "Cursor",
        "command-code" => "Command Code",
        other => other,
    }
}

/// The modes `agent` takes, as a sentence.
pub fn accepted_modes(agent: &str) -> String {
    format!("{} offers {}.", label(agent), offered(agent).join(", "))
}

/// `Ok` when `agent` offers `mode`; else the `permission_mode_unavailable`
/// sentence.
pub fn check(agent: &str, mode: &str) -> Result<(), String> {
    if offered(agent).contains(&mode) {
        return Ok(());
    }
    let shown: String = mode.chars().take(40).collect();
    Err(format!(
        "{shown:?} is not a permission mode Storm can give {}. {}",
        label(agent),
        accepted_modes(agent)
    ))
}

/// A child's mode when its start names none (151 N5): its parent's when
/// its agent offers it, else `default`, so its own config decides.
pub fn inherit(agent: &str, parent: &str) -> &'static str {
    offered(agent)
        .iter()
        .find(|m| **m == parent)
        .copied()
        .unwrap_or(DEFAULT)
}

/// What is stored and sent: nothing for `default`.
pub fn stored(mode: &str) -> Option<String> {
    (mode != DEFAULT).then(|| mode.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vectors() -> serde_json::Value {
        serde_json::from_str(include_str!(
            "../../../../docs/agent-permission-mode-vectors.json"
        ))
        .unwrap()
    }

    #[test]
    fn a_name_that_means_one_agent_is_its_id() {
        for (asked, id) in [
            ("claude", "claude-code"),
            ("Claude Code", "claude-code"),
            ("claude_code", "claude-code"),
            ("claude-code", "claude-code"),
            ("Open Code", "opencode"),
            ("opencode", "opencode"),
            ("Codex", "codex"),
            ("claude-haiku", "claude-haiku"),
            ("gpt", "gpt"),
        ] {
            assert_eq!(agent_id(asked), id, "{asked}");
        }
    }

    /// 146b, 151 N5: the server offers exactly what the host's profile
    /// maps, and refuses the rest; a child inherits as the vectors say.
    #[test]
    fn modes_agree_with_the_vectors() {
        let v = vectors();
        let modes: Vec<&str> = v["modes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m.as_str().unwrap())
            .collect();
        assert_eq!(modes, MODES);
        for (agent, modes) in v["offered"].as_object().unwrap() {
            let modes: Vec<&str> = modes
                .as_array()
                .unwrap()
                .iter()
                .map(|m| m.as_str().unwrap())
                .collect();
            assert_eq!(offered(agent), modes.as_slice(), "{agent}");
        }
        for a in v["accepted"].as_array().unwrap() {
            let (agent, mode) = (a["agent"].as_str().unwrap(), a["mode"].as_str().unwrap());
            assert_eq!(check(agent, mode), Ok(()), "{agent} {mode}");
        }
        for r in v["refused"].as_array().unwrap() {
            let (agent, mode) = (r["agent"].as_str().unwrap(), r["mode"].as_str().unwrap());
            let sentence = check(agent, mode).expect_err(mode);
            assert!(sentence.contains(&accepted_modes(agent)), "{sentence}");
        }
        for i in v["inherit"].as_array().unwrap() {
            assert_eq!(
                inherit(i["agent"].as_str().unwrap(), i["parent"].as_str().unwrap()),
                i["child"].as_str().unwrap(),
                "{i}"
            );
        }
    }
}
