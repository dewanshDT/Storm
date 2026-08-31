//! The tunnel client: how a Storm server is reachable from outside the LAN.
//!
//! A relay accepts a trunk from this server and multiplexes client requests
//! over it (`docs/srp-v1.md`). What arrives is an ordinary HTTP request, and
//! it is served by the ordinary router — **the relay adds no server-side logic
//! path (R13)**. See `dispatch.rs` for why that matters and what it costs.
//!
//! The relay half lives in `apps/relay/` and shares no code with this: both
//! are written from the spec, which is what makes the spec the contract.

mod client;
mod dispatch;
mod proto;
#[cfg(test)]
mod tests;

use std::sync::Arc;

use axum::Router;
use axum::http::HeaderValue;
use tokio::sync::watch;

pub use client::Tunnel;

use crate::auth::ServerIdentity;
use crate::registry::RegisteredRelays;

/// Live tunnel supervisors, one per configured relay.
pub struct Tunnels {
    shutdown: watch::Sender<bool>,
    handles: Vec<tokio::task::JoinHandle<()>>,
}

impl Tunnels {
    /// Starts a supervisor per configured relay.
    ///
    /// Reads the relays the registry already holds (`PUT /v1/config/relays`);
    /// there is deliberately no flag for this, because a relay is a setting the
    /// app changes and a flag would need a restart the client cannot perform.
    ///
    /// **Configured is not registered.** Nothing is advertised until a
    /// registration actually succeeds — see [`RegisteredRelays`].
    pub fn spawn(
        relays: &[String],
        identity: Arc<ServerIdentity>,
        router: Router,
        registered: RegisteredRelays,
        bind_host: &str,
    ) -> Self {
        let (shutdown, _) = watch::channel(false);

        // The `Host` every relayed request is rewritten to carry. It is the
        // *bind* address, matching what `mcp::allowed_hosts` was built from —
        // not `listen_addr`, which resolves a wildcard to an advertisable
        // address and would therefore not be on that list.
        let host_header = HeaderValue::from_str(bind_host).unwrap_or_else(|_| {
            tracing::warn!(bind_host, "bind address is not a valid Host header");
            HeaderValue::from_static("localhost")
        });

        let tunnel = Tunnel {
            identity,
            router,
            registered,
            host_header,
        };

        let handles = relays
            .iter()
            .map(|relay| {
                tokio::spawn(client::supervise(
                    relay.clone(),
                    tunnel.clone(),
                    shutdown.subscribe(),
                ))
            })
            .collect();

        Self { shutdown, handles }
    }

    /// Signals every supervisor and waits for it to send `DEREGISTER`.
    ///
    /// Waiting is the point: a supervisor that is dropped mid-flight leaves the
    /// relay holding the `server_id` until a heartbeat timeout, and every
    /// client trying to reach this server waits it out.
    pub async fn shutdown(self) {
        let _ = self.shutdown.send(true);
        for handle in self.handles {
            let _ = handle.await;
        }
    }
}
