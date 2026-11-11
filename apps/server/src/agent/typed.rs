//! A Shell launched with a command typed in (decision 153-S3): the text the
//! host types into the new Shell's terminal once it is ready, never followed
//! by Enter, and each agent's own sign-in command for the Sign in fix.
//!
//! The text is terminal input, never argv or the environment: it rides the
//! `start` as data and the host writes it to the PTY.
//! `docs/typed-input-vectors.json` holds both crates to one rule.

/// The provider that may be launched with text typed in.
pub const PROVIDER: &str = "shell";

/// The most bytes a launch may type.
pub const MAX_BYTES: usize = 1024;

/// Characters that reorder or hide what the terminal shows, so the line a
/// person reads before pressing Enter is the line that runs.
const INVISIBLE: [std::ops::RangeInclusive<char>; 4] = [
    '\u{200B}'..='\u{200F}',
    '\u{202A}'..='\u{202E}',
    '\u{2060}'..='\u{2069}',
    '\u{FEFF}'..='\u{FEFF}',
];

/// `Ok` when `text` may be typed: one line of at most [`MAX_BYTES`] with no
/// control character, so nothing it holds presses Enter or sends a key.
pub fn check(text: &str) -> Result<(), String> {
    if text.len() > MAX_BYTES {
        return Err(format!(
            "typed_input_invalid: at most {MAX_BYTES} bytes may be typed"
        ));
    }
    if text.chars().any(|c| c == '\n' || c == '\r') {
        return Err("typed_input_invalid: typed input is one line; it is never run for you".into());
    }
    if text
        .chars()
        .any(|c| c.is_control() || INVISIBLE.iter().any(|r| r.contains(&c)))
    {
        return Err("typed_input_invalid: typed input has no control characters".into());
    }
    Ok(())
}

/// The command that signs `agent` in on a host, typed into a Shell for the
/// person to run. `None` for an agent with no sign-in command of its own.
pub fn login_command(agent: &str) -> Option<&'static str> {
    match agent {
        "claude-code" => Some("claude auth login"),
        "opencode" => Some("opencode auth login"),
        "codex" => Some("codex login"),
        "cursor" => Some("cursor-agent login"),
        "command-code" => Some("command-code login"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vectors() -> serde_json::Value {
        serde_json::from_str(include_str!("../../../../docs/typed-input-vectors.json")).unwrap()
    }

    #[test]
    fn the_vectors_rule_is_the_servers() {
        let v = vectors();
        assert_eq!(v["max_bytes"], MAX_BYTES);
        for ok in v["accepted"].as_array().unwrap() {
            check(ok.as_str().unwrap()).unwrap();
        }
        for bad in v["refused"].as_array().unwrap() {
            let bad = bad.as_str().unwrap();
            assert!(
                check(bad).unwrap_err().starts_with("typed_input_invalid: "),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn every_installable_agent_has_its_sign_in_and_each_one_types() {
        for agent in super::super::jobs::INSTALLABLE {
            let command = login_command(agent).unwrap_or_else(|| panic!("{agent}"));
            check(command).unwrap();
        }
        assert_eq!(login_command("shell"), None);
        assert_eq!(login_command("my-agent"), None);
    }
}
