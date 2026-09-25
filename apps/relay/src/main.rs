//! `storm-relay` — run the relay.
//!
//! Configuration is flags (with `STORM_RELAY_*` env fallbacks) plus one
//! optional file, the allowlist. Flags rather than a config file for the relay
//! itself because there are three knobs and two of them are addresses; the
//! allowlist is a file because it is the one thing that grows, gets diffed
//! after a refused registration, and wants comments next to each entry.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use storm_relay::state::Bindings;
use storm_relay::{Allowlist, CONNECT_PATH, Config, REGISTER_PATH, Relay};

#[derive(Parser, Debug)]
#[command(name = "storm-relay", version, about = "Storm Relay Protocol v1 relay")]
struct Args {
    /// Address to listen on.
    #[arg(long, env = "STORM_RELAY_BIND", default_value = "127.0.0.1:8484")]
    bind: SocketAddr,

    /// Scheme, host and port clients should dial, without a trailing slash —
    /// `wss://relay.example`. Servers are handed
    /// `{public-base}/connect/{server_id}` as their `public_address`.
    ///
    /// Defaults to `wss://` plus the bind address, which is right for a real
    /// deployment and wrong for a local one: run a plaintext dev relay with
    /// `--public-base ws://127.0.0.1:8484`.
    #[arg(long, env = "STORM_RELAY_PUBLIC_BASE")]
    public_base: Option<String>,

    /// Path to a pubkey allowlist: one `<server_id> <pubkey>` per line, `#`
    /// comments. When given, it *is* the binding — an unlisted `server_id` or a
    /// mismatched key is refused, with no trust-on-first-use even on first
    /// sight. Without it the relay trusts the first key it sees for a
    /// `server_id` and refuses every later one.
    #[arg(long, env = "STORM_RELAY_ALLOWLIST")]
    allowlist: Option<PathBuf>,

    /// Where trust-on-first-use bindings are kept, so a restart does not
    /// re-open the first-use window. Same format as the allowlist, written by
    /// the relay, and a binding is on disk before its server is told it is
    /// registered. Created if missing; a file that does not parse stops the
    /// relay. Meaningless with --allowlist, which switches TOFU off.
    #[arg(long, env = "STORM_RELAY_BINDINGS", conflicts_with = "allowlist")]
    bindings: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "storm_relay=info,tower_http=info".into()),
        )
        .init();

    let args = Args::parse();
    let public_base = args
        .public_base
        .unwrap_or_else(|| format!("wss://{}", args.bind));
    let public_base = public_base.trim_end_matches('/').to_string();
    if !public_base.starts_with("ws://") && !public_base.starts_with("wss://") {
        anyhow::bail!(
            "--public-base must start with ws:// or wss://; got {public_base}. \
             It is handed to clients verbatim as the start of a public_address."
        );
    }

    let allowlist = match &args.allowlist {
        Some(path) => Some(Allowlist::load(path).context("loading the pubkey allowlist")?),
        None => None,
    };

    let bindings = match &args.bindings {
        Some(path) => Some(Bindings::load(path)?),
        None => None,
    };

    let mut config = Config::new(args.bind, &public_base);
    match (&allowlist, &bindings) {
        (Some(list), _) => tracing::info!(
            entries = list.len(),
            "allowlist loaded; trust-on-first-use is off"
        ),
        (None, Some(bound)) => tracing::info!(
            entries = bound.len(),
            "trust-on-first-use, with bindings persisted to --bindings"
        ),
        // Worth saying out loud. Without a file a restart re-opens the
        // first-use window for every server that has not yet reconnected.
        (None, None) => tracing::warn!(
            "no allowlist and no --bindings: trust-on-first-use held in memory \
             only. A restart re-opens the first-use window; do not run a public \
             relay this way."
        ),
    }
    config.allowlist = allowlist;

    let listener = tokio::net::TcpListener::bind(config.bind)
        .await
        .with_context(|| format!("binding {}", config.bind))?;
    let bound = listener.local_addr()?;
    tracing::info!(
        %bound,
        %public_base,
        register_path = REGISTER_PATH,
        connect_path = CONNECT_PATH,
        "storm-relay listening"
    );

    let mut relay = Relay::new(config);
    if let Some(bindings) = bindings {
        relay = relay.with_bindings(bindings);
    }
    storm_relay::serve(listener, Arc::new(relay)).await
}
