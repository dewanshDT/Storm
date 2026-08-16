//! Measures Argon2id verify latency on the machine it runs on.
//!
//! Storm's A1 fixes Argon2id and says the parameters are **measured on the
//! deployment box**, targeting 150–300 ms for one verify: a KDF tuned on an
//! M-series laptop is a denial of service on a 2-vCPU guest. This is the
//! instrument for that, and it answers Q18.
//!
//! It measures `verify_password` against a real PHC string, which is exactly
//! what login does — Argon2 verification recomputes the hash, so a verify costs
//! the same as a hash and there is no cheaper path to measure.
//!
//! Deliberately dependency-light and pure Rust so it cross-compiles to
//! `x86_64-unknown-linux-musl` with plain cargo, and needs nothing installed on
//! the box being measured.

use std::time::{Duration, Instant};

use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::{Algorithm, Argon2, Params, Version};

/// Target window from A1. Below it the hash is too cheap to be worth much;
/// above it a login burst starts costing real latency.
const TARGET_MS: (f64, f64) = (150.0, 300.0);

/// Ceiling on what the burst test may allocate at once, in MiB.
///
/// This runs on a homelab box that is *also* serving the vault. A burst of
/// `concurrency x memory` is the one moment this program could push a live
/// server into swap, which would be a self-inflicted outage in the name of
/// measurement. Override with `STORM_BENCH_BURST_MIB` on a box with room.
const DEFAULT_BURST_BUDGET_MIB: f64 = 512.0;

fn burst_budget_mib() -> f64 {
    std::env::var("STORM_BENCH_BURST_MIB")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_BURST_BUDGET_MIB)
}

const PASSWORD: &[u8] = b"correct horse battery staple";

struct Sample {
    m_kib: u32,
    t: u32,
    p: u32,
    median_ms: f64,
    min_ms: f64,
    max_ms: f64,
}

impl Sample {
    /// Peak memory one verify holds, which is what multiplies under a burst.
    fn mib(&self) -> f64 {
        self.m_kib as f64 / 1024.0
    }

    fn in_target(&self) -> bool {
        self.median_ms >= TARGET_MS.0 && self.median_ms <= TARGET_MS.1
    }
}

fn argon2_for(m_kib: u32, t: u32, p: u32) -> Argon2<'static> {
    let params = Params::new(m_kib, t, p, None).expect("valid params");
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
}

/// One verify, timed. Returns the elapsed time and asserts it actually verified
/// — a benchmark that silently measures a rejection is measuring nothing.
fn time_verify(argon: &Argon2, phc: &str) -> Duration {
    let parsed = PasswordHash::new(phc).expect("parseable PHC string");
    let start = Instant::now();
    let ok = argon.verify_password(PASSWORD, &parsed).is_ok();
    let elapsed = start.elapsed();
    assert!(ok, "the benchmark verified the wrong password");
    elapsed
}

fn measure(m_kib: u32, t: u32, p: u32, samples: usize) -> Sample {
    let argon = argon2_for(m_kib, t, p);
    let salt = SaltString::from_b64("c3Rvcm1iZW5jaA").expect("valid salt");
    let phc = argon
        .hash_password(PASSWORD, &salt)
        .expect("hash succeeds")
        .to_string();

    // One warm-up, discarded: the first run pays for page faults on a fresh
    // allocation of up to a few hundred MiB, which is not what a running
    // server's tenth login looks like.
    let _ = time_verify(&argon, &phc);

    let mut times: Vec<f64> = (0..samples)
        .map(|_| time_verify(&argon, &phc).as_secs_f64() * 1000.0)
        .collect();
    times.sort_by(|a, b| a.partial_cmp(b).unwrap());

    Sample {
        m_kib,
        t,
        p,
        median_ms: times[times.len() / 2],
        min_ms: times[0],
        max_ms: times[times.len() - 1],
    }
}

/// What a login burst actually costs.
///
/// Storm runs the verify on `spawn_blocking`, so N simultaneous logins are N
/// threads each holding the full memory cost. The single-verify number is the
/// latency; this is the one that says whether the box survives several at once.
fn measure_burst(m_kib: u32, t: u32, p: u32, concurrency: usize) -> (f64, f64) {
    let salt = SaltString::from_b64("c3Rvcm1iZW5jaA").expect("valid salt");
    let phc = argon2_for(m_kib, t, p)
        .hash_password(PASSWORD, &salt)
        .expect("hash succeeds")
        .to_string();

    let start = Instant::now();
    let worst = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..concurrency)
            .map(|_| {
                let phc = phc.clone();
                scope.spawn(move || {
                    let argon = argon2_for(m_kib, t, p);
                    time_verify(&argon, &phc).as_secs_f64() * 1000.0
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("worker finished"))
            .fold(0.0_f64, f64::max)
    });
    (start.elapsed().as_secs_f64() * 1000.0, worst)
}

fn main() {
    let samples: usize = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(5);

    println!("Argon2id parameter sweep — Storm Q18 / decision A1");
    println!("target: {}-{} ms per verify\n", TARGET_MS.0, TARGET_MS.1);
    println!(
        "  {:>9}  {:>3}  {:>2}  {:>9}  {:>9}  {:>9}",
        "memory", "t", "p", "median", "min", "max"
    );
    println!("  {}", "-".repeat(56));

    // The grid spans the two OWASP recommendations (19 MiB/t=2 and 46 MiB/t=1)
    // upwards. Memory-hardness is the axis worth buying, so the sweep climbs
    // memory and keeps iterations low rather than the reverse.
    const MEMORY: [u32; 7] = [19456, 32768, 47104, 65536, 98304, 131072, 196608];
    const TIME: [u32; 3] = [1, 2, 3];
    const PARALLEL: [u32; 2] = [1, 2];

    let mut all = Vec::new();
    for &p in &PARALLEL {
        for &t in &TIME {
            for &m in &MEMORY {
                let s = measure(m, t, p, samples);
                println!(
                    "  {:>6.0} MiB  {:>3}  {:>2}  {:>7.1} ms  {:>7.1} ms  {:>7.1} ms{}",
                    s.mib(),
                    s.t,
                    s.p,
                    s.median_ms,
                    s.min_ms,
                    s.max_ms,
                    if s.in_target() { "   <- in target" } else { "" }
                );
                let too_slow = s.median_ms > TARGET_MS.1 * 4.0;
                all.push(s);
                // Nothing above this point in the sweep can come back under the
                // ceiling, and each sample costs seconds on a small guest.
                if too_slow {
                    break;
                }
            }
        }
    }

    // Prefer the most memory-hard configuration that still lands in the window,
    // and among equals the lowest iteration count: memory is what makes a GPU
    // attack expensive, iterations only cost the defender.
    let best = all
        .iter()
        .filter(|s| s.in_target())
        .max_by(|a, b| {
            a.m_kib
                .cmp(&b.m_kib)
                .then(b.t.cmp(&a.t))
                .then(b.p.cmp(&a.p))
        });

    println!("\n{}", "=".repeat(58));
    match best {
        Some(s) => {
            println!("RECOMMENDED for this box:");
            println!(
                "  m = {} KiB ({:.0} MiB), t = {}, p = {}",
                s.m_kib,
                s.mib(),
                s.t,
                s.p
            );
            println!("  one verify: {:.1} ms (median)", s.median_ms);
            println!("  Params::new({}, {}, {}, None)", s.m_kib, s.t, s.p);

            // The headline maximises memory, which is the axis that costs an
            // attacker. Someone re-measuring on a smaller box may want to trade
            // that for a smaller blast radius under a login burst, so name the
            // cheapest in-window option rather than making them re-read the
            // table for it.
            if let Some(lean) = all
                .iter()
                .filter(|c| c.in_target())
                .min_by_key(|c| (c.m_kib, c.t))
                && lean.m_kib != s.m_kib
            {
                println!(
                    "\n  leanest in-window alternative: m = {} KiB ({:.0} MiB), t = {}, p = {} \
                     at {:.1} ms",
                    lean.m_kib,
                    lean.mib(),
                    lean.t,
                    lean.p,
                    lean.median_ms
                );
                println!(
                    "  ({:.0} MiB less per concurrent verify; take it only if the box \
                     cannot afford the burst)",
                    s.mib() - lean.mib()
                );
            }

            let cpus = std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(2);
            let budget = burst_budget_mib();
            let affordable = (budget / s.mib()).floor().max(1.0) as usize;
            let concurrency = cpus.min(affordable);
            if concurrency < cpus {
                println!(
                    "\n  (burst capped at {concurrency} of {cpus} threads to stay under \
                     the {budget:.0} MiB budget — this box is serving something)"
                );
            }

            let (wall, worst) = measure_burst(s.m_kib, s.t, s.p, concurrency);
            println!(
                "\n  burst of {concurrency} simultaneous verifies: {wall:.1} ms wall, \
                 slowest {worst:.1} ms"
            );
            println!(
                "  peak memory for that burst: ~{:.0} MiB ({} x {:.0} MiB)",
                s.mib() * concurrency as f64,
                concurrency,
                s.mib()
            );
        }
        None => {
            println!("NOTHING in the sweep landed in the target window.");
            println!("Widen the grid — this box is far faster or far slower than expected.");
        }
    }
    println!("{}", "=".repeat(58));
}
