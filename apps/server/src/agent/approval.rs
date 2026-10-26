//! The approval engine (decision 134-3): requests for start, message and end
//! wait for a person's decision (134o, A5), under P-1…P-6 of "Decision 134 —
//! dependencies".
//!
//! Dark: C0 refuses on every host until a host's own report says its
//! identity and isolation are verified.

use std::time::{Duration, Instant};

use super::credential::{self, CredentialOutcome};
use super::store::SessionRecord;
use super::store::spawn::{
    Decision, HostNow, MAX_MESSAGE_BYTES, MAX_TASK_BYTES, NewRequest, Refusal, Request,
};
use super::{AgentError, AgentManager, AgentResult, Capabilities, now};

/// The agents a host can start a child in besides its parent's own (143a):
/// installed for the terminal, given Storm's servers there (`mcp: true`,
/// 130a), and never `shell`. `(id, kind)`, in the host's order.
pub fn child_agents(caps: &Capabilities) -> Vec<(String, String)> {
    caps.providers
        .iter()
        .filter(|p| {
            p.id != "shell"
                && p.available
                && p.mcp == Some(true)
                && p.interactions.iter().any(|i| i == "terminal")
        })
        .map(|p| (p.id.clone(), p.kind.clone()))
        .collect()
}

impl AgentManager {
    /// C0 for the store's checks, read from the live link while the store's
    /// lock is held, and what the link reports when it holds.
    fn host_check(&self, host_id: &str) -> std::result::Result<HostNow, &'static str> {
        self.spawn_gate(host_id)?;
        Ok(self
            .with_live(host_id, |caps| HostNow {
                max_sessions: caps.max_sessions,
                agents: child_agents(caps),
            })
            .unwrap_or_default())
    }

    /// What `session_id` may ask a child to run now (143b): its own agent
    /// first, then each other its host's live link reports for a child.
    pub fn startable_agents(&self, session_id: &str) -> AgentResult<Vec<String>> {
        let r = self.get(session_id)?;
        let others = self.with_live(&r.host_id, child_agents).unwrap_or_default();
        Ok(std::iter::once(r.provider.clone())
            .chain(
                others
                    .into_iter()
                    .map(|(id, _)| id)
                    .filter(|id| *id != r.provider),
            )
            .collect())
    }

    /// The credential a child running `agent` on `host_id` would start on
    /// now (143d): what a person's own launch of it there would name, from
    /// `deliverable`, the synced login the account would deliver to that
    /// host. A host away is told nothing.
    pub fn child_credential(
        &self,
        host_id: &str,
        agent: &str,
        deliverable: Option<&'static str>,
    ) -> CredentialOutcome {
        let caps = self.with_live(host_id, Clone::clone).unwrap_or_default();
        credential::decide(agent, deliverable, &caps)
    }

    fn requests_changed(&self) {
        self.requests.send_modify(|v| *v += 1);
    }

    /// P-2: a session's agent asks for a start, a message or an end. The
    /// inner error is the refusal, naming the cap for `limit`; nothing is
    /// recorded then.
    pub fn create_request(
        &self,
        requester: &str,
        new: NewRequest,
    ) -> AgentResult<std::result::Result<Request, Refusal>> {
        match &new {
            NewRequest::Start { task, .. } if task.len() > MAX_TASK_BYTES => {
                return Err(AgentError::BadRequest(format!(
                    "a task is at most {MAX_TASK_BYTES} bytes"
                )));
            }
            NewRequest::Message { text, .. } if text.len() > MAX_MESSAGE_BYTES => {
                return Err(AgentError::BadRequest(format!(
                    "a message is at most {MAX_MESSAGE_BYTES} bytes"
                )));
            }
            _ => {}
        }
        self.get(requester)?;
        let id = crate::auth::identity::random_id("agr_");
        let created =
            self.store
                .lock()
                .unwrap()
                .create_request(&id, requester, &new, &now(), &|host| self.host_check(host))?;
        if created.is_ok() {
            self.requests_changed();
        }
        Ok(created)
    }

    pub fn request(&self, id: &str) -> AgentResult<Request> {
        self.store
            .lock()
            .unwrap()
            .request(id)?
            .ok_or(AgentError::NotFound("no such request"))
    }

    /// P-3: a person's device approves. `write_vault` is D2's toggle. A
    /// start's child is told to its host after the decision commits, with
    /// `deliverable`, the synced login a person's launch on that host would
    /// name now (143d).
    pub fn approve_request(
        &self,
        id: &str,
        device: &str,
        write_vault: bool,
        deliverable: Option<&'static str>,
    ) -> AgentResult<Decision> {
        let decision = self
            .store
            .lock()
            .unwrap()
            .approve_request(id, device, write_vault, &now(), &|host| {
                self.host_check(host)
            })?
            .ok_or(AgentError::NotFound("no such request"))?;
        if let Decision::Decided { child, request, .. } = &decision {
            if let Some(child) = child {
                self.dispatch_spawned(child, deliverable)?;
            }
            match (request.outcome.as_deref(), request.target.as_deref()) {
                // A delivered message wakes the child's read (134-6).
                (Some("delivered"), Some(target)) => self.bump(target),
                // An approved end (134-7). Its own descendants are stopped
                // when its end is recorded (`human_end`, C1); a host that
                // dropped since the re-check leaves it running, and the
                // parent may ask again.
                (Some("ended"), Some(target)) => {
                    if let Err(e) = self.send_end(target) {
                        tracing::warn!(session = target, error = ?e, "approved end not sent");
                    }
                }
                _ => {}
            }
            self.requests_changed();
        }
        Ok(decision)
    }

    pub fn deny_request(&self, id: &str, device: &str) -> AgentResult<Decision> {
        let decision = self
            .store
            .lock()
            .unwrap()
            .deny_request(id, device, &now())?
            .ok_or(AgentError::NotFound("no such request"))?;
        if matches!(decision, Decision::Decided { .. }) {
            self.requests_changed();
        }
        Ok(decision)
    }

    /// Every request a session made, oldest first.
    pub fn requests_of(&self, requester: &str) -> AgentResult<Vec<Request>> {
        Ok(self.store.lock().unwrap().requests_of(requester)?)
    }

    /// The sweeper's pass (P-5): the requests it expired.
    pub fn expire_requests(&self) -> AgentResult<Vec<Request>> {
        let swept = self.store.lock().unwrap().expire_requests(&now())?;
        if !swept.is_empty() {
            self.requests_changed();
        }
        Ok(swept)
    }

    /// Waits up to `timeout` for a request to be decided, and answers with it
    /// as it then is: what 134-5's tools wait on, below each adapter's
    /// tool-call timeout (134i).
    pub async fn wait_request(&self, id: &str, timeout: Duration) -> AgentResult<Request> {
        let deadline = Instant::now() + timeout;
        let mut changed = self.requests.subscribe();
        loop {
            changed.borrow_and_update();
            let r = self.request(id)?;
            let left = deadline.saturating_duration_since(Instant::now());
            if r.state == "decided" || left.is_zero() {
                return Ok(r);
            }
            // An ending or a dismissal refuses requests without a bump; the
            // tick bounds how late a waiter hears of it.
            let _ =
                tokio::time::timeout(left.min(Duration::from_millis(500)), changed.changed()).await;
        }
    }

    /// After anything the host says about itself: P-4 refuses the pending
    /// requests of its sessions while C0 fails on a live link, and its
    /// agents with spawn on are told their tools changed when C0 flips. A
    /// host that is away keeps its requests until they expire (L-6), and
    /// hears of a flip when it is back.
    pub(super) fn reassess_spawn_gate(&self, host_id: &str) {
        let gate = self.spawn_gate(host_id);
        let flipped = {
            let mut live = self.live.lock().unwrap();
            if !live.hosts.contains_key(host_id) {
                return;
            }
            // Never told counts as not offered: no spawn tool is listed then.
            let told = live.c0.insert(host_id.to_string(), gate.is_ok());
            told.unwrap_or(false) != gate.is_ok()
        };
        if let Err(reason) = gate {
            match self
                .store
                .lock()
                .unwrap()
                .refuse_requests_on_host(host_id, reason, &now())
            {
                Ok(0) => {}
                Ok(refused) => {
                    tracing::info!(host = host_id, reason, refused, "spawn requests refused");
                    self.requests_changed();
                }
                Err(e) => {
                    tracing::warn!(error = %e, host = host_id, "could not refuse spawn requests")
                }
            }
        }
        if flipped {
            self.tell_spawning_agents(host_id);
        }
    }

    fn tell_spawning_agents(&self, host_id: &str) {
        let sessions = {
            let store = self.store.lock().unwrap();
            store
                .for_host(host_id)
                .unwrap_or_default()
                .into_iter()
                .filter(|r| !r.is_ended() && store.spawn_enabled(&r.id).unwrap_or(false))
                .filter(|r| {
                    matches!(
                        store.grant(&r.id, crate::gateway::connections::BUILTIN_ID),
                        Ok(Some(_))
                    )
                })
                .map(|r| r.id)
                .collect::<Vec<_>>()
        };
        for session in sessions {
            self.mcp_message(
                &session,
                crate::gateway::connections::BUILTIN_ID,
                serde_json::json!({"jsonrpc": "2.0", "method": "notifications/tools/list_changed"}),
            );
        }
    }

    /// The host is told of an approved child after commit (P-3): its start
    /// names the parent's workspace and the parent, so the host makes the
    /// child's own workspace from it (137-1) before it starts; the child
    /// runs its own agent (143a) with its narrowed grants, the spawned
    /// opening prompt and the asks (134-8), on the credential a person's
    /// launch of that agent there would get (143d). The decision cannot change once
    /// made, so a start that fails here or on the host is the child's own
    /// `failed`, and the request keeps `started`.
    pub(super) fn dispatch_spawned(
        &self,
        child: &SessionRecord,
        deliverable: Option<&'static str>,
    ) -> AgentResult<()> {
        let parent = self
            .store
            .lock()
            .unwrap()
            .lineage(&child.id)?
            .parent_session;
        let parent = parent.map(|p| self.get(&p)).transpose()?;
        let start = parent.and_then(|parent| {
            let caps = self.host_live(&child.host_id).capabilities?;
            let (mcp, asks) = {
                let store = self.store.lock().unwrap();
                (
                    store.grants_of(&child.id).ok()?,
                    store.spawn_asks(&child.id).ok()?,
                )
            };
            let reads_storm = mcp
                .iter()
                .any(|g| g.id == crate::gateway::connections::BUILTIN_ID);
            let credential = credential::decide(&child.provider, deliverable, &caps);
            Some((
                credential.kind(),
                super::Command::Start {
                    session: child.id.clone(),
                    workspace: parent.workspace.clone(),
                    provider: child.provider.clone(),
                    interaction: child.interaction.clone(),
                    terminal: super::TerminalSize {
                        cols: child.cols,
                        rows: child.rows,
                    },
                    mcp,
                    context: reads_storm,
                    resume: None,
                    provider_credential: credential.kind().map(Into::into),
                    spawn_asks: asks.then_some(super::SpawnStart { spawned: true }),
                    resumed_from: None,
                    child_of: Some(parent.id.clone()),
                },
            ))
        });
        let sent = match start {
            Some((credential, command)) => {
                let mut session_live = super::SessionLive::new();
                session_live.credential = credential;
                self.live
                    .lock()
                    .unwrap()
                    .sessions
                    .insert(child.id.clone(), session_live);
                self.send(&child.host_id, command).is_ok()
            }
            None => false,
        };
        let mut r = child.clone();
        if sent {
            r.status = "starting".into();
        } else {
            r.status = "failed".into();
            r.end_reason = Some("start_failure".into());
            r.ended_at = Some(now());
            tracing::warn!(
                session = r.id,
                "spawned session not started: its host is away"
            );
        }
        self.store.lock().unwrap().update(&r)?;
        self.bump(&r.id);
        Ok(())
    }

    /// 139-D5: every spawned child still `creating`, failed `server_restart`,
    /// with its parent. Only at startup, before any host links: the status
    /// check in the same update keeps it from ending a child past `creating`.
    pub fn fail_children_left_creating(&self) -> AgentResult<Vec<(SessionRecord, String)>> {
        let failed = self
            .store
            .lock()
            .unwrap()
            .fail_children_left_creating(&now())?;
        for (r, _) in &failed {
            self.bump(&r.id);
        }
        Ok(failed)
    }

    fn send_end(&self, session: &str) -> AgentResult<()> {
        let r = self.get(session)?;
        self.send(
            &r.host_id,
            super::Command::End {
                session: r.id.clone(),
            },
        )
    }

    /// Test-only: a request written as it is, past every check.
    #[cfg(test)]
    pub fn insert_request_for_tests(&self, r: &Request) {
        self.store.lock().unwrap().insert_request(r).unwrap();
    }

    /// Test-only: the host's inventory says its isolation dropped, as a
    /// helper that stops answering makes it say (136j).
    #[cfg(test)]
    pub fn lose_isolation_for_tests(&self, host_id: &str) {
        if let Some(mut caps) = self.host_live(host_id).capabilities {
            caps.isolation = super::Isolation::unavailable("helper");
            self.inventory(host_id, caps).unwrap();
        }
    }
}
