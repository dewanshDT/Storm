//! The permission mode a person chooses at launch (decision 146b): the
//! agent's own `default`, `accept_edits` or `auto`, never a bypass. A child
//! inherits its parent's and may only be stricter. The host's profile checks
//! a mode again and turns it into the agent's own flag or config;
//! `docs/agent-permission-mode-vectors.json` holds both crates to one rule.
//!
//! A child never asks (decision 148a): its host runs its mode in a no-prompt
//! form (Claude Code `dontAsk` with the child baseline, OpenCode with every
//! ask denied), so only the agents that have one are started as children.

pub const DEFAULT: &str = "default";
pub const ACCEPT_EDITS: &str = "accept_edits";
pub const AUTO: &str = "auto";

/// Strictest first.
pub const ORDER: [&str; 3] = [DEFAULT, ACCEPT_EDITS, AUTO];

/// How loose `mode` is, or `None` for anything that is not a mode.
pub fn rank(mode: &str) -> Option<usize> {
    ORDER.iter().position(|m| *m == mode)
}

/// The agents a child may run (148a): those whose host can keep them from
/// ever asking. Codex prompts for every Storm tool (132c), and nothing
/// documented stops it.
pub const UNATTENDED: [&str; 2] = ["claude-code", "opencode"];

pub fn unattended(agent: &str) -> bool {
    UNATTENDED.contains(&agent)
}

/// Why Codex, which has Storm's servers, is not started as a child, for
/// `agent_unavailable`.
pub fn not_unattended(agent: &str) -> String {
    format!(
        "{} cannot run as a child: nothing documented keeps it from stopping on its own \
         permission prompts, and a child never asks anyone.",
        label(agent)
    )
}

/// The modes `agent` offers, strictest first.
pub fn offered(agent: &str) -> &'static [&'static str] {
    match agent {
        "claude-code" => &ORDER,
        "opencode" => &ORDER[..2],
        _ => &ORDER[..1],
    }
}

fn label(agent: &str) -> &str {
    match agent {
        "claude-code" => "Claude Code",
        "opencode" => "OpenCode",
        "codex" => "Codex",
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

/// A child's mode when its start names none: its parent's, else the nearest
/// stricter one its agent offers.
pub fn inherit(agent: &str, parent: &str) -> &'static str {
    let top = rank(parent).unwrap_or(0);
    ORDER[..=top]
        .iter()
        .rev()
        .find(|m| offered(agent).contains(m))
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

    /// 146b: the server offers exactly what the host's profile maps, and
    /// refuses the rest; no bypass is a mode anywhere.
    #[test]
    fn modes_agree_with_the_vectors() {
        let v = vectors();
        let order: Vec<&str> = v["order"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m.as_str().unwrap())
            .collect();
        assert_eq!(order, ORDER);
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
        let child: Vec<&str> = v["child"]["unattended"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a.as_str().unwrap())
            .collect();
        assert_eq!(child, UNATTENDED);
        for r in v["child"]["refused"].as_array().unwrap() {
            let agent = r["agent"].as_str().unwrap();
            assert!(!unattended(agent) || check(agent, r["mode"].as_str().unwrap()).is_err());
        }
        for agent in ["claude-code", "opencode", "codex", "shell"] {
            for bypass in [
                "bypass",
                "bypassPermissions",
                "dangerously-skip-permissions",
            ] {
                assert!(check(agent, bypass).is_err());
                assert_eq!(rank(bypass), None);
            }
        }
    }
}
