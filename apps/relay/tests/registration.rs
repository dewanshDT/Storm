//! The `REGISTER_SERVER` handshake, driven over a real WebSocket.
//!
//! The handshake is a wire protocol, so these tests speak it rather than
//! calling the state machine: a change that broke the framing but left the
//! functions intact would pass a unit test and fail every real server.
//!
//! Signatures are produced with `ed25519-dalek` directly — the same version
//! `apps/server` pins — rather than by calling anything in this crate, so a
//! test cannot agree with the implementation by sharing its mistake.

use std::sync::Arc;
use std::time::Duration;

use data_encoding::BASE64URL_NOPAD;
use ed25519_dalek::{Signer, SigningKey};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use storm_relay::auth::PublicKey;
use storm_relay::{Allowlist, Config, NonceSource, Relay};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

const SERVER_ID: &str = "srv_01ARZ3NDEKTSV4RRFFQ69G5FAV";

// ---------------------------------------------------------------- key helpers

fn signing_key(seed: u8) -> SigningKey {
    // From fixed bytes rather than an RNG: no rand_core interop to get wrong,
    // and a failing test names the key that produced it.
    SigningKey::from_bytes(&[seed; 32])
}

fn pubkey_b64(seed: u8) -> String {
    BASE64URL_NOPAD.encode(signing_key(seed).verifying_key().as_bytes())
}

fn sign(seed: u8, message: &str) -> String {
    BASE64URL_NOPAD.encode(&signing_key(seed).sign(message.as_bytes()).to_bytes())
}

/// The relay-registration domain, spelled out here rather than imported: this
/// is the wire commitment, and a test that reuses the implementation's builder
/// proves only that the builder is self-consistent.
fn relay_auth(server_id: &str, nonce: &str) -> String {
    format!("storm-relay-auth:v1:{server_id}:{nonce}")
}

/// The *client-facing* identity domain. A signature over this must never
/// register a server.
fn client_challenge(server_id: &str, nonce: &str) -> String {
    format!("storm-challenge:v1:{server_id}:{nonce}")
}

// -------------------------------------------------------------------- harness

struct Harness {
    url: String,
    relay: Arc<Relay>,
}

impl Harness {
    async fn start(config: Config, nonce_source: Option<NonceSource>) -> Self {
        // 127.0.0.1:0 — never the real 8484/8485, and never a fixed port, so
        // the suite can run in parallel with itself.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let relay = Arc::new(match nonce_source {
            Some(source) => Relay::with_nonce_source(config, source),
            None => Relay::new(config),
        });
        let serving = relay.clone();
        tokio::spawn(async move {
            let _ = storm_relay::serve(listener, serving).await;
        });
        Self {
            url: format!("ws://{addr}/register"),
            relay,
        }
    }

    async fn plain() -> Self {
        Self::start(config(), None).await
    }

    async fn connect(&self) -> Conn {
        let (socket, _) = tokio_tungstenite::connect_async(self.url.as_str())
            .await
            .unwrap();
        Conn { socket }
    }
}

fn config() -> Config {
    Config::new("127.0.0.1:0".parse().unwrap(), "wss://relay.example")
}

struct Conn {
    socket: WebSocketStream<MaybeTlsStream<TcpStream>>,
}

impl Conn {
    async fn send(&mut self, value: Value) {
        self.socket
            .send(Message::Text(value.to_string().into()))
            .await
            .unwrap();
    }

    async fn send_raw(&mut self, text: &str) {
        self.socket.send(Message::Text(text.into())).await.unwrap();
    }

    async fn recv(&mut self) -> Value {
        loop {
            match self.socket.next().await.expect("a frame").unwrap() {
                Message::Text(text) => return serde_json::from_str(&text).unwrap(),
                Message::Ping(_) | Message::Pong(_) => continue,
                other => panic!("expected a text control frame, got {other:?}"),
            }
        }
    }

    /// Sends `REGISTER_SERVER` and returns the nonce from the `CHALLENGE`.
    async fn register(&mut self, server_id: &str, pubkey: &str) -> String {
        self.send(json!({
            "v": 1, "type": "REGISTER_SERVER",
            "server_id": server_id, "pubkey": pubkey,
        }))
        .await;
        let reply = self.recv().await;
        assert_eq!(reply["type"], "CHALLENGE", "{reply}");
        reply["nonce"].as_str().unwrap().to_string()
    }

    async fn respond(&mut self, sig: &str) -> Value {
        self.send(json!({ "v": 1, "type": "CHALLENGE_RESPONSE", "sig": sig }))
            .await;
        self.recv().await
    }
}

fn assert_error(reply: &Value, code: &str) {
    assert_eq!(reply["type"], "ERROR", "{reply}");
    assert_eq!(reply["v"], 1, "{reply}");
    assert_eq!(reply["code"], code, "{reply}");
}

// --------------------------------------------------------------- happy path

#[tokio::test]
async fn a_correct_handshake_registers_and_returns_a_derived_public_address() {
    let harness = Harness::plain().await;
    let mut conn = harness.connect().await;

    let nonce = conn.register(SERVER_ID, &pubkey_b64(1)).await;
    // The nonce must satisfy the rule the server-side validator applies before
    // it will sign: 16–128 printable ASCII, no ':' and no '"'.
    assert!((16..=128).contains(&nonce.len()), "{nonce}");
    assert!(
        nonce
            .bytes()
            .all(|b| b.is_ascii_graphic() && b != b':' && b != b'"')
    );

    let reply = conn.respond(&sign(1, &relay_auth(SERVER_ID, &nonce))).await;
    assert_eq!(reply["type"], "REGISTERED", "{reply}");
    assert_eq!(reply["v"], 1, "{reply}");
    assert_eq!(reply["heartbeat_interval_secs"], 15, "{reply}");
    assert!(
        reply["trunk_id"].as_str().unwrap().starts_with("trk_"),
        "{reply}"
    );
    // Derived, not allocated (§4.3): any client holding the server_id can
    // construct this, and the relay hands out no opaque identifier.
    assert_eq!(
        reply["public_address"],
        format!("wss://relay.example/connect/{SERVER_ID}")
    );

    let registration = harness.relay.registrations.get(SERVER_ID).unwrap();
    assert_eq!(registration.trunk_id, reply["trunk_id"].as_str().unwrap());
}

#[tokio::test]
async fn a_registered_trunk_answers_ping_and_frees_its_id_on_deregister() {
    let harness = Harness::plain().await;
    let mut conn = harness.connect().await;
    let nonce = conn.register(SERVER_ID, &pubkey_b64(1)).await;
    conn.respond(&sign(1, &relay_auth(SERVER_ID, &nonce))).await;

    conn.send(json!({ "v": 1, "type": "PING" })).await;
    assert_eq!(conn.recv().await["type"], "PONG");

    conn.send(json!({ "v": 1, "type": "DEREGISTER" })).await;
    // A clean shutdown frees the server_id immediately rather than making
    // clients wait out a timeout for a deliberate restart (§4.2).
    for _ in 0..100 {
        if harness.relay.registrations.is_empty() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("DEREGISTER did not free the registration");
}

// ------------------------------------------------------------ signature rules

#[tokio::test]
async fn a_signature_over_the_client_facing_domain_is_refused() {
    // Domain separation, asserted rather than assumed. `storm-challenge:v1:` is
    // what a server signs to prove its identity *to a client* — an endpoint
    // that will sign whatever a client sends it. If that signature also
    // registered a trunk, any client could turn its own pairing challenge into
    // a relay registration for the server it just talked to.
    let harness = Harness::plain().await;
    let mut conn = harness.connect().await;
    let nonce = conn.register(SERVER_ID, &pubkey_b64(1)).await;

    let reply = conn
        .respond(&sign(1, &client_challenge(SERVER_ID, &nonce)))
        .await;
    assert_error(&reply, "auth_failed");
    assert!(harness.relay.registrations.is_empty());
}

#[tokio::test]
async fn a_signature_over_the_prefix_and_nonce_alone_is_refused() {
    // `server_id` and both colons are inside the signed bytes (§4). Without
    // them one signature would register any id.
    let harness = Harness::plain().await;
    let mut conn = harness.connect().await;
    let nonce = conn.register(SERVER_ID, &pubkey_b64(1)).await;

    let reply = conn
        .respond(&sign(1, &format!("storm-relay-auth:v1:{nonce}")))
        .await;
    assert_error(&reply, "auth_failed");
}

#[tokio::test]
async fn a_signature_for_a_different_server_id_is_refused() {
    let harness = Harness::plain().await;
    let mut conn = harness.connect().await;
    let nonce = conn.register(SERVER_ID, &pubkey_b64(1)).await;

    let reply = conn
        .respond(&sign(1, &relay_auth("srv_SOMEONEELSE", &nonce)))
        .await;
    assert_error(&reply, "auth_failed");
}

#[tokio::test]
async fn a_signature_by_the_wrong_key_is_auth_failed() {
    let harness = Harness::plain().await;
    let mut conn = harness.connect().await;
    let nonce = conn.register(SERVER_ID, &pubkey_b64(1)).await;

    // Correct message, correct nonce, key 2 instead of key 1.
    let reply = conn.respond(&sign(2, &relay_auth(SERVER_ID, &nonce))).await;
    assert_error(&reply, "auth_failed");
    assert!(harness.relay.registrations.is_empty());
    // A failed signature must not leave a first-use binding behind: TOFU
    // records only after a *successful* signature (§4.1).
    assert!(
        harness
            .relay
            .bindings
            .check(None, SERVER_ID, key(2))
            .is_ok()
    );
}

fn key(seed: u8) -> PublicKey {
    PublicKey::from_b64(&pubkey_b64(seed)).unwrap()
}

// ------------------------------------------------------------- nonce handling

/// Pins the nonce so replay is observable. With a fresh nonce per connection a
/// replayed *signature* fails because the nonce differs, which proves nothing
/// about single-use.
fn fixed_nonce(nonce: &'static str) -> NonceSource {
    Arc::new(move || nonce.to_string())
}

#[tokio::test]
async fn a_replayed_nonce_is_refused() {
    let harness = Harness::start(config(), Some(fixed_nonce("fixed-nonce-0123456789"))).await;

    let mut first = harness.connect().await;
    let nonce = first.register(SERVER_ID, &pubkey_b64(1)).await;
    assert_eq!(
        first
            .respond(&sign(1, &relay_auth(SERVER_ID, &nonce)))
            .await["type"],
        "REGISTERED"
    );

    // Same nonce, same key, byte-identical correct signature — and it must
    // still fail, because the nonce was spent. Pinning the nonce is what makes
    // this observable: with a fresh nonce per connection the replay would fail
    // for the uninteresting reason that the signature covers different bytes.
    let mut second = harness.connect().await;
    let replayed = second.register(SERVER_ID, &pubkey_b64(1)).await;
    assert_eq!(replayed, nonce, "the nonce source should have pinned this");
    let reply = second
        .respond(&sign(1, &relay_auth(SERVER_ID, &nonce)))
        .await;
    assert_error(&reply, "auth_failed");
}

#[tokio::test]
async fn a_captured_signature_cannot_be_replayed_onto_a_fresh_connection() {
    // The attack the nonce actually defends against, with the real generator:
    // an observer who saw a legitimate registration cannot reuse its signature,
    // because the next connection is challenged with different bytes.
    let harness = Harness::plain().await;

    let mut victim = harness.connect().await;
    let nonce = victim.register(SERVER_ID, &pubkey_b64(1)).await;
    let captured = sign(1, &relay_auth(SERVER_ID, &nonce));
    assert_eq!(victim.respond(&captured).await["type"], "REGISTERED");

    let mut attacker = harness.connect().await;
    let fresh = attacker.register(SERVER_ID, &pubkey_b64(1)).await;
    assert_ne!(fresh, nonce);
    assert_error(&attacker.respond(&captured).await, "auth_failed");
}

#[tokio::test]
async fn an_expired_nonce_is_refused() {
    let mut config = config();
    // Expired the instant it is issued, so a correct signature over a live,
    // never-replayed nonce can only fail for having aged out.
    config.challenge_ttl = Duration::ZERO;
    let harness = Harness::start(config, None).await;

    let mut conn = harness.connect().await;
    let nonce = conn.register(SERVER_ID, &pubkey_b64(1)).await;
    let reply = conn.respond(&sign(1, &relay_auth(SERVER_ID, &nonce))).await;
    assert_error(&reply, "auth_failed");
    assert!(harness.relay.registrations.is_empty());
}

#[tokio::test]
async fn a_nonce_containing_a_colon_is_never_issued() {
    // The relay generates nonces; it never receives one during registration, so
    // the rule bites at the point of issue. A nonce carrying ':' could forge
    // the signed message's own field boundaries, so the relay refuses to hand
    // one out rather than issuing it and hoping.
    for bad in ["colon:in:the:nonce!!", "quote\"in\"the\"nonce", "short"] {
        let harness = Harness::start(config(), Some(fixed_nonce_owned(bad))).await;
        let mut conn = harness.connect().await;
        conn.send(json!({
            "v": 1, "type": "REGISTER_SERVER",
            "server_id": SERVER_ID, "pubkey": pubkey_b64(1),
        }))
        .await;
        let reply = conn.recv().await;
        assert_ne!(reply["type"], "CHALLENGE", "issued a bad nonce: {reply}");
        assert_error(&reply, "protocol_error");
    }
}

fn fixed_nonce_owned(nonce: &str) -> NonceSource {
    let nonce = nonce.to_string();
    Arc::new(move || nonce.clone())
}

// ---------------------------------------------------------- binding: TOFU

#[tokio::test]
async fn tofu_binds_the_first_key_and_refuses_a_different_one_for_ever() {
    let harness = Harness::plain().await;

    // First successful registration records (server_id, pubkey).
    let mut first = harness.connect().await;
    let nonce = first.register(SERVER_ID, &pubkey_b64(1)).await;
    assert_eq!(
        first
            .respond(&sign(1, &relay_auth(SERVER_ID, &nonce)))
            .await["type"],
        "REGISTERED"
    );

    // A different key for the same id is refused — and refused at
    // REGISTER_SERVER, before a nonce is issued (§4.1). The count is taken
    // before and after because the legitimate registration above left its own
    // spent nonce tracked for the rest of its TTL.
    let before = harness.relay.challenges.tracked_count();
    let mut attacker = harness.connect().await;
    attacker
        .send(json!({
            "v": 1, "type": "REGISTER_SERVER",
            "server_id": SERVER_ID, "pubkey": pubkey_b64(2),
        }))
        .await;
    let reply = attacker.recv().await;
    assert_error(&reply, "auth_failed");
    assert_eq!(
        harness.relay.challenges.tracked_count(),
        before,
        "issued a nonce for a registration that could not succeed"
    );

    // The bound key still registers. This is the reconnect path, and it is also
    // why key rotation is impossible in v1 (§7): the only key that works is the
    // first one ever seen.
    let mut again = harness.connect().await;
    let nonce = again.register(SERVER_ID, &pubkey_b64(1)).await;
    assert_eq!(
        again
            .respond(&sign(1, &relay_auth(SERVER_ID, &nonce)))
            .await["type"],
        "REGISTERED"
    );
    assert_eq!(harness.relay.registrations.len(), 1);
}

#[tokio::test]
async fn tofu_binds_per_server_id() {
    let harness = Harness::plain().await;
    // Held open: dropping a connection closes its trunk, and the relay then
    // releases the registration.
    let mut open = Vec::new();
    for (server_id, seed) in [("srv_AAAA", 1u8), ("srv_BBBB", 2u8)] {
        let mut conn = harness.connect().await;
        let nonce = conn.register(server_id, &pubkey_b64(seed)).await;
        assert_eq!(
            conn.respond(&sign(seed, &relay_auth(server_id, &nonce)))
                .await["type"],
            "REGISTERED"
        );
        open.push(conn);
    }
    assert_eq!(harness.relay.registrations.len(), 2);
}

// ----------------------------------------------------- binding: allowlist

fn allowlisted() -> Config {
    let mut config = config();
    config.allowlist = Some(Allowlist::from_entries([(SERVER_ID.to_string(), key(1))]));
    config
}

#[tokio::test]
async fn an_allowlisted_pubkey_registers() {
    let harness = Harness::start(allowlisted(), None).await;
    let mut conn = harness.connect().await;
    let nonce = conn.register(SERVER_ID, &pubkey_b64(1)).await;
    assert_eq!(
        conn.respond(&sign(1, &relay_auth(SERVER_ID, &nonce))).await["type"],
        "REGISTERED"
    );
}

#[tokio::test]
async fn an_allowlist_refuses_a_mismatched_key_and_an_unlisted_id_on_first_sight() {
    let harness = Harness::start(allowlisted(), None).await;

    // Listed id, wrong key.
    let mut conn = harness.connect().await;
    conn.send(json!({
        "v": 1, "type": "REGISTER_SERVER",
        "server_id": SERVER_ID, "pubkey": pubkey_b64(2),
    }))
    .await;
    assert_error(&conn.recv().await, "auth_failed");

    // Never-seen id. TOFU would adopt this one; an allowlist must not, because
    // an operator who listed which keys may register did not ask for unlisted
    // ones to be adopted on arrival.
    let mut conn = harness.connect().await;
    conn.send(json!({
        "v": 1, "type": "REGISTER_SERVER",
        "server_id": "srv_NEVERSEENBEFORE", "pubkey": pubkey_b64(3),
    }))
    .await;
    assert_error(&conn.recv().await, "auth_failed");

    assert!(harness.relay.registrations.is_empty());
    assert_eq!(harness.relay.challenges.tracked_count(), 0);
}

// ------------------------------------------------------------- version pin

#[tokio::test]
async fn a_wrong_or_missing_version_is_a_protocol_error() {
    let harness = Harness::plain().await;

    for frame in [
        // Version 2, with a body that would otherwise be a valid v1 message.
        json!({ "v": 2, "type": "REGISTER_SERVER",
                "server_id": SERVER_ID, "pubkey": pubkey_b64(1) }),
        json!({ "type": "REGISTER_SERVER",
                "server_id": SERVER_ID, "pubkey": pubkey_b64(1) }),
        json!({ "v": null, "type": "REGISTER_SERVER",
                "server_id": SERVER_ID, "pubkey": pubkey_b64(1) }),
        json!({ "v": "1", "type": "REGISTER_SERVER",
                "server_id": SERVER_ID, "pubkey": pubkey_b64(1) }),
    ] {
        let mut conn = harness.connect().await;
        conn.send(frame.clone()).await;
        assert_error(&conn.recv().await, "protocol_error");
    }
}

// ------------------------------------------------------------ malformed input

#[tokio::test]
async fn a_malformed_frame_is_a_protocol_error_that_names_no_part_of_the_framing() {
    let harness = Harness::plain().await;

    let mut messages = Vec::new();
    for raw in [
        "not json at all",
        "[]",
        "42",
        r#"{"v":1}"#,
        r#"{"v":1,"type":"NOT_A_REAL_TYPE"}"#,
        r#"{"v":1,"type":"CHALLENGE_RESPONSE","sig":"x"}"#,
        r#"{"v":1,"type":"REGISTER_SERVER","server_id":"srv_A"}"#,
        r#"{"v":1,"type":"REGISTER_SERVER","server_id":"srv_A","pubkey":"not base64"}"#,
        r#"{"v":1,"type":"REGISTER_SERVER","server_id":"srv:A","pubkey":"AAAA"}"#,
    ] {
        let mut conn = harness.connect().await;
        conn.send_raw(raw).await;
        let reply = conn.recv().await;
        assert_error(&reply, "protocol_error");
        messages.push(reply["message"].as_str().unwrap().to_string());
    }

    // Every one of those is wrong in a different way, and the relay says the
    // same thing to all of them. A malformed-frame scanner learns nothing about
    // which part of its framing the relay disliked.
    assert!(
        messages.windows(2).all(|pair| pair[0] == pair[1]),
        "protocol_error messages differ by cause: {messages:?}"
    );
}

#[tokio::test]
async fn every_auth_failure_is_indistinguishable_from_outside() {
    // Bad signature, expired nonce and a refused binding must produce byte-
    // identical errors. If they ever diverge, a registration attacker can tell
    // "wrong key" from "this id is already taken" and probe accordingly.
    let bad_signature = {
        let harness = Harness::plain().await;
        let mut conn = harness.connect().await;
        let nonce = conn.register(SERVER_ID, &pubkey_b64(1)).await;
        conn.respond(&sign(2, &relay_auth(SERVER_ID, &nonce))).await
    };

    let expired_nonce = {
        let mut config = config();
        config.challenge_ttl = Duration::ZERO;
        let harness = Harness::start(config, None).await;
        let mut conn = harness.connect().await;
        let nonce = conn.register(SERVER_ID, &pubkey_b64(1)).await;
        conn.respond(&sign(1, &relay_auth(SERVER_ID, &nonce))).await
    };

    let refused_binding = {
        let harness = Harness::start(allowlisted(), None).await;
        let mut conn = harness.connect().await;
        conn.send(json!({
            "v": 1, "type": "REGISTER_SERVER",
            "server_id": SERVER_ID, "pubkey": pubkey_b64(2),
        }))
        .await;
        conn.recv().await
    };

    assert_eq!(bad_signature, expired_nonce);
    assert_eq!(expired_nonce, refused_binding);
    assert_eq!(refused_binding["message"], "authentication failed");
}

// ------------------------------------------------------------ frame ordering

#[tokio::test]
async fn a_binary_frame_during_the_handshake_is_a_protocol_error() {
    // Binary frames are body chunks (§3) and have no meaning on a trunk with no
    // open streams.
    let harness = Harness::plain().await;
    let mut conn = harness.connect().await;
    conn.socket
        .send(Message::Binary(vec![0x01, 0, 0, 0, 1].into()))
        .await
        .unwrap();
    assert_error(&conn.recv().await, "protocol_error");
}

#[tokio::test]
async fn a_second_register_server_before_the_response_is_a_protocol_error() {
    let harness = Harness::plain().await;
    let mut conn = harness.connect().await;
    conn.register(SERVER_ID, &pubkey_b64(1)).await;
    conn.send(json!({
        "v": 1, "type": "REGISTER_SERVER",
        "server_id": SERVER_ID, "pubkey": pubkey_b64(1),
    }))
    .await;
    assert_error(&conn.recv().await, "protocol_error");
}
