use std::time::Duration;

use time::format_description::well_known::Rfc3339;

use crate::api::Shared;

// Scaled down under test so a retry sequence costs milliseconds, not
// minutes or days.
const BASE_BACKOFF: Duration = if cfg!(test) {
    Duration::from_millis(15)
} else {
    Duration::from_secs(1)
};
const MAX_BACKOFF: Duration = if cfg!(test) {
    Duration::from_millis(200)
} else {
    Duration::from_secs(60 * 60)
};
// Comfortably past anything one attempt could take (connect_timeout below,
// plus the real request timeout), so a single slow failure never outruns
// the give-up clock. On Windows a connect to a closed loopback port can
// take ~2 s (the OS retries the SYN before answering RST).
const GIVE_UP_AFTER: Duration = if cfg!(test) {
    Duration::from_secs(120)
} else {
    Duration::from_secs(7 * 24 * 60 * 60)
};
const POLL_INTERVAL: Duration = if cfg!(test) {
    Duration::from_millis(10)
} else {
    Duration::from_secs(5)
};
const BATCH: usize = 25;
const ATTEMPT_TIMEOUT: Duration = Duration::from_secs(30);
// Bounds a dead-connection attempt client-side rather than depend on how
// fast an OS answers a closed port (see GIVE_UP_AFTER above).
const CONNECT_TIMEOUT: Duration = if cfg!(test) {
    Duration::from_millis(300)
} else {
    Duration::from_secs(10)
};

pub fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .use_preconfigured_tls(crate::gateway::upstream::tls_config())
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(ATTEMPT_TIMEOUT)
        .build()
        .expect("the feedback forwarder's HTTP client builds")
}

enum Outcome {
    Delivered,
    Retry {
        after: Option<Duration>,
        reason: String,
        unexpected: bool,
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
                reason: "network".into(),
                unexpected: false,
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
        400 => Outcome::Drop { reason: "400" },
        413 => Outcome::Drop { reason: "413" },
        429 => Outcome::Retry {
            after: retry_after,
            reason: "429".into(),
            unexpected: false,
        },
        503 => Outcome::Retry {
            after: retry_after,
            reason: "503".into(),
            unexpected: false,
        },
        s if (500..600).contains(&s) => Outcome::Retry {
            after: retry_after,
            reason: "5xx".into(),
            unexpected: false,
        },
        s => Outcome::Retry {
            after: retry_after,
            reason: s.to_string(),
            unexpected: true,
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
        let (intake_url, enabled) = {
            let vaults = state.vaults.read().await;
            (
                vaults.registry.feedback_intake_url().to_string(),
                vaults.registry.feedback_enabled(),
            )
        };
        for row in due {
            if age(&row.created_at, &now) >= GIVE_UP_AFTER {
                tracing::warn!(report_id = %row.report_id, attempts = row.attempts, "feedback: giving up after 7 days");
                let _ = state.feedback.queue.remove(&row.report_id).await;
                continue;
            }
            // Off means nothing leaves; the row stays queued and ages
            // toward the 7-day cap regardless.
            if !enabled {
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
                Outcome::Retry {
                    after,
                    reason,
                    unexpected,
                } => {
                    let attempts = row.attempts + 1;
                    let backoff = after.unwrap_or_else(|| exponential_backoff(attempts));
                    let next_attempt = add(&now, backoff);
                    if unexpected {
                        tracing::warn!(report_id = %row.report_id, attempts, status = %reason, "feedback: intake answered an unexpected status; holding for retry");
                    } else {
                        tracing::info!(report_id = %row.report_id, attempts, reason = %reason, "feedback: retrying later");
                    }
                    let _ = state
                        .feedback
                        .queue
                        .reschedule(&row.report_id, attempts, &next_attempt, &reason)
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
