//! The model a launch or a start may name (decision 145b), per agent. The
//! host's profile checks it again and turns it into the agent's own flag;
//! `docs/agent-model-vectors.json` holds both crates to one rule. A model
//! reaches argv, so only ASCII letters, digits and `. _ : / -` are accepted,
//! never first a `-`, and bounded.

/// The agents a model can be passed to.
pub const MODEL_AGENTS: [&str; 3] = ["claude-code", "opencode", "codex"];

fn alnum(c: char) -> bool {
    c.is_ascii_alphanumeric()
}

fn claude(m: &str) -> bool {
    if ["haiku", "sonnet", "opus"].contains(&m) {
        return true;
    }
    let Some(rest) = m.strip_prefix("claude-") else {
        return false;
    };
    m.len() <= 64
        && rest
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && rest
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '-')
}

fn opencode(m: &str) -> bool {
    let Some((provider, model)) = m.split_once('/') else {
        return false;
    };
    m.len() <= 128
        && provider
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && provider
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
        && model.chars().next().is_some_and(alnum)
        && model
            .chars()
            .all(|c| alnum(c) || matches!(c, '.' | '_' | ':' | '/' | '-'))
}

fn codex(m: &str) -> bool {
    m.len() <= 64
        && m.chars().next().is_some_and(alnum)
        && m.chars()
            .all(|c| alnum(c) || matches!(c, '.' | '_' | ':' | '-'))
}

/// The forms `agent` takes, as a sentence.
pub fn accepted_forms(agent: &str) -> String {
    match agent {
        "claude-code" => "Claude Code takes an alias (haiku, sonnet or opus) or a full model id \
            such as claude-sonnet-4-5."
            .into(),
        "opencode" => "OpenCode takes provider/model, such as anthropic/claude-sonnet-4-5.".into(),
        "codex" => "Codex takes a model id such as gpt-5-codex.".into(),
        other => format!("Storm passes no model to {other}: leave model out."),
    }
}

/// `Ok` when `model` may be passed to `agent`; else the `model_unavailable`
/// sentence, naming the forms it takes.
pub fn check(agent: &str, model: &str) -> Result<(), String> {
    let ok = match agent {
        "claude-code" => claude(model),
        "opencode" => opencode(model),
        "codex" => codex(model),
        _ => false,
    };
    if ok {
        Ok(())
    } else {
        let shown: String = model.chars().take(64).collect();
        Err(format!(
            "{shown:?} is not a model Storm can pass to {agent}. {}",
            accepted_forms(agent)
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 145b: the server accepts exactly what the host's profile accepts.
    #[test]
    fn models_agree_with_the_vectors() {
        let v: serde_json::Value =
            serde_json::from_str(include_str!("../../../../docs/agent-model-vectors.json"))
                .unwrap();
        for a in v["accepted"].as_array().unwrap() {
            let (agent, model) = (a["agent"].as_str().unwrap(), a["model"].as_str().unwrap());
            assert_eq!(check(agent, model), Ok(()), "{agent} {model}");
        }
        for r in v["refused"].as_array().unwrap() {
            let (agent, model) = (r["agent"].as_str().unwrap(), r["model"].as_str().unwrap());
            let sentence = check(agent, model).expect_err(model);
            assert!(sentence.contains(&accepted_forms(agent)), "{sentence}");
        }
    }
}
