//! "Some session changed", for `GET /v1/agent/events` (decision 154, AM109).
//!
//! One manager-wide generation, bumped with every session's own version
//! ([`super::SessionLive::bump`]) and by [`AgentManager::views_changed`] for
//! what no session's version shows. A subscriber waits on the generation and
//! compares [`AgentManager::session_versions`] with what it last built.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use tokio::sync::watch;

use super::AgentManager;

pub(crate) struct Changes {
    generation: watch::Sender<u64>,
    /// Bumped when every view may have changed: crews, roles, a dismissal,
    /// a host leaving.
    all: AtomicU64,
    /// This run of the server: output offsets restart with it.
    epoch: String,
}

impl Changes {
    pub(crate) fn new() -> Self {
        Self {
            generation: watch::channel(0).0,
            all: AtomicU64::new(0),
            epoch: crate::auth::identity::random_id("run_"),
        }
    }

    pub(crate) fn touch(&self) {
        self.generation.send_modify(|g| *g += 1);
    }
}

impl AgentManager {
    pub fn subscribe_changes(&self) -> watch::Receiver<u64> {
        self.changes.generation.subscribe()
    }

    /// Every session with live state, at its current version.
    pub fn session_versions(&self) -> HashMap<String, u64> {
        self.live
            .lock()
            .unwrap()
            .sessions
            .iter()
            .map(|(id, s)| (id.clone(), *s.version.borrow()))
            .collect()
    }

    /// Something every view may show changed without any session's
    /// version moving.
    pub fn views_changed(&self) {
        self.changes.all.fetch_add(1, Ordering::SeqCst);
        self.changes.touch();
    }

    pub fn views_generation(&self) -> u64 {
        self.changes.all.load(Ordering::SeqCst)
    }

    pub fn epoch(&self) -> &str {
        &self.changes.epoch
    }
}
