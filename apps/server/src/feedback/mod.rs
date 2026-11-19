//! In-app feedback (decision 162): the preview store, the durable send
//! queue and the background forwarder. The operations themselves —
//! building a preview's bundle, validating a send, persisting it — are in
//! [`crate::ops`], same as every other feature; this module is state and
//! the forwarder's loop.

pub mod forwarder;
pub mod preview;
pub mod queue;

use std::path::Path;

use anyhow::Result;

use preview::PreviewStore;
use queue::Queue;

/// Threaded through `AppState`, one per process — never a global, for the
/// reason every other piece of live state here is not one: a test opens
/// several servers in one process and must not have them share a queue.
pub struct Feedback {
    pub previews: std::sync::Mutex<PreviewStore>,
    pub queue: Queue,
    /// Lets a fresh send skip the forwarder's poll interval rather than
    /// waiting out `POLL_INTERVAL` for its first attempt.
    pub wake: tokio::sync::Notify,
}

impl Feedback {
    pub fn open(state_dir: &Path) -> Result<Self> {
        let queue = Queue::open(&state_dir.join("feedback").join("feedback.db"))?;
        Ok(Self {
            previews: std::sync::Mutex::new(PreviewStore::default()),
            queue,
            wake: tokio::sync::Notify::new(),
        })
    }
}
