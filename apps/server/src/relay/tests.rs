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
    _task: tokio::task::JoinHandle<()>,
}

impl FakeRelay {
    async fn start(behaviour: Behaviour) -> Self {
        // Ephemeral port: never 8484/8485, and safe to run many at once.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let (events_tx, events) = mpsc::unbounded_channel();
        let (to_server, mut commands) = mpsc::unbounded_channel::<Message>();
        let attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));

        let task = tokio::spawn({
            let attempts = attempts.clone();
            async move {
                loop {
                    let Ok((socket, _)) = listener.accept().await else {
                        return;
                    };
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
                            "heartbeat_interval_secs": 15
                        })
                        .to_string(),
                    ))
                    .await
                    .unwrap();
                    let _ = events_tx.send(Event::Registered { server_id, pubkey });

                    if behaviour == Behaviour::HangAfterRegister {
                        // Registered, then dead to the world: never read,
                        // never write, never close. The socket stays open, so
                        // the server has no error to react to.
                        std::future::pending::<()>().await;
                    }

                    // ---- pump ---------------------------------------------
                    loop {
                        tokio::select! {
                            command = commands.recv() => match command {
                                Some(message) => {
                                    if ws.send(message).await.is_err() { break; }
                                }
                                None => break,
                            },
                            incoming = ws.next() => match incoming {
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
            _task: task,
        }
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
