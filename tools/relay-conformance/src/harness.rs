//! What every check shares: the relays under test, the fixture identities, and
//! the steps that are setup rather than the point — registering an origin,
//! opening a client trunk, opening a stream.

use std::net::IpAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, ensure};
use data_encoding::BASE64URL_NOPAD;
use ed25519_dalek::SigningKey;
use rand::Rng;
use serde_json::{Value, json};
use tokio::sync::Mutex;
use tokio::time::Instant;

use crate::fixtures::{Fixtures, Identity, Knobs};
use crate::wire::{self, Conn};

/// How long an expected frame may take. Generous, because a remote relay is a
/// round trip away and a failure here only costs time.
pub const STEP: Duration = Duration::from_secs(5);
/// How long nothing must arrive for an absence to count.
pub const QUIET: Duration = Duration::from_millis(400);

pub struct Relay {
    pub base: String,
    pub name: &'static str,
    pacer: Pacer,
}

pub struct Options {
    pub register_path: String,
    pub connect_path: String,
    pub peer_ip: Option<IpAddr>,
    pub multi_ip: bool,
    pub latency: bool,
}

pub struct Ctx {
    pub relay: Relay,
    pub tofu: Option<Relay>,
    pub open: Option<Relay>,
    pub fixtures: Fixtures,
    pub knobs: Knobs,
    pub options: Options,
    next_identity: AtomicUsize,
}

impl Ctx {
    pub fn new(
        relay: String,
        tofu: Option<String>,
        open: Option<String>,
        fixtures: Fixtures,
        knobs: Knobs,
        options: Options,
    ) -> Self {
        let pacer = || Pacer::new(&knobs);
        Self {
            relay: Relay {
                base: relay.trim_end_matches('/').to_string(),
                name: "relay",
                pacer: pacer(),
            },
            tofu: tofu.map(|base| Relay {
                base: base.trim_end_matches('/').to_string(),
                name: "tofu relay",
                pacer: pacer(),
            }),
            open: open.map(|base| Relay {
                base: base.trim_end_matches('/').to_string(),
                name: "open relay",
                pacer: pacer(),
            }),
            fixtures,
            knobs,
            options,
            next_identity: AtomicUsize::new(0),
        }
    }

    /// A fixture identity no other check in this run has used, so nothing a
    /// previous check left behind — a draining trunk, a dead registration —
    /// is on its `server_id`. Wraps if a run has more checks than identities.
    pub fn identity(&self) -> Identity {
        let n = self.next_identity.fetch_add(1, Ordering::Relaxed);
        self.fixtures.servers[n % self.fixtures.servers.len()].clone()
    }

    pub fn intruder(&self, server_id: &str) -> Identity {
        Identity {
            server_id: server_id.to_string(),
            key: self.fixtures.intruder.clone(),
        }
    }

    pub fn tofu(&self) -> Result<&Relay> {
        self.tofu
            .as_ref()
            .ok_or_else(|| anyhow!("RELAY_TOFU_URL is not set"))
    }

    pub fn open(&self) -> Result<&Relay> {
        self.open
            .as_ref()
            .ok_or_else(|| anyhow!("RELAY_OPEN_URL is not set"))
    }

    /// A client socket dialled with `?pubkey=` (§4.3), before `HELLO`.
    pub async fn dial_connect_keyed(
        &self,
        relay: &Relay,
        server_id: &str,
        pubkey: &str,
    ) -> Result<Conn> {
        relay.pacer.take().await;
        let url = format!("{}?pubkey={pubkey}", self.connect_url(relay, server_id));
        wire::connect(&url, &format!("client@{}", relay.name), None).await
    }

    /// A client naming `identity`'s key in the URL and in `HELLO` (§5), past
    /// `READY`.
    pub async fn keyed_client(&self, relay: &Relay, identity: &Identity) -> Result<Conn> {
        let mut conn = self.keyed_hello(relay, identity).await?;
        let ready = conn
            .recv_type("READY", self.knobs.hello_wait + STEP)
            .await?;
        ensure!(
            ready["client_trunk_id"].is_string(),
            "READY without a string client_trunk_id: {ready}"
        );
        Ok(conn)
    }

    /// The same, before the relay answers.
    pub async fn keyed_hello(&self, relay: &Relay, identity: &Identity) -> Result<Conn> {
        let pubkey = identity.pubkey_b64();
        let conn = self
            .dial_connect_keyed(relay, &identity.server_id, &pubkey)
            .await?;
        conn.send_json(json!({
            "v": 1, "type": "HELLO", "server_id": identity.server_id, "pubkey": pubkey,
        }))
        .await?;
        Ok(conn)
    }

    pub fn heartbeat_interval(&self) -> Duration {
        self.knobs.heartbeat_deadline / 4
    }

    pub async fn dial_register(&self, relay: &Relay) -> Result<Conn> {
        let url = format!("{}{}", relay.base, self.options.register_path);
        wire::connect(&url, &format!("server@{}", relay.name), None).await
    }

    pub fn connect_url(&self, relay: &Relay, server_id: &str) -> String {
        format!("{}{}/{server_id}", relay.base, self.options.connect_path)
    }

    /// A client socket, before `HELLO`. Paced, so that no check but the one
    /// about the `HELLO` limit ever meets it.
    pub async fn dial_connect(&self, relay: &Relay, server_id: &str) -> Result<Conn> {
        self.dial_connect_from(relay, server_id, None).await
    }

    pub async fn dial_connect_from(
        &self,
        relay: &Relay,
        server_id: &str,
        source: Option<IpAddr>,
    ) -> Result<Conn> {
        relay.pacer.take().await;
        self.dial_connect_unpaced(relay, server_id, source).await
    }

    pub async fn dial_connect_unpaced(
        &self,
        relay: &Relay,
        server_id: &str,
        source: Option<IpAddr>,
    ) -> Result<Conn> {
        let label = match source {
            Some(ip) => format!("client@{}({ip})", relay.name),
            None => format!("client@{}", relay.name),
        };
        wire::connect(&self.connect_url(relay, server_id), &label, source).await
    }

    /// `REGISTER_SERVER` and the `CHALLENGE` it must produce.
    pub async fn challenge(conn: &mut Conn, server_id: &str, pubkey: &str) -> Result<String> {
        conn.send_json(json!({
            "v": 1, "type": "REGISTER_SERVER",
            "server_id": server_id, "pubkey": pubkey,
        }))
        .await?;
        let challenge = conn.recv_type("CHALLENGE", STEP).await?;
        challenge["nonce"]
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| anyhow!("CHALLENGE without a string nonce: {challenge}"))
    }

    pub async fn respond(conn: &mut Conn, sig: &str) -> Result<()> {
        conn.send_json(json!({ "v": 1, "type": "CHALLENGE_RESPONSE", "sig": sig }))
            .await
    }

    /// A registered origin, heartbeating.
    pub async fn register(&self, relay: &Relay, identity: &Identity) -> Result<Conn> {
        Ok(self.register_full(relay, identity).await?.0)
    }

    pub async fn register_full(&self, relay: &Relay, identity: &Identity) -> Result<(Conn, Value)> {
        let mut conn = self.dial_register(relay).await?;
        let nonce = Self::challenge(&mut conn, &identity.server_id, &identity.pubkey_b64())
            .await
            .with_context(|| format!("registering {}", identity.server_id))?;
        Self::respond(&mut conn, &identity.relay_auth(&nonce)).await?;
        let registered = conn
            .recv_type("REGISTERED", STEP)
            .await
            .with_context(|| format!("registering {}", identity.server_id))?;
        conn.start_heartbeat(self.heartbeat_interval());
        Ok((conn, registered))
    }

    /// A client trunk past `HELLO`/`READY`.
    pub async fn client(&self, relay: &Relay, server_id: &str) -> Result<Conn> {
        self.client_from(relay, server_id, None).await
    }

    pub async fn client_from(
        &self,
        relay: &Relay,
        server_id: &str,
        source: Option<IpAddr>,
    ) -> Result<Conn> {
        let mut conn = self.dial_connect_from(relay, server_id, source).await?;
        hello(&conn, server_id).await?;
        let ready = conn
            .recv_type("READY", self.knobs.hello_wait + STEP)
            .await?;
        ensure!(
            ready["client_trunk_id"].is_string(),
            "READY without a string client_trunk_id: {ready}"
        );
        Ok(conn)
    }

    /// One registered origin, one client, one acknowledged stream.
    pub async fn stream(&self) -> Result<Pair> {
        let identity = self.identity();
        let mut server = self.register(&self.relay, &identity).await?;
        let mut client = self.client(&self.relay, &identity.server_id).await?;
        let stream_id = open_stream(&mut client, json!("pair")).await?;
        ensure!(accept_stream(&mut server).await? == stream_id);
        Ok(Pair {
            identity,
            server,
            client,
            stream_id,
        })
    }
}

pub struct Pair {
    pub identity: Identity,
    pub server: Conn,
    pub client: Conn,
    pub stream_id: u32,
}

pub async fn hello(conn: &Conn, server_id: &str) -> Result<()> {
    conn.send_json(json!({ "v": 1, "type": "HELLO", "server_id": server_id }))
        .await
}

/// `OPEN_STREAM` → `STREAM_READY`, which must echo `attempt_id` exactly.
pub async fn open_stream(client: &mut Conn, attempt_id: Value) -> Result<u32> {
    client
        .send_json(json!({ "v": 1, "type": "OPEN_STREAM", "attempt_id": attempt_id }))
        .await?;
    let ready = client.recv_type("STREAM_READY", STEP).await?;
    ensure!(
        ready["attempt_id"] == attempt_id,
        "STREAM_READY must echo attempt_id {attempt_id}: {ready}"
    );
    wire::stream_id_of(&ready)
}

/// The origin's side: `STREAM_OPEN`, answered with `STREAM_ACK`.
pub async fn accept_stream(server: &mut Conn) -> Result<u32> {
    let open = server.recv_type("STREAM_OPEN", STEP).await?;
    let stream_id = wire::stream_id_of(&open)?;
    server
        .send_json(json!({ "v": 1, "type": "STREAM_ACK", "stream_id": stream_id }))
        .await?;
    Ok(stream_id)
}

pub fn request_head(stream_id: u32, method: &str, path: &str) -> Value {
    json!({
        "v": 1, "type": "HTTP_REQUEST_HEAD",
        "stream_id": stream_id, "method": method, "path": path,
        "headers": { "authorization": "Bearer inside-the-tunnel" },
    })
}

pub fn response_head(stream_id: u32) -> Value {
    json!({
        "v": 1, "type": "HTTP_RESPONSE_HEAD",
        "stream_id": stream_id, "status": 200,
        "headers": { "content-type": "application/octet-stream" },
    })
}

pub fn close(stream_id: u32) -> Value {
    json!({ "v": 1, "type": "CLOSE", "stream_id": stream_id })
}

pub fn random_bytes(len: usize) -> Vec<u8> {
    let mut bytes = vec![0u8; len];
    rand::rng().fill_bytes(&mut bytes);
    bytes
}

/// An identity on no fixture: a fresh `server_id` and key, for the TOFU
/// relay, where a binding is for ever and a fixed id would be bound by the
/// first run.
pub fn fresh_identity() -> Identity {
    let suffix = BASE64URL_NOPAD
        .encode(&random_bytes(9))
        .replace(['-', '_'], "x");
    let seed: [u8; 32] = random_bytes(32).try_into().expect("32 bytes");
    Identity {
        server_id: format!("srv_tofu_{suffix}"),
        key: SigningKey::from_bytes(&seed),
    }
}

/// Keeps the suite's own `HELLO`s well inside the relay's per-IP limit: a
/// token bucket at a third of it, which stays under the limit for a fixed
/// window as well as for a bucket.
struct Pacer {
    state: Mutex<(f64, Instant)>,
    capacity: f64,
    per_second: f64,
}

impl Pacer {
    fn new(knobs: &Knobs) -> Self {
        let capacity = (knobs.hello_rate_limit as f64 / 3.0).floor().max(1.0);
        Self {
            state: Mutex::new((capacity, Instant::now())),
            capacity,
            per_second: capacity / knobs.hello_rate_window.as_secs_f64().max(0.001),
        }
    }

    async fn take(&self) {
        let mut state = self.state.lock().await;
        loop {
            let now = Instant::now();
            let (tokens, last) = *state;
            let tokens = (tokens + now.duration_since(last).as_secs_f64() * self.per_second)
                .min(self.capacity);
            if tokens >= 1.0 {
                *state = (tokens - 1.0, now);
                return;
            }
            *state = (tokens, now);
            tokio::time::sleep(Duration::from_secs_f64((1.0 - tokens) / self.per_second)).await;
        }
    }

    /// After a check that spent the relay's whole limit on purpose.
    pub async fn exhausted(&self) {
        *self.state.lock().await = (0.0, Instant::now());
    }
}

impl Relay {
    pub async fn rate_limit_spent(&self) {
        self.pacer.exhausted().await;
    }
}
