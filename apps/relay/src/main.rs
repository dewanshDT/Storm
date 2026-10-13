//! `storm-relay` — run the relay.
//!
//! Configuration is flags (with `STORM_RELAY_*` env fallbacks) plus one
//! optional file, the allowlist. Flags rather than a config file for the relay
//! itself because every knob is one scalar, and the timers and limits rarely
//! move from their defaults; the
//! allowlist is a file because it is the one thing that grows, gets diffed
//! after a refused registration, and wants comments next to each entry.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::Parser;
use storm_relay::config;
use storm_relay::state::Bindings;
use storm_relay::tls::CertStore;
use storm_relay::{Allowlist, CONNECT_PATH, Config, REGISTER_PATH, Relay};

#[derive(Parser, Debug)]
#[command(name = "storm-relay", version, about = "Storm Relay Protocol v1 relay")]
struct Args {
    /// Address to listen on. 8486, not 8484: storm-server owns 8484, and the
    /// two run on one machine while a relay is being built and tested.
    #[arg(long, env = "STORM_RELAY_BIND", default_value = "127.0.0.1:8486")]
    bind: SocketAddr,

    /// Scheme, host and port clients should dial, without a trailing slash —
    /// `wss://relay.example`. Servers are handed
    /// `{public-base}/connect/{server_id}` as their `public_address`.
    ///
    /// Defaults to the bind address, as `wss://` when --tls-cert is set and
    /// `ws://` when it is not: the scheme the relay is actually serving. A
    /// relay on a public address wants its real hostname here, since a
    /// certificate names a host, not an address.
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

    /// PEM certificate chain (e.g. certbot's fullchain.pem). With --tls-key,
    /// the relay terminates TLS itself and serves wss:// directly. It must, on
    /// a public address: behind a TLS proxy every client would arrive from the
    /// proxy's address (decision 71). `systemctl reload` re-reads both files.
    #[arg(long, env = "STORM_RELAY_TLS_CERT", requires = "tls_key")]
    tls_cert: Option<PathBuf>,

    /// PEM private key for --tls-cert.
    #[arg(long, env = "STORM_RELAY_TLS_KEY", requires = "tls_cert")]
    tls_key: Option<PathBuf>,

    #[command(flatten)]
    limits: Limits,
}

/// The §4–§6 timers and abuse limits. Each defaults to the value in `config`;
/// the conformance suite (`tools/relay-conformance`) starts the relay with
/// them shortened, which is the only reason most of them are flags at all.
#[derive(clap::Args, Debug)]
#[command(next_help_heading = "Timers and limits")]
struct Limits {
    /// How long a CHALLENGE nonce stays answerable (§4).
    #[arg(long, env = "STORM_RELAY_CHALLENGE_TTL_MS", value_name = "MS",
          default_value_t = millis(config::DEFAULT_CHALLENGE_TTL))]
    challenge_ttl_ms: u64,

    /// How long a HELLO waits for a server trunk before server_unreachable (§5).
    #[arg(long, env = "STORM_RELAY_HELLO_WAIT_MS", value_name = "MS",
          default_value_t = millis(config::DEFAULT_HELLO_WAIT))]
    hello_wait_ms: u64,

    /// How long a STREAM_OPEN waits for STREAM_ACK before server_timeout (§5.1).
    #[arg(long, env = "STORM_RELAY_STREAM_ACK_TIMEOUT_MS", value_name = "MS",
          default_value_t = millis(config::DEFAULT_STREAM_ACK_TIMEOUT))]
    stream_ack_timeout_ms: u64,

    /// How long a server trunk may send nothing before it is dead (§4.2).
    #[arg(long, env = "STORM_RELAY_HEARTBEAT_DEADLINE_MS", value_name = "MS",
          default_value_t = millis(config::DEFAULT_HEARTBEAT_DEADLINE))]
    heartbeat_deadline_ms: u64,

    /// How long a superseded trunk's streams get before trunk_superseded (§4.2).
    #[arg(long, env = "STORM_RELAY_SUPERSESSION_DRAIN_MS", value_name = "MS",
          default_value_t = millis(config::DEFAULT_SUPERSESSION_DRAIN))]
    supersession_drain_ms: u64,

    /// HELLOs one source IP may make per window (§6).
    #[arg(long, env = "STORM_RELAY_HELLO_RATE_LIMIT", value_name = "N",
          default_value_t = config::DEFAULT_HELLO_RATE_LIMIT)]
    hello_rate_limit: usize,

    /// The window for --hello-rate-limit.
    #[arg(long, env = "STORM_RELAY_HELLO_RATE_WINDOW_MS", value_name = "MS",
          default_value_t = millis(config::DEFAULT_HELLO_RATE_WINDOW))]
    hello_rate_window_ms: u64,

    /// Un-acked STREAM_OPENs one client trunk may hold (§5.1).
    #[arg(long, env = "STORM_RELAY_MAX_IN_FLIGHT_STREAMS", value_name = "N",
          default_value_t = config::DEFAULT_MAX_IN_FLIGHT_STREAMS)]
    max_in_flight_streams: usize,

    /// Open streams one server trunk may carry across all its clients (§6).
    #[arg(long, env = "STORM_RELAY_MAX_TOTAL_STREAMS", value_name = "N",
          default_value_t = config::DEFAULT_MAX_TOTAL_STREAMS)]
    max_total_streams: usize,

    /// Bytes queued toward one client trunk before its streams are dropped (§5.4).
    #[arg(long, env = "STORM_RELAY_MAX_CLIENT_BUFFER_BYTES", value_name = "BYTES",
          default_value_t = config::DEFAULT_MAX_CLIENT_BUFFER_BYTES)]
    max_client_buffer_bytes: usize,
}

const fn millis(duration: Duration) -> u64 {
    duration.as_millis() as u64
}

impl Limits {
    fn apply(&self, config: &mut Config) {
        config.challenge_ttl = Duration::from_millis(self.challenge_ttl_ms);
        config.hello_wait = Duration::from_millis(self.hello_wait_ms);
        config.stream_ack_timeout = Duration::from_millis(self.stream_ack_timeout_ms);
        config.heartbeat_deadline = Duration::from_millis(self.heartbeat_deadline_ms);
        config.supersession_drain = Duration::from_millis(self.supersession_drain_ms);
        config.hello_rate_limit = self.hello_rate_limit;
        config.hello_rate_window = Duration::from_millis(self.hello_rate_window_ms);
        config.max_in_flight_streams = self.max_in_flight_streams;
        config.max_total_streams = self.max_total_streams;
        config.max_client_buffer_bytes = self.max_client_buffer_bytes;
    }
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
    let scheme = if args.tls_cert.is_some() { "wss" } else { "ws" };
    let public_base = args
        .public_base
        .clone()
        .unwrap_or_else(|| format!("{scheme}://{}", args.bind));
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

    let certs = match (&args.tls_cert, &args.tls_key) {
        (Some(cert), Some(key)) => Some(Arc::new(CertStore::load(cert, key)?)),
        _ => None,
    };
    // Advertising one scheme while serving the other is a relay no client can
    // reach. wss:// without --tls-cert is allowed, since a TLS proxy in front
    // is still possible, but it costs every client its own address (71).
    match (&certs, public_base.starts_with("wss://")) {
        (Some(_), false) => anyhow::bail!(
            "--tls-cert is set but --public-base is {public_base}; clients would dial \
             plain ws:// at a TLS port. Use wss://."
        ),
        (None, true) => tracing::warn!(
            "--public-base is wss:// but this relay is not terminating TLS. If a \
             proxy terminates it, every client arrives from the proxy's address: \
             one HELLO bucket for everyone, and one relay_peer_ip at the origin. \
             Prefer --tls-cert/--tls-key (decision 71)."
        ),
        _ => {}
    }

    let mut config = Config::new(args.bind, &public_base);
    args.limits.apply(&mut config);
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
    let relay = Arc::new(relay);

    match certs {
        Some(certs) => {
            spawn_reload_on_sighup(certs.clone());
            storm_relay::serve_tls(listener, relay, certs).await
        }
        None => storm_relay::serve(listener, relay).await,
    }
}

/// `systemctl reload` sends SIGHUP; a renewed certificate is picked up without
/// dropping a single trunk. A reload that fails keeps serving the old pair and
/// says why, loudly, because an expiring certificate is a deadline.
fn spawn_reload_on_sighup(certs: Arc<CertStore>) {
    tokio::spawn(async move {
        let mut hup = match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::hangup()) {
            Ok(hup) => hup,
            Err(e) => {
                tracing::error!(error = %e, "cannot listen for SIGHUP; certificate reload is off");
                return;
            }
        };
        while hup.recv().await.is_some() {
            match certs.reload() {
                Ok(()) => tracing::info!("TLS certificate reloaded"),
                Err(e) => tracing::error!(
                    error = %format!("{e:#}"),
                    "TLS certificate reload failed; still serving the previous one"
                ),
            }
        }
    });
}
