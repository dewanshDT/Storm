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

/// Whether a session counts as done for a wait: it ended, or it reported
/// for the round it is in (145). Idle is never done (134h).
pub fn settled(r: &SessionRecord, result: Option<&SpawnResult>, round: i64) -> bool {
    r.is_ended() || result.is_some_and(|res| res.round >= round)
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

    /// Every round's result, oldest first (145).
    pub fn results_of(&self, session_id: &str) -> AgentResult<Vec<SpawnResult>> {
        Ok(self.store.lock().unwrap().results(session_id)?)
    }

    /// The round a child is in (145).
    pub fn round_of(&self, session_id: &str) -> AgentResult<i64> {
        Ok(self.store.lock().unwrap().round(session_id)?)
    }

    /// Whether a descendant has settled for its round now.
    pub fn settled_now(&self, r: &SessionRecord) -> AgentResult<bool> {
        Ok(settled(
            r,
            self.result_of(&r.id)?.as_ref(),
            self.round_of(&r.id)?,
        ))
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
            round: 0,
            outcome: outcome.to_string(),
            summary: summary.to_string(),
            details: details.map(str::to_string),
            reported_at: now(),
        };
        let live = self.live_signals(caller);
        let result = {
            let store = self.store.lock().unwrap();
            store.record_result(caller, &result, &live)?;
            store.result(caller)?.expect("just recorded")
        };
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
                all &= self.settled_now(&r)?;
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

    /// `start_agent {background: false}` (145b): waits up to `bound` for a
    /// child to report its first result or end, re-checking the caller's
    /// offer on each wake. The child as it then is, with its result.
    pub async fn wait_first_result(
        &self,
        child: &str,
        bound: Duration,
        still_offered: &(dyn Fn() -> bool + Send + Sync),
    ) -> AgentResult<(SessionRecord, Option<SpawnResult>)> {
        let deadline = Instant::now() + bound;
        let mut changed = self.watch(child);
        loop {
            changed.borrow_and_update();
            let r = self.get(child)?;
            let result = self.result_of(child)?;
            let left = deadline.saturating_duration_since(Instant::now());
            if result.is_some() || r.is_ended() || left.is_zero() {
                return Ok((r, result));
            }
            if !still_offered() {
                return Err(AgentError::Conflict("not_permitted".into()));
            }
            let _ =
                tokio::time::timeout(left.min(Duration::from_millis(500)), changed.changed()).await;
        }
    }

    /// Test-only: a running child of `parent`, recorded as a start records
    /// one.
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

    /// Test-only: a root `parent` and its child `child` on `host`, the
    /// child in `status`, recorded as an approval records one.
    #[cfg(test)]
    pub fn child_session_for_tests(
        &self,
        owner: &str,
        host: &str,
        parent: &str,
        child: &str,
        status: &str,
    ) {
        let mut store = self.store.lock().unwrap();
        if store.get(parent).unwrap().is_none() {
            store
                .insert(&test_record(owner, host, parent, "completed"))
                .unwrap();
        }
        store
            .insert_spawned(
                &test_record(owner, host, child, status),
                parent,
                "a task",
                &now(),
            )
            .unwrap();
    }

    /// Test-only: a top-level session on `host` in `status`.
    #[cfg(test)]
    pub fn session_for_tests(&self, owner: &str, host: &str, id: &str, status: &str) {
        self.store
            .lock()
            .unwrap()
            .insert(&test_record(owner, host, id, status))
            .unwrap();
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

#[cfg(test)]
fn test_record(owner: &str, host: &str, id: &str, status: &str) -> SessionRecord {
    SessionRecord {
        id: id.into(),
        owner_user_id: owner.into(),
        host_id: host.into(),
        workspace: "app".into(),
        provider: "claude-code".into(),
        provider_kind: "cli".into(),
        interaction: "terminal".into(),
        provider_fallback: None,
        status: status.into(),
        end_reason: None,
        signal: None,
        exit_code: None,
        created_at: now(),
        started_at: None,
        ended_at: None,
        last_activity: None,
        egress: "host".into(),
        cols: 80,
        rows: 24,
        title: None,
        activity: None,
        harness_session: None,
        resumed_from: None,
    }
}
