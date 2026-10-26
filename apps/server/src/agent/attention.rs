//! `needs_attention` (decision 134p, slice 134-9): beside `activity`, not a
//! status, and never inferred from idle (A3). One kind at a time:
//!
//! - `approval`: a request the session made (start, message, end) waits for
//!   a person's decision (134o);
//! - `permission_prompt`: the agent's own CLI prompt waits, as its host's
//!   hooks report it — only for an adapter on the list below, which is
//!   empty;
//! - `blocked`: the session reported a result whose outcome is `blocked`
//!   (134h) and is still live.
//!
//! Every kind is worked out when a view is read, from what is on record
//! (the last host report, for a prompt), so it is set and cleared with the
//! record itself.

use super::store::SessionRecord;
use super::{AgentManager, AgentResult, StatusReport, now};

/// The adapters whose permission prompts are taken from their hosts: each
/// joins only once G-134-7 passes for it, and 134 lands dark, so none is
/// on. The host keeps the same list (`harness::PERMISSION_PROMPT_ADAPTERS`),
/// where the gate's results are.
pub const PERMISSION_PROMPT_ADAPTERS: &[&str] = &[];

pub const APPROVAL: &str = "approval";
pub const PERMISSION_PROMPT: &str = "permission_prompt";
pub const BLOCKED: &str = "blocked";

impl AgentManager {
    /// What a status report says of the agent's permission prompt, taken
    /// only for a live session whose adapter reports it.
    pub(super) fn reported_attention(
        &self,
        r: &SessionRecord,
        report: &StatusReport,
    ) -> Option<String> {
        let on = self
            .permission_prompt
            .lock()
            .unwrap()
            .contains(&r.provider.as_str());
        (on && !r.is_ended() && report.attention.as_deref() == Some(PERMISSION_PROMPT))
            .then(|| PERMISSION_PROMPT.to_string())
    }

    /// The session's `needs_attention`, if any. An ended session needs none.
    /// A waiting request comes first, since a person can answer it in Storm;
    /// a permission prompt shows only while the session runs, as activity
    /// does.
    pub fn needs_attention(&self, r: &SessionRecord) -> AgentResult<Option<&'static str>> {
        if r.is_ended() {
            return Ok(None);
        }
        if self.store.lock().unwrap().awaits_decision(&r.id, &now())? {
            return Ok(Some(APPROVAL));
        }
        let prompt = r.status == "running"
            && self
                .live
                .lock()
                .unwrap()
                .sessions
                .get(&r.id)
                .is_some_and(|s| s.attention.as_deref() == Some(PERMISSION_PROMPT));
        if prompt {
            return Ok(Some(PERMISSION_PROMPT));
        }
        let blocked = self
            .store
            .lock()
            .unwrap()
            .result(&r.id)?
            .is_some_and(|res| res.outcome == BLOCKED);
        Ok(blocked.then_some(BLOCKED))
    }

    /// Test-only: an adapter's permission prompts taken, as once its gate
    /// has passed.
    #[cfg(test)]
    pub fn report_permission_prompts_for_tests(&self, adapters: &'static [&'static str]) {
        *self.permission_prompt.lock().unwrap() = adapters;
    }
}
