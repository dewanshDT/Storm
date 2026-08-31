use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// A token-bucket rate limiter per source IP.
///
/// Used to limit `HELLO` attempts from a single IP. The bucket refills at a
/// steady rate (`limit` tokens per `window`), so a burst up to `limit` is
/// allowed, then the rate is enforced.
#[derive(Debug)]
pub struct RateLimiter {
    limit: usize,
    window: Duration,
    buckets: Mutex<HashMap<IpAddr, Bucket>>,
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
            buckets: Mutex::new(HashMap::new()),
        }
    }

    /// Tries to take one token. Returns `true` if allowed, `false` if rate limited.
    pub fn try_take(&self, ip: IpAddr) -> bool {
        let mut buckets = self.buckets.lock().expect("rate limiter mutex");
        let now = Instant::now();

        let bucket = buckets.entry(ip).or_insert_with(|| Bucket {
            tokens: self.limit as f64,
            last_refill: now,
        });

        // Refill tokens based on elapsed time.
        let elapsed = now.duration_since(bucket.last_refill).as_secs_f64();
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
    /// Call periodically (e.g., on each check) to prevent unbounded growth.
    pub fn prune_stale(&self, now: Instant) {
        let mut buckets = self.buckets.lock().expect("rate limiter mutex");
        let stale_after = self.window * 2;
        buckets.retain(|_, bucket| now.duration_since(bucket.last_refill) < stale_after);
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
        {
            let buckets = limiter.buckets.lock().unwrap();
            assert!(buckets.contains_key(&ip));
        }

        // Advance time beyond 2 * window
        let future = now + Duration::from_millis(150);
        limiter.prune_stale(future);
        {
            let buckets = limiter.buckets.lock().unwrap();
            assert!(!buckets.contains_key(&ip));
        }
    }
}
