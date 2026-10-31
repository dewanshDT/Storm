//! Decision 145e: a finished child is parked, not counted. A child that has
//! its round's result and has stayed idle for [`PARK_AFTER`], by a gated
//! idle signal of its agent's, is stopped the way Resume expects: its host
//! ends its process, keeps its conversation and its workspace, and reports
//! it `parked`. A parked child counts toward [`MAX_PARKED`] and its tree's
//! total, never toward the live cap. A message to it resumes it, in the same
//! session, from its kept conversation, with the message as its next
//! round's task (145d).
//!
//! No agent has a gated idle signal yet (`attention::GATED_SIGNALS`), so
//! nothing parks until one passes its gate.

use std::time::Duration;

use super::attention::IDLE;
use super::store::SessionRecord;
use super::{AgentError, AgentManager, AgentResult, Command, SessionLive, credential, events, now};

/// How long a child that has reported stays idle before it is parked.
pub const PARK_AFTER: Duration = Duration::from_secs(120);

/// The most parked children a host keeps: each holds its conversation and
/// its workspace there.
pub const MAX_PARKED: i64 = 12;

impl AgentManager {
    /// Whether Storm asked `session_id`'s host to park it; asking is
    /// answered once.
    pub(super) fn take_parking(&self, session_id: &str) -> bool {
        self.live
            .lock()
            .unwrap()
            .sessions
            .get_mut(session_id)
            .is_some_and(|s| std::mem::take(&mut s.parking))
    }

    /// The park pass: every running child whose round is reported, with
    /// nothing unread, idle by a gated signal for [`PARK_AFTER`], is sent
    /// `park`, while its host keeps fewer than [`MAX_PARKED`]. Returns those
    /// sent it.
    pub fn park_idle_children(&self) -> AgentResult<Vec<String>> {
        let at = self.clock();
        let idle: Vec<(String, Option<&'static str>)> = self
            .live
            .lock()
            .unwrap()
            .sessions
            .iter()
            .filter(|(_, s)| {
                !s.parking
                    && s.health.as_deref() == Some("idle")
                    && s.idle_since.is_some_and(|since| {
                        at.duration_since(since).unwrap_or_default() >= PARK_AFTER
                    })
            })
            .map(|(id, s)| (id.clone(), s.health_source))
            .collect();
        let mut parked = Vec::new();
        for (id, source) in idle {
            let due = {
                let store = self.store.lock().unwrap();
                match store.get(&id)? {
                    Some(r)
                        if r.status == "running"
                            && store.lineage(&id)?.parent_session.is_some()
                            && store.reported_round(&id, store.round(&id)?)?
                            && !store.has_unread(&id)?
                            && store.parked_on(&r.host_id)? < MAX_PARKED =>
                    {
                        Some(r)
                    }
                    _ => None,
                }
            };
            let Some(r) = due else { continue };
            // Only on the agent's own gated idle signal, never inferred.
            if !self.gated(&r.provider, IDLE, source) {
                continue;
            }
            self.live
                .lock()
                .unwrap()
                .sessions
                .entry(id.clone())
                .or_insert_with(SessionLive::new)
                .parking = true;
            match self.send(
                &r.host_id,
                Command::Park {
                    session: id.clone(),
                },
            ) {
                Ok(()) => {
                    tracing::info!(session = %id, "parking a child whose round is done");
                    parked.push(id);
                }
                Err(_) => {
                    self.take_parking(&id);
                }
            }
        }
        Ok(parked)
    }

    /// Resumes a parked child (145d): its host starts the same session again
    /// from the conversation it kept, in the workspace it kept, with the
    /// prompt that sends it to `read_messages`, where its next task waits.
    /// `deliverable` is the synced login a person's launch there would name
    /// now (143d). `false` when it is not parked, or its host is away (the
    /// park pass tries again once it is back).
    pub fn resume_parked(
        &self,
        session_id: &str,
        deliverable: Option<&'static str>,
    ) -> AgentResult<bool> {
        let r = self.get(session_id)?;
        if r.status != "parked" {
            return Ok(false);
        }
        let Some(caps) = self.host_live(&r.host_id).capabilities else {
            return Ok(false);
        };
        let (parent, mcp, model, permission_mode) = {
            let store = self.store.lock().unwrap();
            let launch = store.launch(session_id)?;
            (
                store.lineage(session_id)?.parent_session,
                store.grants_of(session_id)?,
                launch.as_ref().and_then(|l| l.model.clone()),
                launch.and_then(|l| l.permission_mode),
            )
        };
        let parent = parent.map(|p| self.get(&p)).transpose()?;
        let (Some(parent), Some(conversation)) = (parent, r.harness_session.clone()) else {
            // Nothing to resume from: it can never run again.
            return self
                .end_parked(&r, Some("start_failure".into()))
                .map(|()| false);
        };
        if !caps.resume.contains(&r.provider) {
            return self
                .end_parked(&r, Some("start_failure".into()))
                .map(|()| false);
        }
        let reads_storm = mcp
            .iter()
            .any(|g| g.id == crate::gateway::connections::BUILTIN_ID);
        let credential = credential::decide(&r.provider, deliverable, &caps);
        let start = Command::Start {
            session: r.id.clone(),
            workspace: parent.workspace.clone(),
            provider: r.provider.clone(),
            interaction: r.interaction.clone(),
            terminal: super::TerminalSize {
                cols: r.cols,
                rows: r.rows,
            },
            mcp,
            context: reads_storm,
            resume: Some(conversation),
            provider_credential: credential.kind().map(Into::into),
            model,
            permission_mode,
            spawn: false,
            // Its own kept conversation: the same session, resumed.
            resumed_from: Some(r.id.clone()),
            child_of: Some(parent.id.clone()),
        };
        self.live
            .lock()
            .unwrap()
            .sessions
            .entry(r.id.clone())
            .or_insert_with(SessionLive::new)
            .credential = credential.kind();
        match self.send(&r.host_id, start) {
            Ok(()) => {}
            Err(AgentError::HostOffline) => return Ok(false),
            Err(e) => return Err(e),
        }
        let mut resumed = r;
        resumed.status = "starting".into();
        self.update_record(&resumed, events::STORM)?;
        self.bump(&resumed.id);
        tracing::info!(session = %resumed.id, "resuming a parked child for its next round");
        Ok(true)
    }

    /// The parked children with a message waiting, as the pass that resumes
    /// any a host was away for.
    pub fn parked_with_unread(&self) -> AgentResult<Vec<SessionRecord>> {
        let store = self.store.lock().unwrap();
        let mut out = Vec::new();
        for r in store.list()? {
            if r.status == "parked" && store.has_unread(&r.id)? {
                out.push(r);
            }
        }
        Ok(out)
    }

    /// A parked child ended for good: no host runs it, so its end is
    /// recorded here, as its host's report of a stop would be, with its
    /// tree's rules (C1, L-4) applied in the same transaction.
    pub(super) fn end_parked(
        &self,
        r: &SessionRecord,
        end_reason: Option<String>,
    ) -> AgentResult<()> {
        let mut r = r.clone();
        r.status = if end_reason.as_deref() == Some("start_failure") {
            "failed".into()
        } else {
            "stopped".into()
        };
        r.end_reason = end_reason;
        r.ended_at.get_or_insert_with(now);
        self.update_record(&r, events::STORM)?;
        self.forget_stop(&r);
        self.bump(&r.id);
        Ok(())
    }

    /// Test-only: `session_id` parked, as its host's answer to `park`
    /// leaves it.
    #[cfg(test)]
    pub fn park_for_tests(&self, host_id: &str, session_id: &str) {
        self.live
            .lock()
            .unwrap()
            .sessions
            .entry(session_id.to_string())
            .or_insert_with(SessionLive::new)
            .parking = true;
        let report: super::StatusReport =
            serde_json::from_value(serde_json::json!({"status": "parked"})).unwrap();
        self.host_status(host_id, session_id, &report).unwrap();
    }
}
