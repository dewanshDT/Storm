//! Decision 161, slice 161-6: the server's encrypted push sender — Web
//! Push and UnifiedPush share it (AM125, "Architecture", "Payload text by
//! channel", "Delivery queue").
//!
//! - [`crypto`]: RFC 8291 message encryption and RFC 8292 VAPID, against
//!   `ring` directly (see that module's doc comment for why).
//! - [`vapid`]: the server's VAPID keypair, loaded once at boot.
//! - [`delivery`]: the seam the Notifier's own code never has to import —
//!   a background task watches the notification stream the same way a
//!   client would (`watch_notifications` + `list_notifications_since_update`)
//!   and the auth.db delivery queue, so a second sink (161-9, ntfy) can be
//!   added the exact same way without the two ever touching one file.
//!
//! Storage (`push_endpoints`, `push_deliveries`, `vapid_keypair`) lives in
//! `auth.db` — see `crate::auth::db` for the schema and
//! `crate::auth::push_endpoints` for the CRUD.

pub mod crypto;
pub mod delivery;
pub mod vapid;

use crate::agent::notifications::Notification;

/// TTL (seconds) by urgency — decision 161's "Delivery queue": high 10
/// min, normal 4 h, low 24 h. An urgency the push protocol itself does not
/// know (there is none outside those three today) gets the middle value
/// rather than a panic.
pub fn ttl_secs(urgency: &str) -> u32 {
    match urgency {
        "high" => 600,
        "low" => 86_400,
        _ => 14_400,
    }
}

/// RFC 8030's `Urgency` header takes the notice's own urgency verbatim —
/// `"low"`, `"normal"` or `"high"` are exactly its three defined values.
pub fn urgency_header(urgency: &str) -> &str {
    match urgency {
        "low" | "high" => urgency,
        _ => "normal",
    }
}

/// The `Topic` header (RFC 8030 §5.4): at most 32 base64url characters, so
/// a replacement notice collapses the push service's still-queued copy of
/// the same thing instead of piling up. Derived from the notice's own
/// dedupe `key` when it has one (so two notices that would have replaced
/// each other in the inbox also replace each other in the push queue),
/// else its id.
pub fn collapse_key(notice_key: Option<&str>, notice_id: &str) -> String {
    use data_encoding::BASE64URL_NOPAD;
    use ring::digest;
    let basis = notice_key.unwrap_or(notice_id);
    let hash = digest::digest(&digest::SHA256, basis.as_bytes());
    // 24 bytes of a SHA-256 digest -> exactly 32 base64url characters
    // (24 * 4 / 3), the full width RFC 8030 allows.
    BASE64URL_NOPAD.encode(&hash.as_ref()[..24])
}

/// What actually gets encrypted: the notice's text, cut down to what a
/// device's own preview level allows (decision 161, "Payload text by
/// channel") — applied here, on the server, before `crypto::encrypt` ever
/// sees it, so a lost or compromised device never leaks more than its own
/// setting promised.
///
/// The exact JSON shape a receiving client reads is 161-7/161-8's to
/// settle; `id`/`kind`/`session_id`/`note` are included at every level
/// because they are routing, not content — tapping a `storm`-level
/// notification must still open the right session. Only `title`/`body`
/// are preview-gated.
pub fn preview_payload(notice: &Notification, preview: &str) -> serde_json::Value {
    let (title, body) = match preview {
        "full" => (notice.title.clone(), notice.body.clone()),
        "session" => (
            format!(
                "{} · {}",
                notice.session_title.as_deref().unwrap_or("Storm"),
                kind_label(&notice.kind)
            ),
            None,
        ),
        // "storm", and anything this server does not recognise: the
        // safest level, never the fuller one, is the right default for an
        // unknown setting.
        _ => (format!("Storm · {}", kind_label(&notice.kind)), None),
    };
    serde_json::json!({
        "id": notice.id,
        "kind": notice.kind,
        "urgency": notice.urgency,
        "session_id": notice.session_id,
        "note_vault_id": notice.note_vault_id,
        "note_id": notice.note_id,
        "title": title,
        "body": body,
    })
}

fn kind_label(kind: &str) -> &'static str {
    use crate::agent::notifications::*;
    match kind {
        NEEDS_YOU => "needs you",
        DELETE_REQUEST => "wants to delete something",
        FINISHED => "finished its turn",
        ERROR => "hit an error",
        USAGE_LIMIT => "hit a usage limit",
        HOST_OFFLINE => "host offline",
        CHILD_FINISHED => "child finished",
        AGENT_MESSAGE => "sent a message",
        TERMINAL_NOTICE => "sent a message",
        SUMMARY => "summary",
        TEST => "test",
        _ => "notification",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ttl_matches_the_spec_table() {
        assert_eq!(ttl_secs("high"), 600);
        assert_eq!(ttl_secs("normal"), 14_400);
        assert_eq!(ttl_secs("low"), 86_400);
        assert_eq!(ttl_secs("unknown"), 14_400);
    }

    #[test]
    fn collapse_key_is_stable_and_within_32_chars() {
        let a = collapse_key(Some("ses:1:need:x"), "ntf_a");
        let b = collapse_key(Some("ses:1:need:x"), "ntf_b");
        assert_eq!(a, b, "same dedupe key collapses to the same topic");
        assert!(a.len() <= 32);
        let c = collapse_key(None, "ntf_a");
        let d = collapse_key(None, "ntf_b");
        assert_ne!(c, d, "no key: falls back to the notice's own id");
    }
}
