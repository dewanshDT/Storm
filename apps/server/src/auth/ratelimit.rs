//! Login-path rate limiting — the ceiling that keeps Argon2id purchasable.
//!
//! `login()` runs a full Argon2id verify even for a username that does not
//! exist (deliberately, so response time cannot enumerate users), and
//! [`crate::auth::Hasher`] bounds *concurrency*, not rate: two permits at
//! ~174 ms each is ~11 verifies/sec for the entire server. Anyone who can
//! fetch the web app can pair a device and flood `/v1/auth/login` until login
//! is dead for every real user — and a junk username never trips the per-user
//! lockout, because there is no account to lock. Relay Security §8.1 calls
//! this a hard blocker for anything reachable off-LAN.
//!
//! Two buckets, per the design's "generous per-caller, strict global":
//!
//! - **Per-caller** (30/min, burst 30) — generous, because a relay collapses a
//!   household behind one NAT address and must not lock out a family.
//! - **Global** (60/min, burst 60) — strict, ~9% of the Argon2 ceiling, so a
//!   distributed flood still leaves real logins headroom.
//!
//! Hand-rolled token buckets rather than `governor`/`tower-governor`: this is
//! ~100 testable lines against a new dependency, and the existing per-IP
//! throttle (`web_bootstrap_nonce`) is already hand-rolled in the handler.
//! House pattern.
//!
//! **The budget is charged before the Argon2 permit is acquired**, and
//! refunded on a successful login — a server under attack must still let real
//! users in, which means the attacker's spent budget must not be the thing
//! that keeps a legitimate attempt out.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// One bucket's shape: how many attempts it holds, and how fast it refills.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    capacity: f64,
    rate_per_sec: f64,
}

impl Limits {
    /// `burst` attempts held, refilling `per_minute` back over a minute.
    pub const fn per_minute(burst: f64, per_minute: f64) -> Self {
        Self {
            capacity: burst,
            rate_per_sec: per_minute / 60.0,
        }
    }
}

/// Generous: a relay collapses a household behind one NAT address.
const PER_CALLER: Limits = Limits::per_minute(30.0, 30.0);

/// Strict: this is the half that protects the two Argon2 permits.
const GLOBAL: Limits = Limits::per_minute(60.0, 60.0);

/// A per-caller entry idle this long is evicted, so a scan of many source IPs
/// cannot grow the map without bound. Twice the time it takes a full bucket
/// to refill — an address quiet that long has lost nothing an eviction costs.
const IDLE_EVICTION: Duration = Duration::from_secs(120);

/// Who the server thinks is calling. [`CallerKey::Unattributed`] exists for
/// callers with no socket — the relay's in-process dispatch reconstructs an
/// `http::Request` and calls the router as a tower service, so there is no
/// `ConnectInfo`. It maps to its own single shared bucket: an unknown caller
/// is still bounded, never unlimited. This is the seam `relay_peer_ip`
/// fills in during relay phase 2 ([[Relay Server Integration]] §3).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CallerKey {
    Ip(IpAddr),
    Unattributed,
}

#[derive(Debug)]
struct Bucket {
    tokens: f64,
    updated: Instant,
}

impl Bucket {
    fn full(limits: Limits) -> Self {
        Self {
            tokens: limits.capacity,
            updated: Instant::now(),
        }
    }

    fn refill(&mut self, limits: Limits) {
        let now = Instant::now();
        let elapsed = now.duration_since(self.updated).as_secs_f64();
        self.tokens = (self.tokens + elapsed * limits.rate_per_sec).min(limits.capacity);
        self.updated = now;
    }

    /// Takes one token, or returns the seconds until one is available.
    fn take(&mut self, limits: Limits) -> Result<(), i64> {
        self.refill(limits);
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            Ok(())
        } else {
            let secs = ((1.0 - self.tokens) / limits.rate_per_sec).ceil().max(1.0);
            Err(secs as i64)
        }
    }

    /// Returns one token on a successful login. Capped at capacity, like a
    /// refill would be.
    fn refund(&mut self, limits: Limits) {
        self.refill(limits);
        self.tokens = (self.tokens + 1.0).min(limits.capacity);
    }
}

/// One instance in [`crate::api::AppState`], beside the hasher, for the same
/// reason the hasher lives there: a per-handler limiter would be decorative in
/// exactly the way a per-handler `Hasher::new()` was.
///
/// Deliberately **not** keyed on `X-Forwarded-For`: the header is
/// client-forgeable, and `FORWARDING_HEADERS` (`api.rs`) is load-bearing for a
/// different, non-security check. The relay carries the peer IP out of band;
/// until then the socket address is the whole truth.
pub struct LoginLimiter {
    per_caller_limits: Limits,
    global_limits: Limits,
    global: Mutex<Bucket>,
    per_caller: Mutex<HashMap<CallerKey, Bucket>>,
}

impl Default for LoginLimiter {
    fn default() -> Self {
        Self::new()
    }
}

impl LoginLimiter {
    pub fn new() -> Self {
        Self::with_limits(PER_CALLER, GLOBAL)
    }

    /// The production limits are the ones in this module; tests pass their own
    /// so a burst can be exhausted in three requests rather than thirty. That
    /// is not a convenience: each attempt that reaches the handler pays a real
    /// Argon2id verify, and in a debug build the sustained refill outruns the
    /// KDF — a bucket sized for production never empties, and the test fails
    /// by asserting a 429 that correct code will not produce.
    pub fn with_limits(per_caller_limits: Limits, global_limits: Limits) -> Self {
        Self {
            per_caller_limits,
            global_limits,
            global: Mutex::new(Bucket::full(global_limits)),
            per_caller: Mutex::new(HashMap::new()),
        }
    }

    /// Charges both buckets for one login attempt. `Err(secs)` means refused:
    /// it is already the `Retry-After` value, so the handler can map it
    /// straight onto the existing 429 shape.
    ///
    /// A caller whose own bucket refuses does not touch the global bucket —
    /// one noisy address must not spend everyone's budget on refusals.
    pub fn check(&self, caller: &CallerKey) -> Result<(), i64> {
        {
            let mut per_caller = self.per_caller.lock().unwrap();
            Self::evict_idle(&mut per_caller, self.per_caller_limits);
            let limits = self.per_caller_limits;
            let bucket = per_caller
                .entry(caller.clone())
                .or_insert_with(|| Bucket::full(limits));
            // The lock is released before the global bucket is charged, so a
            // slow caller cannot hold the whole map while everyone waits.
            bucket.take(limits)?;
        }
        self.global.lock().unwrap().take(self.global_limits)
    }

    /// Returns one token to each bucket after a successful login.
    pub fn refund(&self, caller: &CallerKey) {
        self.global.lock().unwrap().refund(self.global_limits);
        if let Some(bucket) = self.per_caller.lock().unwrap().get_mut(caller) {
            bucket.refund(self.per_caller_limits);
        }
    }

    fn evict_idle(map: &mut HashMap<CallerKey, Bucket>, limits: Limits) {
        // `updated` is refreshed by every take and refund, so it *is* the
        // last-activity stamp; a bucket idle this long has refilled to full
        // anyway, so dropping it loses nothing — the caller comes back to a
        // fresh bucket holding exactly what the kept one would have held.
        let idle = IDLE_EVICTION.max(Duration::from_secs_f64(
            limits.capacity / limits.rate_per_sec,
        ));
        map.retain(|_, bucket| bucket.updated.elapsed() < idle);
    }
}

/// Visible to the api.rs router tests, which drive the same numbers
/// through HTTP so the two suites cannot drift apart.
#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr};

    /// What the router tests build their limiter from: a burst of two, so a
    /// third attempt is refused after two Argon2id verifies instead of thirty.
    /// The refill is slow enough that no realistic test runtime tops the
    /// bucket back up mid-assertion.
    pub(crate) const TEST_BURST: usize = 2;
    pub(crate) const TEST_LIMITS: Limits = Limits::per_minute(TEST_BURST as f64, 1.0);

    fn ip(a: u8) -> CallerKey {
        CallerKey::Ip(IpAddr::V4(Ipv4Addr::new(192, 168, 1, a)))
    }

    #[test]
    fn a_full_bucket_allows_a_burst_then_refuses() {
        let limiter = LoginLimiter::new();
        let caller = ip(2);
        for _ in 0..PER_CALLER.capacity as i32 {
            assert!(limiter.check(&caller).is_ok());
        }
        let retry_after = limiter.check(&caller).unwrap_err();
        assert!(
            retry_after >= 1,
            "Retry-After must be a positive whole second"
        );
    }

    #[test]
    fn buckets_refill_over_time() {
        let limits = Limits {
            capacity: 2.0,
            rate_per_sec: 1.0,
        };
        let mut bucket = Bucket::full(limits);
        assert!(bucket.take(limits).is_ok());
        assert!(bucket.take(limits).is_ok());
        assert!(bucket.take(limits).is_err());
        // One second at 1 token/sec buys exactly the next attempt.
        bucket.tokens = 0.0;
        bucket.updated -= Duration::from_secs(1);
        assert!(bucket.take(limits).is_ok());
    }

    #[test]
    fn callers_are_isolated() {
        let limiter = LoginLimiter::new();
        let flooder = ip(3);
        for _ in 0..PER_CALLER.capacity as i32 {
            limiter.check(&flooder).unwrap();
        }
        assert!(limiter.check(&flooder).is_err());
        // The flood did not touch anyone else's budget...
        assert!(limiter.check(&ip(4)).is_ok());
        // ...and an IPv6 caller is a different caller, not a parse failure.
        assert!(
            limiter
                .check(&CallerKey::Ip(IpAddr::V6(Ipv6Addr::LOCALHOST)))
                .is_ok()
        );
    }

    #[test]
    fn the_global_bucket_trips_while_per_caller_still_has_budget() {
        // The distributed-flood case: every caller stays under its own limit,
        // and the global ceiling is what saves the Argon2 semaphore.
        let limiter = LoginLimiter::new();
        let mut caller = 10u8;
        // One attempt each from a fresh address, until the global bucket says
        // no. Every one of them is comfortably inside its own per-caller
        // budget, so a per-IP limit alone would have let all of them through.
        while limiter.check(&ip(caller)).is_ok() {
            caller = caller
                .checked_add(1)
                .expect("ran out of test addresses before the global bucket tripped");
        }
        // The refusing caller had barely dented its own bucket.
        let per_caller = limiter.per_caller.lock().unwrap();
        let bucket = &per_caller[&ip(caller)];
        assert!(bucket.tokens > PER_CALLER.capacity / 2.0);
    }

    #[test]
    fn refund_on_success_restores_budget() {
        let limiter = LoginLimiter::new();
        let caller = ip(5);
        // Spend this caller's whole burst.
        for _ in 0..PER_CALLER.capacity as i32 {
            limiter.check(&caller).unwrap();
        }
        assert!(limiter.check(&caller).is_err());
        // A success refunds, so the caller is not stranded at zero: the next
        // attempt goes through instead of waiting a full refill window.
        limiter.refund(&caller);
        assert!(limiter.check(&caller).is_ok());
    }

    #[test]
    fn unattributed_callers_share_one_bounded_bucket() {
        // In-process dispatch has no socket. It must be bounded, not unlimited
        // — but bounded by *one* shared bucket, not one per imaginary caller.
        let limiter = LoginLimiter::new();
        let unattributed = CallerKey::Unattributed;
        for _ in 0..PER_CALLER.capacity as i32 {
            assert!(limiter.check(&unattributed).is_ok());
        }
        assert!(limiter.check(&unattributed).is_err());
        assert_eq!(limiter.per_caller.lock().unwrap().len(), 1);
    }

    #[test]
    fn idle_buckets_are_evicted_so_the_map_cannot_grow_without_bound() {
        let limiter = LoginLimiter::new();
        for a in 0..50u8 {
            limiter.check(&ip(a)).unwrap();
        }
        assert_eq!(limiter.per_caller.lock().unwrap().len(), 50);
        // Age everything past the eviction window with a full bucket.
        for bucket in limiter.per_caller.lock().unwrap().values_mut() {
            bucket.updated -= IDLE_EVICTION * 2;
        }
        limiter.check(&ip(99)).unwrap();
        assert_eq!(
            limiter.per_caller.lock().unwrap().len(),
            1,
            "only the new caller survives eviction"
        );
    }
}
