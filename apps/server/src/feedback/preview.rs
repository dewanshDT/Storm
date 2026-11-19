use std::collections::HashMap;
use std::time::{Duration, Instant};

// Scaled down under test, but not so far that a slow CI runner expires one
// mid-test.
pub const TTL: Duration = if cfg!(test) {
    Duration::from_secs(2)
} else {
    Duration::from_secs(10 * 60)
};

const MAX_PREVIEWS: usize = 500;

#[derive(Debug, Clone)]
pub struct Preview {
    pub kind: String,
    pub client: serde_json::Value,
    pub bundle: serde_json::Value,
    created_at: Instant,
}

impl Preview {
    pub fn expires_at(&self) -> Instant {
        self.created_at + TTL
    }

    fn is_expired(&self, now: Instant) -> bool {
        now >= self.expires_at()
    }
}

#[derive(Default)]
pub struct PreviewStore {
    previews: HashMap<String, Preview>,
}

impl PreviewStore {
    pub fn insert(
        &mut self,
        preview_id: String,
        kind: String,
        client: serde_json::Value,
        bundle: serde_json::Value,
    ) -> Instant {
        let now = Instant::now();
        self.previews.retain(|_, p| !p.is_expired(now));
        if self.previews.len() >= MAX_PREVIEWS
            && let Some(oldest) = self
                .previews
                .iter()
                .min_by_key(|(_, p)| p.created_at)
                .map(|(id, _)| id.clone())
        {
            self.previews.remove(&oldest);
        }
        let preview = Preview {
            kind,
            client,
            bundle,
            created_at: now,
        };
        let expires_at = preview.expires_at();
        self.previews.insert(preview_id, preview);
        expires_at
    }

    pub fn get(&self, preview_id: &str) -> Option<Preview> {
        let preview = self.previews.get(preview_id)?;
        if preview.is_expired(Instant::now()) {
            return None;
        }
        Some(preview.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stored_preview_is_returned_until_it_expires() {
        let mut store = PreviewStore::default();
        store.insert(
            "prv_1".into(),
            "bug".into(),
            serde_json::json!({"version": "1"}),
            serde_json::json!({}),
        );
        let got = store.get("prv_1").expect("still there");
        assert_eq!(got.kind, "bug");
        assert_eq!(got.client["version"], "1");
    }

    #[test]
    fn an_unknown_id_is_none_like_an_expired_one() {
        let store = PreviewStore::default();
        assert!(store.get("prv_nope").is_none());
    }

    #[test]
    fn a_preview_is_gone_once_its_ttl_passes() {
        let mut store = PreviewStore::default();
        store.insert(
            "prv_1".into(),
            "bug".into(),
            serde_json::json!({}),
            serde_json::json!({}),
        );
        std::thread::sleep(TTL + Duration::from_millis(50));
        assert!(store.get("prv_1").is_none());
    }

    #[test]
    fn past_the_cap_the_oldest_preview_is_evicted() {
        let mut store = PreviewStore::default();
        for i in 0..MAX_PREVIEWS {
            store.insert(
                format!("prv_{i}"),
                "bug".into(),
                serde_json::json!({}),
                serde_json::json!({}),
            );
            std::thread::sleep(Duration::from_micros(1));
        }
        assert!(store.get("prv_0").is_some());
        store.insert(
            "prv_overflow".into(),
            "bug".into(),
            serde_json::json!({}),
            serde_json::json!({}),
        );
        assert!(store.get("prv_0").is_none(), "the oldest was evicted");
        assert!(store.get("prv_overflow").is_some());
    }
}
