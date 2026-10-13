//! `relay-conformance` — a black-box SRP v1 conformance suite.
//!
//! It plays the origin server and the clients against a relay it knows only by
//! URL, and reports each check under the section of `docs/srp-v1.md` it
//! enforces. See README.md for the fixture contract a relay must be started
//! with.

mod checks;
mod fixtures;
mod harness;
mod wire;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use anyhow::{Context, Result, bail};

use checks::{Check, Needs};
use harness::{Ctx, Options};

/// Longer than any check should need; a check that hangs fails rather than
/// stalling the run.
const CHECK_TIMEOUT: Duration = Duration::from_secs(120);

fn main() -> ExitCode {
    match run() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("relay-conformance: {e:#}");
            ExitCode::from(2)
        }
    }
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

fn flag(name: &str) -> bool {
    env(name).is_some_and(|v| v != "0")
}

fn fixtures_dir() -> PathBuf {
    env("RELAY_FIXTURES")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures"))
}

fn run() -> Result<bool> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("gen-fixtures") {
        let dir = args.get(1).map(PathBuf::from).unwrap_or_else(fixtures_dir);
        fixtures::write(&dir)?;
        return Ok(true);
    }
    let list = args.iter().any(|a| a == "--list");
    let filters: Vec<&String> = args.iter().filter(|a| !a.starts_with("--")).collect();
    let selected: Vec<Check> = checks::all()
        .into_iter()
        .filter(|c| {
            filters.is_empty()
                || filters
                    .iter()
                    .any(|f| c.section.contains(f.as_str()) || c.name.contains(f.as_str()))
        })
        .collect();

    if list {
        for check in &selected {
            println!("{:<6} {}", check.section, check.name);
        }
        return Ok(true);
    }

    let relay = env("RELAY_URL").context("RELAY_URL is required (ws:// or wss://)")?;
    for url in std::iter::once(&relay).chain(env("RELAY_TOFU_URL").as_ref()) {
        if !url.starts_with("ws://") && !url.starts_with("wss://") {
            bail!("{url} is not a ws:// or wss:// URL");
        }
    }
    let dir = fixtures_dir();
    let options = Options {
        register_path: env("RELAY_REGISTER_PATH").unwrap_or_else(|| "/register".into()),
        connect_path: env("RELAY_CONNECT_PATH")
            .unwrap_or_else(|| "/connect".into())
            .trim_end_matches('/')
            .to_string(),
        peer_ip: env("RELAY_PEER_IP")
            .map(|ip| ip.parse())
            .transpose()
            .context("RELAY_PEER_IP")?,
        multi_ip: flag("RELAY_MULTI_IP"),
        latency: flag("RELAY_LATENCY"),
    };
    let ctx = Ctx::new(
        relay,
        env("RELAY_TOFU_URL"),
        fixtures::Fixtures::load(&dir)?,
        fixtures::Knobs::load(&dir.join("relay.env"))?,
        options,
    );

    // tungstenite's wss:// client uses the process default provider.
    let _ = rustls::crypto::ring::default_provider().install_default();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    Ok(runtime.block_on(run_checks(&ctx, &selected)))
}

#[derive(Default)]
struct Tally {
    passed: usize,
    failed: usize,
    skipped: usize,
}

async fn run_checks(ctx: &Ctx, selected: &[Check]) -> bool {
    println!(
        "SRP v1 relay conformance: {} (tofu: {})",
        ctx.relay.base,
        ctx.tofu.as_ref().map_or("not given", |r| r.base.as_str())
    );
    let mut sections: BTreeMap<&str, Tally> = BTreeMap::new();
    let mut failures = Vec::new();
    for check in selected {
        let tally = sections.entry(check.section).or_default();
        let skip = match check.needs {
            Needs::Nothing => None,
            Needs::Tofu if ctx.tofu.is_none() => Some("needs RELAY_TOFU_URL"),
            Needs::MultiIp if !ctx.options.multi_ip => Some("needs RELAY_MULTI_IP=1"),
            Needs::Latency if !ctx.options.latency => Some("needs RELAY_LATENCY=1"),
            _ => None,
        };
        if let Some(why) = skip {
            println!("skip  {:<6} {} ({why})", check.section, check.name);
            tally.skipped += 1;
            continue;
        }
        let started = std::time::Instant::now();
        let outcome = match tokio::time::timeout(CHECK_TIMEOUT, (check.run)(ctx)).await {
            Ok(outcome) => outcome,
            Err(_) => Err(anyhow::anyhow!("did not finish within {CHECK_TIMEOUT:?}")),
        };
        let took = started.elapsed();
        match outcome {
            Ok(()) => {
                println!("pass  {:<6} {} ({took:.1?})", check.section, check.name);
                tally.passed += 1;
            }
            Err(e) => {
                println!("FAIL  {:<6} {} ({took:.1?})", check.section, check.name);
                println!("        {e:#}");
                tally.failed += 1;
                failures.push(format!("{} {}", check.section, check.name));
            }
        }
    }

    println!();
    println!("{:<6} {:>5} {:>5} {:>5}", "", "pass", "fail", "skip");
    let mut total = Tally::default();
    for (section, tally) in &sections {
        println!(
            "{section:<6} {:>5} {:>5} {:>5}",
            tally.passed, tally.failed, tally.skipped
        );
        total.passed += tally.passed;
        total.failed += tally.failed;
        total.skipped += tally.skipped;
    }
    println!(
        "{:<6} {:>5} {:>5} {:>5}",
        "total", total.passed, total.failed, total.skipped
    );
    if !failures.is_empty() {
        println!("\nSpec violations:");
        for failure in &failures {
            println!("  {failure}");
        }
    }
    failures.is_empty()
}
