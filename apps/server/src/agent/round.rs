//! Decision 145c: a child's final message is its round's result, and 145f:
//! what the agent said the round cost. Both come from the agent's own hook
//! or plugin (Claude Code's `Stop` with `last_assistant_message`, OpenCode's
//! idle plus the session's last message), through its host, and each agent
//! is taken only once its gate G-145-R-<provider> passes on the real CLI.
//!
//! The text is the child's own words: it becomes the round's 134h result,
//! read through `agent_status` and `wait_agent`'s views, and never part of
//! an event or a wake (144d). A round the child reported itself keeps its
//! own report.

use serde::Deserialize;

use super::events::{PROVIDER_HOOK, Usage};
use super::store::spawn::{MAX_DETAILS_BYTES, MAX_SUMMARY_BYTES, SpawnResult};
use super::{AgentManager, AgentResult, now};

/// One agent's final message, through one source, taken as a round's
/// result once its gate passes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResultSource {
    pub provider: &'static str,
    /// `provider_hook`.
    pub source: &'static str,
}

/// No agent's final message has passed its gate. The host keeps the same
/// list (`harness::RESULT_SOURCES`), spelled the same: a test holds the two
/// equal.
pub const RESULT_SOURCES: &[ResultSource] = &[];

/// What a host reports when a child's turn ends.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct FinalMessage {
    /// The agent's last assistant message.
    pub text: String,
    /// `provider_hook`, the only source there is.
    #[serde(default)]
    pub source: Option<String>,
    /// The round's tokens, as the agent itself counted them.
    #[serde(default)]
    pub usage: Option<Usage>,
}

/// The longest final message a host may send; what is kept is bounded as a
/// `report_result` is.
pub const MAX_FINAL_MESSAGE_BYTES: usize = 64 * 1024;

fn cut(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// A final message as a result's summary and details, within
/// `report_result`'s bounds: the summary is its start, and details the
/// whole of a longer one, cut at 16 KiB. Nothing for an empty message.
pub fn bounded(text: &str) -> Option<(String, Option<String>)> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let summary = cut(text, MAX_SUMMARY_BYTES).to_string();
    let details = (text.len() > summary.len()).then(|| cut(text, MAX_DETAILS_BYTES).to_string());
    Some((summary, details))
}

impl AgentManager {
    /// Whether `provider`'s final message, through `source`, is a result.
    pub fn result_source_on(&self, provider: &str, source: &str) -> bool {
        self.result_sources
            .lock()
            .unwrap()
            .iter()
            .any(|s| s.provider == provider && s.source == source)
    }

    /// Test-only: final messages taken, as once their gates have passed.
    #[cfg(test)]
    pub fn gate_result_sources_for_tests(&self, sources: &'static [ResultSource]) {
        *self.result_sources.lock().unwrap() = sources;
    }

    /// A host's report of a child's final message (145c). Taken only from
    /// the child's own host, for a running spawned session, from a gated
    /// source; otherwise dropped, with nothing written. It becomes the
    /// round's result, outcome `succeeded`, unless the child reported the
    /// round itself; the usage is kept either way. Returns the result it
    /// made.
    pub fn final_message(
        &self,
        host_id: &str,
        session_id: &str,
        message: &FinalMessage,
    ) -> AgentResult<Option<SpawnResult>> {
        let r = self.owned_by(host_id, session_id)?;
        if r.status != "running" {
            return Ok(None);
        }
        let source = match message.source.as_deref() {
            None | Some(PROVIDER_HOOK) => PROVIDER_HOOK,
            Some(_) => return Ok(None),
        };
        if !self.result_source_on(&r.provider, source) {
            return Ok(None);
        }
        let live = self.live_signals(session_id);
        let store = self.store.lock().unwrap();
        if store.lineage(session_id)?.parent_session.is_none() {
            return Ok(None);
        }
        let text = cut(&message.text, MAX_FINAL_MESSAGE_BYTES);
        let Some((summary, details)) = bounded(text) else {
            return Ok(None);
        };
        let result = SpawnResult {
            round: 0,
            outcome: "succeeded".into(),
            summary,
            details,
            reported_at: now(),
        };
        let became = store.record_final_message(
            session_id,
            &result,
            message.usage.as_ref(),
            &live,
            source,
        )?;
        let made = if became {
            store.result(session_id)?
        } else {
            None
        };
        drop(store);
        self.bump(session_id);
        Ok(made)
    }
}
