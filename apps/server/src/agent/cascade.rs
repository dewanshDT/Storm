//! Decision 134-7: ending and cascades (C1, C2, C5, C8, L-3…L-8 of
//! "Decision 134 — dependencies"). A person's end of a parent stops every
//! live descendant at once (`human_end`), or after C9's notice when they let
//! children report first; "End all children" does the same and leaves the
//! parent running (`end_all_children`); a parent's own end is 134-6's
//! `parent_end`. Every stop is one `end` per target, retried once its host is
//! back, and the first cause wins. A tree is dismissed only as a unit.

use super::store::SessionRecord;
use super::store::spawn::{Cascade, DueStop, SpawnResult};
use super::{AgentError, AgentManager, AgentResult, Command, Crew, now};

/// One `end` the stop pass sent, or found its host away for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stop {
    pub session_id: String,
    pub host_id: String,
    pub cascade_id: String,
    pub cause: String,
    pub sent: bool,
}

/// What dismissing removed beyond the session asked for.
#[derive(Debug, Clone, Default)]
pub struct Dismissal {
    /// Every session dismissed, the one asked for first.
    pub sessions: Vec<String>,
    /// The crews it emptied, deleted with them (decision 138c).
    pub crews: Vec<Crew>,
    /// Each dismissed child and its host: its workspace is kept until its
    /// tree is dismissed (137g).
    pub children: Vec<(String, String)>,
}

/// How far back a resume chain is followed for C3.
const MAX_PREDECESSORS: usize = 64;

impl AgentManager {
    /// C1's confirmation: `id`'s descendants that still have a record,
    /// nearest first, as running (live, `unknown` included) and ended.
    pub fn descendants_of(
        &self,
        id: &str,
    ) -> AgentResult<(Vec<SessionRecord>, Vec<SessionRecord>)> {
        self.get(id)?;
        let ids = self.store.lock().unwrap().descendants(id, false)?;
        let mut running = Vec::new();
        let mut ended = Vec::new();
        for d in ids {
            let r = self.with_activity(self.get(&d)?);
            if r.is_ended() {
                ended.push(r);
            } else {
                running.push(r);
            }
        }
        Ok((running, ended))
    }

    /// A person ends `id` (C1). With live descendants it needs `confirmed`,
    /// the ids the person was shown: every live one must be among them, or
    /// nothing happens. The parent's `end` is sent first and the cascade
    /// opened under the same lock, so its report cannot come between; the
    /// targets' stops are the stop pass's. `report_first` gives them C9's
    /// notice and 30 s.
    pub fn end_by_person(
        &self,
        id: &str,
        confirmed: Option<&[String]>,
        report_first: bool,
    ) -> AgentResult<Option<Cascade>> {
        let mut store = self.store.lock().unwrap();
        let r = store
            .get(id)?
            .ok_or(AgentError::NotFound("no such session"))?;
        if r.is_ended() {
            return Err(AgentError::Conflict("the session has ended".into()));
        }
        let live = store.descendants(id, true)?;
        check_confirmed(&live, confirmed)?;
        // Store, then live: the documented order.
        self.send(
            &r.host_id,
            Command::End {
                session: r.id.clone(),
            },
        )?;
        if live.is_empty() {
            return Ok(None);
        }
        let cascade = crate::auth::identity::random_id("spc_");
        store.open_cascade(
            &cascade,
            "human_end",
            id,
            Some("stopped"),
            report_first,
            &now(),
        )?;
        let opened = store.cascade(&cascade)?.expect("just opened");
        drop(store);
        for target in &opened.targets {
            self.bump(&target.0);
        }
        self.requests.send_modify(|v| *v += 1);
        Ok(Some(opened))
    }

    /// "End all children" (C5): every live descendant of `id` is stopped
    /// as `cascade`; the parent keeps running, with its switch and its
    /// requests as they were (LT-8).
    pub fn end_children(
        &self,
        id: &str,
        confirmed: &[String],
        report_first: bool,
    ) -> AgentResult<Cascade> {
        let mut store = self.store.lock().unwrap();
        store
            .get(id)?
            .ok_or(AgentError::NotFound("no such session"))?;
        let live = store.descendants(id, true)?;
        if live.is_empty() {
            return Err(AgentError::Conflict("no_live_descendants".into()));
        }
        check_confirmed(&live, Some(confirmed))?;
        let cascade = crate::auth::identity::random_id("spc_");
        store.open_cascade(&cascade, "end_all_children", id, None, report_first, &now())?;
        let opened = store.cascade(&cascade)?.expect("just opened");
        drop(store);
        for target in &opened.targets {
            self.bump(&target.0);
        }
        Ok(opened)
    }

    pub fn cascade(&self, id: &str) -> AgentResult<Cascade> {
        self.store
            .lock()
            .unwrap()
            .cascade(id)?
            .ok_or(AgentError::NotFound("no such cascade"))
    }

    /// The stop pass (L-3, L-8): each cascade target whose stop is due is
    /// sent `end`, once per host link. A target whose host is away is
    /// `unreachable`, and is sent its `end` once the host is back; an `end`
    /// lost with a replaced link is sent again on the new one (a duplicate
    /// is harmless, L-3). Returns each `end` sent or found unreachable.
    pub fn stop_cascade_targets(&self, now: &str) -> AgentResult<Vec<Stop>> {
        let due: Vec<DueStop> = self.store.lock().unwrap().due_stops(now)?;
        let mut stops = Vec::new();
        for d in due {
            {
                let mut live = self.live.lock().unwrap();
                let generation = live.hosts.get(&d.host_id).map(|l| l.generation);
                if d.state == "unreachable" && generation.is_none() {
                    continue;
                }
                if generation.is_some() && live.stopping.get(&d.session_id) == generation.as_ref() {
                    continue;
                }
                // Generations start at 1: an `end` with no link is never a match.
                live.stopping
                    .insert(d.session_id.clone(), generation.unwrap_or(0));
            }
            let stop = |sent| Stop {
                session_id: d.session_id.clone(),
                host_id: d.host_id.clone(),
                cascade_id: d.cascade_id.clone(),
                cause: d.cause.clone(),
                sent,
            };
            match self.send(
                &d.host_id,
                Command::End {
                    session: d.session_id.clone(),
                },
            ) {
                Ok(()) => stops.push(stop(true)),
                Err(AgentError::HostOffline) => {
                    self.live.lock().unwrap().stopping.remove(&d.session_id);
                    if d.state == "pending" {
                        self.store.lock().unwrap().set_cascade_target(
                            &d.cascade_id,
                            &d.session_id,
                            "unreachable",
                            now,
                        )?;
                        stops.push(stop(false));
                    }
                }
                Err(e) => return Err(e),
            }
        }
        Ok(stops)
    }

    /// Whether Storm's stop explains this ended record, and the reason it
    /// then takes (L-4). A host restart is never relabelled (LT-5).
    pub(super) fn storm_stop(&self, r: &SessionRecord) -> AgentResult<Option<String>> {
        if !r.is_ended() || r.end_reason.as_deref() == Some("host_restart") {
            return Ok(None);
        }
        let sent = self.live.lock().unwrap().stopping.contains_key(&r.id);
        Ok(self
            .store
            .lock()
            .unwrap()
            .stop_reason(&r.id, &now(), sent)?)
    }

    /// C3: the results of the children of every session `caller` resumes,
    /// following its resume chain back. Read-only: none of them is in
    /// `caller`'s scope for anything else (LT-9).
    pub fn predecessor_results(
        &self,
        caller: &str,
    ) -> AgentResult<Vec<(String, SessionRecord, SpawnResult)>> {
        let store = self.store.lock().unwrap();
        let mut out = Vec::new();
        let mut at = store.get(caller)?.and_then(|r| r.resumed_from);
        for _ in 0..MAX_PREDECESSORS {
            let Some(predecessor) = at else { break };
            for child in store.children(&predecessor)? {
                if let Some(result) = store.result(&child.id)? {
                    out.push((predecessor.clone(), child, result));
                }
            }
            at = store.get(&predecessor)?.and_then(|r| r.resumed_from);
        }
        Ok(out)
    }

    /// Removes an ended session from the list (freeze §7.4), with its crew
    /// membership and role label. A session in a tree (decision 134, C8) is
    /// dismissed only with its whole tree, once every member has ended, and
    /// only when the person confirmed the tree (`tree`).
    pub fn dismiss_session(&self, id: &str, tree: bool) -> AgentResult<Dismissal> {
        let mut store = self.store.lock().unwrap();
        let r = store
            .get(id)?
            .ok_or(AgentError::NotFound("no such session"))?;
        if !r.is_ended() {
            return Err(AgentError::Conflict(
                "only an ended session can be dismissed".into(),
            ));
        }
        let root = store.lineage(id)?.root_session;
        let mut members: Vec<String> = std::iter::once(root.clone())
            .chain(store.descendants(&root, false)?)
            .filter(|m| matches!(store.get(m), Ok(Some(_))))
            .collect();
        let mut children = Vec::new();
        let crews = if members.len() <= 1 {
            members = vec![id.to_string()];
            store.delete(id)?.1.into_iter().collect()
        } else {
            for m in &members {
                if store.get(m)?.is_some_and(|s| !s.is_ended()) {
                    return Err(AgentError::Conflict("tree_live".into()));
                }
            }
            if !tree {
                return Err(AgentError::Conflict("confirm_tree".into()));
            }
            members.retain(|m| m != id);
            members.insert(0, id.to_string());
            for m in &members {
                if store.lineage(m)?.parent_session.is_some()
                    && let Some(r) = store.get(m)?
                {
                    children.push((m.clone(), r.host_id));
                }
            }
            store.delete_all(&members)?
        };
        drop(store);
        let mut live = self.live.lock().unwrap();
        for m in &members {
            live.sessions.remove(m);
        }
        Ok(Dismissal {
            sessions: members,
            crews,
            children,
        })
    }

    #[cfg(test)]
    pub fn set_resumed_from_for_tests(&self, session_id: &str, from: &str) {
        self.store
            .lock()
            .unwrap()
            .set_resumed_from_for_tests(session_id, from);
    }
}

/// Every live descendant must be among those the person confirmed (P-6):
/// one they were not shown is never stopped.
fn check_confirmed(live: &[String], confirmed: Option<&[String]>) -> AgentResult<()> {
    if live.is_empty() {
        return Ok(());
    }
    let Some(confirmed) = confirmed else {
        return Err(AgentError::Conflict("confirm_descendants".into()));
    };
    if live.iter().any(|d| !confirmed.contains(d)) {
        return Err(AgentError::Conflict("descendants_changed".into()));
    }
    Ok(())
}
