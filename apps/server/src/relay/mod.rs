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

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use axum::Router;
use axum::http::HeaderValue;
use tokio::sync::{oneshot, watch};

pub use client::Tunnel;

use crate::auth::ServerIdentity;
use crate::registry::RegisteredRelays;

/// Live tunnel supervisors, one per configured relay, keyed by its URL.
///
/// Each has its own shutdown channel, so one relay can be dropped from the
/// configuration without touching the others' trunks.
pub struct Tunnels {
    tunnel: Tunnel,
    supervisors: BTreeMap<String, Supervisor>,
}

struct Supervisor {
    shutdown: watch::Sender<bool>,
    handle: tokio::task::JoinHandle<()>,
}

impl Supervisor {
    /// Signals it and waits for its `DEREGISTER`. Waiting is the point: a
    /// supervisor dropped mid-flight leaves the relay holding the `server_id`
    /// until a heartbeat timeout, and every client waits that out.
    async fn stop(self) {
        let _ = self.shutdown.send(true);
        let _ = self.handle.await;
    }
}

impl Tunnels {
    /// Starts a supervisor per configured relay.
    ///
    /// Reads the relays the registry already holds (`PUT /v1/config/relays`);
    /// there is deliberately no flag for this, because a relay is a setting the
    /// app changes. Changes after boot arrive through [`manage`].
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
        // The `Host` every relayed request is rewritten to carry. It is the
        // *bind* address, matching what `mcp::allowed_hosts` was built from —
        // not `listen_addr`, which resolves a wildcard to an advertisable
        // address and would therefore not be on that list.
        let host_header = HeaderValue::from_str(bind_host).unwrap_or_else(|_| {
            tracing::warn!(bind_host, "bind address is not a valid Host header");
            HeaderValue::from_static("localhost")
        });

        let mut tunnels = Self {
            tunnel: Tunnel {
                identity,
                router,
                registered,
                host_header,
            },
            supervisors: BTreeMap::new(),
        };
        for relay in relays {
            tunnels.start(relay);
        }
        tunnels
    }

    fn start(&mut self, relay: &str) {
        if self.supervisors.contains_key(relay) {
            return;
        }
        let (shutdown, watching) = watch::channel(false);
        let handle = tokio::spawn(client::supervise(
            relay.to_string(),
            self.tunnel.clone(),
            watching,
        ));
        self.supervisors
            .insert(relay.to_string(), Supervisor { shutdown, handle });
    }

    /// Makes the running set match `relays` (decision 74): stops the ones no
    /// longer listed, each sending its `DEREGISTER` first, and starts the new
    /// ones. **A relay in both lists is not touched**, so saving an unchanged
    /// list, or adding a second relay, never drops a working trunk.
    pub async fn reconcile(&mut self, relays: &[String]) {
        let wanted: BTreeSet<&str> = relays.iter().map(String::as_str).collect();
        let gone: Vec<String> = self
            .supervisors
            .keys()
            .filter(|url| !wanted.contains(url.as_str()))
            .cloned()
            .collect();
        // Stopped together: each waits on its own relay, and one relay that is
        // slow to take a DEREGISTER must not hold up the others.
        let stopping: Vec<_> = gone
            .iter()
            .filter_map(|url| self.supervisors.remove(url))
            .map(Supervisor::stop)
            .collect();
        futures_util::future::join_all(stopping).await;
        for url in &gone {
            tracing::info!(relay = %url, "relay removed from the configuration; disconnected");
        }
        for relay in relays {
            if !self.supervisors.contains_key(relay) {
                tracing::info!(relay = %relay, "relay added to the configuration; connecting");
                self.start(relay);
            }
        }
    }

    /// Signals every supervisor and waits for each to send `DEREGISTER`.
    pub async fn shutdown(self) {
        let stopping: Vec<_> = self
            .supervisors
            .into_values()
            .map(Supervisor::stop)
            .collect();
        futures_util::future::join_all(stopping).await;
    }
}

/// Owns the tunnels for the life of the server, applying every change to the
/// configured relay list as it is saved, until `stop` fires.
///
/// A change is applied **to completion** before the next is read, and `stop`
/// is only noticed between changes: `reconcile` is in the arm's body, not
/// raced by it, so a DEREGISTER in flight is never cancelled (the rule the
/// supervisor itself follows).
pub async fn manage(
    mut tunnels: Tunnels,
    mut relays: watch::Receiver<Vec<String>>,
    mut stop: oneshot::Receiver<()>,
) {
    loop {
        tokio::select! {
            _ = &mut stop => break,
            changed = relays.changed() => {
                if changed.is_err() {
                    // The sender lives in `AppState`, so it is gone only when
                    // the server is. Nothing more will change; wait to stop.
                    let _ = (&mut stop).await;
                    break;
                }
                let next = relays.borrow_and_update().clone();
                tunnels.reconcile(&next).await;
            }
        }
    }
    tunnels.shutdown().await;
}
