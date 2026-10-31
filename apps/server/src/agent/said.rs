//! Decision 145g, as amended 2026-10-31: what a child said. `read_output`
//! read only a child's terminal, and a Claude Code child redraws in place and
//! then waits inside `read_messages`, so its screen never showed its answer.
//! Its own hook or plugin now reports each assistant text it writes, through
//! its host, and the server keeps the newest few per child, in memory.
//!
//! The text is the child's own words: data, never authority (144d). It is
//! shown to the parent through `read_output` and is never part of an event,
//! a wake or anything written to a terminal (C9). Each agent is taken only
//! once its gate G-145-O-<provider> passes on the real CLI.

use std::collections::{HashMap, VecDeque};

use serde::{Deserialize, Serialize};

use super::events::PROVIDER_HOOK;
use super::{AgentManager, AgentResult, now};

/// The agents whose children's words are taken, once G-145-O passes for
/// them; it passed locally on 2026-10-31 (Claude Code 2.1.314, OpenCode
/// 1.19.4). The host keeps the same list (`harness::SAID`), spelled the
/// same: a test holds the two equal.
pub const SAID: &[&str] = &["claude-code", "opencode"];

/// The longest message kept, in bytes.
pub const MAX_SAID_BYTES: usize = 4 * 1024;
/// The most messages kept per child: the newest.
pub const MAX_SAID_PER_CHILD: usize = 20;
/// The most children whose messages are kept; past it, the one heard from
/// longest ago is forgotten.
pub const MAX_SAID_CHILDREN: usize = 512;

/// What a host reports when a child said something.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SaidReport {
    pub messages: Vec<SaidText>,
    /// `provider_hook`, the only source there is.
    #[serde(default)]
    pub source: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct SaidText {
    pub text: String,
}

/// One thing a child said, with when Storm heard it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChildMessage {
    pub at: String,
    pub text: String,
}

#[derive(Debug, Default)]
pub struct Said {
    children: HashMap<String, (u64, VecDeque<ChildMessage>)>,
    tick: u64,
}

impl Said {
    fn push(&mut self, session: &str, messages: Vec<ChildMessage>) {
        self.tick += 1;
        if !self.children.contains_key(session) && self.children.len() >= MAX_SAID_CHILDREN {
            let oldest = self
                .children
                .iter()
                .min_by_key(|(_, (tick, _))| *tick)
                .map(|(id, _)| id.clone());
            if let Some(id) = oldest {
                self.children.remove(&id);
            }
        }
        let (tick, ring) = self.children.entry(session.to_string()).or_default();
        *tick = self.tick;
        ring.extend(messages);
        while ring.len() > MAX_SAID_PER_CHILD {
            ring.pop_front();
        }
    }

    fn recent(&self, session: &str) -> Vec<ChildMessage> {
        self.children
            .get(session)
            .map(|(_, ring)| ring.iter().cloned().collect())
            .unwrap_or_default()
    }
}

/// A message as it is kept: no control characters but newlines and tabs,
/// trimmed, at most [`MAX_SAID_BYTES`]; none when empty.
pub fn kept(text: &str) -> Option<String> {
    let clean: String = text
        .chars()
        .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
        .collect();
    let clean = clean.trim();
    if clean.is_empty() {
        return None;
    }
    let mut end = clean.len().min(MAX_SAID_BYTES);
    while !clean.is_char_boundary(end) {
        end -= 1;
    }
    Some(clean[..end].to_string())
}

impl AgentManager {
    /// Whether `provider`'s children's words are taken.
    pub fn said_on(&self, provider: &str) -> bool {
        self.said_gate.lock().unwrap().contains(&provider)
    }

    /// Test-only: words taken, as once their gates have passed.
    #[cfg(test)]
    pub fn gate_said_for_tests(&self, providers: &'static [&'static str]) {
        *self.said_gate.lock().unwrap() = providers;
    }

    /// A host's report of what a child said. Taken only from the child's own
    /// host, for a running spawned session whose agent's gate lets its words
    /// through, from its agent's own hook; otherwise dropped, with nothing
    /// kept. Returns how many messages it kept.
    pub fn said(&self, host_id: &str, session_id: &str, report: &SaidReport) -> AgentResult<usize> {
        let r = self.owned_by(host_id, session_id)?;
        if r.status != "running" || !self.said_on(&r.provider) {
            return Ok(0);
        }
        if !matches!(report.source.as_deref(), None | Some(PROVIDER_HOOK)) {
            return Ok(0);
        }
        if self
            .store
            .lock()
            .unwrap()
            .lineage(session_id)?
            .parent_session
            .is_none()
        {
            return Ok(0);
        }
        let at = now();
        let skip = report.messages.len().saturating_sub(MAX_SAID_PER_CHILD);
        let messages: Vec<ChildMessage> = report.messages[skip..]
            .iter()
            .filter_map(|m| kept(&m.text))
            .map(|text| ChildMessage {
                at: at.clone(),
                text,
            })
            .collect();
        let n = messages.len();
        if n > 0 {
            self.said.lock().unwrap().push(session_id, messages);
        }
        Ok(n)
    }

    /// The newest things `session_id` said, oldest first.
    pub fn child_messages(&self, session_id: &str) -> Vec<ChildMessage> {
        self.said.lock().unwrap().recent(session_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_childs_words_are_bounded() {
        let mut said = Said::default();
        let m = |n: usize| ChildMessage {
            at: "t".into(),
            text: format!("m{n}"),
        };
        said.push("a", (0..30).map(m).collect());
        let recent = said.recent("a");
        assert_eq!(recent.len(), MAX_SAID_PER_CHILD);
        assert_eq!(recent[0].text, "m10");
        assert_eq!(recent.last().unwrap().text, "m29");
        for n in 0..MAX_SAID_CHILDREN {
            said.push(&format!("c{n}"), vec![m(n)]);
        }
        assert!(
            said.recent("a").is_empty(),
            "the one heard from longest ago goes"
        );
        assert_eq!(said.children.len(), MAX_SAID_CHILDREN);
        said.push("c0", vec![m(1)]);
        said.push("new", vec![m(2)]);
        assert_eq!(said.recent("c0").len(), 2, "c0 was heard from again");
        assert!(said.recent("c1").is_empty());

        assert_eq!(
            kept("  \x1b[31mhi\x07 there\n  ").as_deref(),
            Some("[31mhi there")
        );
        assert_eq!(kept("\n\t "), None);
        let long = kept(&"é".repeat(MAX_SAID_BYTES)).unwrap();
        assert!(long.len() <= MAX_SAID_BYTES && long.chars().all(|c| c == 'é'));
    }
}
