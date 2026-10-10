//! Images pasted into a session, held only until its host has staged them
//! (D15 AM46). Never on disk here: a restart loses an in-flight paste.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::oneshot;

pub const MAX_IMAGE_BYTES: usize = 10 * 1024 * 1024;
const PER_SESSION: usize = 4;
const TOTAL_BYTES: usize = 64 * 1024 * 1024;

/// The file extension an image's own bytes say it is.
pub fn sniff(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("png")
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("jpg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("gif")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("webp")
    } else {
        None
    }
}

/// Whether a declared `Content-Type` names the type the bytes are.
pub fn declared_matches(content_type: &str, ext: &str) -> bool {
    let mime = content_type.split(';').next().unwrap_or("").trim();
    matches!(
        (mime, ext),
        ("image/png", "png")
            | ("image/jpeg", "jpg")
            | ("image/gif", "gif")
            | ("image/webp", "webp")
    )
}

pub type Outcome = Result<String, String>;

pub struct Pending {
    pub session: String,
    pub host: String,
    pub bytes: Arc<Vec<u8>>,
    pub done: Option<oneshot::Sender<Outcome>>,
}

#[derive(Default)]
pub struct Images {
    pending: HashMap<String, Pending>,
}

impl Images {
    /// False when the session or the server already holds too much.
    pub fn admit(&mut self, image: String, p: Pending) -> bool {
        let for_session = self
            .pending
            .values()
            .filter(|q| q.session == p.session)
            .count();
        let total: usize = self.pending.values().map(|q| q.bytes.len()).sum();
        if for_session >= PER_SESSION || total + p.bytes.len() > TOTAL_BYTES {
            return false;
        }
        self.pending.insert(image, p);
        true
    }

    pub fn get(&self, image: &str) -> Option<&Pending> {
        self.pending.get(image)
    }

    pub fn finish(&mut self, image: &str, outcome: Outcome) -> bool {
        match self.pending.get_mut(image).and_then(|p| p.done.take()) {
            Some(tx) => tx.send(outcome).is_ok(),
            None => false,
        }
    }

    pub fn remove(&mut self, image: &str) {
        self.pending.remove(image);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bytes_decide_the_type() {
        assert_eq!(sniff(b"\x89PNG\r\n\x1a\n...."), Some("png"));
        assert_eq!(sniff(&[0xFF, 0xD8, 0xFF, 0xE0]), Some("jpg"));
        assert_eq!(sniff(b"GIF89a.."), Some("gif"));
        assert_eq!(sniff(b"RIFF\0\0\0\0WEBPVP8 "), Some("webp"));
        assert_eq!(sniff(b"hello, a text file"), None);
        assert_eq!(sniff(b""), None);
        assert!(declared_matches("image/png", "png"));
        assert!(declared_matches("image/jpeg; q=1", "jpg"));
        assert!(!declared_matches("image/png", "jpg"));
        assert!(!declared_matches("text/plain", "png"));
    }

    fn pending(session: &str, len: usize) -> Pending {
        Pending {
            session: session.into(),
            host: "hst_A".into(),
            bytes: Arc::new(vec![0; len]),
            done: None,
        }
    }

    #[test]
    fn limits_per_session_and_in_total() {
        let mut im = Images::default();
        for n in 0..PER_SESSION {
            assert!(im.admit(format!("img_{n}"), pending("ags_A", 1)));
        }
        assert!(!im.admit("img_x".into(), pending("ags_A", 1)), "a fifth");
        assert!(im.admit("img_y".into(), pending("ags_B", 1)));
        let big = MAX_IMAGE_BYTES;
        let mut im = Images::default();
        for n in 0..6 {
            assert!(im.admit(format!("img_{n}"), pending(&format!("ags_{n}"), big)));
        }
        assert!(
            !im.admit("img_z".into(), pending("ags_Z", big)),
            "over 64 MiB"
        );
    }
}
