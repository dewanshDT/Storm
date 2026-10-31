//! A parent's acts (decision 134-3 as amended by 145a): a start, a message
//! and an end act at once, with no approval card. The launch toggle is the
//! person's standing approval (141e); every check of 134 stays (C0, the
//! switch, scope, the caps), and every act is recorded and audited.
//!
//! Dark: C0 refuses on every host until a host's own report says its
//! identity and isolation are verified.

use super::credential;
use super::store::SessionRecord;
use super::store::spawn::{
    HostNow, MAX_MESSAGE_BYTES, MAX_TASK_BYTES, Refusal, Request, StartPayload,
};
use super::{AgentError, AgentManager, AgentResult, Capabilities, now};

/// The agents a host can start a child in besides its parent's own (143a):
/// installed for the terminal, given Storm's servers there (`mcp: true`,
/// 130a), and never `shell`; since 148a, only an agent that can run without
/// ever asking. `(id, kind)`, in the host's order.
pub fn child_agents(caps: &Capabilities) -> Vec<(String, String)> {
    caps.providers
        .iter()
        .filter(|p| {
            p.id != "shell"
                && super::permission::unattended(&p.id)
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
    /// first, when it can run as a child (148a), then each other its host's
    /// live link reports for a child.
    pub fn startable_agents(&self, session_id: &str) -> AgentResult<Vec<String>> {
        let r = self.get(session_id)?;
        let others = self.with_live(&r.host_id, child_agents).unwrap_or_default();
        Ok(std::iter::once(r.provider.clone())
            .filter(|own| super::permission::unattended(own))
            .chain(
                others
                    .into_iter()
                    .map(|(id, _)| id)
                    .filter(|id| *id != r.provider),
            )
            .collect())
    }

    /// A start (145b), at once: the inner error is the refusal, naming the
    /// cap for `limit`; nothing is recorded then. The child's host is told
    /// after commit, with `deliverable`, the synced login a person's launch
    /// on that host would name now (143d).
    pub fn start_child(
        &self,
        requester: &str,
        payload: StartPayload,
        deliverable: Option<&'static str>,
    ) -> AgentResult<std::result::Result<SessionRecord, Refusal>> {
        if payload.task.len() > MAX_TASK_BYTES {
            return Err(AgentError::BadRequest(format!(
                "a task is at most {MAX_TASK_BYTES} bytes"
            )));
        }
        self.get(requester)?;
        let act = crate::auth::identity::random_id("agr_");
        let started =
            self.store
                .lock()
                .unwrap()
                .start_child(&act, requester, &payload, &now(), &|host| {
                    self.host_check(host)
                })?;
        if let Ok(child) = &started {
            self.dispatch_spawned(child, deliverable)?;
        }
        Ok(started.map(|child| self.get(&child.id).unwrap_or(child)))
    }

    /// A message to a live descendant (145d), at once: the round it belongs
    /// to, and the child's waiting read is woken.
    pub fn message_child(
        &self,
        requester: &str,
        target: &str,
        text: &str,
    ) -> AgentResult<std::result::Result<i64, Refusal>> {
        if text.len() > MAX_MESSAGE_BYTES {
            return Err(AgentError::BadRequest(format!(
                "a message is at most {MAX_MESSAGE_BYTES} bytes"
            )));
        }
        self.get(requester)?;
        let act = crate::auth::identity::random_id("agr_");
        let sent = self.store.lock().unwrap().message_child(
            &act,
            requester,
            target,
            text,
            &now(),
            &|host| self.host_check(host),
        )?;
        if sent.is_ok() {
            self.bump(target);
            self.wake_poke.notify_one();
        }
        Ok(sent)
    }

    /// An end of a live descendant (134-7), at once: it is sent `end`, and
    /// its own descendants are stopped when its end is recorded
    /// (`human_end`, C1). A host that dropped since the check leaves it
    /// running, and the parent may end it again.
    pub fn end_child(
        &self,
        requester: &str,
        target: &str,
    ) -> AgentResult<std::result::Result<(), Refusal>> {
        self.get(requester)?;
        let act = crate::auth::identity::random_id("agr_");
        let ended =
            self.store
                .lock()
                .unwrap()
                .end_child(&act, requester, target, &now(), &|host| {
                    self.host_check(host)
                })?;
        if ended.is_ok() {
            // A parked child runs nowhere: its end is recorded here (145e).
            let r = self.get(target)?;
            if r.status == "parked" {
                self.end_parked(&r, None)?;
            } else if let Err(e) = self.send_end(target) {
                tracing::warn!(session = target, error = ?e, "an agent's end not sent");
            }
        }
        Ok(ended)
    }

    /// Every act a session made, oldest first.
    pub fn requests_of(&self, requester: &str) -> AgentResult<Vec<Request>> {
        Ok(self.store.lock().unwrap().requests_of(requester)?)
    }

    /// After anything the host says about itself: its agents with spawn on
    /// are told their tools changed when C0 flips. A host that is away hears
    /// of a flip when it is back.
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

    /// The host is told of a child after its start commits: its start names
    /// the parent's workspace and the parent, so the host makes the child's
    /// own workspace from it (137-1) before it starts; the child runs its
    /// own agent (143a) on its model (145b) with its narrowed grants and the
    /// spawned opening prompt, on the credential a person's launch of that
    /// agent there would get (143d). A start that fails here or on the host
    /// is the child's own `failed`, and its act keeps `started`.
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
            let (mcp, model, permission_mode) = {
                let store = self.store.lock().unwrap();
                let launch = store.launch(&child.id).ok()?;
                (
                    store.grants_of(&child.id).ok()?,
                    launch.as_ref().and_then(|l| l.model.clone()),
                    launch.and_then(|l| l.permission_mode),
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
                    model,
                    permission_mode,
                    spawn: false,
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
        self.update_record(&r, super::events::STORM)?;
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

    /// Test-only: a request written as it is, past every check, as an
    /// older server left one.
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
