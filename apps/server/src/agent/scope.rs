//! Decision 134-4: what a session may see of others, and its own report.
//! A parent sees only its descendants (L-2, the checks table's scope row);
//! a spawned session writes only its own result. An id outside that scope
//! is `not found`, never "not yours", so it says nothing about other trees.

use std::time::{Duration, Instant};

use super::store::SessionRecord;
use super::store::spawn::{
    Lineage, MAX_DETAILS_BYTES, MAX_SUMMARY_BYTES, MAX_TREE, RESULT_OUTCOMES, SpawnResult,
};
use super::{AgentError, AgentManager, AgentResult, now};

/// The longest a `wait_agent` holds a call (134i), per adapter. Each is
/// below the shortest timeout on the path: the gateway's own 60 s
/// (`gateway::session::CALL_TIMEOUT`), Codex's 60 s `tool_timeout_sec` and
/// the MCP SDK's 60 s that OpenCode falls back to. G-134-1 checks it
/// against the real binaries.
pub fn wait_cap(provider: &str) -> Duration {
    match provider {
        "claude-code" | "opencode" | "codex" => Duration::from_secs(30),
        _ => Duration::ZERO,
    }
}

/// A wait never asks for more than its adapter's cap.
pub fn wait_bound(provider: &str, requested: Option<Duration>) -> Duration {
    let cap = wait_cap(provider);
    requested.map_or(cap, |r| r.min(cap))
}

/// At most one tree's worth of sessions per wait.
pub const MAX_WAIT_IDS: usize = MAX_TREE as usize;

/// Whether a session counts as done for a wait: it ended, or it reported.
/// Idle is never done (134h).
pub fn settled(r: &SessionRecord, result: Option<&SpawnResult>) -> bool {
    r.is_ended() || result.is_some()
}

impl AgentManager {
    pub fn lineage(&self, session_id: &str) -> AgentResult<Lineage> {
        Ok(self.store.lock().unwrap().lineage(session_id)?)
    }

    pub fn task_of(&self, session_id: &str) -> AgentResult<Option<String>> {
        Ok(self.store.lock().unwrap().task(session_id)?)
    }

    pub fn result_of(&self, session_id: &str) -> AgentResult<Option<SpawnResult>> {
        Ok(self.store.lock().unwrap().result(session_id)?)
    }

    /// Whether a session is offered the spawned tools now: it was started
    /// by another (it has lineage), it is live, and C0 holds. Checked on
    /// every call, never cached.
    pub fn spawned_offered(&self, session_id: &str) -> bool {
        let (record, depth) = {
            let store = self.store.lock().unwrap();
            (
                store.get(session_id).ok().flatten(),
                store.lineage(session_id).map_or(0, |l| l.depth),
            )
        };
        depth > 0 && record.is_some_and(|r| !r.is_ended() && self.spawn_gate(&r.host_id).is_ok())
    }

    /// `target`, if it is one of `caller`'s descendants; else not found.
    pub fn descendant(&self, caller: &str, target: &str) -> AgentResult<SessionRecord> {
        let ours = self.store.lock().unwrap().is_descendant(caller, target)?;
        if !ours {
            return Err(AgentError::NotFound("no such agent among your descendants"));
        }
        self.get(target)
    }

    /// `caller`'s own children, oldest first.
    pub fn children_of(&self, caller: &str) -> AgentResult<Vec<SessionRecord>> {
        let children = self.store.lock().unwrap().children(caller)?;
        Ok(children
            .into_iter()
            .map(|r| self.with_activity(r))
            .collect())
    }

    /// The cap a start by `caller` would hit now, if any.
    pub fn start_cap(&self, caller: &str) -> AgentResult<Option<&'static str>> {
        let r = self.get(caller)?;
        let max_sessions = self
            .live
            .lock()
            .unwrap()
            .hosts
            .get(&r.host_id)
            .and_then(|link| link.capabilities.as_ref())
            .map_or(0, |caps| caps.max_sessions);
        Ok(self.store.lock().unwrap().start_cap(&r, max_sessions)?)
    }

    /// `report_result` (134h): the caller's own record, replacing an
    /// earlier report. Its waiting parent is woken.
    pub fn report_result(
        &self,
        caller: &str,
        outcome: &str,
        summary: &str,
        details: Option<&str>,
    ) -> AgentResult<SpawnResult> {
        if !RESULT_OUTCOMES.contains(&outcome) {
            return Err(AgentError::BadRequest(format!(
                "outcome is one of {}",
                RESULT_OUTCOMES.join(", ")
            )));
        }
        if summary.trim().is_empty() {
            return Err(AgentError::BadRequest("a summary is required".into()));
        }
        if summary.len() > MAX_SUMMARY_BYTES {
            return Err(AgentError::BadRequest(format!(
                "a summary is at most {MAX_SUMMARY_BYTES} bytes"
            )));
        }
        if details.is_some_and(|d| d.len() > MAX_DETAILS_BYTES) {
            return Err(AgentError::BadRequest(format!(
                "details are at most {MAX_DETAILS_BYTES} bytes"
            )));
        }
        let result = SpawnResult {
            outcome: outcome.to_string(),
            summary: summary.to_string(),
            details: details.map(str::to_string),
            reported_at: now(),
        };
        self.store.lock().unwrap().set_result(caller, &result)?;
        self.bump(caller);
        Ok(result)
    }

    /// `wait_agent` (134i): returns once every listed descendant has
    /// settled, or at `bound`, whichever is first; `true` when the bound
    /// ended it. Every id is checked against the caller's scope first, and
    /// the caller's offer is checked again on each wake, so a revocation
    /// ends the wait.
    pub async fn wait_agents(
        &self,
        caller: &str,
        ids: &[String],
        bound: Duration,
        still_offered: &(dyn Fn() -> bool + Send + Sync),
    ) -> AgentResult<(Vec<SessionRecord>, bool)> {
        if ids.is_empty() {
            return Err(AgentError::BadRequest("name at least one agent".into()));
        }
        if ids.len() > MAX_WAIT_IDS {
            return Err(AgentError::BadRequest(format!(
                "wait for at most {MAX_WAIT_IDS} agents at once"
            )));
        }
        for id in ids {
            self.descendant(caller, id)?;
        }
        let deadline = Instant::now() + bound;
        let mut watches: Vec<_> = ids.iter().map(|id| self.watch(id)).collect();
        loop {
            for w in &mut watches {
                w.borrow_and_update();
            }
            let mut now_seen = Vec::with_capacity(ids.len());
            let mut all = true;
            for id in ids {
                let r = self.descendant(caller, id)?;
                all &= settled(&r, self.result_of(id)?.as_ref());
                now_seen.push(r);
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if all || left.is_zero() {
                return Ok((now_seen, !all));
            }
            if !still_offered() {
                return Err(AgentError::Conflict("not_permitted".into()));
            }
            let any_change =
                futures_util::future::select_all(watches.iter_mut().map(|w| Box::pin(w.changed())));
            // The tick bounds how late a revocation or a dismissal is seen.
            let _ = tokio::time::timeout(left.min(Duration::from_millis(500)), any_change).await;
        }
    }

    /// Test-only: a running child of `parent`, recorded as an approval
    /// records one.
    #[cfg(test)]
    pub fn spawn_child_for_tests(&self, parent: &str, task: &str) -> SessionRecord {
        let child = self
            .store
            .lock()
            .unwrap()
            .spawn_child_for_tests(parent, task, false)
            .unwrap();
        self.report_children_for_tests(&child.host_id);
        child
    }

    /// Test-only: the switch's row written as a person's enable writes it,
    /// past the route's checks (C0, WS-9).
    #[cfg(test)]
    pub fn force_spawn_for_tests(&self, session_id: &str) {
        self.store
            .lock()
            .unwrap()
            .set_spawn(session_id, true, Some("dev_X"), &now())
            .unwrap();
    }
}
