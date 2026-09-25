//! `storm-relay` — the Storm Relay Protocol v1 relay.
//!
//! A **multiplexed reverse proxy routed by `server_id`**. Not a VPN, not a
//! generic forwarder: its only destination concept is a `server_id`, and it
//! moves the same HTTP requests a LAN client would send, over one persistent
//! tunnel. `docs/srp-v1.md` is normative.
//!
//! Three properties shape everything here, and none of them is negotiable:
//!
//! - **It authenticates servers, never clients (R12).** A client presents no
//!   credential to the relay; its credential rides *inside* the tunnelled
//!   request and is checked by the origin server. Adding client authentication
//!   is the mistake that made the first draft of the spec wrong.
//! - **It is never an authority (R5).** No user database, no vault data, no
//!   authorization. If a change would have this crate know what a Storm *user*
//!   is, the change is wrong.
//! - **It must never become mandatory (R6).** Hence no dependency on
//!   `apps/server`, and no Cargo workspace joining the two.
//!
//! ## What is here
//!
//! Both trunk kinds and the routing between them: the `REGISTER_SERVER` →
//! `CHALLENGE` → `CHALLENGE_RESPONSE` → `REGISTERED` handshake with its §4.1
//! binding rules (`register`), the client trunk and stream lifecycle
//! (`connect`), and the tables that map a `stream_id` to exactly one client
//! trunk (`state`).
//!
//! Trunk liveness (§4.2): the 45 s heartbeat deadline (`register`) and the
//! 30 s supersession drain (`state`). The abuse controls: the per-IP `HELLO`
//! limit (`rate_limit`), the in-flight and total stream caps (`state`), and
//! the per-client-trunk byte budget (`trunk`).
//!
//! TOFU bindings persist to a file when given one (`--bindings`); otherwise
//! they live in memory and a restart re-opens the first-use window (`state`).
//!
//! Not here: the §6 bandwidth cap.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use axum::extract::connect_info::{ConnectInfo, Connected};
use axum::extract::{Path, State, WebSocketUpgrade};
use axum::response::Response;
use axum::routing::get;
use axum::serve::IncomingStream;

pub mod auth;
pub mod config;
pub mod connect;
pub mod proto;
pub mod rate_limit;
pub mod register;
pub mod state;
pub mod tls;
pub mod trunk;

pub use config::{Allowlist, Config};

/// Where a server opens its trunk.
///
/// §4.3 fixes the *client*-facing path (`/connect/<server_id>`) and never names
/// the server-facing one, so this is a choice, not a quotation. Distinct paths
/// keep the two roles apart at the router: a client can never reach the
/// registration state machine by accident, whatever it sends.
pub const REGISTER_PATH: &str = "/register";

/// Where a client opens its trunk (§4.3).
///
/// `public_address` is `{public_base}/connect/{server_id}` — derived, not
/// allocated — so this template is a wire commitment, not a routing preference.
pub const CONNECT_PATH: &str = "/connect/{server_id}";

/// How the relay mints nonces.
///
/// Behind a function pointer so tests can pin a nonce and prove replay and
/// expiry are refused — properties that are otherwise unobservable from
/// outside, because a fresh nonce per connection means a stale signature fails
/// for the uninteresting reason instead.
pub type NonceSource = Arc<dyn Fn() -> String + Send + Sync>;

pub struct Relay {
    pub config: Config,
    pub bindings: state::Bindings,
    pub challenges: state::Challenges,
    pub registrations: state::Registrations,
    /// Rate limiter for HELLO attempts per source IP.
    hello_rate_limiter: rate_limit::RateLimiter,
    nonce_source: NonceSource,
}

impl Relay {
    pub fn new(config: Config) -> Self {
        Self::with_nonce_source(config, Arc::new(state::new_nonce))
    }

    pub fn with_nonce_source(config: Config, nonce_source: NonceSource) -> Self {
        let hello_rate_limiter =
            rate_limit::RateLimiter::new(config.hello_rate_limit, config.hello_rate_window);
        Self {
            config,
            bindings: state::Bindings::default(),
            challenges: state::Challenges::default(),
            registrations: state::Registrations::default(),
            hello_rate_limiter,
            nonce_source,
        }
    }

    /// Replaces the binding store, for a relay whose TOFU bindings live in a
    /// file ([`state::Bindings::load`]). Construction stays infallible; loading
    /// the file is the caller's step and the caller's error.
    pub fn with_bindings(mut self, bindings: state::Bindings) -> Self {
        self.bindings = bindings;
        self
    }

    pub fn mint_nonce(&self) -> String {
        (self.nonce_source)()
    }

    /// Checks the HELLO rate limit for a source IP.
    /// Returns `true` if allowed, `false` if rate limited.
    pub fn check_hello_rate_limit(&self, ip: std::net::IpAddr) -> bool {
        self.hello_rate_limiter.try_take(ip)
    }
}

pub fn router(relay: Arc<Relay>) -> Router {
    Router::new()
        .route(REGISTER_PATH, get(register_upgrade))
        .route(CONNECT_PATH, get(connect_upgrade))
        .with_state(relay)
}

async fn register_upgrade(ws: WebSocketUpgrade, State(relay): State<Arc<Relay>>) -> Response {
    ws.on_upgrade(move |socket| register::serve(socket, relay))
}

async fn connect_upgrade(
    ws: WebSocketUpgrade,
    Path(server_id): Path<String>,
    // The client's real source address, taken from the accepted socket. This is
    // the *only* place `relay_peer_ip` may come from: a header-derived value is
    // client-forgeable, which is the whole reason the origin strips forwarding
    // headers at dispatch (§5.2).
    ConnectInfo(PeerAddr(peer)): ConnectInfo<PeerAddr>,
    State(relay): State<Arc<Relay>>,
) -> Response {
    ws.on_upgrade(move |socket| connect::serve(socket, relay, server_id, peer))
}

/// The address of the socket a connection arrived on, over plain TCP or TLS.
///
/// A newtype because axum only provides `ConnectInfo<SocketAddr>` for its own
/// `TcpListener`, and the orphan rule forbids providing it for
/// [`tls::TlsListener`]. It also makes the type say what the value is: the
/// socket's peer, which is the only acceptable source of `relay_peer_ip`.
#[derive(Debug, Clone, Copy)]
pub struct PeerAddr(pub SocketAddr);

impl Connected<IncomingStream<'_, tokio::net::TcpListener>> for PeerAddr {
    fn connect_info(stream: IncomingStream<'_, tokio::net::TcpListener>) -> Self {
        Self(*stream.remote_addr())
    }
}

impl Connected<IncomingStream<'_, tls::TlsListener>> for PeerAddr {
    fn connect_info(stream: IncomingStream<'_, tls::TlsListener>) -> Self {
        // The TCP peer, recorded before the handshake. TLS changes nothing
        // about who is on the other end of the socket.
        Self(*stream.remote_addr())
    }
}

/// Serves until the listener errors or the process is asked to stop.
pub async fn serve(listener: tokio::net::TcpListener, relay: Arc<Relay>) -> anyhow::Result<()> {
    // `into_make_service_with_connect_info` is what makes the peer address
    // reachable from a handler at all. Without it `relay_peer_ip` would have no
    // source but a header, and the field would be `X-Forwarded-For` renamed.
    axum::serve(
        listener,
        router(relay).into_make_service_with_connect_info::<PeerAddr>(),
    )
    .await?;
    Ok(())
}

/// [`serve`], with TLS terminated here rather than by a proxy in front
/// (decision 71). The certificate reloads through `certs` without a restart.
pub async fn serve_tls(
    listener: tokio::net::TcpListener,
    relay: Arc<Relay>,
    certs: Arc<tls::CertStore>,
) -> anyhow::Result<()> {
    serve_tls_with(listener, relay, certs, tls::DEFAULT_HANDSHAKE_TIMEOUT).await
}

/// [`serve_tls`] with the handshake deadline chosen by the caller, so a test
/// can prove a stalled handshake is cut off without waiting the real 10 s.
pub async fn serve_tls_with(
    listener: tokio::net::TcpListener,
    relay: Arc<Relay>,
    certs: Arc<tls::CertStore>,
    handshake_timeout: std::time::Duration,
) -> anyhow::Result<()> {
    let listener = tls::TlsListener::new(listener, certs.server_config()?, handshake_timeout);
    axum::serve(
        listener,
        router(relay).into_make_service_with_connect_info::<PeerAddr>(),
    )
    .await?;
    Ok(())
}
