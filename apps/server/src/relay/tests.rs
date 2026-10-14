//! The tunnel client against a fake relay.
//!
//! The fake relay is the other half of SRP §4/§5, written from the spec the
//! same way `apps/relay/` was and sharing no code with either. That is the
//! point: if this suite and the real relay both pass, the spec is what they
//! agreed on.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::Router;
use axum::body::Bytes;
use data_encoding::BASE64URL_NOPAD;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use futures_util::{SinkExt, StreamExt};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

use super::proto;
use crate::auth::ServerIdentity;
use crate::registry::RegisteredRelays;

// ---------------------------------------------------------------------------
// The fake relay
// ---------------------------------------------------------------------------

/// What the relay does once a server connects.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Behaviour {
    /// Register, then relay whatever the test drives.
    Normal,
    /// Fail the challenge, the way a relay refuses a `server_id` bound to a
    /// different key (§4.1).
    RefuseAuth,
    /// Register, then stop reading the socket entirely — the Q17 case.
    HangAfterRegister,
}

/// Something the server sent us.
#[derive(Debug)]
enum Event {
    Registered { server_id: String, pubkey: String },
    Text(serde_json::Value),
    Binary(Vec<u8>),
    Disconnected,
}

struct FakeRelay {
    url: String,
    events: mpsc::UnboundedReceiver<Event>,
    to_server: mpsc::UnboundedSender<Message>,
    /// Registration attempts seen, so a reconnect test can count them.
    attempts: Arc<std::sync::atomic::AtomicUsize>,
    /// Drops the live trunk's socket without a `CLOSE`: the relay vanishing.
    vanish: Arc<tokio::sync::Notify>,
    /// While `false` the relay stops reading the trunk but still writes to it.
    reading: tokio::sync::watch::Sender<bool>,
    _task: tokio::task::JoinHandle<()>,
}

impl FakeRelay {
    async fn start(behaviour: Behaviour) -> Self {
        Self::start_with(behaviour, 15).await
    }

    async fn start_with(behaviour: Behaviour, heartbeat_secs: u64) -> Self {
        // Ephemeral port: never 8484/8485, and safe to run many at once.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let (events_tx, events) = mpsc::unbounded_channel();
        let (to_server, mut commands) = mpsc::unbounded_channel::<Message>();
        let attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let vanish = Arc::new(tokio::sync::Notify::new());
        let (reading, reading_rx) = tokio::sync::watch::channel(true);

        let task = tokio::spawn({
            let attempts = attempts.clone();
            let vanish = vanish.clone();
            let mut reading_rx = reading_rx;
            async move {
                loop {
                    let Ok((socket, _)) = listener.accept().await else {
                        return;
                    };
                    // As the real relay does, so a delay measured through the
                    // tunnel can only be the server's own socket.
                    socket.set_nodelay(true).unwrap();
                    attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    let Ok(mut ws) = tokio_tungstenite::accept_async(socket).await else {
                        continue;
                    };

                    // ---- §4.1 registration --------------------------------
                    let Some(register) = read_json(&mut ws).await else {
                        continue;
                    };
                    assert_eq!(register["type"], "REGISTER_SERVER");
                    assert_eq!(register["v"], 1);
                    let server_id = register["server_id"].as_str().unwrap().to_string();
                    let pubkey = register["pubkey"].as_str().unwrap().to_string();

                    // 20 characters, no colon, no quote — what
                    // `validate_nonce` accepts.
                    let nonce = "0123456789abcdef0123";
                    ws.send(Message::text(
                        serde_json::json!({"v":1,"type":"CHALLENGE","nonce":nonce}).to_string(),
                    ))
                    .await
                    .unwrap();

                    let Some(response) = read_json(&mut ws).await else {
                        continue;
                    };
                    assert_eq!(response["type"], "CHALLENGE_RESPONSE");
                    let sig = response["sig"].as_str().unwrap();

                    // The relay's whole job at this point: prove the server
                    // holds the key behind the `server_id` it is claiming.
                    assert!(
                        verify_relay_auth(&pubkey, &server_id, nonce, sig),
                        "the server's signature must verify against its published key"
                    );

                    if behaviour == Behaviour::RefuseAuth {
                        let _ = ws
                            .send(Message::text(
                                serde_json::json!({
                                    "v":1,"type":"ERROR",
                                    "code":"auth_failed","message":"authentication failed"
                                })
                                .to_string(),
                            ))
                            .await;
                        let _ = ws.close(None).await;
                        continue;
                    }

                    ws.send(Message::text(
                        serde_json::json!({
                            "v":1,"type":"REGISTERED","trunk_id":"trk_test",
                            "public_address": format!("ws://relay.test/connect/{server_id}"),
                            "heartbeat_interval_secs": heartbeat_secs
                        })
                        .to_string(),
                    ))
                    .await
                    .unwrap();
                    let _ = events_tx.send(Event::Registered { server_id, pubkey });

                    if behaviour == Behaviour::HangAfterRegister {
                        // Registered, then dead to the world: never read,
                        // never write, never close. The socket stays open, so
                        // the server has no error to react to. Held on its own
                        // task so a redial can still be accepted and counted.
                        tokio::spawn(async move {
                            let _held = ws;
                            std::future::pending::<()>().await;
                        });
                        continue;
                    }

                    // ---- pump ---------------------------------------------
                    loop {
                        tokio::select! {
                            _ = vanish.notified() => {
                                let _ = events_tx.send(Event::Disconnected);
                                break;
                            }
                            Ok(()) = reading_rx.changed() => continue,
                            command = commands.recv() => match command {
                                Some(message) => {
                                    if ws.send(message).await.is_err() { break; }
                                }
                                None => break,
                            },
                            incoming = ws.next(), if *reading_rx.borrow() => match incoming {
                                Some(Ok(Message::Text(text))) => {
                                    let value: serde_json::Value =
                                        serde_json::from_str(&text).unwrap();
                                    // Every control frame must carry the
                                    // envelope, or the real relay drops it.
                                    assert_eq!(value["v"], 1, "missing version envelope: {text}");
                                    let _ = events_tx.send(Event::Text(value));
                                }
                                Some(Ok(Message::Binary(bytes))) => {
                                    let _ = events_tx.send(Event::Binary(bytes.to_vec()));
                                }
                                Some(Ok(_)) => {}
                                Some(Err(_)) | None => {
                                    let _ = events_tx.send(Event::Disconnected);
                                    break;
                                }
                            },
                        }
                    }
                }
            }
        });

        Self {
            url,
            events,
            to_server,
            attempts,
            vanish,
            reading,
            _task: task,
        }
    }

    /// Drops the socket mid-whatever, with no `CLOSE` and no `ERROR`.
    fn vanish(&self) {
        self.vanish.notify_one();
    }

    fn send(&self, value: serde_json::Value) {
        self.to_server
            .send(Message::text(value.to_string()))
            .unwrap();
    }

    async fn next_event(&mut self) -> Event {
        tokio::time::timeout(Duration::from_secs(10), self.events.recv())
            .await
            .expect("timed out waiting for the server")
            .expect("relay channel closed")
    }

    /// The next control frame, skipping the heartbeat.
    async fn next_control(&mut self) -> serde_json::Value {
        loop {
            match self.next_event().await {
                Event::Text(value) if value["type"] == "PING" => continue,
                Event::Text(value) => return value,
                other => panic!("expected a control frame, got {other:?}"),
            }
        }
    }

    /// The `(server_id, pubkey)` the server actually registered under.
    ///
    /// Both halves, because §4.1 binds them together for ever: a server that
    /// registered the right id under the wrong key would look identical to a
    /// caller that only ever read the id back.
    async fn await_registration(&mut self) -> (String, String) {
        match self.next_event().await {
            Event::Registered { server_id, pubkey } => (server_id, pubkey),
            other => panic!("expected registration, got {other:?}"),
        }
    }

    /// Drives one whole request/response exchange (§5.1 + §5.2).
    async fn request(&mut self, stream_id: u32, req: TunnelRequest<'_>) -> TunnelResponse {
        self.send(serde_json::json!({
            "v":1,"type":"STREAM_OPEN","stream_id":stream_id
        }));
        let ack = self.next_control().await;
        assert_eq!(ack["type"], "STREAM_ACK", "a stream must be acknowledged");
        assert_eq!(ack["stream_id"], stream_id);

        let mut head = serde_json::json!({
            "v":1,"type":"HTTP_REQUEST_HEAD","stream_id":stream_id,
            "method":req.method,"path":req.path,
            "headers": req.headers.iter().cloned().collect::<serde_json::Map<_,_>>(),
        });
        if let Some(peer) = req.peer {
            head["relay_peer_ip"] = serde_json::json!(peer);
        }
        self.send(head);

        if let Some(body) = req.body {
            self.to_server
                .send(Message::binary(proto::encode_body_frame(
                    proto::FRAME_REQUEST_BODY,
                    stream_id,
                    body,
                )))
                .unwrap();
            // The end-of-body convention: a zero-length frame.
            self.to_server
                .send(Message::binary(proto::encode_body_frame(
                    proto::FRAME_REQUEST_BODY,
                    stream_id,
                    b"",
                )))
                .unwrap();
        }

        let head = self.next_control().await;
        assert_eq!(head["type"], "HTTP_RESPONSE_HEAD", "got {head}");
        assert_eq!(head["stream_id"], stream_id);
        let status = head["status"].as_u64().unwrap() as u16;
        let headers = head["headers"].clone();

        let mut body = Vec::new();
        loop {
            match self.next_event().await {
                Event::Binary(frame) => {
                    let (kind, id, payload) = proto::decode_body_frame(&frame).unwrap();
                    assert_eq!(kind, proto::FRAME_RESPONSE_BODY);
                    assert_eq!(id, stream_id);
                    body.extend_from_slice(payload);
                }
                Event::Text(value) if value["type"] == "PING" => continue,
                Event::Text(value) if value["type"] == "CLOSE" => {
                    assert_eq!(value["stream_id"], stream_id);
                    break;
                }
                other => panic!("unexpected {other:?}"),
            }
        }

        TunnelResponse {
            status,
            headers,
            body,
        }
    }
}

#[derive(Default)]
struct TunnelRequest<'a> {
    method: &'a str,
    path: &'a str,
    headers: Vec<(String, serde_json::Value)>,
    peer: Option<&'a str>,
    body: Option<&'a [u8]>,
}

impl<'a> TunnelRequest<'a> {
    fn get(path: &'a str) -> Self {
        Self {
            method: "GET",
            path,
            ..Default::default()
        }
    }
    fn header(mut self, name: &str, value: &str) -> Self {
        self.headers
            .push((name.to_string(), serde_json::json!(value)));
        self
    }
    fn peer(mut self, ip: &'a str) -> Self {
        self.peer = Some(ip);
        self
    }
}

struct TunnelResponse {
    status: u16,
    headers: serde_json::Value,
    body: Vec<u8>,
}

impl TunnelResponse {
    fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body).unwrap_or(serde_json::Value::Null)
    }
}

async fn read_json<S>(ws: &mut tokio_tungstenite::WebSocketStream<S>) -> Option<serde_json::Value>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    loop {
        match ws.next().await? {
            Ok(Message::Text(text)) => return serde_json::from_str(&text).ok(),
            Ok(Message::Close(_)) | Err(_) => return None,
            Ok(_) => continue,
        }
    }
}

fn verify_relay_auth(pubkey_b64: &str, server_id: &str, nonce: &str, sig_b64: &str) -> bool {
    let Ok(raw_key) = BASE64URL_NOPAD.decode(pubkey_b64.as_bytes()) else {
        return false;
    };
    let Ok(bytes) = <[u8; 32]>::try_from(raw_key.as_slice()) else {
        return false;
    };
    let Ok(key) = VerifyingKey::from_bytes(&bytes) else {
        return false;
    };
    let Ok(raw_sig) = BASE64URL_NOPAD.decode(sig_b64.as_bytes()) else {
        return false;
    };
    let Ok(signature) = Signature::from_slice(&raw_sig) else {
        return false;
    };
    key.verify(
        &crate::auth::identity::relay_auth_message(server_id, nonce),
        &signature,
    )
    .is_ok()
}

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

fn start_tunnel(
    relay_url: &str,
    router: Router,
    identity: Arc<ServerIdentity>,
) -> (RegisteredRelays, super::Tunnels) {
    let registered = RegisteredRelays::default();
    let tunnels = super::Tunnels::spawn(
        std::slice::from_ref(&relay_url.to_string()),
        identity,
        router,
        registered.clone(),
        "127.0.0.1:8484",
    );
    (registered, tunnels)
}

/// A router that reports what actually reached it — the only way to assert on
/// the `Host` rewrite and the header strip end to end rather than at the seam.
fn echo_router() -> Router {
    Router::new().route(
        "/echo",
        axum::routing::get(|request: axum::extract::Request| async move {
            let headers: std::collections::BTreeMap<String, String> = request
                .headers()
                .iter()
                .map(|(name, value)| {
                    (
                        name.as_str().to_string(),
                        value.to_str().unwrap_or_default().to_string(),
                    )
                })
                .collect();
            let peer = request
                .extensions()
                .get::<axum::extract::ConnectInfo<SocketAddr>>()
                .map(|c| c.0.ip().to_string());
            axum::Json(serde_json::json!({ "headers": headers, "peer": peer }))
        }),
    )
}

fn test_identity(dir: &std::path::Path) -> Arc<ServerIdentity> {
    let state_dir = dir.join("state");
    std::fs::create_dir_all(&state_dir).unwrap();
    let mut db = crate::auth::AuthDb::open(&state_dir).unwrap();
    Arc::new(
        crate::auth::identity::load_or_create(&mut db, &state_dir, "2026-08-31T00:00:00Z").unwrap(),
    )
}

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

/// Waits until the tunnel client is advertising `count` relays.
///
/// `FakeRelay::await_registration` resolves when the relay *sends* `REGISTERED`,
/// which is necessarily before the client has received and acted on it. Reading
/// the advertised set — or shutting the supervisor down — the instant that event
/// arrives is a race, and one that a loaded machine loses reliably rather than
/// occasionally.
///
/// Waiting for the state keeps the assertion honest: a registration that never
/// lands still fails the test, it just fails on the deadline instead of on a
/// misleading empty set.
async fn await_advertised(registered: &RegisteredRelays, count: usize) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while registered.snapshot().len() != count {
        if tokio::time::Instant::now() >= deadline {
            panic!(
                "timed out waiting for {count} advertised relay(s); have {:?}",
                registered.snapshot()
            );
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn registration_round_trips_and_the_signature_verifies() {
    let dir = tempdir::TempDir::new("storm-relay-register").unwrap();
    let identity = test_identity(dir.path());
    let mut relay = FakeRelay::start(Behaviour::Normal).await;

    let (registered, tunnels) = start_tunnel(&relay.url, echo_router(), identity.clone());

    // The fake relay asserts the signature itself; reaching here means
    // REGISTER_SERVER → CHALLENGE → CHALLENGE_RESPONSE → REGISTERED completed
    // and the signature verified against the published key.
    let (server_id, pubkey) = relay.await_registration().await;
    assert_eq!(server_id, identity.server_id);
    // The key as well as the id. The relay binds the pair permanently (§4.1),
    // so registering under the right id with the wrong key is the one mistake
    // there is no way back from.
    assert_eq!(pubkey, identity.public_key_b64());

    await_advertised(&registered, 1).await;
    assert_eq!(
        registered.snapshot(),
        vec![relay.url.clone()],
        "a successful registration is what puts a relay on the wire"
    );
    tunnels.shutdown().await;
}

#[tokio::test]
async fn a_relay_that_refuses_registration_is_never_advertised() {
    // **Registered, never configured.** A client races its candidate
    // addresses on a ~2 s budget, so a dead path burns part of that budget on
    // every reconnect.
    let dir = tempdir::TempDir::new("storm-relay-refused").unwrap();
    let identity = test_identity(dir.path());
    let relay = FakeRelay::start(Behaviour::RefuseAuth).await;

    let (registered, tunnels) = start_tunnel(&relay.url, echo_router(), identity);

    // Give the supervisor time to try and be refused.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        registered.is_empty(),
        "a relay that refused registration must not be advertised"
    );
    tunnels.shutdown().await;
}

#[tokio::test]
async fn registration_failure_retries_with_backoff() {
    let dir = tempdir::TempDir::new("storm-relay-retry").unwrap();
    let identity = test_identity(dir.path());
    let relay = FakeRelay::start(Behaviour::RefuseAuth).await;
    let attempts = relay.attempts.clone();

    let (_registered, tunnels) = start_tunnel(&relay.url, echo_router(), identity);

    // The first attempt is immediate and the second waits `BACKOFF_MIN` (1 s).
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        attempts.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "the retry must wait rather than spin"
    );

    tokio::time::sleep(Duration::from_millis(1_200)).await;
    let after_backoff = attempts.load(std::sync::atomic::Ordering::SeqCst);
    assert!(
        after_backoff >= 2,
        "it must retry after the backoff, saw {after_backoff} attempts"
    );
    // Doubling means far fewer than one attempt per 100 ms.
    assert!(
        after_backoff <= 4,
        "backoff must slow retries down, saw {after_backoff} attempts in 1.5 s"
    );
    tunnels.shutdown().await;
}

#[tokio::test]
async fn registered_relays_empties_when_the_trunk_drops() {
    let dir = tempdir::TempDir::new("storm-relay-drop").unwrap();
    let identity = test_identity(dir.path());
    let mut relay = FakeRelay::start(Behaviour::Normal).await;

    let (registered, tunnels) = start_tunnel(&relay.url, echo_router(), identity);
    let _ = relay.await_registration().await;
    await_advertised(&registered, 1).await;

    // Shutting the supervisor down is a disconnect; the set must not keep
    // advertising a trunk that no longer exists.
    tunnels.shutdown().await;
    assert!(
        registered.is_empty(),
        "a dropped trunk must stop being advertised"
    );
}

#[tokio::test]
async fn deregister_is_sent_on_clean_shutdown() {
    // Without it the relay holds the `server_id` until a heartbeat timeout and
    // every client waits that out — a restart would look like an outage.
    let dir = tempdir::TempDir::new("storm-relay-deregister").unwrap();
    let identity = test_identity(dir.path());
    let mut relay = FakeRelay::start(Behaviour::Normal).await;

    let (registered, tunnels) = start_tunnel(&relay.url, echo_router(), identity);
    let _ = relay.await_registration().await;
    // DEREGISTER is only sent for a trunk the client knows it registered, so
    // shutting down before it has processed REGISTERED tests nothing.
    await_advertised(&registered, 1).await;

    tunnels.shutdown().await;

    let mut saw_deregister = false;
    while let Ok(event) = tokio::time::timeout(Duration::from_secs(2), relay.events.recv()).await {
        match event {
            Some(Event::Text(value)) if value["type"] == "DEREGISTER" => {
                saw_deregister = true;
                break;
            }
            Some(_) => continue,
            None => break,
        }
    }
    assert!(saw_deregister, "clean shutdown must send DEREGISTER");
}

/// Waits for the relay to see a `DEREGISTER`, skipping heartbeats and the rest.
async fn saw_deregister(relay: &mut FakeRelay) -> bool {
    while let Ok(event) = tokio::time::timeout(Duration::from_secs(3), relay.events.recv()).await {
        match event {
            Some(Event::Text(value)) if value["type"] == "DEREGISTER" => return true,
            Some(_) => continue,
            None => return false,
        }
    }
    false
}

#[tokio::test]
async fn relays_are_added_and_removed_without_a_restart() {
    // Decision 74. Before it, `PUT /v1/config/relays` stored the list and
    // nothing connected until the server restarted.
    let dir = tempdir::TempDir::new("storm-relay-reconcile").unwrap();
    let identity = test_identity(dir.path());
    let mut a = FakeRelay::start(Behaviour::Normal).await;
    let mut b = FakeRelay::start(Behaviour::Normal).await;
    let registered = RegisteredRelays::default();
    let mut tunnels = super::Tunnels::spawn(
        &[],
        identity,
        echo_router(),
        registered.clone(),
        "127.0.0.1:8484",
    );

    tunnels.reconcile(std::slice::from_ref(&a.url)).await;
    let _ = a.await_registration().await;
    await_advertised(&registered, 1).await;

    // Adding a second relay must not touch the first one's trunk.
    tunnels.reconcile(&[a.url.clone(), b.url.clone()]).await;
    let _ = b.await_registration().await;
    await_advertised(&registered, 2).await;
    assert_eq!(
        a.attempts.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "an unchanged relay was reconnected"
    );

    // Removing one sends it a DEREGISTER and stops advertising it; the other
    // stays up.
    tunnels.reconcile(std::slice::from_ref(&b.url)).await;
    assert!(
        saw_deregister(&mut a).await,
        "a removed relay must get DEREGISTER"
    );
    await_advertised(&registered, 1).await;
    assert_eq!(registered.snapshot(), vec![b.url.clone()]);

    tunnels.shutdown().await;
    assert!(saw_deregister(&mut b).await);
}

#[tokio::test]
async fn a_saved_relay_list_reaches_the_tunnels_through_manage() {
    // The wiring `put_relays` relies on: a value sent on the channel is
    // applied, and stopping the manager still deregisters cleanly.
    let dir = tempdir::TempDir::new("storm-relay-manage").unwrap();
    let identity = test_identity(dir.path());
    let mut relay = FakeRelay::start(Behaviour::Normal).await;
    let registered = RegisteredRelays::default();
    let tunnels = super::Tunnels::spawn(
        &[],
        identity,
        echo_router(),
        registered.clone(),
        "127.0.0.1:8484",
    );
    let (changes, rx) = tokio::sync::watch::channel(Vec::new());
    let (stop, stop_rx) = tokio::sync::oneshot::channel();
    let manager = tokio::spawn(super::manage(tunnels, rx, stop_rx));

    changes.send_replace(vec![relay.url.clone()]);
    let _ = relay.await_registration().await;
    await_advertised(&registered, 1).await;

    let _ = stop.send(());
    tokio::time::timeout(Duration::from_secs(10), manager)
        .await
        .expect("the manager did not stop")
        .unwrap();
    assert!(saw_deregister(&mut relay).await);
}

// ---------------------------------------------------------------------------
// Tunnelled requests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_tunnelled_get_reaches_the_router_and_comes_back_intact() {
    let dir = tempdir::TempDir::new("storm-relay-get").unwrap();
    let (router, identity, _state) = crate::api::tests::test_router_with_state(dir.path());
    let mut relay = FakeRelay::start(Behaviour::Normal).await;

    let (_registered, tunnels) = start_tunnel(&relay.url, router.clone(), identity);
    let _ = relay.await_registration().await;

    let response = relay.request(1, TunnelRequest::get("/v1/health")).await;
    assert_eq!(response.status, 200);
    assert_eq!(
        response.headers["content-type"], "application/json",
        "response headers survive the tunnel"
    );
    assert_eq!(
        response.json()["status"],
        "ok",
        "and so does the body: {}",
        String::from_utf8_lossy(&response.body)
    );

    tunnels.shutdown().await;
}

/// SRP §4.2 (decision 108c): every `PING` is answered, including one a relay
/// sends while the trunk's outbound queue is full. A relay may `PING` to learn
/// how much the server has read, so a dropped `PONG` reads as a stalled server.
#[tokio::test]
async fn a_relay_ping_is_answered_even_behind_a_full_outbound_queue() {
    static BIG: std::sync::OnceLock<Bytes> = std::sync::OnceLock::new();
    let dir = tempdir::TempDir::new("storm-relay-ping").unwrap();
    let identity = test_identity(dir.path());
    let router = Router::new().route(
        "/big",
        axum::routing::get(|| async {
            BIG.get_or_init(|| Bytes::from(vec![0u8; 4 << 20])).clone()
        }),
    );
    let mut relay = FakeRelay::start(Behaviour::Normal).await;
    let (_registered, tunnels) = start_tunnel(&relay.url, router, identity);
    let _ = relay.await_registration().await;

    relay.reading.send_replace(false);
    for stream_id in 1..=24u32 {
        relay.send(serde_json::json!({"v":1,"type":"STREAM_OPEN","stream_id":stream_id}));
        relay.send(serde_json::json!({
            "v":1,"type":"HTTP_REQUEST_HEAD","stream_id":stream_id,
            "method":"GET","path":"/big","headers":{}
        }));
    }
    tokio::time::sleep(Duration::from_millis(500)).await;
    relay.send(serde_json::json!({"v":1,"type":"PING"}));
    tokio::time::sleep(Duration::from_millis(200)).await;
    relay.reading.send_replace(true);

    loop {
        match relay.next_event().await {
            Event::Text(value) if value["type"] == "PONG" => break,
            Event::Disconnected => panic!("the trunk dropped before the PONG"),
            _ => {}
        }
    }

    tunnels.shutdown().await;
}

fn big_download_router() -> Router {
    static BIG: std::sync::OnceLock<Bytes> = std::sync::OnceLock::new();
    Router::new().route(
        "/big",
        axum::routing::get(|| async {
            BIG.get_or_init(|| Bytes::from(vec![0u8; 4 << 20])).clone()
        }),
    )
}

/// Fills the outbound queue: 24 downloads of 4 MiB toward a relay that has
/// stopped reading.
fn stall_behind_downloads(relay: &FakeRelay) {
    relay.reading.send_replace(false);
    for stream_id in 1..=24u32 {
        relay.send(serde_json::json!({"v":1,"type":"STREAM_OPEN","stream_id":stream_id}));
        relay.send(serde_json::json!({
            "v":1,"type":"HTTP_REQUEST_HEAD","stream_id":stream_id,
            "method":"GET","path":"/big","headers":{}
        }));
    }
}

/// Body frames the relay reads before the first control frame `wanted` picks.
/// What the sockets themselves buffer arrives first, so a frame that went
/// ahead of the queue still comes after a few dozen.
async fn body_frames_before(
    relay: &mut FakeRelay,
    wanted: impl Fn(&serde_json::Value) -> bool,
) -> usize {
    let mut bodies = 0;
    loop {
        match relay.next_event().await {
            Event::Text(value) if wanted(&value) => return bodies,
            Event::Binary(_) => bodies += 1,
            Event::Disconnected => panic!("the trunk dropped"),
            _ => {}
        }
    }
}

/// SRP §5.1/§5.2 (decision 116b): `STREAM_ACK` goes ahead of the response
/// bodies already queued, and is never dropped for want of room. Behind a
/// full queue it used to be dropped, so every request made during a large
/// download got `server_timeout`.
#[tokio::test]
async fn a_stream_ack_overtakes_a_full_download_queue() {
    let dir = tempdir::TempDir::new("storm-relay-ack").unwrap();
    let identity = test_identity(dir.path());
    let mut relay = FakeRelay::start(Behaviour::Normal).await;
    let (_registered, tunnels) = start_tunnel(&relay.url, big_download_router(), identity);
    let _ = relay.await_registration().await;

    stall_behind_downloads(&relay);
    tokio::time::sleep(Duration::from_millis(500)).await;
    relay.send(serde_json::json!({"v":1,"type":"STREAM_OPEN","stream_id":100}));
    tokio::time::sleep(Duration::from_millis(200)).await;
    relay.reading.send_replace(true);

    let bodies = body_frames_before(&mut relay, |v| {
        v["type"] == "STREAM_ACK" && v["stream_id"] == 100
    })
    .await;
    assert!(
        bodies < super::client::OUTBOUND_CAPACITY / 2,
        "the ACK waited behind {bodies} body frames"
    );

    tunnels.shutdown().await;
}

/// SRP §5.2 (decision 116b): the heartbeat overtakes queued response bodies,
/// and the trunk is not judged silent while its own `PING` is still waiting
/// to be written. A long download over a slow uplink used to drop the trunk
/// at the deadline although the relay was fine.
#[tokio::test]
async fn a_heartbeat_overtakes_a_full_download_queue_and_a_stalled_trunk_is_not_silent() {
    let dir = tempdir::TempDir::new("storm-relay-hb-bulk").unwrap();
    let identity = test_identity(dir.path());
    // A 1 s heartbeat makes the silence deadline 3 s.
    let mut relay = FakeRelay::start_with(Behaviour::Normal, 1).await;
    let (registered, tunnels) = start_tunnel(&relay.url, big_download_router(), identity);
    let _ = relay.await_registration().await;
    await_advertised(&registered, 1).await;

    stall_behind_downloads(&relay);
    tokio::time::sleep(Duration::from_secs(5)).await;
    assert_eq!(
        registered.snapshot().len(),
        1,
        "the trunk was judged silent while its own PING could not be written"
    );
    assert_eq!(relay.attempts.load(std::sync::atomic::Ordering::SeqCst), 1);

    relay.reading.send_replace(true);
    let bodies = body_frames_before(&mut relay, |v| v["type"] == "PING").await;
    assert!(
        bodies < super::client::OUTBOUND_CAPACITY / 2,
        "the PING waited behind {bodies} body frames"
    );

    tunnels.shutdown().await;
}

/// A tunnelled response is a head, body chunks and a `CLOSE`, written one after
/// another. With Nagle on the tunnel, each after the first waited out the
/// relay's delayed ACK: about 40 ms a request on localhost (P1, decision 91).
#[tokio::test]
async fn a_tunnelled_request_does_not_wait_on_delayed_acks() {
    let dir = tempdir::TempDir::new("storm-relay-latency").unwrap();
    let (router, identity, _state) = crate::api::tests::test_router_with_state(dir.path());
    let mut relay = FakeRelay::start(Behaviour::Normal).await;

    let (_registered, tunnels) = start_tunnel(&relay.url, router.clone(), identity);
    let _ = relay.await_registration().await;

    let mut samples = Vec::new();
    for stream_id in 1..=21 {
        let started = std::time::Instant::now();
        let response = relay
            .request(stream_id, TunnelRequest::get("/v1/health"))
            .await;
        assert_eq!(response.status, 200);
        samples.push(started.elapsed());
    }

    samples.sort();
    let median = samples[samples.len() / 2];
    assert!(
        median < Duration::from_millis(15),
        "median tunnelled request took {median:?}; TCP_NODELAY missing?"
    );

    tunnels.shutdown().await;
}

#[tokio::test]
async fn a_tunnelled_response_matches_what_the_same_request_gets_on_the_lan() {
    // R13 stated as an assertion: one `Service`, two ways in.
    let dir = tempdir::TempDir::new("storm-relay-parity").unwrap();
    let (router, identity, _state) = crate::api::tests::test_router_with_state(dir.path());
    let mut relay = FakeRelay::start(Behaviour::Normal).await;

    let (_registered, tunnels) = start_tunnel(&relay.url, router.clone(), identity);
    let _ = relay.await_registration().await;

    let tunnelled = relay.request(1, TunnelRequest::get("/v1/health")).await;

    use tower::ServiceExt;
    let lan = router
        .oneshot(
            axum::http::Request::builder()
                .uri("/v1/health")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let lan_status = lan.status().as_u16();
    let lan_body = axum::body::to_bytes(lan.into_body(), 1 << 20)
        .await
        .unwrap();

    assert_eq!(tunnelled.status, lan_status);
    assert_eq!(tunnelled.body, lan_body.to_vec());

    tunnels.shutdown().await;
}

#[tokio::test]
async fn a_tunnelled_request_is_authenticated_exactly_as_a_lan_one() {
    // **The test that proves nothing was bypassed.** An earlier design had the
    // tunnel serve the change feed from `state.events` directly, which skipped
    // `require_auth` entirely — every vault's changes to anyone who could open
    // a trunk. A relayed request must be exactly as authenticated as a LAN one.
    let dir = tempdir::TempDir::new("storm-relay-auth").unwrap();
    let (router, identity, state) = crate::api::tests::test_router_with_state(dir.path());
    let mut relay = FakeRelay::start(Behaviour::Normal).await;

    let (_registered, tunnels) = start_tunnel(&relay.url, router, identity);
    let _ = relay.await_registration().await;

    // No credential over the tunnel is a 401, not a served response.
    let anonymous = relay.request(1, TunnelRequest::get("/v1/vaults")).await;
    assert_eq!(
        anonymous.status,
        401,
        "an unauthenticated relayed request must be refused: {}",
        String::from_utf8_lossy(&anonymous.body)
    );

    // A real session works, over the tunnel, with no handler changed.
    let user = crate::api::tests::seed_owner(&state).await;
    let token = crate::api::tests::session_token(&state, &user).await;
    let authenticated = relay
        .request(
            2,
            TunnelRequest::get("/v1/vaults").header("authorization", &format!("Bearer {token}")),
        )
        .await;
    assert_eq!(
        authenticated.status,
        200,
        "a real session must work over the tunnel: {}",
        String::from_utf8_lossy(&authenticated.body)
    );

    // And a forged one is still refused.
    let forged = relay
        .request(
            3,
            TunnelRequest::get("/v1/vaults").header("authorization", "Bearer not-a-real-token"),
        )
        .await;
    assert_eq!(forged.status, 401);

    tunnels.shutdown().await;
}

#[tokio::test]
async fn host_is_rewritten_and_forwarding_headers_are_stripped() {
    let dir = tempdir::TempDir::new("storm-relay-headers").unwrap();
    let identity = test_identity(dir.path());
    let mut relay = FakeRelay::start(Behaviour::Normal).await;

    let (_registered, tunnels) = start_tunnel(&relay.url, echo_router(), identity);
    let _ = relay.await_registration().await;

    let response = relay
        .request(
            1,
            TunnelRequest::get("/echo")
                .header("host", "relay.example.com")
                .header("x-forwarded-for", "9.9.9.9")
                .header("x-forwarded-host", "evil.example")
                .header("x-real-ip", "9.9.9.9")
                .header("forwarded", "for=9.9.9.9")
                .header("accept", "application/json"),
        )
        .await;

    let seen = response.json();
    let headers = &seen["headers"];

    // The relay's hostname is never in `mcp::allowed_hosts`, so leaving it
    // would fail every relayed MCP call for a reason nothing names.
    assert_eq!(
        headers["host"], "127.0.0.1:8484",
        "Host must be rewritten to the bind address"
    );

    // Client-supplied end to end. Left in place, a client could switch web
    // bootstrap off for every relayed request by setting one.
    for stripped in crate::api::FORWARDING_HEADERS {
        assert!(
            headers[stripped].is_null(),
            "{stripped} reached the router: {headers}"
        );
    }
    assert_eq!(headers["accept"], "application/json");

    tunnels.shutdown().await;
}

#[tokio::test]
async fn the_relay_peer_ip_reaches_the_router_as_the_requests_peer() {
    let dir = tempdir::TempDir::new("storm-relay-peer").unwrap();
    let identity = test_identity(dir.path());
    let mut relay = FakeRelay::start(Behaviour::Normal).await;

    let (_registered, tunnels) = start_tunnel(&relay.url, echo_router(), identity);
    let _ = relay.await_registration().await;

    // A forwarding header naming a different address must change nothing:
    // `relay_peer_ip` arrives out of band precisely so it cannot be forged.
    let response = relay
        .request(
            1,
            TunnelRequest::get("/echo")
                .peer("203.0.113.7")
                .header("x-forwarded-for", "9.9.9.9"),
        )
        .await;
    assert_eq!(response.json()["peer"], "203.0.113.7");

    tunnels.shutdown().await;
}

#[tokio::test]
async fn relay_peer_ip_becomes_the_limiters_caller_identity() {
    // The seam is `CallerKey`: `relay_peer_ip` → `ConnectInfo` → `CallerKey::Ip`.
    // Without it every relayed login would share the `Unattributed` bucket, so
    // one caller flooding would throttle everyone else over the tunnel.
    let dir = tempdir::TempDir::new("storm-relay-limiter").unwrap();
    let (router, identity, state) = crate::api::tests::test_router_with_limiter(
        dir.path(),
        crate::api::tests::throttling_limiter(),
    );
    let device = crate::api::tests::pair_a_device(&state).await;
    let mut relay = FakeRelay::start(Behaviour::Normal).await;

    let (_registered, tunnels) = start_tunnel(&relay.url, router, identity);
    let _ = relay.await_registration().await;

    let login = |peer: &'static str, device: String| {
        let body = serde_json::json!({"username":"nobody","password":"a-long-enough-password"})
            .to_string();
        move |stream_id: u32| (stream_id, peer, device.clone(), body.clone())
    };
    let make = login("198.51.100.5", device.clone());

    // The burst fits the budget and reaches the credential check.
    let mut stream_id = 1;
    for _ in 0..crate::auth::ratelimit::tests::TEST_BURST {
        let (id, peer, device, body) = make(stream_id);
        let response = relay
            .request(
                id,
                TunnelRequest {
                    method: "POST",
                    path: "/v1/auth/login",
                    headers: vec![
                        ("authorization".into(), serde_json::json!(device)),
                        ("content-type".into(), serde_json::json!("application/json")),
                    ],
                    peer: Some(peer),
                    body: Some(body.as_bytes()),
                },
            )
            .await;
        assert_eq!(
            response.status,
            401,
            "the burst reaches the credential check: {}",
            String::from_utf8_lossy(&response.body)
        );
        stream_id += 1;
    }

    // The next attempt from that same attested address is throttled.
    let (_, peer, device_h, body) = make(stream_id);
    let throttled = relay
        .request(
            stream_id,
            TunnelRequest {
                method: "POST",
                path: "/v1/auth/login",
                headers: vec![
                    ("authorization".into(), serde_json::json!(device_h)),
                    ("content-type".into(), serde_json::json!("application/json")),
                ],
                peer: Some(peer),
                body: Some(body.as_bytes()),
            },
        )
        .await;
    assert_eq!(
        throttled.status, 429,
        "per-caller rate limiting must work over the tunnel"
    );
    stream_id += 1;

    // A different attested address is not throttled — proving the key is the
    // peer and not one shared bucket for everything relayed.
    let other = relay
        .request(
            stream_id,
            TunnelRequest {
                method: "POST",
                path: "/v1/auth/login",
                headers: vec![
                    ("authorization".into(), serde_json::json!(device)),
                    ("content-type".into(), serde_json::json!("application/json")),
                ],
                peer: Some("198.51.100.99"),
                body: Some(body.as_bytes()),
            },
        )
        .await;
    assert_eq!(
        other.status, 401,
        "a different caller must still get a credential check"
    );

    tunnels.shutdown().await;
}

#[tokio::test]
async fn a_stream_id_already_open_is_refused_rather_than_acked_twice() {
    // §5.1, stated exactly: a duplicate is `stream_closed`, not a second ack.
    let dir = tempdir::TempDir::new("storm-relay-dup").unwrap();
    let identity = test_identity(dir.path());
    let mut relay = FakeRelay::start(Behaviour::Normal).await;

    let (_registered, tunnels) = start_tunnel(&relay.url, echo_router(), identity);
    let _ = relay.await_registration().await;

    relay.send(serde_json::json!({"v":1,"type":"STREAM_OPEN","stream_id":7}));
    assert_eq!(relay.next_control().await["type"], "STREAM_ACK");

    relay.send(serde_json::json!({"v":1,"type":"STREAM_OPEN","stream_id":7}));
    let second = relay.next_control().await;
    assert_eq!(second["type"], "ERROR");
    assert_eq!(second["code"], "stream_closed");
    assert_eq!(second["stream_id"], 7);

    tunnels.shutdown().await;
}

#[tokio::test]
async fn a_request_head_for_an_unopened_stream_is_refused() {
    let dir = tempdir::TempDir::new("storm-relay-nostream").unwrap();
    let identity = test_identity(dir.path());
    let mut relay = FakeRelay::start(Behaviour::Normal).await;

    let (_registered, tunnels) = start_tunnel(&relay.url, echo_router(), identity);
    let _ = relay.await_registration().await;

    relay.send(serde_json::json!({
        "v":1,"type":"HTTP_REQUEST_HEAD","stream_id":42,
        "method":"GET","path":"/echo","headers":{}
    }));
    let response = relay.next_control().await;
    assert_eq!(response["type"], "ERROR");
    assert_eq!(response["code"], "stream_closed");

    tunnels.shutdown().await;
}

// ---------------------------------------------------------------------------
// Q17: the tunnel must never wedge the local listeners
// ---------------------------------------------------------------------------

/// One LAN request over a real socket, returning how long it took.
///
/// Raw HTTP/1.1 rather than a client crate: the point is to measure the real
/// listener, and adding a dependency to do it would be its own risk.
async fn lan_health(addr: SocketAddr) -> Duration {
    let start = Instant::now();
    let mut socket = tokio::net::TcpStream::connect(addr).await.unwrap();
    socket
        .write_all(b"GET /v1/health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut buffer = Vec::new();
    socket.read_to_end(&mut buffer).await.unwrap();
    let elapsed = start.elapsed();
    let text = String::from_utf8_lossy(&buffer);
    assert!(
        text.starts_with("HTTP/1.1 200"),
        "the LAN listener must still answer: {text}"
    );
    elapsed
}

#[tokio::test]
async fn lan_requests_are_unaffected_while_the_relay_is_hung() {
    // **The requirement that would otherwise be believed rather than checked.**
    // The relay registers and then stops reading its socket forever. Response
    // tasks fill the bounded outbound queue and block; the writer blocks on a
    // socket nobody drains. None of that may touch the listener.
    let dir = tempdir::TempDir::new("storm-relay-hung").unwrap();
    let (router, identity, _state) = crate::api::tests::test_router_with_state(dir.path());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let served = router.clone();
    tokio::spawn(async move {
        axum::serve(
            listener,
            served.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });

    // A baseline before any relay exists at all.
    let mut baseline = Duration::ZERO;
    for _ in 0..10 {
        baseline += lan_health(addr).await;
    }
    let baseline = baseline / 10;

    let relay = FakeRelay::start(Behaviour::HangAfterRegister).await;
    let (_registered, tunnels) = start_tunnel(&relay.url, router, identity);

    // The relay never reports registration to us (it hangs first), so wait for
    // the handshake to have happened rather than for an event.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        relay.attempts.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "the trunk should be established and then hung"
    );

    // Pile relayed work onto the hung trunk: far more than the outbound
    // channel can hold, so the queue is full and tasks are blocked on it.
    for stream_id in 0..(super::client::OUTBOUND_CAPACITY as u32 * 3) {
        relay.send(serde_json::json!({
            "v":1,"type":"STREAM_OPEN","stream_id":stream_id
        }));
        relay.send(serde_json::json!({
            "v":1,"type":"HTTP_REQUEST_HEAD","stream_id":stream_id,
            "method":"GET","path":"/v1/health","headers":{}
        }));
    }
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Now measure. Every one of these must still be served, promptly.
    let mut worst = Duration::ZERO;
    let mut total = Duration::ZERO;
    for _ in 0..20 {
        let elapsed = lan_health(addr).await;
        worst = worst.max(elapsed);
        total += elapsed;
    }
    let mean = total / 20;

    // A wedge is not a slowdown, it is a hang — so a generous bound still
    // catches the failure this test exists for, without being flaky on a busy
    // machine.
    assert!(
        worst < Duration::from_secs(2),
        "a hung relay delayed a LAN request by {worst:?} \
         (mean {mean:?}, baseline {baseline:?})"
    );

    tunnels.shutdown().await;
}

#[tokio::test]
async fn a_relay_that_never_completes_its_handshake_does_not_block_startup() {
    // The connect path is a background task; a relay that accepts TCP and then
    // says nothing must cost a task and nothing else.
    let dir = tempdir::TempDir::new("storm-relay-silent").unwrap();
    let (router, identity, _state) = crate::api::tests::test_router_with_state(dir.path());

    // A listener that accepts and never speaks WebSocket at all.
    let dead = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let dead_url = format!("ws://{}", dead.local_addr().unwrap());
    tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((socket, _)) = dead.accept().await {
            held.push(socket); // accepted, never answered
        }
    });

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let served = router.clone();
    tokio::spawn(async move {
        axum::serve(
            listener,
            served.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });

    let (registered, tunnels) = start_tunnel(&dead_url, router, identity);

    let elapsed = lan_health(addr).await;
    assert!(
        elapsed < Duration::from_secs(2),
        "a silent relay delayed a LAN request by {elapsed:?}"
    );
    assert!(
        registered.is_empty(),
        "a relay stuck in its handshake is not registered"
    );

    tunnels.shutdown().await;
}

#[tokio::test]
async fn the_terminal_stream_crosses_the_relay_on_the_same_handler_and_auth() {
    // AC-T2 (decision 77c). Relay connectivity is not V1 acceptance, but the
    // protocol must stay relay-shaped: the terminal stream, dispatched in
    // process through the tunnel, is served by the same handler under the
    // same `require_auth` and the same owner check as on the LAN (R13).
    let dir = tempdir::TempDir::new("storm-relay-agent").unwrap();
    let (router, identity, state) = crate::api::tests::test_router_with_state(dir.path());

    // A session that has ended, with output, so the stream sends everything
    // and closes — what lets the fake relay read the whole response.
    let (_generation, _rx) = state.agent.connect_host("hst_01HB6V3Z7Q2M4N8P0R5S9T1W3X");
    state
        .agent
        .hello(
            "hst_01HB6V3Z7Q2M4N8P0R5S9T1W3X",
            serde_json::from_value(serde_json::json!({
                "providers": [{"id": "fake", "kind": "fake", "interactions": ["terminal"], "available": true}],
                "workspaces": ["storm"],
                "max_sessions": 4,
            }))
            .unwrap(),
        )
        .unwrap();
    let owner = crate::api::tests::seed_owner(&state).await;
    let record = state
        .agent
        .launch(
            &owner,
            serde_json::from_value(serde_json::json!({
                "host_id": "hst_01HB6V3Z7Q2M4N8P0R5S9T1W3X",
                "workspace": "storm",
                "provider": "fake",
                "terminal": {"cols": 80, "rows": 24},
            }))
            .unwrap(),
            Default::default(),
            Vec::new(),
        )
        .unwrap()
        .0;
    let host = "hst_01HB6V3Z7Q2M4N8P0R5S9T1W3X";
    state
        .agent
        .host_output(host, &record.id, 0, b"over the relay")
        .unwrap();
    state
        .agent
        .host_status(
            host,
            &record.id,
            &serde_json::from_value(serde_json::json!({"status": "completed", "exit_code": 0}))
                .unwrap(),
        )
        .unwrap();

    let mut relay = FakeRelay::start(Behaviour::Normal).await;
    let (_registered, tunnels) = start_tunnel(&relay.url, router, identity);
    let _ = relay.await_registration().await;
    let path = format!("/v1/agent/sessions/{}/terminal/stream?offset=0", record.id);

    let anonymous = relay.request(1, TunnelRequest::get(&path)).await;
    assert_eq!(
        anonymous.status, 401,
        "no credential, no stream — relayed or not"
    );

    let token = crate::api::tests::session_token(&state, &owner).await;
    let streamed = relay
        .request(
            2,
            TunnelRequest::get(&path).header("authorization", &format!("Bearer {token}")),
        )
        .await;
    assert_eq!(streamed.status, 200);
    assert!(
        streamed.headers["content-type"]
            .as_str()
            .unwrap_or_default()
            .starts_with("text/event-stream"),
        "{}",
        streamed.headers
    );
    let body = String::from_utf8_lossy(&streamed.body);
    assert!(body.starts_with("event: status"), "status first: {body}");
    assert!(body.contains("event: output"), "{body}");
    assert!(
        body.contains(&data_encoding::BASE64.encode(b"over the relay")),
        "{body}"
    );

    // And the host tier is the host tier over the relay too.
    let whoami = relay
        .request(
            4,
            TunnelRequest::get("/v1/runtime/whoami")
                .header("authorization", &format!("Bearer {token}")),
        )
        .await;
    assert_eq!(whoami.status, 401);

    tunnels.shutdown().await;
}

// ---------------------------------------------------------------------------
// MCP over the tunnel (relay TODO §5, decision 95)
// ---------------------------------------------------------------------------

impl<'a> TunnelRequest<'a> {
    fn mcp(body: &'a [u8], auth: &str) -> Self {
        Self {
            method: "POST",
            path: "/mcp",
            body: Some(body),
            ..Default::default()
        }
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .header("authorization", auth)
    }
}

fn rpc(method: &str, params: serde_json::Value) -> Vec<u8> {
    serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params})
        .to_string()
        .into_bytes()
}

fn tool(name: &str, args: serde_json::Value) -> Vec<u8> {
    rpc(
        "tools/call",
        serde_json::json!({"name": name, "arguments": args}),
    )
}

async fn lan(router: &Router, body: &[u8], auth: &str, host: &str) -> (u16, serde_json::Value) {
    use tower::ServiceExt;
    let response = router
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/mcp")
                .header("content-type", "application/json")
                .header("accept", "application/json, text/event-stream")
                .header("authorization", auth)
                .header("host", host)
                .body(axum::body::Body::from(body.to_vec()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status().as_u16();
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 22)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
    )
}

async fn mcp_vault(router: &Router, bearer: &str, name: &str) -> String {
    use tower::ServiceExt;
    let response = router
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/v1/vaults")
                .header("content-type", "application/json")
                .header("authorization", bearer)
                .body(axum::body::Body::from(
                    serde_json::json!({ "name": name }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .unwrap();
    let created: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    created["id"]
        .as_str()
        .unwrap_or_else(|| panic!("{created}"))
        .to_string()
}

fn set_mcp(state: &crate::api::Shared, enabled: bool, writable: bool) {
    use std::sync::atomic::Ordering::Relaxed;
    state.mcp_enabled.store(enabled, Relaxed);
    state.mcp_writable.store(writable, Relaxed);
}

fn structured(response: &serde_json::Value) -> &serde_json::Value {
    &response["result"]["structuredContent"]
}

struct McpTunnel {
    relay: FakeRelay,
    tunnels: super::Tunnels,
    router: Router,
    state: crate::api::Shared,
    bearer: String,
    next_stream: u32,
    _dir: tempdir::TempDir,
}

impl McpTunnel {
    async fn start(name: &str) -> Self {
        let dir = tempdir::TempDir::new(name).unwrap();
        let (router, identity, state) = crate::api::tests::test_router_with_state(dir.path());
        Self::over(dir, router, identity, state).await
    }

    async fn over(
        dir: tempdir::TempDir,
        router: Router,
        identity: Arc<ServerIdentity>,
        state: crate::api::Shared,
    ) -> Self {
        let owner = crate::api::tests::seed_owner(&state).await;
        let bearer = format!(
            "Bearer {}",
            crate::api::tests::session_token(&state, &owner).await
        );
        let mut relay = FakeRelay::start(Behaviour::Normal).await;
        let (_registered, tunnels) = start_tunnel(&relay.url, router.clone(), identity);
        let _ = relay.await_registration().await;
        Self {
            relay,
            tunnels,
            router,
            state,
            bearer,
            next_stream: 1,
            _dir: dir,
        }
    }

    async fn relayed(&mut self, body: &[u8]) -> (u16, serde_json::Value) {
        let bearer = self.bearer.clone();
        self.relayed_as(body, &bearer).await
    }

    async fn relayed_as(&mut self, body: &[u8], auth: &str) -> (u16, serde_json::Value) {
        let id = self.next_stream;
        self.next_stream += 1;
        let response = self.relay.request(id, TunnelRequest::mcp(body, auth)).await;
        (response.status, response.json())
    }

    async fn lan(&self, body: &[u8]) -> (u16, serde_json::Value) {
        lan(&self.router, body, &self.bearer, "localhost").await
    }
}

#[tokio::test]
async fn mcp_over_the_tunnel_answers_exactly_as_on_the_lan() {
    let mut t = McpTunnel::start("storm-relay-mcp-parity").await;
    set_mcp(&t.state, true, true);
    let vault = mcp_vault(&t.router, &t.bearer, "Notes").await;

    let (status, made) = t
        .relayed(&tool(
            "create_note",
            serde_json::json!({"vault": vault, "path": "Relayed.md", "content": "# Relayed\n\nbody\n"}),
        ))
        .await;
    assert_eq!(status, 200, "{made}");
    let note_id = structured(&made)["note"]["id"]
        .as_str()
        .unwrap_or_else(|| panic!("{made}"))
        .to_string();

    let calls = [
        rpc(
            "initialize",
            serde_json::json!({
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "relay-test", "version": "0"},
            }),
        ),
        rpc("tools/list", serde_json::json!({})),
        tool("get_vault", serde_json::json!({"vault": vault})),
        tool(
            "get_note",
            serde_json::json!({"vault": vault, "note_id": note_id}),
        ),
    ];
    for call in calls {
        let relayed = t.relayed(&call).await;
        let direct = t.lan(&call).await;
        assert_eq!(relayed.0, 200, "{}", relayed.1);
        assert_eq!(relayed, direct, "{}", String::from_utf8_lossy(&call));
    }

    t.tunnels.shutdown().await;
}

#[tokio::test]
async fn a_relayed_mcp_write_merges_against_a_concurrent_lan_edit() {
    let mut t = McpTunnel::start("storm-relay-mcp-merge").await;
    set_mcp(&t.state, true, true);
    let vault = mcp_vault(&t.router, &t.bearer, "Notes").await;
    let original = "# Shared\n\nfirst line\n\nmiddle\n\nlast line\n";

    let (_, made) = t
        .lan(&tool(
            "create_note",
            serde_json::json!({"vault": vault, "path": "Shared.md", "content": original}),
        ))
        .await;
    let note = structured(&made)["note"].clone();
    let (_, read) = t
        .lan(&tool(
            "get_note",
            serde_json::json!({"vault": vault, "note_id": note["id"]}),
        ))
        .await;
    let base = structured(&read)["version"]
        .as_i64()
        .unwrap_or_else(|| panic!("{read}"));
    let original = structured(&read)["content"].as_str().unwrap().to_string();

    // The phone's edit lands first, on the LAN…
    let (_, phone) = t
        .lan(&tool(
            "update_note",
            serde_json::json!({"vault": vault, "note_id": note["id"], "base_version": base,
                "content": original.replace("first line", "first line, from the phone")}),
        ))
        .await;
    assert_eq!(structured(&phone)["merged"], false, "{phone}");

    // …and the agent's, written against the same base, arrives over the relay.
    let (status, agent) = t
        .relayed(&tool(
            "update_note",
            serde_json::json!({"vault": vault, "note_id": note["id"], "base_version": base,
                "content": original.replace("last line", "last line, from the agent")}),
        ))
        .await;
    assert_eq!(status, 200, "{agent}");
    let result = structured(&agent);
    assert_eq!(result["merged"], true, "{agent}");
    assert_eq!(result["conflict"], false, "{agent}");
    let content = result["content"].as_str().unwrap();
    assert!(content.contains("from the phone"), "{content}");
    assert!(content.contains("from the agent"), "{content}");

    t.tunnels.shutdown().await;
}

#[tokio::test]
async fn a_tool_error_over_the_tunnel_is_still_a_tool_error() {
    let mut t = McpTunnel::start("storm-relay-mcp-tool-error").await;
    set_mcp(&t.state, true, false);
    let vault = mcp_vault(&t.router, &t.bearer, "Notes").await;

    let missing = tool(
        "get_note",
        serde_json::json!({"vault": vault, "note_id": "no-such-note"}),
    );
    let relayed = t.relayed(&missing).await;
    assert_eq!(relayed.0, 200, "a 4xx inside a tool is not an HTTP failure");
    assert_eq!(relayed.1["result"]["isError"], true, "{}", relayed.1);
    assert_eq!(relayed, t.lan(&missing).await);

    t.tunnels.shutdown().await;
}

#[tokio::test]
async fn the_mcp_switches_gate_a_relayed_call_as_they_gate_a_lan_one() {
    let mut t = McpTunnel::start("storm-relay-mcp-switches").await;
    let vault = mcp_vault(&t.router, &t.bearer, "Notes").await;
    let list = rpc("tools/list", serde_json::json!({}));
    let write = tool(
        "create_note",
        serde_json::json!({"vault": vault, "path": "Nope.md", "content": "x"}),
    );

    set_mcp(&t.state, false, false);
    let relayed = t.relayed(&list).await;
    assert_eq!(relayed.0, 404, "{}", relayed.1);
    assert_eq!(relayed, t.lan(&list).await);

    set_mcp(&t.state, true, false);
    let relayed = t.relayed(&list).await;
    let names: Vec<&str> = relayed.1["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    for write_tool in crate::mcp::WRITE_TOOLS {
        assert!(!names.contains(write_tool), "{write_tool} listed read-only");
    }
    assert_eq!(relayed, t.lan(&list).await);

    let relayed = t.relayed(&write).await;
    assert_eq!(relayed, t.lan(&write).await);
    assert!(
        relayed.1.get("error").is_some() || relayed.1["result"]["isError"] == true,
        "a write must be refused while MCP is read-only: {}",
        relayed.1
    );
    let (_, described) = t
        .lan(&tool("get_vault", serde_json::json!({"vault": vault})))
        .await;
    assert_eq!(
        structured(&described)["note_count"],
        0,
        "nothing was written: {described}"
    );

    t.tunnels.shutdown().await;
}

#[tokio::test]
async fn an_mcp_key_works_over_the_tunnel_on_mcp_and_nowhere_else() {
    let mut t = McpTunnel::start("storm-relay-mcp-key").await;
    set_mcp(&t.state, true, false);

    let key = {
        use tower::ServiceExt;
        let response = t
            .router
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/v1/keys")
                    .header("content-type", "application/json")
                    .header("authorization", &t.bearer)
                    .body(axum::body::Body::from(r#"{"name":"relayed agent"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
            .await
            .unwrap();
        let created: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        format!("Bearer {}", created["secret"].as_str().unwrap())
    };
    assert!(key.starts_with("Bearer stk_"), "{key}");

    let (status, listed) = t
        .relayed_as(&rpc("tools/list", serde_json::json!({})), &key)
        .await;
    assert_eq!(status, 200, "{listed}");
    assert!(listed["result"]["tools"].as_array().is_some(), "{listed}");

    let id = t.next_stream;
    let rest = t
        .relay
        .request(
            id,
            TunnelRequest::get("/v1/vaults").header("authorization", &key),
        )
        .await;
    assert_eq!(
        rest.status, 401,
        "an MCP key never reaches REST, relayed or not"
    );

    t.tunnels.shutdown().await;
}

#[tokio::test]
async fn a_relayed_mcp_call_passes_rebinding_protection_on_a_concrete_bind() {
    // The `Host` rewrite (dispatch.rs) end to end: with the server bound to a
    // real address, the relay's hostname would be refused by rmcp.
    let dir = tempdir::TempDir::new("storm-relay-mcp-host").unwrap();
    let (router, identity, state) = crate::api::tests::test_router_with_mcp_hosts(
        dir.path(),
        crate::mcp::allowed_hosts("127.0.0.1", 8484),
    );
    let mut t = McpTunnel::over(dir, router, identity, state).await;
    set_mcp(&t.state, true, false);
    let list = rpc("tools/list", serde_json::json!({}));

    let bearer = t.bearer.clone();
    let (direct_status, _) = lan(&t.router, &list, &bearer, "relay.example.com").await;
    assert_ne!(
        direct_status, 200,
        "the protection is live for a foreign Host"
    );

    let id = t.next_stream;
    let relayed = t
        .relay
        .request(
            id,
            TunnelRequest::mcp(&list, &bearer).header("host", "relay.example.com"),
        )
        .await;
    assert_eq!(
        relayed.status,
        200,
        "{}",
        String::from_utf8_lossy(&relayed.body)
    );

    t.tunnels.shutdown().await;
}

#[tokio::test]
async fn a_relayed_write_whose_stream_closes_before_its_body_ends_writes_nothing() {
    let mut t = McpTunnel::start("storm-relay-mcp-cut").await;
    set_mcp(&t.state, true, true);
    let vault = mcp_vault(&t.router, &t.bearer, "Notes").await;
    let call = tool(
        "create_note",
        serde_json::json!({"vault": vault, "path": "Half.md", "content": "never whole"}),
    );

    t.relay
        .send(serde_json::json!({"v":1,"type":"STREAM_OPEN","stream_id":7}));
    assert_eq!(t.relay.next_control().await["type"], "STREAM_ACK");
    t.relay.send(serde_json::json!({
        "v":1,"type":"HTTP_REQUEST_HEAD","stream_id":7,"method":"POST","path":"/mcp",
        "headers": {"content-type":"application/json",
            "accept":"application/json, text/event-stream","authorization": t.bearer},
    }));
    t.relay
        .to_server
        .send(Message::binary(proto::encode_body_frame(
            proto::FRAME_REQUEST_BODY,
            7,
            &call[..call.len() / 2],
        )))
        .unwrap();
    t.relay
        .send(serde_json::json!({"v":1,"type":"CLOSE","stream_id":7}));

    tokio::time::sleep(Duration::from_millis(300)).await;
    let (_, listed) = t
        .lan(&tool(
            "search",
            serde_json::json!({"vault": vault, "query": "never whole"}),
        ))
        .await;
    assert!(
        !listed.to_string().contains("Half.md"),
        "a request cut off mid-body must not run: {listed}"
    );

    t.tunnels.shutdown().await;
}

/// A route that holds its body unread until `release`, then records how many
/// bytes and which digest it got.
fn held_upload_router(
    release: Arc<tokio::sync::Notify>,
    served: Arc<std::sync::Mutex<Option<(usize, blake3::Hash)>>>,
) -> Router {
    Router::new().route(
        "/upload",
        axum::routing::post(move |body: axum::body::Body| async move {
            release.notified().await;
            if let Ok(bytes) = axum::body::to_bytes(body, usize::MAX).await {
                *served.lock().unwrap() = Some((bytes.len(), blake3::hash(&bytes)));
            }
            "stored"
        }),
    )
}

/// Opens stream 3 as `POST /upload` and sends `frames` request-body frames of
/// `frame` bytes back to back, then the end of body — a client at line rate.
async fn upload_at_line_rate(relay: &mut FakeRelay, frames: usize, frame: &[u8]) {
    relay.send(serde_json::json!({"v":1,"type":"STREAM_OPEN","stream_id":3}));
    assert_eq!(relay.next_control().await["type"], "STREAM_ACK");
    relay.send(serde_json::json!({
        "v":1,"type":"HTTP_REQUEST_HEAD","stream_id":3,"method":"POST","path":"/upload","headers":{},
    }));
    for _ in 0..frames {
        relay
            .to_server
            .send(Message::binary(proto::encode_body_frame(
                proto::FRAME_REQUEST_BODY,
                3,
                frame,
            )))
            .unwrap();
    }
    relay
        .to_server
        .send(Message::binary(proto::encode_body_frame(
            proto::FRAME_REQUEST_BODY,
            3,
            b"",
        )))
        .unwrap();
}

#[tokio::test]
async fn a_large_upload_at_line_rate_reaches_a_slow_handler_whole() {
    // 20 MiB as the Dart client sends it, 80 frames of 256 KiB, to a handler
    // that has not started reading. Decision 95 bounded the buffer at 32
    // chunks and shed this ordinary upload; decision 102 bounds it in bytes.
    let dir = tempdir::TempDir::new("storm-relay-body-line-rate").unwrap();
    let identity = test_identity(dir.path());
    let release = Arc::new(tokio::sync::Notify::new());
    let served = Arc::new(std::sync::Mutex::new(None));
    let router = held_upload_router(release.clone(), served.clone());
    let mut relay = FakeRelay::start(Behaviour::Normal).await;
    let (_registered, tunnels) = start_tunnel(&relay.url, router, identity);
    let _ = relay.await_registration().await;

    let frame: Vec<u8> = (0..256 * 1024).map(|i| (i % 251) as u8).collect();
    upload_at_line_rate(&mut relay, 80, &frame).await;
    // Every frame is buffered before the handler reads a byte.
    tokio::time::sleep(Duration::from_millis(300)).await;
    release.notify_waiters();

    let head = relay.next_control().await;
    assert_eq!(head["type"], "HTTP_RESPONSE_HEAD", "{head}");
    assert_eq!(head["status"], 200);
    let expected = blake3::hash(&frame.repeat(80));
    assert_eq!(*served.lock().unwrap(), Some((80 * frame.len(), expected)));

    tunnels.shutdown().await;
}

#[tokio::test]
async fn a_body_over_its_buffer_budget_is_closed_not_truncated() {
    // Before decision 95 a full body channel was dropped, which the handler
    // read as end-of-body: it served the prefix as the whole request.
    let dir = tempdir::TempDir::new("storm-relay-body-shed").unwrap();
    let identity = test_identity(dir.path());
    let release = Arc::new(tokio::sync::Notify::new());
    let served = Arc::new(std::sync::Mutex::new(None));
    let router = held_upload_router(release.clone(), served.clone());
    let mut relay = FakeRelay::start(Behaviour::Normal).await;
    let (_registered, tunnels) = start_tunnel(&relay.url, router, identity);
    let _ = relay.await_registration().await;

    let frame = vec![b'x'; 1024 * 1024];
    let frames = super::client::STREAM_BODY_BUDGET / frame.len() + 1;
    upload_at_line_rate(&mut relay, frames, &frame).await;

    let closed = relay.next_control().await;
    assert_eq!(closed["type"], "CLOSE", "{closed}");
    assert_eq!(closed["stream_id"], 3);
    release.notify_waiters();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        *served.lock().unwrap(),
        None,
        "the handler must never see a truncated body as a whole one"
    );

    tunnels.shutdown().await;
}

#[tokio::test]
async fn a_response_leaves_the_server_chunk_by_chunk() {
    // A long tool call or the change feed must reach the client as it is
    // produced; the second chunk exists only after the first has arrived.
    let dir = tempdir::TempDir::new("storm-relay-unbuffered").unwrap();
    let identity = test_identity(dir.path());
    let gate = Arc::new(tokio::sync::Notify::new());
    let router = Router::new().route(
        "/slow",
        axum::routing::get({
            let gate = gate.clone();
            move || async move {
                let stream = futures_util::stream::unfold(0u8, move |step| {
                    let gate = gate.clone();
                    async move {
                        match step {
                            0 => Some((
                                Ok::<_, std::io::Error>(axum::body::Bytes::from_static(b"first")),
                                1,
                            )),
                            1 => {
                                gate.notified().await;
                                Some((Ok(axum::body::Bytes::from_static(b"second")), 2))
                            }
                            _ => None,
                        }
                    }
                });
                axum::body::Body::from_stream(stream)
            }
        }),
    );
    let mut relay = FakeRelay::start(Behaviour::Normal).await;
    let (_registered, tunnels) = start_tunnel(&relay.url, router, identity);
    let _ = relay.await_registration().await;

    relay.send(serde_json::json!({"v":1,"type":"STREAM_OPEN","stream_id":5}));
    assert_eq!(relay.next_control().await["type"], "STREAM_ACK");
    relay.send(serde_json::json!({
        "v":1,"type":"HTTP_REQUEST_HEAD","stream_id":5,"method":"GET","path":"/slow","headers":{},
    }));
    assert_eq!(relay.next_control().await["type"], "HTTP_RESPONSE_HEAD");
    match relay.next_event().await {
        Event::Binary(frame) => {
            let (_, _, payload) = proto::decode_body_frame(&frame).unwrap();
            assert_eq!(payload, b"first");
        }
        other => panic!("the first chunk must arrive before the second exists: {other:?}"),
    }
    gate.notify_one();
    match relay.next_event().await {
        Event::Binary(frame) => {
            let (_, _, payload) = proto::decode_body_frame(&frame).unwrap();
            assert_eq!(payload, b"second");
        }
        other => panic!("{other:?}"),
    }

    tunnels.shutdown().await;
}

// ---------------------------------------------------------------------------
// Bodies larger than a WebSocket frame (decision 98)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_response_larger_than_a_websocket_frame_crosses_in_bounded_frames() {
    // A 17 MiB attachment used to leave as one 0x02 frame. The relay refuses a
    // frame over 16 MiB by closing the connection, and that connection is the
    // server's trunk: every relayed client lost the server, not just this one.
    let dir = tempdir::TempDir::new("storm-relay-big-response").unwrap();
    let identity = test_identity(dir.path());
    const SIZE: usize = 20 * 1024 * 1024;
    let router = Router::new().route(
        "/big",
        axum::routing::get(|| async { axum::body::Bytes::from(vec![7u8; SIZE]) }),
    );
    let mut relay = FakeRelay::start(Behaviour::Normal).await;
    let (_registered, tunnels) = start_tunnel(&relay.url, router, identity);
    let _ = relay.await_registration().await;

    relay.send(serde_json::json!({"v":1,"type":"STREAM_OPEN","stream_id":9}));
    assert_eq!(relay.next_control().await["type"], "STREAM_ACK");
    relay.send(serde_json::json!({
        "v":1,"type":"HTTP_REQUEST_HEAD","stream_id":9,"method":"GET","path":"/big","headers":{},
    }));
    assert_eq!(relay.next_control().await["type"], "HTTP_RESPONSE_HEAD");

    let (mut received, mut largest) = (0usize, 0usize);
    loop {
        match relay.next_event().await {
            Event::Binary(frame) => {
                let (_, _, payload) = proto::decode_body_frame(&frame).unwrap();
                received += payload.len();
                largest = largest.max(payload.len());
            }
            Event::Text(value) if value["type"] == "PING" => continue,
            Event::Text(value) if value["type"] == "CLOSE" => break,
            other => panic!("the trunk must survive a large response: {other:?}"),
        }
    }
    assert_eq!(received, SIZE);
    assert!(
        largest <= proto::MAX_BODY_FRAME_PAYLOAD,
        "a frame carried {largest} bytes"
    );

    tunnels.shutdown().await;
}

#[tokio::test]
async fn a_chunked_upload_sent_back_to_back_lands_whole() {
    // Decision 95 closes a stream whose body outruns its handler by 32 chunks.
    // A sender splitting at the recommended 256 KiB (§5.2) sends a 15 MiB
    // attachment as 60 frames with nothing in between, so the real attachment
    // route must keep up with that or the limit is a defect, not a guard.
    let dir = tempdir::TempDir::new("storm-relay-chunked-upload").unwrap();
    let (router, identity, state) = crate::api::tests::test_router_with_state(dir.path());
    let user = crate::api::tests::seed_owner(&state).await;
    let bearer = format!(
        "Bearer {}",
        crate::api::tests::session_token(&state, &user).await
    );
    let vault = mcp_vault(&router, &bearer, "uploads").await;
    let mut relay = FakeRelay::start(Behaviour::Normal).await;
    let (_registered, tunnels) = start_tunnel(&relay.url, router, identity);
    let _ = relay.await_registration().await;
    let path = format!("/v1/vaults/{vault}/attachments/big.bin");

    const SIZE: usize = 15 * 1024 * 1024;
    let body: Vec<u8> = (0..SIZE).map(|i| (i * 31 + 7) as u8).collect();
    relay.send(serde_json::json!({"v":1,"type":"STREAM_OPEN","stream_id":2}));
    assert_eq!(relay.next_control().await["type"], "STREAM_ACK");
    relay.send(serde_json::json!({
        "v":1,"type":"HTTP_REQUEST_HEAD","stream_id":2,"method":"PUT","path":path,
        "headers":{"authorization":bearer},
    }));
    for piece in body.chunks(proto::MAX_BODY_FRAME_PAYLOAD) {
        relay
            .to_server
            .send(Message::binary(proto::encode_body_frame(
                proto::FRAME_REQUEST_BODY,
                2,
                piece,
            )))
            .unwrap();
    }
    relay
        .to_server
        .send(Message::binary(proto::encode_body_frame(
            proto::FRAME_REQUEST_BODY,
            2,
            b"",
        )))
        .unwrap();
    let head = relay.next_control().await;
    assert_eq!(head["type"], "HTTP_RESPONSE_HEAD", "{head}");
    assert_eq!(head["status"], 200, "{head}");
    loop {
        match relay.next_event().await {
            Event::Text(value) if value["type"] == "CLOSE" => break,
            Event::Text(value) if value["type"] == "PING" => continue,
            Event::Binary(_) => continue,
            other => panic!("{other:?}"),
        }
    }

    let fetched = relay
        .request(
            3,
            TunnelRequest::get(&path).header("authorization", &bearer),
        )
        .await;
    assert_eq!(fetched.status, 200);
    assert!(
        fetched.body == body,
        "{} of {SIZE} bytes came back",
        fetched.body.len()
    );

    tunnels.shutdown().await;
}

// A response that fails mid-body (decision 101)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_response_body_that_fails_midway_is_aborted_not_closed() {
    let dir = tempdir::TempDir::new("storm-relay-abort").unwrap();
    let (router, identity, _state) = crate::api::tests::test_router_with_state(dir.path());
    let broken = Router::new().route(
        "/broken",
        axum::routing::get(|| async {
            let body = futures_util::stream::iter([
                Ok::<_, std::io::Error>(Bytes::from_static(b"first")),
                Err(std::io::Error::other("disk went away")),
            ]);
            axum::body::Body::from_stream(body)
        }),
    );
    let mut relay = FakeRelay::start(Behaviour::Normal).await;
    let (registered, tunnels) = start_tunnel(&relay.url, router.merge(broken), identity);
    let _ = relay.await_registration().await;
    await_advertised(&registered, 1).await;

    relay.send(serde_json::json!({"v":1,"type":"STREAM_OPEN","stream_id":1}));
    assert_eq!(relay.next_control().await["type"], "STREAM_ACK");
    relay.send(serde_json::json!({
        "v":1,"type":"HTTP_REQUEST_HEAD","stream_id":1,
        "method":"GET","path":"/broken","headers":{}
    }));
    let head = relay.next_control().await;
    assert_eq!(head["status"], 200, "{head}");

    let mut body = Vec::new();
    let end = loop {
        match relay.next_event().await {
            Event::Binary(frame) => {
                let (_, id, payload) = proto::decode_body_frame(&frame).unwrap();
                assert_eq!(id, 1);
                body.extend_from_slice(payload);
            }
            Event::Text(value) if value["type"] == "PING" => continue,
            Event::Text(value) => break value,
            other => panic!("expected the body or its end, got {other:?}"),
        }
    };
    assert_eq!(body, b"first");
    assert_eq!(end["type"], "CLOSE", "{end}");
    assert_eq!(end["stream_id"], 1, "{end}");
    assert_eq!(
        end["aborted"], true,
        "a plain CLOSE tells the client a truncated body is complete: {end}"
    );

    tunnels.shutdown().await;
}

// ---------------------------------------------------------------------------
// §9: failure handling (decision 94)
// ---------------------------------------------------------------------------

/// Signals when it is dropped, so a test can see a handler being cancelled.
struct DropSignal(Option<tokio::sync::oneshot::Sender<()>>);

impl Drop for DropSignal {
    fn drop(&mut self) {
        if let Some(tx) = self.0.take() {
            let _ = tx.send(());
        }
    }
}

/// `/slow` answers a head and one chunk, then never finishes: the shape of a
/// long download or the change feed. `dropped` fires when the response body
/// is dropped, which is what cancelling the dispatch task does.
fn slow_router() -> (Router, tokio::sync::oneshot::Receiver<()>) {
    let (tx, dropped) = tokio::sync::oneshot::channel();
    let tx = Arc::new(std::sync::Mutex::new(Some(tx)));
    let router = Router::new().route(
        "/slow",
        axum::routing::get(move || {
            let signal = DropSignal(tx.lock().unwrap().take());
            async move {
                let body = futures_util::stream::once(async {
                    Ok::<_, std::io::Error>(Bytes::from_static(b"first"))
                })
                .chain(futures_util::stream::pending())
                .map(move |chunk| {
                    let _keep = &signal;
                    chunk
                });
                axum::body::Body::from_stream(body)
            }
        }),
    );
    (router, dropped)
}

/// Opens `/slow` on `stream_id` and waits for its first chunk.
async fn start_slow_response(relay: &mut FakeRelay, stream_id: u32) {
    relay.send(serde_json::json!({"v":1,"type":"STREAM_OPEN","stream_id":stream_id}));
    assert_eq!(relay.next_control().await["type"], "STREAM_ACK");
    relay.send(serde_json::json!({
        "v":1,"type":"HTTP_REQUEST_HEAD","stream_id":stream_id,
        "method":"GET","path":"/slow","headers":{}
    }));
    let head = relay.next_control().await;
    assert_eq!(head["type"], "HTTP_RESPONSE_HEAD", "{head}");
    loop {
        match relay.next_event().await {
            Event::Binary(frame) => {
                let (_, id, payload) = proto::decode_body_frame(&frame).unwrap();
                assert_eq!((id, payload), (stream_id, &b"first"[..]));
                return;
            }
            Event::Text(value) if value["type"] == "PING" => continue,
            other => panic!("expected the first chunk, got {other:?}"),
        }
    }
}

async fn serve_lan(router: Router) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(
            listener,
            router.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });
    addr
}

async fn await_attempts(relay: &FakeRelay, count: usize, within: Duration) {
    let deadline = tokio::time::Instant::now() + within;
    while relay.attempts.load(std::sync::atomic::Ordering::SeqCst) < count {
        assert!(
            tokio::time::Instant::now() < deadline,
            "expected {count} connection attempts within {within:?}, saw {}",
            relay.attempts.load(std::sync::atomic::Ordering::SeqCst)
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn a_relay_vanishing_mid_response_cancels_the_handler_and_the_trunk_comes_back() {
    // The load-bearing R6 case the hung and silent relays did not cover: the
    // relay goes away while a relayed response is half sent.
    let dir = tempdir::TempDir::new("storm-relay-vanish").unwrap();
    let (router, identity, _state) = crate::api::tests::test_router_with_state(dir.path());
    let (slow, dropped) = slow_router();
    let router = router.merge(slow);
    let lan = serve_lan(router.clone()).await;

    let mut relay = FakeRelay::start(Behaviour::Normal).await;
    let (registered, tunnels) = start_tunnel(&relay.url, router, identity);
    let _ = relay.await_registration().await;
    await_advertised(&registered, 1).await;

    start_slow_response(&mut relay, 1).await;
    relay.vanish();

    tokio::time::timeout(Duration::from_secs(5), dropped)
        .await
        .expect("an in-flight relayed handler must be cancelled when its trunk dies")
        .unwrap();
    // Not advertised while there is no trunk, or clients race a dead path.
    await_advertised(&registered, 0).await;

    let elapsed = lan_health(lan).await;
    assert!(elapsed < Duration::from_secs(2), "LAN took {elapsed:?}");

    // Served-then-closed resets the backoff, so the redial comes after 1 s.
    loop {
        match relay.next_event().await {
            Event::Registered { .. } => break,
            Event::Disconnected | Event::Text(_) => continue,
            other => panic!("expected a redial, got {other:?}"),
        }
    }
    await_advertised(&registered, 1).await;
    let response = relay.request(2, TunnelRequest::get("/v1/health")).await;
    assert_eq!(response.status, 200);

    tunnels.shutdown().await;
}

#[tokio::test]
async fn a_relay_closing_a_stream_mid_response_cancels_only_that_handler() {
    // What the relay sends when the *client* hop dies mid-response (§5.4).
    let dir = tempdir::TempDir::new("storm-relay-close-mid").unwrap();
    let (router, identity, _state) = crate::api::tests::test_router_with_state(dir.path());
    let (slow, dropped) = slow_router();
    let router = router.merge(slow);

    let mut relay = FakeRelay::start(Behaviour::Normal).await;
    let (registered, tunnels) = start_tunnel(&relay.url, router, identity);
    let _ = relay.await_registration().await;
    await_advertised(&registered, 1).await;

    start_slow_response(&mut relay, 7).await;
    relay.send(serde_json::json!({"v":1,"type":"CLOSE","stream_id":7}));

    tokio::time::timeout(Duration::from_secs(5), dropped)
        .await
        .expect("CLOSE must cancel the handler, not leave it streaming into nothing")
        .unwrap();

    // The trunk and its other streams are untouched.
    let response = relay.request(8, TunnelRequest::get("/v1/health")).await;
    assert_eq!(response.status, 200);
    assert_eq!(registered.snapshot().len(), 1);

    tunnels.shutdown().await;
}

#[tokio::test]
async fn a_relay_erroring_a_stream_mid_response_cancels_that_handler() {
    // `ERROR{.., stream_id}` toward the server — a relay-side timeout or limit.
    let dir = tempdir::TempDir::new("storm-relay-error-mid").unwrap();
    let (router, identity, _state) = crate::api::tests::test_router_with_state(dir.path());
    let (slow, dropped) = slow_router();
    let router = router.merge(slow);

    let mut relay = FakeRelay::start(Behaviour::Normal).await;
    let (registered, tunnels) = start_tunnel(&relay.url, router, identity);
    let _ = relay.await_registration().await;
    await_advertised(&registered, 1).await;

    start_slow_response(&mut relay, 3).await;
    relay.send(serde_json::json!({
        "v":1,"type":"ERROR","code":"rate_limited","message":"limit","stream_id":3
    }));

    tokio::time::timeout(Duration::from_secs(5), dropped)
        .await
        .expect("a stream-scoped ERROR must cancel the handler")
        .unwrap();
    assert_eq!(registered.snapshot().len(), 1, "only the stream ends");

    tunnels.shutdown().await;
}

#[tokio::test]
async fn a_relay_gone_silent_is_dropped_at_the_heartbeat_deadline_and_redialled() {
    // A sleep, a network change or a NAT timeout leaves the socket open with
    // nobody behind it. Before decision 94 the trunk stayed advertised until
    // TCP gave up, which is many minutes.
    let dir = tempdir::TempDir::new("storm-relay-silent-deadline").unwrap();
    let identity = test_identity(dir.path());
    // A 1 s heartbeat makes the deadline 3 s.
    let relay = FakeRelay::start_with(Behaviour::HangAfterRegister, 1).await;
    let (registered, tunnels) = start_tunnel(&relay.url, echo_router(), identity);

    await_advertised(&registered, 1).await;
    let registered_at = Instant::now();

    let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
    while !registered.is_empty() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "a relay that stopped answering PING is still advertised after {:?}",
            registered_at.elapsed()
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let dropped_after = registered_at.elapsed();
    assert!(
        dropped_after >= Duration::from_millis(2500),
        "dropped after {dropped_after:?}, before three heartbeats had been missed"
    );

    await_attempts(&relay, 2, Duration::from_secs(5)).await;

    tunnels.shutdown().await;
}

#[tokio::test]
async fn a_relay_that_answers_heartbeats_is_kept_past_the_deadline() {
    let dir = tempdir::TempDir::new("storm-relay-alive").unwrap();
    let identity = test_identity(dir.path());
    let mut relay = FakeRelay::start_with(Behaviour::Normal, 1).await;
    let (registered, tunnels) = start_tunnel(&relay.url, echo_router(), identity);
    let _ = relay.await_registration().await;
    await_advertised(&registered, 1).await;

    // Answer every PING for twice the deadline.
    let until = tokio::time::Instant::now() + Duration::from_secs(6);
    while tokio::time::Instant::now() < until {
        if let Ok(Some(Event::Text(value))) =
            tokio::time::timeout(Duration::from_millis(200), relay.events.recv()).await
            && value["type"] == "PING"
        {
            relay.send(serde_json::json!({"v":1,"type":"PONG"}));
        }
    }

    assert_eq!(registered.snapshot().len(), 1, "a live relay must be kept");
    assert_eq!(
        relay.attempts.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "and never redialled"
    );
    tunnels.shutdown().await;
}
