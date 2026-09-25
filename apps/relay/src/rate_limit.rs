use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// A token-bucket rate limiter per source IP.
///
/// Used to limit `HELLO` attempts from a single IP. The bucket refills at a
/// steady rate (`limit` tokens per `window`), so a burst up to `limit` is
/// allowed, then the rate is enforced.
///
/// **It prunes itself.** Every distinct source address gets a bucket, so a
/// map nobody pruned would grow by one entry per address for the life of the
/// process, which on a public address is a memory leak an anonymous caller
/// drives. `prune_stale` existed from the start with no caller outside its own
/// test. Now `try_take` runs it at most once per window.
#[derive(Debug)]
pub struct RateLimiter {
    limit: usize,
    window: Duration,
    inner: Mutex<Inner>,
}

#[derive(Debug)]
struct Inner {
    buckets: HashMap<IpAddr, Bucket>,
    last_prune: Instant,
}

#[derive(Debug, Clone, Copy)]
struct Bucket {
    tokens: f64,
    last_refill: Instant,
}

impl RateLimiter {
    pub fn new(limit: usize, window: Duration) -> Self {
        Self {
            limit,
            window,
            inner: Mutex::new(Inner {
                buckets: HashMap::new(),
                last_prune: Instant::now(),
            }),
        }
    }

    /// Tries to take one token. Returns `true` if allowed, `false` if rate limited.
    pub fn try_take(&self, ip: IpAddr) -> bool {
        self.try_take_at(ip, Instant::now())
    }

    /// [`try_take`](Self::try_take) at a given instant, so a test can move time
    /// forward instead of sleeping through two windows.
    fn try_take_at(&self, ip: IpAddr, now: Instant) -> bool {
        let mut inner = self.inner.lock().expect("rate limiter mutex");

        // Amortised: one O(n) sweep per window, never one per call.
        if now.saturating_duration_since(inner.last_prune) >= self.window {
            Self::prune(&mut inner.buckets, self.window, now);
            inner.last_prune = now;
        }

        let bucket = inner.buckets.entry(ip).or_insert_with(|| Bucket {
            tokens: self.limit as f64,
            last_refill: now,
        });

        // Refill tokens based on elapsed time.
        let elapsed = now
            .saturating_duration_since(bucket.last_refill)
            .as_secs_f64();
        let refill_rate = self.limit as f64 / self.window.as_secs_f64();
        bucket.tokens = (bucket.tokens + elapsed * refill_rate).min(self.limit as f64);
        bucket.last_refill = now;

        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            true
        } else {
            false
        }
    }

    /// Removes entries that have not been used for `2 * window`.
    ///
    /// Pruning is **lossless**, which is why it can run whenever it likes. A
    /// bucket untouched for a full window has refilled to `limit`, which is
    /// exactly the fresh bucket that would replace it, so forgetting one never
    /// lets through a caller who would otherwise have been refused. The second
    /// window is margin, not correctness.
    pub fn prune_stale(&self, now: Instant) {
        let mut inner = self.inner.lock().expect("rate limiter mutex");
        Self::prune(&mut inner.buckets, self.window, now);
    }

    fn prune(buckets: &mut HashMap<IpAddr, Bucket>, window: Duration, now: Instant) {
        let stale_after = window * 2;
        buckets.retain(|_, bucket| now.saturating_duration_since(bucket.last_refill) < stale_after);
    }

    #[cfg(test)]
    fn tracked(&self) -> usize {
        self.inner.lock().expect("rate limiter mutex").buckets.len()
    }
}

impl Default for RateLimiter {
    fn default() -> Self {
        use crate::config::{DEFAULT_HELLO_RATE_LIMIT, DEFAULT_HELLO_RATE_WINDOW};
        Self::new(DEFAULT_HELLO_RATE_LIMIT, DEFAULT_HELLO_RATE_WINDOW)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};
    use std::time::Duration;

    #[test]
    fn allows_burst_up_to_limit() {
        let limiter = RateLimiter::new(3, Duration::from_secs(60));
        let ip = IpAddr::V4(Ipv4Addr::new(1, 2, 3, 4));

        assert!(limiter.try_take(ip));
        assert!(limiter.try_take(ip));
        assert!(limiter.try_take(ip));
        assert!(!limiter.try_take(ip)); // 4th is denied
    }

    #[test]
    fn refills_over_time() {
        let limiter = RateLimiter::new(2, Duration::from_millis(100));
        let ip = IpAddr::V4(Ipv4Addr::new(1, 2, 3, 4));

        assert!(limiter.try_take(ip));
        assert!(limiter.try_take(ip));
        assert!(!limiter.try_take(ip));

        std::thread::sleep(Duration::from_millis(150));
        // ~3 tokens should have been added (150ms / 100ms * 2 = 3), capped at 2
        assert!(limiter.try_take(ip));
    }

    #[test]
    fn separate_ips_independent() {
        let limiter = RateLimiter::new(1, Duration::from_secs(60));
        let ip1 = IpAddr::V4(Ipv4Addr::new(1, 2, 3, 4));
        let ip2 = IpAddr::V4(Ipv4Addr::new(5, 6, 7, 8));

        assert!(limiter.try_take(ip1));
        assert!(!limiter.try_take(ip1)); // ip1 exhausted
        assert!(limiter.try_take(ip2)); // ip2 independent
    }

    #[test]
    fn prune_removes_stale_entries() {
        let limiter = RateLimiter::new(10, Duration::from_millis(50));
        let ip = IpAddr::V4(Ipv4Addr::new(1, 2, 3, 4));
        limiter.try_take(ip);

        let now = Instant::now();
        limiter.prune_stale(now); // not stale yet
        assert_eq!(limiter.tracked(), 1);

        // Advance time beyond 2 * window
        let future = now + Duration::from_millis(150);
        limiter.prune_stale(future);
        assert_eq!(limiter.tracked(), 0);
    }

    #[test]
    fn the_map_is_pruned_by_ordinary_use_not_only_when_asked() {
        // The regression: `prune_stale` had no production caller, so every
        // source address ever seen stayed in the map for good.
        let window = Duration::from_secs(60);
        let limiter = RateLimiter::new(10, window);
        let start = Instant::now();
        for n in 0..=255u8 {
            limiter.try_take_at(IpAddr::V4(Ipv4Addr::new(10, 0, 0, n)), start);
        }
        assert_eq!(limiter.tracked(), 256);

        // Three windows later, one ordinary call from a new address.
        let later = start + window * 3;
        assert!(limiter.try_take_at(IpAddr::V4(Ipv4Addr::new(10, 0, 1, 1)), later));
        assert_eq!(limiter.tracked(), 1);
    }

    #[test]
    fn pruning_forgets_nobody_who_was_still_limited() {
        // An exhausted bucket from within the last window must survive a
        // sweep, or pruning would be a way to reset your own limit.
        let window = Duration::from_secs(60);
        let limiter = RateLimiter::new(2, window);
        let ip = IpAddr::V4(Ipv4Addr::new(1, 2, 3, 4));
        let start = Instant::now();
        assert!(limiter.try_take_at(ip, start));
        assert!(limiter.try_take_at(ip, start));
        assert!(!limiter.try_take_at(ip, start));

        // A sweep runs on this call (a window has passed since construction),
        // yet the bucket was used moments ago and is still limited.
        let just_after = start + window + Duration::from_millis(1);
        let other = IpAddr::V4(Ipv4Addr::new(9, 9, 9, 9));
        limiter.try_take_at(other, just_after);
        assert_eq!(limiter.tracked(), 2);
    }
}
