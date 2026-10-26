//! Decision 134-6: a child's inbox (134j) and C9's notice. A parent's
//! message reaches only its own descendant's inbox, after a person approves
//! it (A2, 134o); a child reads it with `read_messages`. Nothing here, or
//! anywhere an agent reaches, writes to a terminal: a child learns of a
//! message only by asking.

use std::time::{Duration, Instant};

use super::store::spawn::Message;
use super::{AgentError, AgentManager, AgentResult};

impl AgentManager {
    /// `read_messages` (134j): at least once, bounded like `wait_agent`
    /// (134i). Returns at once when something is there; otherwise waits up
    /// to `bound` for a message, re-checking the caller's offer on each
    /// wake. `true` when the bound ended it with nothing.
    pub async fn read_messages(
        &self,
        caller: &str,
        after: Option<i64>,
        bound: Duration,
        still_offered: &(dyn Fn() -> bool + Send + Sync),
    ) -> AgentResult<(Vec<Message>, bool)> {
        if after.is_some_and(|a| a < 0) {
            return Err(AgentError::BadRequest(
                "after is a message's number, from 1".into(),
            ));
        }
        let deadline = Instant::now() + bound;
        let mut changed = self.watch(caller);
        let mut after = after;
        loop {
            changed.borrow_and_update();
            let read = self
                .store
                .lock()
                .unwrap()
                .read_messages(caller, after.take())?;
            let left = deadline.saturating_duration_since(Instant::now());
            if !read.is_empty() || left.is_zero() {
                let timed_out = read.is_empty();
                return Ok((read, timed_out));
            }
            if !still_offered() {
                return Err(AgentError::Conflict("not_permitted".into()));
            }
            // A notice is queued as its parent's end is recorded, which
            // bumps the parent, not the child: the tick bounds how late a
            // waiting child hears of it.
            let _ =
                tokio::time::timeout(left.min(Duration::from_millis(500)), changed.changed()).await;
        }
    }

    /// The messages `sender` asked to put in `target`'s inbox, with their
    /// states: how a parent sees delivery and acknowledgement.
    pub fn messages_from(&self, sender: &str, target: &str) -> AgentResult<Vec<Message>> {
        Ok(self.store.lock().unwrap().messages_from(sender, target)?)
    }

    #[cfg(test)]
    pub fn requests_of_for_tests(&self, requester: &str) -> Vec<super::store::spawn::Request> {
        self.store.lock().unwrap().requests_of(requester).unwrap()
    }

    #[cfg(test)]
    pub fn inbox_of(&self, session_id: &str) -> AgentResult<Vec<Message>> {
        Ok(self.store.lock().unwrap().inbox(session_id)?)
    }
}
