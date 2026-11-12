//! A Shell started with a line typed in (decision 153-S3). The server sends
//! the line on `start` as data; the host writes it to the terminal once the
//! shell is ready, and never presses Enter: the person reads it and runs it.
//! It never reaches argv or the environment.
//!
//! The rule is `docs/typed-input-vectors.json`'s, which the server applies
//! first; a line it refuses fails the start here too.

use std::time::Duration;

/// The only provider a line is typed into.
pub const PROVIDER: &str = "shell";

pub const MAX_BYTES: usize = 1024;

const INVISIBLE: [std::ops::RangeInclusive<char>; 4] = [
    '\u{200B}'..='\u{200F}',
    '\u{202A}'..='\u{202E}',
    '\u{2060}'..='\u{2069}',
    '\u{FEFF}'..='\u{FEFF}',
];

/// One line of at most [`MAX_BYTES`] with no control character: nothing in
/// it presses Enter or sends a key, and it reads as it runs.
pub fn allowed(text: &str) -> bool {
    text.len() <= MAX_BYTES
        && !text
            .chars()
            .any(|c| c.is_control() || INVISIBLE.iter().any(|r| r.contains(&c)))
}

/// The shell is ready once it has printed something and then gone quiet for
/// this long: a prompt drawn by an rc file in several writes is drawn whole,
/// and line editing is on, before the line arrives.
pub const QUIET: Duration = Duration::from_millis(200);

/// A shell that prints nothing by then gets the line anyway; its terminal
/// holds it until the shell reads.
pub const DEADLINE: Duration = Duration::from_secs(10);

const POLL: Duration = Duration::from_millis(25);

/// Waits until a session whose output has reached `end()` bytes is ready
/// for its line, by [`QUIET`] after its first output or [`DEADLINE`]. False
/// when it ended first.
pub async fn ready(end: impl Fn() -> u64, ended: impl Fn() -> bool) -> bool {
    let begun = tokio::time::Instant::now();
    let mut last = 0;
    let mut since = begun;
    loop {
        if ended() {
            return false;
        }
        let now = tokio::time::Instant::now();
        let seen = end();
        if seen != last {
            last = seen;
            since = now;
        } else if seen > 0 && now - since >= QUIET {
            return true;
        }
        if now - begun >= DEADLINE {
            return true;
        }
        tokio::time::sleep(POLL).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

    #[test]
    fn the_vectors_rule_is_the_hosts() {
        let v: serde_json::Value =
            serde_json::from_str(include_str!("../../../docs/typed-input-vectors.json")).unwrap();
        assert_eq!(v["max_bytes"], MAX_BYTES);
        for ok in v["accepted"].as_array().unwrap() {
            assert!(allowed(ok.as_str().unwrap()), "{ok}");
        }
        for bad in v["refused"].as_array().unwrap() {
            assert!(!allowed(bad.as_str().unwrap()), "{bad}");
        }
    }

    #[tokio::test(start_paused = true)]
    async fn ready_waits_for_output_then_quiet() {
        let out = Arc::new(AtomicU64::new(0));
        let o = out.clone();
        let feeder = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(2)).await;
            o.store(10, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(150)).await;
            o.store(20, Ordering::SeqCst);
        });
        let begun = tokio::time::Instant::now();
        let o = out.clone();
        assert!(ready(move || o.load(Ordering::SeqCst), || false).await);
        let took = begun.elapsed();
        assert!(
            took >= Duration::from_millis(2150) + QUIET && took < Duration::from_secs(3),
            "{took:?}"
        );
        feeder.await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn a_silent_shell_gets_its_line_at_the_deadline_and_an_ended_one_never() {
        let begun = tokio::time::Instant::now();
        assert!(ready(|| 0, || false).await);
        assert!(begun.elapsed() >= DEADLINE);
        let ended = Arc::new(AtomicBool::new(false));
        let e = ended.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(1)).await;
            e.store(true, Ordering::SeqCst);
        });
        assert!(!ready(|| 0, move || ended.load(Ordering::SeqCst)).await);
    }
}
