//! The seam the Notifier never imports.
//!
//! Decision 161's "Duplicates with the app in front" picks a channel in
//! order: a device showing the session (nothing), any other active device
//! (toast), a backgrounded desktop (local OS notification) — all three
//! already exist (161-3/161-4, `agent::notifications::deliver_hint_for`).
//! **Push is the fourth tier: nobody connected at all.** Rather than teach
//! the Notifier's own code about push, this module watches the same public
//! feed a client would (`AgentManager::watch_notifications` +
//! `list_notifications_since_update`) from the outside, the same way
//! `crate::feedback::forwarder` watches the feedback queue. A second
//! delivery sink (161-9, ntfy) can be added the identical way — its own
//! module, its own background task, reading the same feed — without ever
//! touching this file or `agent/notifications.rs`.
//!
//! Two independent loops, run together by [`run`]:
//! - [`watch_notices`]: the 15 s grace and the "is anyone connected at
//!   all" check, which decides *whether* to enqueue a push at all.
//! - [`drain_queue`]: the persisted `push_deliveries` queue — retries,
//!   backoff, TTL/Urgency/Topic, and turning a 404/410 into an endpoint
//!   deletion.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use time::format_description::well_known::Rfc3339;

use crate::agent::notifications::Notification;
use crate::api::Shared;
use crate::auth::push_endpoints::PushDeliveryRow;

const GRACE: Duration = if cfg!(test) {
    Duration::from_millis(30)
} else {
    Duration::from_secs(15)
};
const NOTICE_POLL_INTERVAL: Duration = if cfg!(test) {
    Duration::from_millis(5)
} else {
    Duration::from_secs(2)
};
const QUEUE_POLL_INTERVAL: Duration = if cfg!(test) {
    Duration::from_millis(5)
} else {
    Duration::from_secs(3)
};
const BATCH: i64 = 50;
// "Exponential retry up to 1 h" (decision 161, slice 161-6): the ceiling on
// how long one delivery is retried before it is dropped.
const GIVE_UP_AFTER: Duration = if cfg!(test) {
    Duration::from_millis(400)
} else {
    Duration::from_secs(60 * 60)
};
const BASE_BACKOFF: Duration = if cfg!(test) {
    Duration::from_millis(20)
} else {
    Duration::from_secs(30)
};
const MAX_BACKOFF: Duration = if cfg!(test) {
    Duration::from_millis(200)
} else {
    Duration::from_secs(60 * 30)
};
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const ATTEMPT_TIMEOUT: Duration = Duration::from_secs(20);

/// The server URL used as the VAPID `sub` claim (RFC 8292 §2.1 allows
/// either a `mailto:` or an `https:` contact URI). Storm's own repo, not an
/// operator's personal address — this is read by push services, not by a
/// person, and nothing in this codebase has an operator contact email to
/// offer instead.
const VAPID_SUB: &str = "https://github.com/dewanshDT/Storm";
/// RFC 8292 §2: "MUST NOT be more than 24 hours"; decision 161-6 holds to a
/// tighter ceiling.
const VAPID_TTL_SECS: i64 = 12 * 60 * 60;

pub fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .use_preconfigured_tls(crate::gateway::upstream::tls_config())
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(ATTEMPT_TIMEOUT)
        .build()
        .expect("the push sender's HTTP client builds")
}

/// Runs both loops for the process's whole life. Never returns.
pub async fn run(state: Shared) {
    let client = http_client();
    tokio::join!(watch_notices(state.clone()), drain_queue(state, client));
}

struct Grace {
    due: Instant,
    notice: Notification,
}

/// Decision 161's events table, mapped onto `device_notification_prefs`'s
/// six toggles (`crate::auth::device_prefs::CLASSES`). `None` means "not
/// gated by any toggle" — a summary and a test notification are each
/// explicitly requested (by ending quiet hours, or by the person's own
/// "send a test" button), so neither is filtered by the six per-kind
/// switches.
fn class_of(kind: &str) -> Option<&'static str> {
    use crate::agent::notifications::*;
    match kind {
        NEEDS_YOU => Some("needs_you"),
        DELETE_REQUEST => Some("delete_requests"),
        FINISHED | CHILD_FINISHED => Some("finished"),
        ERROR | USAGE_LIMIT => Some("errors"),
        AGENT_MESSAGE | TERMINAL_NOTICE => Some("agent_messages"),
        HOST_OFFLINE => Some("hosts"),
        _ => None,
    }
}

/// The 15 s grace and the one-shot-per-notice decision: watches the same
/// feed a device's own `/v1/agent/events` connection would, polling
/// [`crate::agent::AgentManager::list_notifications_since_update`] rather
/// than holding a stream open, since this has no connection of its own to
/// read from.
async fn watch_notices(state: Shared) {
    // Starting at "now" rather than 0: a restart raises nothing for a
    // notice that already existed, the same rule the Notifier and the
    // client's own toasts follow for a snapshot.
    let mut cursor = state.agent.notifications_update_cursor().unwrap_or(0);
    let mut changed = state.agent.watch_notifications();
    let mut pending: HashMap<String, Grace> = HashMap::new();

    loop {
        tokio::select! {
            _ = changed.changed() => {}
            _ = tokio::time::sleep(NOTICE_POLL_INTERVAL) => {}
        }

        match state.agent.list_notifications_since_update(cursor, 200) {
            Ok(rows) => {
                for n in rows {
                    cursor = cursor.max(n.updated_seq);
                    if n.silent || n.cancelled || n.read_at.is_some() {
                        // The need cleared, or it was never going to
                        // interrupt in the first place: nothing to push,
                        // and anything already pending for it is moot.
                        pending.remove(&n.id);
                        continue;
                    }
                    match pending.get_mut(&n.id) {
                        Some(g) => g.notice = n,
                        None => {
                            let id = n.id.clone();
                            pending.insert(
                                id,
                                Grace {
                                    due: Instant::now() + GRACE,
                                    notice: n,
                                },
                            );
                        }
                    }
                }
            }
            Err(e) => tracing::warn!(error = ?e, "push: could not poll notifications"),
        }

        let now = Instant::now();
        let due: Vec<String> = pending
            .iter()
            .filter(|(_, g)| g.due <= now)
            .map(|(id, _)| id.clone())
            .collect();
        for id in due {
            let Some(g) = pending.remove(&id) else {
                continue;
            };
            // A device becoming active within the grace is step 4's
            // cancellation condition; the other one (the need clearing) was
            // already handled above, by never re-inserting a now-silent,
            // cancelled or read notice.
            if state.agent.any_device_connected() {
                continue;
            }
            if let Err(e) = enqueue_for_notice(&state, &g.notice).await {
                tracing::warn!(error = ?e, notice = %g.notice.id, "push: could not enqueue delivery");
            }
        }
    }
}

/// Fans one notice out to every device that qualifies: a push endpoint
/// registered, notifications on for that device, and (when the notice's
/// kind maps to one) that class allowed. One row per endpoint — a device
/// could in principle hold more than one (a browser profile re-subscribing
/// without the old endpoint being cleaned up yet), and each is owed its own
/// attempt.
async fn enqueue_for_notice(state: &Shared, notice: &Notification) -> anyhow::Result<()> {
    let now = crate::index::now_rfc3339();
    let mut auth = state.auth_db.lock().await;
    // The one-shot guard: a notice is evaluated for push exactly once,
    // whether this is the first tick to see it due or a restart replaying
    // the same decision. A later update to the same notice (e.g. the
    // mark-as-unread "reopened" case) is deliberately not re-queued here —
    // decision 161-6: "send nothing for now ... next push will replace."
    if auth.push_delivery_exists_for_notice(&notice.id)? {
        return Ok(());
    }
    let class = class_of(&notice.kind);
    let collapse = super::collapse_key(notice.key.as_deref(), &notice.id);
    for device in auth.list_device_prefs()? {
        if device.revoked {
            continue;
        }
        // No `device_notification_prefs` row at all, or explicitly off:
        // never push to it. This is also what keeps a desktop — which gets
        // its OS notification from the existing `os` deliver-hint tier,
        // never from this queue — out of this loop in the common case
        // where it has no push endpoint to begin with.
        if !device.enabled.unwrap_or(false) {
            continue;
        }
        if let (Some(class), Some(classes)) = (class, device.classes.as_deref())
            && !classes.is_empty()
            && !classes.iter().any(|c| c == class)
        {
            continue;
        }
        for endpoint in auth.push_endpoints_for_device(&device.device_id)? {
            auth.enqueue_push_delivery(
                &endpoint.id,
                &notice.id,
                &collapse,
                &notice.urgency,
                &now,
                &now,
            )?;
        }
    }
    Ok(())
}

fn age(stamp: &str, now: &str) -> Duration {
    match (
        time::OffsetDateTime::parse(stamp, &Rfc3339),
        time::OffsetDateTime::parse(now, &Rfc3339),
    ) {
        (Ok(then), Ok(now)) => {
            let millis = (now - then).whole_milliseconds();
            Duration::from_millis(millis.max(0) as u64)
        }
        _ => Duration::ZERO,
    }
}

fn add(stamp: &str, extra: Duration) -> String {
    match time::OffsetDateTime::parse(stamp, &Rfc3339) {
        Ok(t) => (t + time::Duration::milliseconds(extra.as_millis() as i64))
            .format(&Rfc3339)
            .unwrap_or_else(|_| stamp.to_string()),
        Err(_) => stamp.to_string(),
    }
}

fn exponential_backoff(attempts: i64) -> Duration {
    let shift = attempts.clamp(1, 20) as u32 - 1;
    BASE_BACKOFF
        .checked_mul(1u32 << shift)
        .unwrap_or(MAX_BACKOFF)
        .min(MAX_BACKOFF)
}

/// Drains `push_deliveries`: a row that is due, sent, and the result —
/// success, permanent drop, or a backed-off retry — applied. Holds
/// `auth_db`'s lock only for the sqlite round trips either side of the
/// network call, never across it (that mutex also guards login, which must
/// never queue behind a push service being slow).
async fn drain_queue(state: Shared, client: reqwest::Client) {
    loop {
        tokio::time::sleep(QUEUE_POLL_INTERVAL).await;
        let now = crate::index::now_rfc3339();
        let due = {
            let auth = state.auth_db.lock().await;
            match auth.due_push_deliveries(&now, BATCH) {
                Ok(rows) => rows,
                Err(e) => {
                    tracing::warn!(error = ?e, "push: could not read due deliveries");
                    continue;
                }
            }
        };
        for row in due {
            if age(&row.created, &now) >= GIVE_UP_AFTER {
                tracing::warn!(
                    delivery_id = %row.id,
                    attempts = row.attempts,
                    "push: giving up after the retry window"
                );
                let mut auth = state.auth_db.lock().await;
                let _ = auth.delete_push_delivery(&row.id);
                continue;
            }
            send_one(&state, &client, &row, &now).await;
        }
    }
}

enum Outcome {
    Delivered,
    /// 404/410: the subscription is gone.
    EndpointGone,
    Retry {
        after: Option<Duration>,
        reason: String,
    },
    /// Anything else (400, 403, ...): not this endpoint's fault in a way a
    /// retry would fix, and not proof the endpoint itself is dead either.
    Drop {
        reason: String,
    },
}

/// One send attempt: fetches the endpoint and the notice fresh (never from
/// the grace scan's copy — a preview-level change or a cancellation since
/// enqueue must still be honoured), encrypts, and posts. Only the host of
/// the endpoint ever reaches a log line — never the endpoint path, which
/// is itself part of the subscriber's secret, and never `p256dh` or the
/// opened auth secret.
async fn send_one(state: &Shared, client: &reqwest::Client, row: &PushDeliveryRow, now: &str) {
    let (endpoint, preview) = {
        let auth = state.auth_db.lock().await;
        let endpoint = match auth.push_endpoint(&row.endpoint_id) {
            Ok(Some(e)) => e,
            Ok(None) => {
                // The endpoint was deleted (revoke, or an earlier 404/410
                // on another of its rows) since this was enqueued.
                drop(auth);
                let mut auth = state.auth_db.lock().await;
                let _ = auth.delete_push_delivery(&row.id);
                return;
            }
            Err(e) => {
                tracing::warn!(error = ?e, delivery_id = %row.id, "push: could not read the endpoint");
                return;
            }
        };
        let preview = auth
            .device_prefs(&endpoint.device_id)
            .ok()
            .flatten()
            .map(|p| p.preview)
            .unwrap_or_else(|| "storm".to_string());
        (endpoint, preview)
    };

    let notice = match state.agent.notification_by_id(&row.notice_id) {
        Ok(Some(n)) if !n.cancelled && n.read_at.is_none() => n,
        Ok(_) => {
            // Cancelled, replaced by key, or read on another device before
            // this attempt ran: nothing left to send.
            let mut auth = state.auth_db.lock().await;
            let _ = auth.delete_push_delivery(&row.id);
            return;
        }
        Err(e) => {
            tracing::warn!(error = ?e, delivery_id = %row.id, "push: could not read the notice");
            return;
        }
    };

    let outcome = attempt(
        client,
        &state.vapid,
        &state.push_secrets,
        &endpoint,
        &preview,
        &notice,
        row,
    )
    .await;

    let mut auth = state.auth_db.lock().await;
    match outcome {
        Outcome::Delivered => {
            let _ = auth.record_push_ok(&endpoint.id, now);
            let _ = auth.delete_push_delivery(&row.id);
        }
        Outcome::EndpointGone => {
            tracing::info!(host = %origin_host(&endpoint.endpoint), "push: endpoint gone (404/410), deleting it");
            let _ = auth.delete_push_endpoint_unconditionally(&endpoint.id);
        }
        Outcome::Drop { reason } => {
            tracing::warn!(host = %origin_host(&endpoint.endpoint), reason, "push: dropped permanently");
            let _ = auth.record_push_failure(&endpoint.id);
            let _ = auth.delete_push_delivery(&row.id);
        }
        Outcome::Retry { after, reason } => {
            let attempts = row.attempts + 1;
            let backoff = after.unwrap_or_else(|| exponential_backoff(attempts));
            let next = add(now, backoff);
            tracing::info!(
                host = %origin_host(&endpoint.endpoint),
                attempts,
                reason,
                "push: retrying later"
            );
            let _ = auth.record_push_failure(&endpoint.id);
            let _ = auth.reschedule_push_delivery(&row.id, &next, &reason);
        }
    }
}

/// The scheme+host (+ port) a `k=`/`aud=` VAPID claim and a log line may
/// both show — never the path, which a subscriber's push URL embeds a
/// capability token in.
fn origin_host(endpoint: &str) -> String {
    url::Url::parse(endpoint)
        .map(|u| u.origin().ascii_serialization())
        .unwrap_or_else(|_| "(unparseable)".to_string())
}

async fn attempt(
    client: &reqwest::Client,
    vapid: &super::vapid::VapidKey,
    push_secrets: &super::crypto::SecretKey,
    endpoint: &crate::auth::push_endpoints::PushEndpoint,
    preview: &str,
    notice: &Notification,
    row: &PushDeliveryRow,
) -> Outcome {
    let aud = origin_host(&endpoint.endpoint);
    let now_unix = time::OffsetDateTime::now_utc().unix_timestamp();
    let jwt = match vapid.jwt(&aud, VAPID_SUB, now_unix, VAPID_TTL_SECS) {
        Ok(j) => j,
        Err(e) => {
            return Outcome::Retry {
                after: None,
                reason: format!("vapid: {e}"),
            };
        }
    };

    let p256dh = match data_encoding::BASE64URL_NOPAD
        .decode(endpoint.p256dh.trim_end_matches('=').as_bytes())
    {
        Ok(b) => b,
        Err(_) => {
            return Outcome::Drop {
                reason: "p256dh does not decode".into(),
            };
        }
    };
    let auth_secret: [u8; super::crypto::AUTH_SECRET_LEN] =
        match push_secrets.open(&endpoint.auth_secret_enc).and_then(|b| {
            b.try_into()
                .map_err(|_| anyhow::anyhow!("auth secret is not 16 bytes"))
        }) {
            Ok(a) => a,
            Err(e) => {
                return Outcome::Drop {
                    reason: format!("auth secret: {e}"),
                };
            }
        };

    let payload = super::preview_payload(notice, preview);
    let body = match super::crypto::encrypt(
        &p256dh,
        &auth_secret,
        &serde_json::to_vec(&payload).unwrap_or_default(),
    ) {
        Ok(b) => b,
        Err(e) => {
            return Outcome::Drop {
                reason: format!("encrypt: {e}"),
            };
        }
    };

    let response = client
        .post(&endpoint.endpoint)
        .header("content-encoding", "aes128gcm")
        .header("content-type", "application/octet-stream")
        .header("ttl", super::ttl_secs(&row.urgency).to_string())
        .header("urgency", super::urgency_header(&row.urgency))
        .header("topic", row.collapse_key.as_str())
        .header(
            "authorization",
            format!("vapid t={jwt}, k={}", vapid.public_key_b64()),
        )
        .body(body)
        .send()
        .await;
    let response = match response {
        Ok(r) => r,
        Err(_) => {
            return Outcome::Retry {
                after: None,
                reason: "network".into(),
            };
        }
    };
    let status = response.status();
    if status.is_success() {
        return Outcome::Delivered;
    }
    let retry_after = response
        .headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        .map(Duration::from_secs);
    match status.as_u16() {
        404 | 410 => Outcome::EndpointGone,
        429 => Outcome::Retry {
            after: retry_after,
            reason: "429".into(),
        },
        s if (500..600).contains(&s) => Outcome::Retry {
            after: retry_after,
            reason: "5xx".into(),
        },
        s => Outcome::Drop {
            reason: s.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn class_of_matches_the_six_device_pref_toggles() {
        use crate::agent::notifications::*;
        for kind in [
            NEEDS_YOU,
            DELETE_REQUEST,
            FINISHED,
            ERROR,
            AGENT_MESSAGE,
            HOST_OFFLINE,
        ] {
            let class = class_of(kind).unwrap();
            assert!(
                crate::auth::device_prefs::CLASSES.contains(&class),
                "{kind} maps to an unknown class {class}"
            );
        }
    }

    #[test]
    fn summary_and_test_bypass_the_class_filter() {
        use crate::agent::notifications::{SUMMARY, TEST};
        assert_eq!(class_of(SUMMARY), None);
        assert_eq!(class_of(TEST), None);
    }

    #[test]
    fn backoff_doubles_and_is_capped() {
        assert_eq!(exponential_backoff(1), BASE_BACKOFF);
        assert_eq!(exponential_backoff(2), BASE_BACKOFF * 2);
        assert_eq!(exponential_backoff(3), BASE_BACKOFF * 4);
        assert_eq!(exponential_backoff(30), MAX_BACKOFF);
    }

    #[test]
    fn add_moves_a_stamp_forward_by_exactly_the_backoff() {
        let moved = add("2026-11-21T00:00:00Z", Duration::from_secs(90));
        assert_eq!(age("2026-11-21T00:00:00Z", &moved), Duration::from_secs(90));
    }

    #[test]
    fn origin_host_drops_the_path_and_query() {
        assert_eq!(
            origin_host("https://push.example.net/subscribe/JzLQ3raZJfFBR0aqvOMsLrt54w4rJUsV?x=1"),
            "https://push.example.net"
        );
    }

    // ---- A local mock push service ---------------------------------------
    //
    // Same pattern as `gateway::upstream`'s tests: a real `axum::serve` on
    // an ephemeral loopback port, so `attempt()`'s HTTP handling (status
    // mapping, headers, the encrypted body) runs against a real socket
    // rather than a stub.

    struct MockPush {
        url: String,
        calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    }

    async fn spawn_mock_push(status: u16, retry_after: Option<&'static str>) -> MockPush {
        use axum::response::IntoResponse;
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let calls_in = calls.clone();
        let app = axum::Router::new().route(
            "/push/sub1",
            axum::routing::post(move |_body: axum::body::Bytes| {
                let calls_in = calls_in.clone();
                async move {
                    calls_in.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    let code = axum::http::StatusCode::from_u16(status).unwrap();
                    match retry_after {
                        Some(ra) => (code, [("retry-after", ra)], ()).into_response(),
                        None => code.into_response(),
                    }
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        MockPush {
            url: format!("http://{addr}/push/sub1"),
            calls,
        }
    }

    /// A subscriber keypair plus a sealed `push_endpoints` row pointing at
    /// `url` — everything [`attempt`] needs on the endpoint side.
    fn fixture_endpoint(
        secrets: &super::super::crypto::SecretKey,
        url: &str,
    ) -> crate::auth::push_endpoints::PushEndpoint {
        let rng = ring::rand::SystemRandom::new();
        let ua_private =
            ring::agreement::EphemeralPrivateKey::generate(&ring::agreement::ECDH_P256, &rng)
                .unwrap();
        let ua_public = ua_private.compute_public_key().unwrap();
        let mut auth_secret = [0u8; super::super::crypto::AUTH_SECRET_LEN];
        ring::rand::SecureRandom::fill(&rng, &mut auth_secret).unwrap();
        let sealed = secrets.seal(&auth_secret).unwrap();
        crate::auth::push_endpoints::PushEndpoint {
            id: "pep_test".into(),
            device_id: "dev_test".into(),
            kind: "webpush".into(),
            endpoint: url.to_string(),
            p256dh: data_encoding::BASE64URL_NOPAD.encode(ua_public.as_ref()),
            auth_secret_enc: sealed,
            created: "2026-11-21T00:00:00Z".into(),
            last_ok: None,
            failures: 0,
        }
    }

    fn fixture_notice(kind: &'static str) -> Notification {
        Notification {
            id: "ntf_test".into(),
            seq: 1,
            updated_seq: 1,
            created: "2026-11-21T00:00:00Z".into(),
            source: "storm".into(),
            kind: kind.into(),
            session_id: Some("ags_test".into()),
            host_id: None,
            provider: Some("claude-code".into()),
            session_title: Some("tidier".into()),
            note_vault_id: None,
            note_id: None,
            title: "tidier needs you".into(),
            body: Some("approve rm -rf build/".into()),
            urgency: "high".into(),
            key: Some("ses:1:need:x".into()),
            silent: false,
            silent_reason: None,
            read_at: None,
            cancelled: false,
            reopened: false,
        }
    }

    fn fixture_row(endpoint_id: &str, notice_id: &str) -> PushDeliveryRow {
        PushDeliveryRow {
            id: "pdl_test".into(),
            endpoint_id: endpoint_id.into(),
            notice_id: notice_id.into(),
            collapse_key: "a".repeat(32),
            urgency: "high".into(),
            created: "2026-11-21T00:00:00Z".into(),
            not_before: "2026-11-21T00:00:00Z".into(),
            attempts: 0,
            last_error: None,
        }
    }

    fn test_vapid_and_secrets(
        dir: &std::path::Path,
    ) -> (
        super::super::vapid::VapidKey,
        super::super::crypto::SecretKey,
    ) {
        let mut auth_db = crate::auth::AuthDb::open(dir).unwrap();
        let vapid = super::super::vapid::load_or_create(&mut auth_db, dir).unwrap();
        let secrets = super::super::crypto::SecretKey::load_or_create(dir).unwrap();
        (vapid, secrets)
    }

    #[tokio::test]
    async fn attempt_returns_delivered_on_a_success_status() {
        let dir = tempdir::TempDir::new("push-attempt-201").unwrap();
        let (vapid, secrets) = test_vapid_and_secrets(dir.path());
        let mock = spawn_mock_push(201, None).await;
        let endpoint = fixture_endpoint(&secrets, &mock.url);
        let notice = fixture_notice(crate::agent::notifications::NEEDS_YOU);
        let row = fixture_row(&endpoint.id, &notice.id);

        let client = http_client();
        let outcome = attempt(&client, &vapid, &secrets, &endpoint, "full", &notice, &row).await;
        assert!(matches!(outcome, Outcome::Delivered));
        assert_eq!(mock.calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn attempt_returns_endpoint_gone_on_410() {
        let dir = tempdir::TempDir::new("push-attempt-410").unwrap();
        let (vapid, secrets) = test_vapid_and_secrets(dir.path());
        let mock = spawn_mock_push(410, None).await;
        let endpoint = fixture_endpoint(&secrets, &mock.url);
        let notice = fixture_notice(crate::agent::notifications::NEEDS_YOU);
        let row = fixture_row(&endpoint.id, &notice.id);

        let client = http_client();
        let outcome = attempt(&client, &vapid, &secrets, &endpoint, "full", &notice, &row).await;
        assert!(matches!(outcome, Outcome::EndpointGone));
    }

    #[tokio::test]
    async fn attempt_retries_429_honouring_retry_after() {
        let dir = tempdir::TempDir::new("push-attempt-429").unwrap();
        let (vapid, secrets) = test_vapid_and_secrets(dir.path());
        let mock = spawn_mock_push(429, Some("120")).await;
        let endpoint = fixture_endpoint(&secrets, &mock.url);
        let notice = fixture_notice(crate::agent::notifications::NEEDS_YOU);
        let row = fixture_row(&endpoint.id, &notice.id);

        let client = http_client();
        let outcome = attempt(&client, &vapid, &secrets, &endpoint, "full", &notice, &row).await;
        match outcome {
            Outcome::Retry { after, reason } => {
                assert_eq!(after, Some(Duration::from_secs(120)));
                assert_eq!(reason, "429");
            }
            _ => panic!("expected a 429 retry"),
        }
    }

    #[tokio::test]
    async fn attempt_retries_5xx_with_no_retry_after() {
        let dir = tempdir::TempDir::new("push-attempt-503").unwrap();
        let (vapid, secrets) = test_vapid_and_secrets(dir.path());
        let mock = spawn_mock_push(503, None).await;
        let endpoint = fixture_endpoint(&secrets, &mock.url);
        let notice = fixture_notice(crate::agent::notifications::NEEDS_YOU);
        let row = fixture_row(&endpoint.id, &notice.id);

        let client = http_client();
        let outcome = attempt(&client, &vapid, &secrets, &endpoint, "full", &notice, &row).await;
        match outcome {
            Outcome::Retry { after, reason } => {
                assert_eq!(after, None);
                assert_eq!(reason, "5xx");
            }
            _ => panic!("expected a 5xx retry"),
        }
    }

    // ---- The 15 s grace and its cancellation (watch_notices) ------------
    //
    // `GRACE`, `NOTICE_POLL_INTERVAL` and `QUEUE_POLL_INTERVAL` are all
    // scaled down under `cfg(test)` to a few tens of milliseconds, the
    // same convention `feedback::forwarder` uses for its own backoff
    // constants — real time, just fast, rather than `tokio::time::pause`.

    fn test_shared(dir: &std::path::Path) -> Shared {
        let state_dir = dir.join("state");
        std::fs::create_dir_all(&state_dir).unwrap();
        let root = dir.join("vaults");
        std::fs::create_dir_all(&root).unwrap();

        let mut auth_db = crate::auth::AuthDb::open(&state_dir).unwrap();
        let identity = std::sync::Arc::new(
            crate::auth::identity::load_or_create(&mut auth_db, &state_dir, "2026-11-21T00:00:00Z")
                .unwrap(),
        );
        let vapid = std::sync::Arc::new(
            super::super::vapid::load_or_create(&mut auth_db, &state_dir).unwrap(),
        );
        let push_secrets = std::sync::Arc::new(
            super::super::crypto::SecretKey::load_or_create(&state_dir).unwrap(),
        );
        let (events, _) = tokio::sync::broadcast::channel(8);
        let (root_changed, _) = tokio::sync::broadcast::channel(2);
        let registry = crate::registry::Registry::load(&state_dir, &root).unwrap();
        let agent = std::sync::Arc::new(crate::agent::AgentManager::open(&state_dir).unwrap());
        let gateway = std::sync::Arc::new(
            crate::gateway::Gateway::open(&state_dir, "2026-11-21T00:00:00Z").unwrap(),
        );
        let host_names = auth_db.host_names().unwrap();
        let feedback = crate::feedback::Feedback::open(&state_dir).unwrap();

        std::sync::Arc::new(crate::api::AppState {
            vaults: tokio::sync::RwLock::new(crate::api::VaultSet {
                registry,
                open: std::collections::HashMap::new(),
            }),
            events,
            state_dir,
            identity,
            root_changed,
            relays_changed: tokio::sync::watch::channel(Vec::new()).0,
            mcp_enabled: std::sync::atomic::AtomicBool::new(false),
            mcp_writable: std::sync::atomic::AtomicBool::new(false),
            agent_writes: std::sync::atomic::AtomicBool::new(false),
            auth_db: std::sync::Arc::new(tokio::sync::Mutex::new(auth_db)),
            host_names: std::sync::Arc::new(std::sync::RwLock::new(host_names)),
            listen_addr: "127.0.0.1:8484".into(),
            vault_policy: std::sync::Arc::new(crate::auth::authz::StormPolicy),
            hasher: crate::auth::Hasher::new(),
            login_limiter: crate::auth::ratelimit::LoginLimiter::new(),
            host_limiter: crate::auth::ratelimit::LoginLimiter::new(),
            discovery: crate::discovery::Discovery::new(false, true),
            agent,
            gateway,
            diag_ring: storm_diag::Ring::default(),
            feedback,
            vapid,
            push_secrets,
        })
    }

    /// Registers a device, enables push on it for every class, and gives
    /// it one endpoint pointed at `mock_url` — everything `enqueue_for_notice`
    /// needs to find it eligible.
    async fn device_with_push_endpoint(state: &Shared, mock_url: &str) -> String {
        let now = "2026-11-21T00:00:00Z";
        let mut auth_db = state.auth_db.lock().await;
        let device =
            crate::auth::devices::create_synthetic(&mut auth_db, "test-device", now).unwrap();
        auth_db
            .ensure_device_prefs(&device.id, Some("web"), now)
            .unwrap();
        auth_db
            .update_device_prefs(&device.id, Some(true), Some(&[]), None, now)
            .unwrap();
        let rng = ring::rand::SystemRandom::new();
        let ua_private =
            ring::agreement::EphemeralPrivateKey::generate(&ring::agreement::ECDH_P256, &rng)
                .unwrap();
        let ua_public = ua_private.compute_public_key().unwrap();
        let mut auth_secret = [0u8; super::super::crypto::AUTH_SECRET_LEN];
        ring::rand::SecureRandom::fill(&rng, &mut auth_secret).unwrap();
        let sealed = state.push_secrets.seal(&auth_secret).unwrap();
        auth_db
            .upsert_push_endpoint(
                &device.id,
                "webpush",
                mock_url,
                &data_encoding::BASE64URL_NOPAD.encode(ua_public.as_ref()),
                &sealed,
                now,
            )
            .unwrap();
        device.id
    }

    #[tokio::test]
    async fn an_unread_notice_is_pushed_after_the_grace_once_nobody_is_connected() {
        let dir = tempdir::TempDir::new("push-grace-enqueue").unwrap();
        let state = test_shared(dir.path());
        device_with_push_endpoint(&state, "http://127.0.0.1:1/unused").await;

        // The watcher starts its cursor at "now" (nothing on a snapshot),
        // so it must already be running before the notice is created.
        let handle = tokio::spawn(watch_notices(state.clone()));
        tokio::time::sleep(Duration::from_millis(20)).await;
        let notice = state.agent.send_test_notification(None).unwrap();

        // Comfortably past GRACE + a couple of poll ticks (all in the
        // low tens of milliseconds under cfg(test)).
        tokio::time::sleep(Duration::from_millis(400)).await;
        handle.abort();

        let enqueued = state
            .auth_db
            .lock()
            .await
            .push_delivery_exists_for_notice(&notice.id)
            .unwrap();
        assert!(
            enqueued,
            "an unread, non-silent notice must be pushed once nobody is connected"
        );
    }

    #[tokio::test]
    async fn a_device_becoming_active_within_the_grace_cancels_the_push() {
        let dir = tempdir::TempDir::new("push-grace-cancel").unwrap();
        let state = test_shared(dir.path());
        device_with_push_endpoint(&state, "http://127.0.0.1:1/unused").await;

        let handle = tokio::spawn(watch_notices(state.clone()));
        tokio::time::sleep(Duration::from_millis(20)).await;
        let notice = state.agent.send_test_notification(None).unwrap();

        // Before the (scaled-down) grace elapses: a device connects.
        tokio::time::sleep(Duration::from_millis(5)).await;
        state.agent.set_presence("conn_1", "dev_other", true, None);

        tokio::time::sleep(Duration::from_millis(400)).await;
        handle.abort();

        let enqueued = state
            .auth_db
            .lock()
            .await
            .push_delivery_exists_for_notice(&notice.id)
            .unwrap();
        assert!(
            !enqueued,
            "a device going active within the grace must cancel the push"
        );
    }

    #[tokio::test]
    async fn a_notice_read_within_the_grace_is_never_pushed() {
        let dir = tempdir::TempDir::new("push-grace-read").unwrap();
        let state = test_shared(dir.path());
        device_with_push_endpoint(&state, "http://127.0.0.1:1/unused").await;

        let handle = tokio::spawn(watch_notices(state.clone()));
        tokio::time::sleep(Duration::from_millis(20)).await;
        let notice = state.agent.send_test_notification(None).unwrap();

        tokio::time::sleep(Duration::from_millis(5)).await;
        state
            .agent
            .mark_notifications_read(std::slice::from_ref(&notice.id), None)
            .unwrap();

        tokio::time::sleep(Duration::from_millis(400)).await;
        handle.abort();

        let enqueued = state
            .auth_db
            .lock()
            .await
            .push_delivery_exists_for_notice(&notice.id)
            .unwrap();
        assert!(
            !enqueued,
            "the need clearing within the grace must cancel the push"
        );
    }

    // ---- No secret ever reaches a log line -------------------------------

    #[derive(Clone, Default)]
    struct LogCapture(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    impl std::io::Write for LogCapture {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for LogCapture {
        type Writer = LogCapture;
        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    /// `#[tokio::test(flavor = "current_thread")]`: `tracing::subscriber::
    /// set_default` is thread-local, so capturing every log line this test
    /// cares about needs the whole body — including `send_one`'s `.await`s
    /// — to stay on the one thread that installed it.
    #[tokio::test(flavor = "current_thread")]
    async fn push_sending_never_logs_a_secret() {
        let dir = tempdir::TempDir::new("push-no-secret-log").unwrap();
        let state = test_shared(dir.path());

        // 410 (endpoint gone) exercises `send_one`'s own log line, which
        // names the endpoint's *origin* — the subscriber-capability token
        // below lives in the path, which must never appear.
        let secret_marker = "SUBSCRIBER-CAPABILITY-TOKEN-9f8e7d2c1b";
        let mock = spawn_mock_push(410, None).await;
        let url_with_secret_path = format!("{}?cap={secret_marker}", mock.url);

        let now = "2026-11-21T00:00:00Z";
        let rng = ring::rand::SystemRandom::new();
        let ua_private =
            ring::agreement::EphemeralPrivateKey::generate(&ring::agreement::ECDH_P256, &rng)
                .unwrap();
        let ua_public = ua_private.compute_public_key().unwrap();
        let mut auth_secret = [0u8; super::super::crypto::AUTH_SECRET_LEN];
        ring::rand::SecureRandom::fill(&rng, &mut auth_secret).unwrap();
        let p256dh_b64 = data_encoding::BASE64URL_NOPAD.encode(ua_public.as_ref());

        let (endpoint_id, notice_id, row) = {
            let mut auth_db = state.auth_db.lock().await;
            let device = crate::auth::devices::create_synthetic(&mut auth_db, "d", now).unwrap();
            let sealed = state.push_secrets.seal(&auth_secret).unwrap();
            let endpoint = auth_db
                .upsert_push_endpoint(
                    &device.id,
                    "webpush",
                    &url_with_secret_path,
                    &p256dh_b64,
                    &sealed,
                    now,
                )
                .unwrap();
            drop(auth_db);
            let notice = state.agent.send_test_notification(None).unwrap();
            let mut auth_db = state.auth_db.lock().await;
            let row = auth_db
                .enqueue_push_delivery(&endpoint.id, &notice.id, "key", "normal", now, now)
                .unwrap();
            (endpoint.id, notice.id, row)
        };

        let capture = LogCapture::default();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(capture.clone())
            .with_ansi(false)
            .finish();
        let _guard = tracing::subscriber::set_default(subscriber);
        send_one(&state, &http_client(), &row, now).await;
        drop(_guard);

        // The endpoint really was deleted (410), which is what makes this a
        // meaningful run of `send_one`'s logging path rather than a no-op.
        assert!(
            state
                .auth_db
                .lock()
                .await
                .push_endpoint(&endpoint_id)
                .unwrap()
                .is_none()
        );

        let logs = String::from_utf8(capture.0.lock().unwrap().clone()).unwrap();
        assert!(!logs.contains(secret_marker), "logs:\n{logs}");
        assert!(!logs.contains(&url_with_secret_path), "logs:\n{logs}");
        assert!(!logs.contains(&p256dh_b64), "logs:\n{logs}");
        assert!(
            !logs.contains(&data_encoding::BASE64URL_NOPAD.encode(&auth_secret)),
            "logs:\n{logs}"
        );
        // The notice id and the endpoint's *origin* are fine to log — only
        // the endpoint's own path (its capability token) is the secret.
        let _ = notice_id;
    }

    #[tokio::test]
    async fn attempt_sends_a_valid_vapid_authorization_header() {
        // Proves the Authorization header's JWT verifies under the
        // server's own VAPID public key, end to end through a real HTTP
        // request rather than only through crypto's own unit tests.
        let dir = tempdir::TempDir::new("push-attempt-vapid").unwrap();
        let (vapid, secrets) = test_vapid_and_secrets(dir.path());
        let seen_auth = std::sync::Arc::new(std::sync::Mutex::new(None::<String>));
        let seen_in = seen_auth.clone();
        let app = axum::Router::new().route(
            "/push/sub1",
            axum::routing::post(
                move |headers: axum::http::HeaderMap, _body: axum::body::Bytes| {
                    let seen_in = seen_in.clone();
                    async move {
                        *seen_in.lock().unwrap() = headers
                            .get("authorization")
                            .and_then(|v| v.to_str().ok())
                            .map(str::to_string);
                        axum::http::StatusCode::CREATED
                    }
                },
            ),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let url = format!("http://{addr}/push/sub1");

        let endpoint = fixture_endpoint(&secrets, &url);
        let notice = fixture_notice(crate::agent::notifications::NEEDS_YOU);
        let row = fixture_row(&endpoint.id, &notice.id);
        let client = http_client();
        let outcome = attempt(&client, &vapid, &secrets, &endpoint, "full", &notice, &row).await;
        assert!(matches!(outcome, Outcome::Delivered));

        let header = seen_auth
            .lock()
            .unwrap()
            .clone()
            .expect("Authorization header sent");
        assert!(header.starts_with("vapid t="));
        let t_start = header.find("t=").unwrap() + 2;
        let t_end = header[t_start..].find(',').map(|i| t_start + i).unwrap();
        let jwt = &header[t_start..t_end];
        let parts: Vec<&str> = jwt.split('.').collect();
        assert_eq!(parts.len(), 3);
        let signing_input = format!("{}.{}", parts[0], parts[1]);
        let sig = data_encoding::BASE64URL_NOPAD
            .decode(parts[2].as_bytes())
            .unwrap();
        ring::signature::UnparsedPublicKey::new(
            &ring::signature::ECDSA_P256_SHA256_FIXED,
            &vapid.public_key_bytes(),
        )
        .verify(signing_input.as_bytes(), &sig)
        .expect("the Authorization header's JWT must verify under the VAPID public key");
    }
}
