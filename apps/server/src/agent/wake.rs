//! Decisions 144c and 145d: telling a session's host that it has something
//! unread, so the host's waker can use its agent's documented wake. A parent
//! has unread events (its queue past its acknowledged cursor), a child an
//! unread message (one still `queued` in its inbox). The host is sent a
//! `wake` command with the kind and a mark that grows with each new item,
//! and 0 once nothing is unread; the host wakes the agent once per mark.
//!
//! The command holds a kind and a number, never content: the agent reads
//! what is waiting through `wait_agent` or `read_messages`. Only agents on
//! [`events::WAKE`] are told, and that list is empty until a gate
//! (G-144-W-<provider>, G-145-W-<provider>) passes. Nothing here writes to a
//! terminal (C9); the only command this file sends is the wake.

use super::{AgentManager, AgentResult, Command};

/// A parent's wake: unread events.
pub const EVENTS: &str = "events";
/// A child's wake: an unread message.
pub const MESSAGE: &str = "message";

impl AgentManager {
    /// Whether Storm may wake `provider`'s sessions.
    pub fn wake_on(&self, provider: &str) -> bool {
        self.wakes
            .lock()
            .unwrap()
            .iter()
            .any(|w| w.provider == provider)
    }

    /// Test-only: wakes as once their gates have passed.
    #[cfg(test)]
    pub fn gate_wake_for_tests(&self, wakes: &'static [super::events::GatedWake]) {
        *self.wakes.lock().unwrap() = wakes;
    }

    /// A host that reconnected is told every mark again.
    pub(super) fn forget_wakes_sent(&self, host_id: &str) {
        let sessions: Vec<String> = match self.store.lock().unwrap().for_host(host_id) {
            Ok(records) => records.into_iter().map(|r| r.id).collect(),
            Err(_) => return,
        };
        {
            let mut live = self.live.lock().unwrap();
            for id in sessions {
                if let Some(s) = live.sessions.get_mut(&id) {
                    s.wake_sent = None;
                }
            }
        }
        self.wake_poke.notify_one();
    }

    /// What `session_id` has unread now: its kind and mark, 0 for nothing.
    pub fn unread_mark(&self, session_id: &str) -> AgentResult<(&'static str, i64)> {
        let store = self.store.lock().unwrap();
        Ok(if store.lineage(session_id)?.parent_session.is_some() {
            (MESSAGE, store.unread_mark(session_id)?)
        } else {
            (EVENTS, store.unread_events_mark(session_id)?)
        })
    }

    /// Sends each running wakeable session's mark to its host when it
    /// changed. Returns how many were sent.
    pub fn sync_wakes(&self) -> AgentResult<usize> {
        if self.wakes.lock().unwrap().is_empty() {
            return Ok(0);
        }
        let records = self.store.lock().unwrap().list()?;
        let mut sent = 0;
        for r in records {
            if r.status != "running" || !self.wake_on(&r.provider) {
                continue;
            }
            let (kind, mark) = self.unread_mark(&r.id)?;
            let unchanged = self
                .live
                .lock()
                .unwrap()
                .sessions
                .get(&r.id)
                .is_some_and(|s| s.wake_sent == Some((kind, mark)));
            if unchanged {
                continue;
            }
            let wake = Command::Wake {
                session: r.id.clone(),
                kind: kind.into(),
                mark,
            };
            if self.send(&r.host_id, wake).is_ok() {
                self.live
                    .lock()
                    .unwrap()
                    .sessions
                    .entry(r.id.clone())
                    .or_insert_with(super::SessionLive::new)
                    .wake_sent = Some((kind, mark));
                sent += 1;
            }
        }
        Ok(sent)
    }

    /// A receiver that changes whenever an event is written: what the
    /// wake pass waits on.
    pub fn event_writes(&self) -> tokio::sync::watch::Receiver<u64> {
        self.store.lock().unwrap().events.subscribe()
    }
}
