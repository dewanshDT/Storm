//! Client trunks, streams, and the routing between a client and a server
//! trunk, driven over real WebSockets on ephemeral ports.
//!
//! These speak the wire rather than calling the state machine, for the same
//! reason `registration.rs` does: a change that broke the framing but left the
//! functions intact would pass a unit test and fail every real client.
//!
//! The sharpest test in the file is
//! `one_clients_response_never_reaches_another_client`. Everything else here
//! is a failure mode; that one is the correctness trap the whole `stream_id`
//! design exists to close.

use std::sync::Arc;
use std::time::Duration;

use data_encoding::BASE64URL_NOPAD;
use ed25519_dalek::{Signer, SigningKey};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use storm_relay::{Config, Relay};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

const SERVER_ID: &str = "srv_01ARZ3NDEKTSV4RRFFQ69G5FAV";

/// Short enough that no test waits on a real 5 s, long enough that a loaded
/// machine does not trip a timeout that was not the point of the test.
const FAST: Duration = Duration::from_millis(120);

// ---------------------------------------------------------------- key helpers

fn signing_key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

fn pubkey_b64(seed: u8) -> String {
    BASE64URL_NOPAD.encode(signing_key(seed).verifying_key().as_bytes())
}

/// The wire commitment, spelled out rather than imported: a test that reuses
/// the implementation's builder proves only that the builder is self-consistent.
fn sign_relay_auth(seed: u8, server_id: &str, nonce: &str) -> String {
    let message = format!("storm-relay-auth:v1:{server_id}:{nonce}");
    BASE64URL_NOPAD.encode(&signing_key(seed).sign(message.as_bytes()).to_bytes())
}

// -------------------------------------------------------------------- harness

struct Harness {
    addr: std::net::SocketAddr,
    relay: Arc<Relay>,
}

/// Every timeout the routing path can hit, shortened together.
///
/// The knobs exist so this suite never sleeps five real seconds; they are
/// separate fields rather than one scale factor so a test can lengthen exactly
/// the interval it is *not* exercising — `no_stream_ack_within_the_timeout...`
/// needs a short ack timeout and a long `hello_wait`, and the in-flight cap
/// test needs the opposite.
fn config() -> Config {
    let mut config = Config::new("127.0.0.1:0".parse().unwrap(), "wss://relay.example");
    config.hello_wait = FAST;
    config.stream_ack_timeout = FAST;
    config
}

impl Harness {
    async fn start(config: Config) -> Self {
        // 127.0.0.1:0 — never the real 8484/8485, and never a fixed port, so
        // the suite can run in parallel with itself.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let relay = Arc::new(Relay::new(config));
        let serving = relay.clone();
        tokio::spawn(async move {
            let _ = storm_relay::serve(listener, serving).await;
        });
        Self { addr, relay }
    }

    async fn plain() -> Self {
        Self::start(config()).await
    }

    async fn dial(&self, path: &str) -> Conn {
        let (socket, _) = tokio_tungstenite::connect_async(format!("ws://{}{path}", self.addr))
            .await
            .unwrap();
        Conn { socket }
    }

    /// A fully registered server trunk, held open by the caller.
    async fn server(&self) -> Conn {
        self.server_as(SERVER_ID, 1).await
    }

    async fn server_as(&self, server_id: &str, seed: u8) -> Conn {
        let mut conn = self.dial("/register").await;
        conn.send(json!({
            "v": 1, "type": "REGISTER_SERVER",
            "server_id": server_id, "pubkey": pubkey_b64(seed),
        }))
        .await;
        let challenge = conn.recv().await;
        assert_eq!(challenge["type"], "CHALLENGE", "{challenge}");
        let nonce = challenge["nonce"].as_str().unwrap().to_string();
        conn.send(json!({
            "v": 1, "type": "CHALLENGE_RESPONSE",
            "sig": sign_relay_auth(seed, server_id, &nonce),
        }))
        .await;
        let registered = conn.recv().await;
        assert_eq!(registered["type"], "REGISTERED", "{registered}");
        conn
    }

    /// A client trunk that has completed `HELLO`/`READY`.
    async fn client(&self) -> Conn {
        let mut conn = self.dial(&format!("/connect/{SERVER_ID}")).await;
        conn.send(json!({ "v": 1, "type": "HELLO", "server_id": SERVER_ID }))
            .await;
        let ready = conn.recv().await;
        assert_eq!(ready["type"], "READY", "{ready}");
        assert!(ready["client_trunk_id"].as_str().is_some(), "{ready}");
        conn
    }
}

struct Conn {
    socket: WebSocketStream<MaybeTlsStream<TcpStream>>,
}

enum Frame {
    Text(Value),
    Body {
        kind: u8,
        stream_id: u32,
        payload: Vec<u8>,
    },
}

impl Conn {
    async fn send(&mut self, value: Value) {
        self.socket
            .send(Message::Text(value.to_string().into()))
            .await
            .unwrap();
    }

    async fn send_body(&mut self, kind: u8, stream_id: u32, payload: &[u8]) {
        let mut frame = vec![kind];
        frame.extend_from_slice(&stream_id.to_be_bytes());
        frame.extend_from_slice(payload);
        self.socket
            .send(Message::Binary(frame.into()))
            .await
            .unwrap();
    }

    /// The next text frame, skipping body chunks.
    async fn recv(&mut self) -> Value {
        loop {
            if let Frame::Text(value) = self.next_frame().await {
                return value;
            }
        }
    }

    async fn next_frame(&mut self) -> Frame {
        loop {
            match self.socket.next().await.expect("a frame").unwrap() {
                Message::Text(text) => return Frame::Text(serde_json::from_str(&text).unwrap()),
                Message::Binary(bytes) => {
                    let stream_id = u32::from_be_bytes([bytes[1], bytes[2], bytes[3], bytes[4]]);
                    return Frame::Body {
                        kind: bytes[0],
                        stream_id,
                        payload: bytes[5..].to_vec(),
                    };
                }
                Message::Ping(_) | Message::Pong(_) => continue,
                other => panic!("unexpected frame {other:?}"),
            }
        }
    }

    /// Opens a stream and returns its relay-assigned `stream_id`.
    async fn open_stream(&mut self, attempt_id: &str) -> u32 {
        self.send(json!({ "v": 1, "type": "OPEN_STREAM", "attempt_id": attempt_id }))
            .await;
        let ready = self.recv().await;
        assert_eq!(ready["type"], "STREAM_READY", "{ready}");
        assert_eq!(ready["attempt_id"], attempt_id, "{ready}");
        ready["stream_id"].as_u64().unwrap() as u32
    }

    /// Consumes the `STREAM_OPEN` the relay sends server-ward and acks it.
    async fn expect_stream_open_and_ack(&mut self) -> u32 {
        let open = self.recv().await;
        assert_eq!(open["type"], "STREAM_OPEN", "{open}");
        let stream_id = open["stream_id"].as_u64().unwrap() as u32;
        self.send(json!({ "v": 1, "type": "STREAM_ACK", "stream_id": stream_id }))
            .await;
        stream_id
    }

    /// `true` if nothing arrives within `window`. Used to assert an *absence*,
    /// which is the only way to prove a frame did not go to the wrong client.
    async fn is_silent_for(&mut self, window: Duration) -> bool {
        tokio::time::timeout(window, self.next_frame())
            .await
            .is_err()
    }
}

fn assert_error(reply: &Value, code: &str) {
    assert_eq!(reply["type"], "ERROR", "{reply}");
    assert_eq!(reply["v"], 1, "{reply}");
    assert_eq!(reply["code"], code, "{reply}");
}

fn request_head(stream_id: u32, path: &str) -> Value {
    json!({
        "v": 1, "type": "HTTP_REQUEST_HEAD",
        "stream_id": stream_id, "method": "GET", "path": path,
        "headers": { "authorization": "Bearer inside-the-tunnel" },
    })
}

// ------------------------------------------------------------- the happy path

#[tokio::test]
async fn a_request_and_response_travel_client_relay_server_client_with_bodies() {
    let harness = Harness::plain().await;
    let mut server = harness.server().await;
    let mut client = harness.client().await;

    let stream_id = client.open_stream("attempt-1").await;
    assert_eq!(server.expect_stream_open_and_ack().await, stream_id);

    client.send(request_head(stream_id, "/v1/notes")).await;
    client.send_body(0x01, stream_id, b"request body").await;

    let head = server.recv().await;
    assert_eq!(head["type"], "HTTP_REQUEST_HEAD", "{head}");
    assert_eq!(head["stream_id"], stream_id, "{head}");
    assert_eq!(head["method"], "GET", "{head}");
    assert_eq!(head["path"], "/v1/notes", "{head}");
    // The relay does not parse a credential; it moves the header it was given
    // and the origin checks it exactly as on the LAN (R12).
    assert_eq!(
        head["headers"]["authorization"], "Bearer inside-the-tunnel",
        "{head}"
    );

    match server.next_frame().await {
        Frame::Body {
            kind,
            stream_id: id,
            payload,
        } => {
            assert_eq!(kind, 0x01);
            assert_eq!(id, stream_id);
            assert_eq!(payload, b"request body");
        }
        Frame::Text(value) => panic!("expected a body chunk, got {value}"),
    }

    server
        .send(json!({
            "v": 1, "type": "HTTP_RESPONSE_HEAD",
            "stream_id": stream_id, "status": 200,
            "headers": { "content-type": "application/json" },
        }))
        .await;
    server.send_body(0x02, stream_id, b"[]").await;

    let response = client.recv().await;
    assert_eq!(response["type"], "HTTP_RESPONSE_HEAD", "{response}");
    assert_eq!(response["stream_id"], stream_id, "{response}");
    assert_eq!(response["status"], 200, "{response}");
    assert_eq!(
        response["headers"]["content-type"], "application/json",
        "{response}"
    );

    match client.next_frame().await {
        Frame::Body {
            kind,
            stream_id: id,
            payload,
        } => {
            assert_eq!(kind, 0x02);
            assert_eq!(id, stream_id);
            assert_eq!(payload, b"[]");
        }
        Frame::Text(value) => panic!("expected a body chunk, got {value}"),
    }
}

// --------------------------------------------------- the cross-talk trap (§3)

#[tokio::test]
async fn one_clients_response_never_reaches_another_client() {
    // The reason `stream_id` is relay-assigned rather than client-asserted.
    // Many clients share ONE server trunk, so every response the origin sends
    // arrives on a socket that all of them are multiplexed onto; the only thing
    // standing between client B and client A's note contents is the relay's
    // record of who owns which id.
    let harness = Harness::plain().await;
    let mut server = harness.server().await;
    let mut alice = harness.client().await;
    let mut bob = harness.client().await;

    // Concurrent opens on the same server trunk. Both clients chose the same
    // `attempt_id`, which must not collide: it is echoed per trunk and never
    // routed on.
    alice
        .send(json!({ "v": 1, "type": "OPEN_STREAM", "attempt_id": "a1" }))
        .await;
    bob.send(json!({ "v": 1, "type": "OPEN_STREAM", "attempt_id": "a1" }))
        .await;

    let alice_ready = alice.recv().await;
    let bob_ready = bob.recv().await;
    let alice_stream = alice_ready["stream_id"].as_u64().unwrap() as u32;
    let bob_stream = bob_ready["stream_id"].as_u64().unwrap() as u32;
    assert_ne!(
        alice_stream, bob_stream,
        "two clients were handed the same stream_id"
    );

    // Two STREAM_OPENs arrive server-ward, in some order; ack both.
    for _ in 0..2 {
        server.expect_stream_open_and_ack().await;
    }

    // The origin answers Alice, and only Alice.
    server
        .send(json!({
            "v": 1, "type": "HTTP_RESPONSE_HEAD",
            "stream_id": alice_stream, "status": 200, "headers": {},
        }))
        .await;
    server
        .send_body(0x02, alice_stream, b"alice's private note")
        .await;

    let head = alice.recv().await;
    assert_eq!(head["type"], "HTTP_RESPONSE_HEAD", "{head}");
    assert_eq!(head["stream_id"], alice_stream, "{head}");
    match alice.next_frame().await {
        Frame::Body { payload, .. } => assert_eq!(payload, b"alice's private note"),
        Frame::Text(value) => panic!("expected Alice's body, got {value}"),
    }

    // Bob's socket must have seen nothing at all — not the head, not the body,
    // not an error naming Alice's stream.
    assert!(
        bob.is_silent_for(FAST).await,
        "a response crossed to the wrong client trunk"
    );
}

#[tokio::test]
async fn a_stream_id_a_client_asserts_is_not_honoured() {
    // A client cannot pick a `stream_id`: `OPEN_STREAM` carries none. The only
    // way to assert one is to name it on a later frame, so that is what is
    // tested — with the id of a *live stream belonging to someone else*, which
    // is the case that matters.
    let harness = Harness::plain().await;
    let mut server = harness.server().await;
    let mut alice = harness.client().await;
    let mut bob = harness.client().await;

    let alice_stream = alice.open_stream("a1").await;
    server.expect_stream_open_and_ack().await;

    // Bob names Alice's stream on a request head.
    bob.send(request_head(alice_stream, "/v1/notes")).await;
    let reply = bob.recv().await;
    // Indistinguishable from naming a dead stream, on purpose: otherwise this
    // is an oracle for which ids are in use by other clients.
    assert_error(&reply, "stream_closed");
    assert_eq!(reply["stream_id"], alice_stream, "{reply}");

    // And on a body chunk.
    bob.send_body(0x01, alice_stream, b"injected").await;
    assert_error(&bob.recv().await, "stream_closed");

    // Nothing Bob sent reached the origin.
    assert!(
        server.is_silent_for(FAST).await,
        "a client asserted another client's stream_id and the frame was routed"
    );
    // And Alice's stream is untouched.
    assert!(alice.is_silent_for(FAST).await);
}

// ---------------------------------------------------------- relay_peer_ip

#[tokio::test]
async fn the_relay_sets_relay_peer_ip_from_the_socket() {
    let harness = Harness::plain().await;
    let mut server = harness.server().await;
    let mut client = harness.client().await;

    let stream_id = client.open_stream("a1").await;
    server.expect_stream_open_and_ack().await;
    client.send(request_head(stream_id, "/v1/notes")).await;

    let head = server.recv().await;
    assert_eq!(head["type"], "HTTP_REQUEST_HEAD", "{head}");
    // Derived from the accepted socket, never from a header. The suite dials
    // over loopback, so the client's real address is 127.0.0.1 — and the field
    // is the IP alone, not `ip:port`, because it is what the origin buckets
    // rate limits by.
    assert_eq!(head["relay_peer_ip"], "127.0.0.1", "{head}");
}

#[tokio::test]
async fn a_client_that_sends_relay_peer_ip_is_refused() {
    // Half two of the §5.2 rule. Without it, the overwrite in half one is only
    // as good as the relay's diligence on every forwarding path that will ever
    // be added; with it, a `relay_peer_ip` on a server-ward head is
    // structurally one the relay wrote.
    let harness = Harness::plain().await;
    let mut server = harness.server().await;
    let mut client = harness.client().await;

    let stream_id = client.open_stream("a1").await;
    server.expect_stream_open_and_ack().await;

    let mut head = request_head(stream_id, "/v1/notes");
    head["relay_peer_ip"] = json!("10.0.0.7");
    client.send(head).await;

    let reply = client.recv().await;
    assert_error(&reply, "protocol_error");

    // The refusal closes the connection (§6), and that teardown frees the
    // client's stream on the server side — so the origin does hear something.
    // What it must never hear is the head itself, forged address and all.
    let next = server.recv().await;
    assert_eq!(next["type"], "CLOSE", "{next}");
    assert_eq!(next["stream_id"], stream_id, "{next}");
    assert!(
        server.is_silent_for(FAST).await,
        "a client-set relay_peer_ip was forwarded"
    );
}

#[tokio::test]
async fn even_a_null_relay_peer_ip_from_a_client_is_refused() {
    // The check is on the field's presence, not on its value: a `null` that
    // parsed as "absent" would be a way to probe whether the check exists at
    // all, and a later refactor could easily let it through.
    let harness = Harness::plain().await;
    let mut server = harness.server().await;
    let mut client = harness.client().await;

    let stream_id = client.open_stream("a1").await;
    server.expect_stream_open_and_ack().await;

    let mut head = request_head(stream_id, "/v1/notes");
    head["relay_peer_ip"] = Value::Null;
    client.send(head).await;
    assert_error(&client.recv().await, "protocol_error");
}

// -------------------------------------------------------------- failure modes

#[tokio::test]
async fn hello_for_an_unregistered_server_id_is_server_unreachable() {
    let harness = Harness::plain().await;
    // No server trunk at all.
    let mut client = harness.dial(&format!("/connect/{SERVER_ID}")).await;

    let started = tokio::time::Instant::now();
    client
        .send(json!({ "v": 1, "type": "HELLO", "server_id": SERVER_ID }))
        .await;
    let reply = client.recv().await;
    assert_error(&reply, "server_unreachable");
    // It held the connection rather than refusing instantly — the wait is what
    // covers a server mid-restart (§5).
    assert!(started.elapsed() >= FAST, "HELLO did not wait");
}

#[tokio::test]
async fn a_server_that_registers_during_the_wait_still_gets_the_client() {
    // The point of the wait, asserted rather than assumed: a client that
    // arrives a moment before its server finishes reconnecting is served, not
    // refused.
    let mut config = config();
    config.hello_wait = Duration::from_secs(2);
    let harness = Harness::start(config).await;

    let mut client = harness.dial(&format!("/connect/{SERVER_ID}")).await;
    client
        .send(json!({ "v": 1, "type": "HELLO", "server_id": SERVER_ID }))
        .await;

    tokio::time::sleep(Duration::from_millis(100)).await;
    let _server = harness.server().await;

    let ready = client.recv().await;
    assert_eq!(ready["type"], "READY", "{ready}");
}

#[tokio::test]
async fn no_stream_ack_within_the_timeout_is_server_timeout() {
    let harness = Harness::plain().await;
    // Registered, and deliberately never acking.
    let mut server = harness.server().await;
    let mut client = harness.client().await;

    let stream_id = client.open_stream("a1").await;
    let open = server.recv().await;
    assert_eq!(open["type"], "STREAM_OPEN", "{open}");

    let reply = client.recv().await;
    assert_error(&reply, "server_timeout");
    assert_eq!(reply["stream_id"], stream_id, "{reply}");

    // The relay frees the stream on the server side too, rather than leaving
    // the origin holding one that can no longer be answered.
    let closed = server.recv().await;
    assert_eq!(closed["type"], "CLOSE", "{closed}");
    assert_eq!(closed["stream_id"], stream_id, "{closed}");
}

#[tokio::test]
async fn an_acked_stream_that_stays_silent_is_never_timed_out() {
    // The 5 s is time to FIRST BYTE, not time to completion (§5.2). The change
    // feed is an ordinary streamed response that can legitimately sit open for
    // hours producing nothing, so a timer that kept running after the ack would
    // break it — and would do so only in production, where feeds are quiet.
    let harness = Harness::plain().await;
    let mut server = harness.server().await;
    let mut client = harness.client().await;

    let stream_id = client.open_stream("feed").await;
    server.expect_stream_open_and_ack().await;
    client.send(request_head(stream_id, "/v1/stream")).await;
    assert_eq!(server.recv().await["type"], "HTTP_REQUEST_HEAD");

    // Well past the ack timeout, with no response head and no body at all.
    tokio::time::sleep(FAST * 4).await;
    assert!(
        client.is_silent_for(FAST).await,
        "a quiet but healthy stream was torn down"
    );

    // Still live: the origin can answer whenever it likes.
    server
        .send(json!({
            "v": 1, "type": "HTTP_RESPONSE_HEAD",
            "stream_id": stream_id, "status": 200,
            "headers": { "content-type": "text/event-stream" },
        }))
        .await;
    assert_eq!(client.recv().await["type"], "HTTP_RESPONSE_HEAD");
}

#[tokio::test]
async fn past_the_in_flight_cap_open_stream_is_rate_limited_immediately() {
    let mut config = config();
    config.max_in_flight_streams = 3;
    // Long enough that nothing ages out mid-test: the refusal under test is the
    // cap, not a timeout.
    config.stream_ack_timeout = Duration::from_secs(30);
    let harness = Harness::start(config).await;

    // Registered and deliberately never acking, so opens stay in flight.
    let mut server = harness.server().await;
    let mut client = harness.client().await;

    for i in 0..3 {
        client.open_stream(&format!("a{i}")).await;
        assert_eq!(server.recv().await["type"], "STREAM_OPEN");
    }

    let started = tokio::time::Instant::now();
    client
        .send(json!({ "v": 1, "type": "OPEN_STREAM", "attempt_id": "over" }))
        .await;
    let reply = client.recv().await;
    assert_error(&reply, "rate_limited");
    // *Immediately*, never queued (§5.1): an overloaded relay and a slow server
    // want opposite responses from a client, so they must not look alike.
    assert!(
        started.elapsed() < FAST,
        "the refusal was queued, not immediate"
    );
    assert!(
        server.is_silent_for(FAST).await,
        "a refused OPEN_STREAM still reached the origin"
    );

    // An ack frees a slot, so the cap bounds in-flight opens rather than
    // permanently capping a trunk.
    server
        .send(json!({ "v": 1, "type": "STREAM_ACK", "stream_id": 1 }))
        .await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    client.open_stream("after-the-ack").await;
}

#[tokio::test]
async fn the_cap_is_per_client_trunk_not_shared_across_clients() {
    // Counted per client trunk on purpose: a cap shared across clients would
    // let one client's un-acked opens deny service to every other client on the
    // same server, which is a denial-of-service handed to any anonymous caller.
    let mut config = config();
    config.max_in_flight_streams = 2;
    config.stream_ack_timeout = Duration::from_secs(30);
    let harness = Harness::start(config).await;

    let mut server = harness.server().await;
    let mut greedy = harness.client().await;
    let mut polite = harness.client().await;

    for i in 0..2 {
        greedy.open_stream(&format!("g{i}")).await;
        assert_eq!(server.recv().await["type"], "STREAM_OPEN");
    }
    greedy
        .send(json!({ "v": 1, "type": "OPEN_STREAM", "attempt_id": "over" }))
        .await;
    assert_error(&greedy.recv().await, "rate_limited");

    polite.open_stream("p0").await;
    assert_eq!(server.recv().await["type"], "STREAM_OPEN");
}

#[tokio::test]
async fn a_dying_server_trunk_gives_every_open_client_trunk_lost() {
    let harness = Harness::plain().await;
    let server = harness.server().await;
    let mut alice = harness.client().await;
    let mut bob = harness.client().await;

    let mut server = server;
    let alice_stream = alice.open_stream("a1").await;
    server.expect_stream_open_and_ack().await;
    let bob_stream = bob.open_stream("b1").await;
    server.expect_stream_open_and_ack().await;

    // The origin's process dies.
    drop(server);

    for (client, stream_id) in [(&mut alice, alice_stream), (&mut bob, bob_stream)] {
        let reply = client.recv().await;
        assert_error(&reply, "trunk_lost");
        // Named per stream, so a client can fail exactly the requests that were
        // in flight rather than guessing.
        assert_eq!(reply["stream_id"], stream_id, "{reply}");
    }

    // The registration is released before the clients are told, so a `HELLO`
    // arriving now waits for the replacement rather than being handed a trunk
    // whose socket is already gone.
    assert!(harness.relay.registrations.is_empty());
}

#[tokio::test]
async fn a_client_holding_no_stream_is_still_told_the_trunk_is_gone() {
    // Its destination is gone. Leaving it holding an apparently healthy trunk
    // means it only finds out when its next request times out — which is the
    // slow, confusing version of the same failure.
    let harness = Harness::plain().await;
    let server = harness.server().await;
    let mut idle = harness.client().await;

    drop(server);

    let reply = idle.recv().await;
    assert_error(&reply, "trunk_lost");
    assert!(reply.get("stream_id").is_none(), "{reply}");
}

#[tokio::test]
async fn a_dying_client_trunk_frees_its_streams_on_the_server_side() {
    let harness = Harness::plain().await;
    let mut server = harness.server().await;
    let mut client = harness.client().await;

    let stream_id = client.open_stream("a1").await;
    server.expect_stream_open_and_ack().await;

    drop(client);

    let closed = server.recv().await;
    assert_eq!(closed["type"], "CLOSE", "{closed}");
    assert_eq!(closed["stream_id"], stream_id, "{closed}");
}

// ------------------------------------------------------------------ streaming

#[tokio::test]
async fn a_response_body_streams_through_chunk_by_chunk() {
    // Unbuffered (§5.2). A relay that accumulated a response and forwarded it
    // whole would pass a "the bytes arrived" test and still break the change
    // feed, so this asserts each chunk *arrives before the next is sent*.
    let harness = Harness::plain().await;
    let mut server = harness.server().await;
    let mut client = harness.client().await;

    let stream_id = client.open_stream("a1").await;
    server.expect_stream_open_and_ack().await;
    client.send(request_head(stream_id, "/v1/stream")).await;
    assert_eq!(server.recv().await["type"], "HTTP_REQUEST_HEAD");

    server
        .send(json!({
            "v": 1, "type": "HTTP_RESPONSE_HEAD",
            "stream_id": stream_id, "status": 200,
            "headers": { "content-type": "text/event-stream" },
        }))
        .await;
    // The head goes out as soon as status and headers are known — before any
    // body exists at all.
    let head = client.recv().await;
    assert_eq!(head["type"], "HTTP_RESPONSE_HEAD", "{head}");

    for chunk in [b"event: change\n".as_slice(), b"data: {}\n", b"\n"] {
        server.send_body(0x02, stream_id, chunk).await;
        match client.next_frame().await {
            Frame::Body {
                kind,
                stream_id: id,
                payload,
            } => {
                assert_eq!(kind, 0x02);
                assert_eq!(id, stream_id);
                assert_eq!(payload, chunk);
            }
            Frame::Text(value) => panic!("expected a body chunk, got {value}"),
        }
    }
}

// ------------------------------------------------------------ framing refusals

#[tokio::test]
async fn a_hello_that_disagrees_with_the_path_is_a_protocol_error() {
    // §5 does not say what a disagreement means, so it is refused rather than
    // resolved: picking one silently would let a client dial one server's
    // address and be routed to another.
    let harness = Harness::plain().await;
    let _server = harness.server().await;

    let mut client = harness.dial(&format!("/connect/{SERVER_ID}")).await;
    client
        .send(json!({ "v": 1, "type": "HELLO", "server_id": "srv_SOMEONEELSE" }))
        .await;
    assert_error(&client.recv().await, "protocol_error");
}

#[tokio::test]
async fn a_client_speaking_the_servers_half_is_a_protocol_error() {
    let harness = Harness::plain().await;
    let mut server = harness.server().await;

    for frame in [
        json!({ "v": 1, "type": "STREAM_ACK", "stream_id": 1 }),
        json!({ "v": 1, "type": "HTTP_RESPONSE_HEAD",
                "stream_id": 1, "status": 200, "headers": {} }),
        json!({ "v": 1, "type": "REGISTER_SERVER",
                "server_id": SERVER_ID, "pubkey": pubkey_b64(1) }),
    ] {
        let mut client = harness.client().await;
        client.send(frame.clone()).await;
        assert_error(&client.recv().await, "protocol_error");
    }

    // A `0x02` body chunk is the response direction; a client sending one is
    // speaking the origin's half.
    let mut client = harness.client().await;
    let stream_id = client.open_stream("a1").await;
    server.expect_stream_open_and_ack().await;
    client.send_body(0x02, stream_id, b"wrong direction").await;
    assert_error(&client.recv().await, "protocol_error");
}

#[tokio::test]
async fn a_malformed_client_frame_names_no_part_of_the_framing() {
    let harness = Harness::plain().await;
    let _server = harness.server().await;

    let mut messages = Vec::new();
    for frame in [
        // Version pin, checked before the body is interpreted.
        json!({ "v": 2, "type": "HELLO", "server_id": SERVER_ID }),
        json!({ "type": "HELLO", "server_id": SERVER_ID }),
        json!({ "v": 1, "type": "NOT_A_REAL_TYPE" }),
        json!({ "v": 1, "type": "HELLO" }),
        // A first frame that is not HELLO.
        json!({ "v": 1, "type": "OPEN_STREAM", "attempt_id": "a1" }),
    ] {
        let mut client = harness.dial(&format!("/connect/{SERVER_ID}")).await;
        client.send(frame.clone()).await;
        let reply = client.recv().await;
        assert_error(&reply, "protocol_error");
        messages.push(reply["message"].as_str().unwrap().to_string());
    }

    // Every one of those is wrong in a different way and the relay says the
    // same thing to all of them (§6).
    assert!(
        messages.windows(2).all(|pair| pair[0] == pair[1]),
        "protocol_error messages differ by cause: {messages:?}"
    );
}

#[tokio::test]
async fn an_open_stream_without_an_attempt_id_is_a_protocol_error() {
    let harness = Harness::plain().await;
    let _server = harness.server().await;
    let mut client = harness.client().await;

    client.send(json!({ "v": 1, "type": "OPEN_STREAM" })).await;
    assert_error(&client.recv().await, "protocol_error");
}

#[tokio::test]
async fn a_request_head_missing_its_routing_or_shape_fields_is_refused() {
    let harness = Harness::plain().await;
    let mut server = harness.server().await;
    let mut client = harness.client().await;
    let stream_id = client.open_stream("a1").await;
    server.expect_stream_open_and_ack().await;

    for frame in [
        json!({ "v": 1, "type": "HTTP_REQUEST_HEAD",
                "method": "GET", "path": "/v1/notes", "headers": {} }),
        json!({ "v": 1, "type": "HTTP_REQUEST_HEAD", "stream_id": stream_id,
                "path": "/v1/notes", "headers": {} }),
        json!({ "v": 1, "type": "HTTP_REQUEST_HEAD", "stream_id": stream_id,
                "method": "GET", "headers": {} }),
        json!({ "v": 1, "type": "HTTP_REQUEST_HEAD", "stream_id": stream_id,
                "method": "GET", "path": "/v1/notes", "headers": "not-an-object" }),
    ] {
        let mut fresh = harness.client().await;
        fresh.send(frame.clone()).await;
        assert_error(&fresh.recv().await, "protocol_error");
    }
}

// ------------------------------------------------------------------- closing

#[tokio::test]
async fn close_with_a_stream_id_ends_one_stream_and_leaves_the_trunk_open() {
    let harness = Harness::plain().await;
    let mut server = harness.server().await;
    let mut client = harness.client().await;

    let first = client.open_stream("a1").await;
    server.expect_stream_open_and_ack().await;

    client
        .send(json!({ "v": 1, "type": "CLOSE", "stream_id": first }))
        .await;
    let closed = server.recv().await;
    assert_eq!(closed["type"], "CLOSE", "{closed}");
    assert_eq!(closed["stream_id"], first, "{closed}");

    // The trunk still works: a second stream opens on it.
    let second = client.open_stream("a2").await;
    assert_ne!(second, first, "a closed stream_id was reused");
    server.expect_stream_open_and_ack().await;
}

#[tokio::test]
async fn a_server_side_close_reaches_the_owning_client_only() {
    let harness = Harness::plain().await;
    let mut server = harness.server().await;
    let mut alice = harness.client().await;
    let mut bob = harness.client().await;

    let alice_stream = alice.open_stream("a1").await;
    server.expect_stream_open_and_ack().await;
    bob.open_stream("b1").await;
    server.expect_stream_open_and_ack().await;

    server
        .send(json!({ "v": 1, "type": "CLOSE", "stream_id": alice_stream }))
        .await;

    let closed = alice.recv().await;
    assert_eq!(closed["type"], "CLOSE", "{closed}");
    assert_eq!(closed["stream_id"], alice_stream, "{closed}");
    assert!(bob.is_silent_for(FAST).await, "a CLOSE crossed trunks");
}
