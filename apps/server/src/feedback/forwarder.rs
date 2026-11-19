//! The background forwarder (decision 162b): delivers durably queued sends
//! to the intake Worker. Backoff from 1 s to a 1 h cap, `Retry-After`
//! honoured when the Worker sends one, a permanent drop on 400/413, and a
//! send older than 7 days is given up on — every one of those removes the
//! row, so nothing durable outlives its own outcome.
//!
//! **Never logs a row's body.** `report_id`, an HTTP status or a fixed
//! reason word are the only things a log line here ever carries.

use std::time::Duration;

use time::format_description::well_known::Rfc3339;

use crate::api::Shared;

/// Exponential backoff base (the feedback API contract's "1 s"). Scaled
/// down under test so a retry sequence costs milliseconds, not minutes.
const BASE_BACKOFF: Duration = if cfg!(test) {
    Duration::from_millis(15)
} else {
    Duration::from_secs(1)
};
/// The contract's "1 h" cap, scaled the same way.
const MAX_BACKOFF: Duration = if cfg!(test) {
    Duration::from_millis(200)
} else {
    Duration::from_secs(60 * 60)
};
/// The contract's "give up after 7 days", scaled the same way — a test
/// backdates a row's `created_at` rather than waiting for real time to pass.
const GIVE_UP_AFTER: Duration = if cfg!(test) {
    Duration::from_millis(500)
} else {
    Duration::from_secs(7 * 24 * 60 * 60)
};
/// How often the forwarder looks for sends that are due, between wakes.
const POLL_INTERVAL: Duration = if cfg!(test) {
    Duration::from_millis(10)
} else {
    Duration::from_secs(5)
};
/// At most this many sends attempted per pass, so one huge backlog cannot
/// starve the loop from ever sleeping and checking for new work.
const BATCH: usize = 25;
/// How long one attempt may take before it counts as unreachable.
const ATTEMPT_TIMEOUT: Duration = Duration::from_secs(30);

/// The forwarder's own HTTP client: rustls on `ring` with the bundled
/// roots, like the gateway's upstream client, and no redirect — the intake
/// URL is configuration, not somewhere a 3xx should be able to redirect a
/// report full of diagnostics to.
pub fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .use_preconfigured_tls(crate::gateway::upstream::tls_config())
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .timeout(ATTEMPT_TIMEOUT)
        .build()
        .expect("the feedback forwarder's HTTP client builds")
}

enum Outcome {
    Delivered,
    Retry {
        after: Option<Duration>,
        reason: &'static str,
    },
    Drop {
        reason: &'static str,
    },
}

async fn attempt(client: &reqwest::Client, intake_url: &str, body: &str) -> Outcome {
    let url = format!("{}/v1/reports", intake_url.trim_end_matches('/'));
    let response = match client
        .post(url)
        .header("content-type", "application/json")
        .body(body.to_string())
        .send()
        .await
    {
        Ok(r) => r,
        Err(_) => {
            return Outcome::Retry {
                after: None,
                reason: "network",
            };
        }
    };
    let status = response.status();
    // 200 and 201 both mean delivered (the feedback API contract).
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
        400 => Outcome::Drop { reason: "400" },
        413 => Outcome::Drop { reason: "413" },
        429 => Outcome::Retry {
            after: retry_after,
            reason: "429",
        },
        503 => Outcome::Retry {
            after: retry_after,
            reason: "503",
        },
        s if (500..600).contains(&s) => Outcome::Retry {
            after: retry_after,
            reason: "5xx",
        },
        // Anything else is a response the Worker's own contract does not
        // describe — never looped on forever, since nothing here says it
        // will ever change.
        _ => Outcome::Drop {
            reason: "unexpected_status",
        },
    }
}

fn exponential_backoff(attempts: i64) -> Duration {
    let shift = attempts.clamp(1, 20) as u32 - 1;
    BASE_BACKOFF
        .checked_mul(1u32 << shift)
        .unwrap_or(MAX_BACKOFF)
        .min(MAX_BACKOFF)
}

fn add(stamp: &str, extra: Duration) -> String {
    match time::OffsetDateTime::parse(stamp, &Rfc3339) {
        Ok(t) => (t + time::Duration::milliseconds(extra.as_millis() as i64))
            .format(&Rfc3339)
            .unwrap_or_else(|_| stamp.to_string()),
        Err(_) => stamp.to_string(),
    }
}

/// How long ago `created_at` was, relative to `now` — both stamps this
/// server wrote itself, so an unparseable one is not expected to happen;
/// treated as "just now" rather than panicking either way.
fn age(created_at: &str, now: &str) -> Duration {
    match (
        time::OffsetDateTime::parse(created_at, &Rfc3339),
        time::OffsetDateTime::parse(now, &Rfc3339),
    ) {
        (Ok(created), Ok(now)) => {
            let millis = (now - created).whole_milliseconds();
            Duration::from_millis(millis.max(0) as u64)
        }
        _ => Duration::ZERO,
    }
}

/// Runs until the process exits. One loop: wait for a wake or the poll
/// interval, fetch what is due, attempt each, reschedule or remove.
/// Resuming pending rows after a restart is nothing special — they are
/// just rows whose `next_attempt` is already in the past.
pub async fn run(state: Shared) {
    let client = http_client();
    loop {
        tokio::select! {
            _ = tokio::time::sleep(POLL_INTERVAL) => {}
            _ = state.feedback.wake.notified() => {}
        }
        let now = crate::index::now_rfc3339();
        let due = match state.feedback.queue.due(&now, BATCH).await {
            Ok(rows) => rows,
            Err(e) => {
                tracing::warn!(error = %e, "feedback queue: could not read due sends");
                continue;
            }
        };
        if due.is_empty() {
            continue;
        }
        let intake_url = {
            let vaults = state.vaults.read().await;
            vaults.registry.feedback_intake_url().to_string()
        };
        for row in due {
            if age(&row.created_at, &now) >= GIVE_UP_AFTER {
                tracing::warn!(report_id = %row.report_id, attempts = row.attempts, "feedback: giving up after 7 days");
                let _ = state.feedback.queue.remove(&row.report_id).await;
                continue;
            }
            match attempt(&client, &intake_url, &row.body).await {
                Outcome::Delivered => {
                    tracing::info!(report_id = %row.report_id, attempts = row.attempts, "feedback delivered");
                    let _ = state.feedback.queue.remove(&row.report_id).await;
                }
                Outcome::Drop { reason } => {
                    tracing::warn!(report_id = %row.report_id, reason, "feedback: dropped permanently");
                    let _ = state.feedback.queue.remove(&row.report_id).await;
                }
                Outcome::Retry { after, reason } => {
                    let attempts = row.attempts + 1;
                    let backoff = after.unwrap_or_else(|| exponential_backoff(attempts));
                    let next_attempt = add(&now, backoff);
                    tracing::info!(report_id = %row.report_id, attempts, reason, "feedback: retrying later");
                    let _ = state
                        .feedback
                        .queue
                        .reschedule(&row.report_id, attempts, &next_attempt, reason)
                        .await;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_doubles_and_is_capped() {
        assert_eq!(exponential_backoff(1), BASE_BACKOFF);
        assert_eq!(exponential_backoff(2), BASE_BACKOFF * 2);
        assert_eq!(exponential_backoff(3), BASE_BACKOFF * 4);
        assert_eq!(exponential_backoff(30), MAX_BACKOFF);
    }

    #[test]
    fn age_is_the_gap_between_two_of_this_servers_own_stamps() {
        let created = "2026-11-19T00:00:00Z";
        let now = "2026-11-19T00:00:01Z";
        assert_eq!(age(created, now), Duration::from_secs(1));
        assert_eq!(age(now, now), Duration::ZERO);
    }

    #[test]
    fn add_moves_a_stamp_forward_by_exactly_the_backoff() {
        let moved = add("2026-11-19T00:00:00Z", Duration::from_secs(90));
        assert_eq!(age("2026-11-19T00:00:00Z", &moved), Duration::from_secs(90));
    }
}
