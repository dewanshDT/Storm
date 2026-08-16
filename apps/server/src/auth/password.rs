//! Argon2id for passwords — and the bound that stops it taking the server down.
//!
//! The parameters are not a preference. A1 fixes the algorithm and the *method*
//! — measure on the deployment box, target 150–300 ms for one verify — and Q18
//! did the measuring on the homelab VM (3 vCPU, 3815 MB): `m = 192 MiB, t = 1,
//! p = 1` at 173.6 ms. `tools/argon2-bench` is the instrument, so re-measuring
//! on different hardware is a repeat rather than a redesign.
//!
//! Two things about that measurement are load-bearing here:
//!
//! - **`p` stays 1.** The `argon2` crate does not thread without its `parallel`
//!   feature, so raising `p` buys the memory cost of extra lanes and no speed.
//! - **Concurrency must be bounded, and this module is where.** A verify runs on
//!   `spawn_blocking`, whose pool defaults to 512 threads. 512 × 192 MiB is an
//!   OOM that takes the vault server down with it — the notes go offline because
//!   someone held the login button. [`Hasher`] holds a semaphore of
//!   [`MAX_CONCURRENT_HASHES`] permits so the peak is ~384 MiB no matter how
//!   many requests arrive. **Never shrink the KDF to work around a missing
//!   bound; the bound is the fix.**
//!
//! Passwords hash with Argon2id and tokens hash with blake3, on purpose — see
//! [`crate::auth`]. That is not an inconsistency to tidy up in either direction.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use anyhow::{Context, Result, anyhow};
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::{Algorithm, Argon2, Params, Version};
use rand::Rng;
use tokio::sync::Semaphore;

/// Measured on the deployment VM, 2026-08-16 (Q18 / A1). 192 MiB.
pub const MEMORY_KIB: u32 = 196_608;
pub const TIME_COST: u32 = 1;
pub const LANES: u32 = 1;

/// Simultaneous Argon2 operations allowed process-wide.
///
/// Two, because two × 192 MiB is a peak this box can hold (measured: 207 ms
/// wall for two at once) and 512 × 192 MiB is not.
pub const MAX_CONCURRENT_HASHES: usize = 2;

/// Counted in characters, not bytes, so a passphrase of short non-ASCII
/// characters is not judged longer than it looks.
pub const MIN_PASSWORD_CHARS: usize = 12;

/// A ceiling that **refuses**, never truncates.
///
/// Silent truncation is the classic version of this bug: accept 200 characters,
/// hash the first 72, and every password sharing that prefix now opens the
/// account. Argon2 has no such limit, so this exists only to stop a
/// megabyte-long body costing 192 MiB of hashing before it is rejected.
pub const MAX_PASSWORD_BYTES: usize = 1024;

/// Checks a *new* password against policy.
///
/// Deliberately not applied when verifying: a stored password predates any
/// later tightening of these rules, and rejecting it at login would lock out
/// exactly the accounts that most need to be able to get in and change it.
pub fn validate_password(password: &str) -> std::result::Result<(), String> {
    if password.len() > MAX_PASSWORD_BYTES {
        return Err(format!(
            "password is {} bytes; the maximum is {MAX_PASSWORD_BYTES}. \
             It is refused rather than shortened — a truncated password would \
             silently accept anything sharing its first {MAX_PASSWORD_BYTES} bytes.",
            password.len()
        ));
    }
    let chars = password.chars().count();
    if chars < MIN_PASSWORD_CHARS {
        return Err(format!(
            "password is {chars} characters; the minimum is {MIN_PASSWORD_CHARS}"
        ));
    }
    // A newline or a NUL in a password is an input-handling accident — a stray
    // line from a pipe, a paste that caught the line ending. Storing it would
    // create an account whose password cannot be typed at a prompt.
    if password.contains(['\n', '\r', '\0']) {
        return Err("password contains a newline or NUL byte".to_string());
    }
    Ok(())
}

/// Argon2id at the measured parameters.
fn argon() -> Argon2<'static> {
    let params =
        Params::new(MEMORY_KIB, TIME_COST, LANES, None).expect("the measured params are valid");
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
}

/// Whether a stored hash is weaker than what this build would write now.
///
/// Login rehashes when this is true, which is how a parameter bump reaches
/// existing accounts — nobody is asked to change their password because the VM
/// got faster. An unreadable or non-Argon2id hash also returns `true`: whatever
/// it is, it is not what this server would store today.
pub fn needs_rehash(phc: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(phc) else {
        return true;
    };
    if parsed.algorithm.as_str() != "argon2id" {
        return true;
    }
    let Ok(params) = Params::try_from(&parsed) else {
        return true;
    };
    params.m_cost() < MEMORY_KIB || params.t_cost() < TIME_COST || params.p_cost() < LANES
}

/// Bounded access to Argon2id.
///
/// Cloneable and shared: the bound is only a bound if every caller goes through
/// the same one. Cloning shares the semaphore rather than minting a second set
/// of permits.
#[derive(Clone)]
pub struct Hasher {
    permits: Arc<Semaphore>,
    in_flight: Arc<AtomicUsize>,
    peak: Arc<AtomicUsize>,
    total_jobs: Arc<AtomicUsize>,
}

impl Default for Hasher {
    fn default() -> Self {
        Self::new()
    }
}

/// Keeps the in-flight count honest even if the hashing job panics.
struct InFlight(Arc<AtomicUsize>);

impl Drop for InFlight {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Hasher {
    pub fn new() -> Self {
        Self::with_permits(MAX_CONCURRENT_HASHES)
    }

    fn with_permits(permits: usize) -> Self {
        Self {
            permits: Arc::new(Semaphore::new(permits)),
            in_flight: Arc::new(AtomicUsize::new(0)),
            peak: Arc::new(AtomicUsize::new(0)),
            total_jobs: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// The highest number of jobs this hasher has ever run at once.
    #[cfg(test)]
    pub fn peak_in_flight(&self) -> usize {
        self.peak.load(Ordering::SeqCst)
    }

    /// Total number of jobs this hasher has completed. Used in tests to prove
    /// that a missing user still pays for a hash.
    #[cfg(test)]
    pub fn jobs_run(&self) -> usize {
        self.total_jobs.load(Ordering::SeqCst)
    }

    /// Runs one memory-hard job on the blocking pool, holding a permit for its
    /// whole duration.
    ///
    /// The permit is acquired *before* the task is spawned and moved into it, so
    /// it is released when the work finishes rather than when this function
    /// returns. Acquiring inside the closure instead would put 512 threads on
    /// the pool queue, each already committed to allocating.
    async fn bounded<T, F>(&self, job: F) -> Result<T>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        let permit = self
            .permits
            .clone()
            .acquire_owned()
            .await
            .expect("the hashing semaphore is never closed");
        let in_flight = self.in_flight.clone();
        let peak = self.peak.clone();
        let total_jobs = self.total_jobs.clone();

        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let running = in_flight.fetch_add(1, Ordering::SeqCst) + 1;
            let _counted = InFlight(in_flight.clone());
            peak.fetch_max(running, Ordering::SeqCst);
            let result = job();
            total_jobs.fetch_add(1, Ordering::SeqCst);
            result
        })
        .await
        .context("the password hashing task failed")
    }

    /// Hashes a new password, returning a PHC string.
    ///
    /// The PHC string is self-describing — algorithm, version, parameters and
    /// salt all travel with the hash — which is what makes [`needs_rehash`]
    /// possible without a schema column per parameter.
    pub async fn hash(&self, password: String) -> Result<String> {
        validate_password(&password).map_err(|e| anyhow!(e))?;
        self.bounded(move || {
            // Salted from the same RNG the server ids use, rather than argon2's
            // own feature-gated generator, so there is one source of randomness
            // in the codebase to reason about. 16 bytes is Argon2's recommended
            // salt length.
            let mut salt_bytes = [0u8; 16];
            rand::rng().fill_bytes(&mut salt_bytes);
            let salt = SaltString::encode_b64(&salt_bytes)
                .map_err(|e| anyhow!("encoding the password salt failed: {e}"))?;
            argon()
                .hash_password(password.as_bytes(), &salt)
                .map(|h| h.to_string())
                .map_err(|e| anyhow!("hashing the password failed: {e}"))
        })
        .await?
    }

    /// Verifies a password against a stored PHC string.
    ///
    /// The parameters come from the *stored* hash, not from this build's
    /// constants, so a hash written before a parameter change still verifies.
    /// A wrong password is `Ok(false)`; only an unreadable stored hash is an
    /// error, because those two need different handling at the call site.
    pub async fn verify(&self, password: String, phc: String) -> Result<bool> {
        self.bounded(move || {
            let parsed = PasswordHash::new(&phc)
                .map_err(|e| anyhow!("the stored password hash is unreadable: {e}"))?;
            match Argon2::default().verify_password(password.as_bytes(), &parsed) {
                Ok(()) => Ok(true),
                Err(argon2::password_hash::Error::Password) => Ok(false),
                Err(e) => Err(anyhow!("verifying the password failed: {e}")),
            }
        })
        .await?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_parameters_are_the_ones_measured_on_the_vm() {
        // Q18's answer, pinned. Changing these is a re-measure on the target
        // box (A1), not an edit — the numbers are only defensible with the
        // hardware they were taken on.
        assert_eq!(MEMORY_KIB, 196_608, "192 MiB");
        assert_eq!(TIME_COST, 1);
        assert_eq!(
            LANES, 1,
            "the argon2 crate does not thread without `parallel`"
        );
    }

    #[test]
    fn a_short_password_is_refused_and_a_long_one_is_not_truncated() {
        assert!(validate_password("short").is_err());
        assert!(validate_password(&"a".repeat(MIN_PASSWORD_CHARS - 1)).is_err());
        assert!(validate_password(&"a".repeat(MIN_PASSWORD_CHARS)).is_ok());

        // At the ceiling: accepted. Over it: refused, and the message says
        // refused rather than shortened.
        assert!(validate_password(&"a".repeat(MAX_PASSWORD_BYTES)).is_ok());
        let err = validate_password(&"a".repeat(MAX_PASSWORD_BYTES + 1)).unwrap_err();
        assert!(err.contains("refused"), "{err}");
    }

    #[test]
    fn a_password_with_a_newline_is_refused() {
        assert!(validate_password("correct horse\n").is_err());
        assert!(validate_password("correct\0horse!").is_err());
    }

    #[tokio::test]
    async fn a_long_password_is_hashed_whole() {
        // The failure this guards is silent truncation: if only the first N
        // bytes were hashed, two passwords sharing that prefix would both open
        // the account. They differ at byte 900 of 1000.
        let hasher = Hasher::new();
        let mut a = "x".repeat(1000);
        let mut b = a.clone();
        a.replace_range(900..901, "1");
        b.replace_range(900..901, "2");

        let phc = hasher.hash(a.clone()).await.unwrap();
        assert!(hasher.verify(a, phc.clone()).await.unwrap());
        assert!(
            !hasher.verify(b, phc).await.unwrap(),
            "a password differing at byte 900 must not verify"
        );
    }

    #[tokio::test]
    async fn a_hash_round_trips_and_a_wrong_password_is_false_not_an_error() {
        let hasher = Hasher::new();
        let phc = hasher.hash("correct horse battery".into()).await.unwrap();

        // The stored parameters are visible in the PHC string, which is what
        // lets needs_rehash work without extra columns.
        assert!(phc.starts_with("$argon2id$"), "{phc}");
        assert!(phc.contains("m=196608,t=1,p=1"), "{phc}");

        assert!(
            hasher
                .verify("correct horse battery".into(), phc.clone())
                .await
                .unwrap()
        );
        assert!(
            !hasher
                .verify("wrong horse battery".into(), phc)
                .await
                .unwrap(),
            "a wrong password is a false, not an error"
        );
    }

    #[tokio::test]
    async fn an_unreadable_stored_hash_is_an_error_not_a_silent_false() {
        // A corrupted row must not be indistinguishable from a wrong password,
        // or the operator debugs the user's memory instead of their database.
        let hasher = Hasher::new();
        let err = hasher
            .verify("correct horse battery".into(), "not-a-phc-string".into())
            .await;
        assert!(err.is_err());
    }

    #[test]
    fn a_weaker_stored_hash_wants_rehashing() {
        // A real PHC string at OWASP's lower setting: weaker than the measured
        // parameters, so login should upgrade it.
        let weak = "$argon2id$v=19$m=19456,t=2,p=1$c29tZXNhbHQ$\
                    aGFzaGhhc2hoYXNoaGFzaGhhc2hoYXNoaGFzaGhhcw";
        assert!(needs_rehash(weak));
        assert!(needs_rehash("not-a-phc-string"));
        // bcrypt, or anything else that is not what this server writes today.
        assert!(needs_rehash(
            "$2b$12$K1p5tS9v9J8m0hZ9r0dQeOa1b2c3d4e5f6g7h8i9j0k1l2m3n4o5p"
        ));
    }

    #[tokio::test]
    async fn the_current_parameters_do_not_want_rehashing() {
        let hasher = Hasher::new();
        let phc = hasher.hash("correct horse battery".into()).await.unwrap();
        assert!(
            !needs_rehash(&phc),
            "a hash this build just wrote must not ask to be rewritten on every login"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_hashing_is_bounded_by_the_semaphore() {
        // The whole point of the bound. Without it this reaches 8 in flight,
        // each holding 192 MiB — 1.5 GB for eight simultaneous logins, and the
        // real pool allows 512.
        let hasher = Hasher::with_permits(MAX_CONCURRENT_HASHES);
        let mut jobs = Vec::new();
        for _ in 0..8 {
            let h = hasher.clone();
            jobs.push(tokio::spawn(async move {
                // Cheap stand-in for the KDF: this test is about the gate, not
                // about Argon2, and running eight real hashes to prove it would
                // cost 1.5 GB on whichever machine ran the suite.
                h.bounded(|| std::thread::sleep(std::time::Duration::from_millis(30)))
                    .await
                    .unwrap();
            }));
        }
        for job in jobs {
            job.await.unwrap();
        }
        assert!(
            hasher.peak_in_flight() <= MAX_CONCURRENT_HASHES,
            "{} ran at once; the semaphore allows {MAX_CONCURRENT_HASHES}",
            hasher.peak_in_flight()
        );
        assert!(
            hasher.peak_in_flight() > 1,
            "the test did not actually overlap, so it proves nothing"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn real_hashing_goes_through_the_bound() {
        // The test above proves the gate works; this one proves `hash` is
        // actually behind it. Deleting the `bounded` call from `hash` leaves
        // that test passing and fails this one.
        let hasher = Hasher::with_permits(1);
        let a = hasher.clone();
        let b = hasher.clone();
        let first = tokio::spawn(async move { a.hash("correct horse battery".into()).await });
        let second = tokio::spawn(async move { b.hash("correct horse battery".into()).await });
        first.await.unwrap().unwrap();
        second.await.unwrap().unwrap();
        assert_eq!(
            hasher.peak_in_flight(),
            1,
            "two hashes overlapped despite a single permit"
        );
    }
}
