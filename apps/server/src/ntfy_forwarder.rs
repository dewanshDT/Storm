//! Decision 161, slice 161-9: the async task that drives the ntfy sink's
//! due queue. `agent::notifications::insert_notice` marks every non-silent
//! notice `pending` with a `ntfy_due_at` 15s out
//! ([`crate::agent::ntfy::GRACE`]); this task polls for rows whose grace
//! has elapsed and, for each, re-checks the two things that can have
//! changed in the meantime — the need cleared (now read or cancelled), or
//! an active device opened the session — before publishing. Lives outside
//! `agent/` (like `feedback::forwarder`) because it needs `Shared`, which
//! `AgentManager` deliberately does not depend on.

use std::time::Duration;

use crate::agent::ntfy;
use crate::api::Shared;

const POLL_INTERVAL: Duration = if cfg!(test) {
    Duration::from_millis(15)
} else {
    Duration::from_secs(2)
};
const BATCH: i64 = 25;

pub async fn run(state: Shared) {
    let client = ntfy::http_client();
    let mut changed = state.agent.watch_notifications();
    loop {
        tokio::select! {
            _ = changed.changed() => {}
            _ = tokio::time::sleep(POLL_INTERVAL) => {}
        }
        let now = crate::index::now_rfc3339();
        let due = match state.agent.due_ntfy_notices(&now, BATCH) {
            Ok(rows) => rows,
            Err(e) => {
                tracing::warn!(error = ?e, "ntfy: could not read the due queue");
                continue;
            }
        };
        if due.is_empty() {
            continue;
        }
        let settings = match state.agent.ntfy_settings() {
            Ok(Some(s)) => s,
            Ok(None) => {
                // The sink is off (or was turned off during the grace):
                // nothing to send, and nothing to keep polling for either.
                for n in &due {
                    let _ = state.agent.mark_ntfy_outcome(&n.id, "skipped");
                }
                continue;
            }
            Err(e) => {
                tracing::warn!(error = ?e, "ntfy: could not read the sink's settings");
                continue;
            }
        };
        for n in due {
            // The need cleared: a device already read it, or it was
            // cancelled (a `key` replace, `cancel_notify`) since it was
            // queued.
            if n.read_at.is_some() || n.cancelled {
                let _ = state.agent.mark_ntfy_outcome(&n.id, "skipped");
                continue;
            }
            // A device became active and opened this very session during
            // the grace — the in-app view already covers it.
            if state
                .agent
                .active_device_has_session_open(n.session_id.as_deref())
            {
                let _ = state.agent.mark_ntfy_outcome(&n.id, "skipped");
                continue;
            }
            let (title, message) = ntfy::render(&settings.preview, &n);
            let priority = ntfy::priority_for(&n.urgency);
            let tags = ntfy::tags_for(&n.kind);
            match ntfy::publish_with_retry(
                &client,
                &settings.url,
                settings.token.as_deref(),
                &title,
                &message,
                priority,
                &tags,
            )
            .await
            {
                Ok(()) => {
                    let _ = state.agent.mark_ntfy_outcome(&n.id, "sent");
                }
                Err(e) => {
                    // The host only — never the path (it carries the
                    // topic) and never the token.
                    tracing::warn!(
                        notice = %n.id,
                        host = %ntfy::host_of(&settings.url),
                        error = %e,
                        "ntfy: delivery failed"
                    );
                    let _ = state.agent.mark_ntfy_outcome(&n.id, "failed");
                }
            }
        }
    }
}
