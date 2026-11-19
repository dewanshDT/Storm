pub mod forwarder;
pub mod preview;
pub mod queue;

use std::path::Path;

use anyhow::Result;

use preview::PreviewStore;
use queue::Queue;

pub struct Feedback {
    pub previews: std::sync::Mutex<PreviewStore>,
    pub queue: Queue,
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
