//! `mcp-connect`'s connection race (decision 105b) through real `SrpTrunk`s
//! to fake relays and over real HTTP to fake servers. The fakes answer on
//! protocol events, never on a timer: a request is answered when its end of
//! body arrives, and a slow path is slow because the test holds it.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use data_encoding::BASE64URL_NOPAD;
use ed25519_dalek::{Signer, SigningKey};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use storm_runtime::connection::Status;
use storm_runtime::identity::relay_set_message;
use storm_runtime::mcp_connect::Adapter;
use storm_runtime::race::{RaceConnector, RaceOptions, read_cache};
use storm_runtime::srp::TrunkConfig;
use storm_runtime::transport::ServerPin;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Notify, mpsc};
use tokio_tungstenite::tungstenite::Message;

const SERVER: &str = "srv_pinned";

async fn within<T>(fut: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(20), fut)
        .await
        .expect("hung: the fakes answer only on protocol events")
}

fn pinned() -> SigningKey {
    SigningKey::from_bytes(&[7; 32])
}

fn impostor() -> SigningKey {
    SigningKey::from_bytes(&[9; 32])
}

/// One request as a fake server received it.
#[derive(Clone, Debug)]
struct Seen {
    path: String,
    headers: Vec<(String, String)>,
    body: Value,
}

impl Seen {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

enum Action {
    Respond(u16, Option<Value>),
    /// The relay ends the stream with `ERROR{code, stream_id}`.
    StreamError(&'static str),
    /// Never answer.
    Hang,
    /// The relay hangs up the whole trunk.
    DropTrunk,
}

/// A Storm server as far as `mcp-connect` can tell: the challenge,
/// `/v1/server` and a minimal `/mcp` that names which server answered.
struct FakeServer {
    name: &'static str,
    signing: SigningKey,
    /// What `/v1/server` advertises.
    relays: Mutex<Vec<String>>,
    relays_sig: Mutex<Option<String>>,
    /// Answers the challenge with this instead of a signature.
    refuse_challenge: Mutex<Option<(u16, Value)>>,
    /// While set, the challenge waits for it: a slow path.
    gate: Mutex<Option<Arc<Notify>>>,
    seen: Mutex<Vec<Seen>>,
    changed: Notify,
}

impl FakeServer {
    fn new(name: &'static str, signing: SigningKey) -> Arc<Self> {
        Arc::new(Self {
            name,
            signing,
            relays: Mutex::default(),
            relays_sig: Mutex::default(),
            refuse_challenge: Mutex::default(),
            gate: Mutex::default(),
            seen: Mutex::default(),
            changed: Notify::new(),
        })
    }

    fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }

    fn calls(&self, tool: &str) -> usize {
        self.seen()
            .iter()
            .filter(|s| s.body.pointer("/params/name") == Some(&json!(tool)))
            .count()
    }

    /// Nothing that reached this server carried a key, and no MCP message
    /// reached it.
    fn never_saw_the_key(&self) -> bool {
        self.seen()
            .iter()
            .all(|s| s.header("authorization").is_none() && s.path != "/mcp")
    }

    async fn saw(&self, pred: impl Fn(&Seen) -> bool) {
        within(async {
            loop {
                let changed = self.changed.notified();
                if self.seen().iter().any(&pred) {
                    return;
                }
                changed.await;
            }
        })
        .await
    }

    async fn respond(&self, seen: Seen) -> Action {
        self.seen.lock().unwrap().push(seen.clone());
        self.changed.notify_waiters();
        match seen.path.as_str() {
            "/v1/server/challenge" => {
                let gate = self.gate.lock().unwrap().clone();
                if let Some(gate) = gate {
                    gate.notified().await;
                }
                if let Some((status, body)) = self.refuse_challenge.lock().unwrap().clone() {
                    return Action::Respond(status, Some(body));
                }
                let nonce = seen.body["nonce"].as_str().unwrap_or_default();
                let sig = self
                    .signing
                    .sign(format!("storm-challenge:v1:{SERVER}:{nonce}").as_bytes());
                Action::Respond(
                    200,
                    Some(json!({"server_id": SERVER,
                                "signature": BASE64URL_NOPAD.encode(&sig.to_bytes())})),
                )
            }
            "/v1/server" => {
                let relays: Vec<Value> = self
                    .relays
                    .lock()
                    .unwrap()
                    .iter()
                    .map(|u| json!({"url": u, "public_address": format!("{u}/connect/{SERVER}")}))
                    .collect();
                let mut info = json!({"server_id": SERVER, "name": "fake", "key_id": "k",
                                      "algorithm": "ed25519", "public_key": "x", "relays": relays});
                if let Some(sig) = self.relays_sig.lock().unwrap().clone() {
                    info["relays_sig"] = json!(sig);
                }
                Action::Respond(200, Some(info))
            }
            "/mcp" => {
                let id = seen.body["id"].clone();
                match (
                    seen.body["method"].as_str(),
                    seen.body.pointer("/params/name").and_then(|n| n.as_str()),
                ) {
                    (Some("tools/call"), Some("dies")) => Action::DropTrunk,
                    (Some("tools/call"), Some("refused")) => Action::StreamError("rate_limited"),
                    (Some("tools/call"), Some("aborted")) => Action::StreamError("stream_aborted"),
                    (Some("tools/call"), Some("superseded")) => {
                        Action::StreamError("trunk_superseded")
                    }
                    (Some("tools/call"), Some("hangs")) => Action::Hang,
                    (Some(m), _) if m.starts_with("notifications/") => Action::Respond(202, None),
                    _ => Action::Respond(
                        200,
                        Some(json!({"jsonrpc": "2.0", "id": id, "result": {"via": self.name}})),
                    ),
                }
            }
            _ => Action::Respond(404, Some(json!({"error": "not found"}))),
        }
    }
}

/// The server's own address over plain HTTP/1.1.
async fn direct(server: Arc<FakeServer>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            tokio::spawn(http_connection(stream, server.clone()));
        }
    });
    addr
}

async fn http_connection(stream: TcpStream, server: Arc<FakeServer>) {
    let mut reader = BufReader::new(stream);
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).await.unwrap_or(0) == 0 {
            return;
        }
        let path = line.split_whitespace().nth(1).unwrap_or("").to_string();
        let mut headers = Vec::new();
        loop {
            let mut h = String::new();
            if reader.read_line(&mut h).await.unwrap_or(0) == 0 {
                return;
            }
            let h = h.trim_end();
            if h.is_empty() {
                break;
            }
            let (n, v) = h.split_once(':').unwrap();
            headers.push((n.trim().to_ascii_lowercase(), v.trim().to_string()));
        }
        let length: usize = headers
            .iter()
            .find(|(n, _)| n == "content-length")
            .map(|(_, v)| v.parse().unwrap())
            .unwrap_or(0);
        let mut body = vec![0; length];
        if reader.read_exact(&mut body).await.is_err() {
            return;
        }
        let body = serde_json::from_slice(&body).unwrap_or(Value::Null);
        let (status, answer) = match server
            .respond(Seen {
                path,
                headers,
                body,
            })
            .await
        {
            Action::Respond(s, a) => (s, a),
            Action::StreamError(_) => return,
            Action::Hang => return std::future::pending().await,
            Action::DropTrunk => return,
        };
        let answer = answer.map(|a| a.to_string()).unwrap_or_default();
        let head = format!(
            "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n",
            answer.len()
        );
        let stream = reader.get_mut();
        if stream.write_all(head.as_bytes()).await.is_err()
            || stream.write_all(answer.as_bytes()).await.is_err()
        {
            return;
        }
    }
}

#[derive(Clone, Copy)]
enum Hello {
    Ready,
    Refuse(&'static str),
}

/// A relay in front of `server`: `HELLO`, then one stream per request.
struct FakeRelay {
    url: String,
    hellos: AtomicUsize,
    heads: Mutex<Vec<Value>>,
    /// The `HELLO.pubkey` of every trunk.
    hello_keys: Mutex<Vec<Value>>,
}

impl FakeRelay {
    /// `host` is how the URL names loopback, which is what lets one test
    /// call a relay public and another self-hosted.
    async fn start(host: &str, hello: Hello, server: Arc<FakeServer>) -> Arc<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let relay = Arc::new(Self {
            url: format!("ws://{host}:{port}"),
            hellos: AtomicUsize::new(0),
            heads: Mutex::default(),
            hello_keys: Mutex::default(),
        });
        let r = relay.clone();
        tokio::spawn(async move {
            while let Ok((tcp, _)) = listener.accept().await {
                tokio::spawn(relay_connection(tcp, hello, server.clone(), r.clone()));
            }
        });
        relay
    }

    fn hellos(&self) -> usize {
        self.hellos.load(Ordering::SeqCst)
    }
}

fn control(ty: &str, mut body: Value) -> Message {
    body["v"] = json!(1);
    body["type"] = json!(ty);
    Message::text(body.to_string())
}

enum Out {
    Send(Message),
    HangUp,
}

async fn relay_connection(
    tcp: TcpStream,
    hello: Hello,
    server: Arc<FakeServer>,
    relay: Arc<FakeRelay>,
) {
    let Ok(ws) = tokio_tungstenite::accept_async(tcp).await else {
        return;
    };
    let (mut sink, mut stream) = ws.split();
    let (out, mut out_rx) = mpsc::unbounded_channel::<Out>();
    let writer = tokio::spawn(async move {
        while let Some(o) = out_rx.recv().await {
            match o {
                Out::Send(m) => {
                    if sink.send(m).await.is_err() {
                        return;
                    }
                }
                Out::HangUp => {
                    let _ = sink.close().await;
                    return;
                }
            }
        }
    });
    let mut next_stream = 0u32;
    let mut heads: HashMap<u32, Value> = HashMap::new();
    let mut bodies: HashMap<u32, Vec<u8>> = HashMap::new();
    while let Some(Ok(message)) = stream.next().await {
        if writer.is_finished() {
            return;
        }
        match message {
            Message::Text(text) => {
                let v: Value = serde_json::from_str(text.as_str()).unwrap();
                match v["type"].as_str().unwrap_or_default() {
                    "HELLO" => {
                        relay.hellos.fetch_add(1, Ordering::SeqCst);
                        relay.hello_keys.lock().unwrap().push(v["pubkey"].clone());
                        match hello {
                            Hello::Ready => {
                                let _ = out.send(Out::Send(control(
                                    "READY",
                                    json!({"client_trunk_id": "ctk_fake"}),
                                )));
                            }
                            Hello::Refuse(code) => {
                                let _ = out.send(Out::Send(control(
                                    "ERROR",
                                    json!({"code": code, "message": code}),
                                )));
                                let _ = out.send(Out::HangUp);
                            }
                        }
                    }
                    "OPEN_STREAM" => {
                        next_stream += 1;
                        let _ = out.send(Out::Send(control(
                            "STREAM_READY",
                            json!({"attempt_id": v["attempt_id"], "stream_id": next_stream}),
                        )));
                    }
                    "HTTP_REQUEST_HEAD" => {
                        relay.heads.lock().unwrap().push(v.clone());
                        heads.insert(v["stream_id"].as_u64().unwrap() as u32, v);
                    }
                    "PING" => {
                        let _ = out.send(Out::Send(control("PONG", json!({}))));
                    }
                    _ => {}
                }
            }
            Message::Binary(frame) => {
                let stream_id = u32::from_be_bytes([frame[1], frame[2], frame[3], frame[4]]);
                let payload = &frame[5..];
                if !payload.is_empty() {
                    bodies
                        .entry(stream_id)
                        .or_default()
                        .extend_from_slice(payload);
                    continue;
                }
                let Some(head) = heads.remove(&stream_id) else {
                    continue;
                };
                let body = bodies.remove(&stream_id).unwrap_or_default();
                let headers = head["headers"]
                    .as_object()
                    .map(|h| {
                        h.iter()
                            .map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string()))
                            .collect()
                    })
                    .unwrap_or_default();
                let seen = Seen {
                    path: head["path"].as_str().unwrap_or("").to_string(),
                    headers,
                    body: serde_json::from_slice(&body).unwrap_or(Value::Null),
                };
                let (server, out) = (server.clone(), out.clone());
                tokio::spawn(async move {
                    match server.respond(seen).await {
                        Action::Respond(status, answer) => {
                            let _ = out.send(Out::Send(control(
                                "HTTP_RESPONSE_HEAD",
                                json!({"stream_id": stream_id, "status": status,
                                       "headers": {"content-type": "application/json"}}),
                            )));
                            if let Some(a) = answer {
                                let mut frame = vec![0x02];
                                frame.extend_from_slice(&stream_id.to_be_bytes());
                                frame.extend_from_slice(a.to_string().as_bytes());
                                let _ = out.send(Out::Send(Message::Binary(frame.into())));
                            }
                            let _ = out
                                .send(Out::Send(control("CLOSE", json!({"stream_id": stream_id}))));
                        }
                        Action::StreamError(code) => {
                            let _ = out.send(Out::Send(control(
                                "ERROR",
                                json!({"code": code, "message": code, "stream_id": stream_id}),
                            )));
                        }
                        Action::Hang => {}
                        Action::DropTrunk => {
                            let _ = out.send(Out::HangUp);
                        }
                    }
                });
            }
            _ => {}
        }
    }
}

/// A port that refuses connections and stays this process's: bound, never
/// listening, and held for good. Dropping a listener instead let Windows,
/// which hands out ephemeral ports in order, give the port to the next test's
/// fake server, and the "down" relay answered as an impostor.
fn closed_port() -> u16 {
    let socket = socket2::Socket::new(socket2::Domain::IPV4, socket2::Type::STREAM, None).unwrap();
    socket
        .bind(&std::net::SocketAddr::from(([127, 0, 0, 1], 0)).into())
        .unwrap();
    let port = socket.local_addr().unwrap().as_socket().unwrap().port();
    std::mem::forget(socket);
    port
}

fn pin() -> ServerPin {
    ServerPin {
        server_id: SERVER.into(),
        key: pinned().verifying_key(),
    }
}

fn options(direct: Option<String>, relays: Vec<String>) -> RaceOptions {
    let mut o = RaceOptions::new(direct, relays);
    o.cache = None;
    o.public_hosts = vec![];
    o.trunk = TrunkConfig::default();
    o
}

fn adapter(options: RaceOptions) -> Adapter<RaceConnector> {
    Adapter::new(RaceConnector::new(pin(), options).unwrap(), "stk_secret")
}

fn call(id: i64, tool: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "method": "tools/call",
           "params": {"name": tool, "arguments": {}}})
}

/// The one answer to one request.
async fn ask(a: &Adapter<RaceConnector>, id: i64, tool: &str) -> Value {
    let mut out = within(a.handle(call(id, tool))).await;
    assert_eq!(out.len(), 1, "{out:?}");
    out.remove(0)
}

fn storm_error(answer: &Value) -> &str {
    answer["error"]["data"]["storm_error"]
        .as_str()
        .unwrap_or("")
}

/// The real server's proof is held until the impostor has been asked, so
/// the impostor is always part of the race.
async fn impostor_on_one_path(impostor_direct: bool) {
    let fake = FakeServer::new("fake", impostor());
    let real = FakeServer::new("real", pinned());
    let gate = Arc::new(Notify::new());
    *real.gate.lock().unwrap() = Some(gate.clone());
    let (at_addr, behind_relay) = if impostor_direct {
        (fake.clone(), real.clone())
    } else {
        (real.clone(), fake.clone())
    };
    let relay = FakeRelay::start("127.0.0.1", Hello::Ready, behind_relay).await;
    let a = Arc::new(adapter(options(
        Some(direct(at_addr).await),
        vec![relay.url.clone()],
    )));
    let asked = tokio::spawn({
        let a = a.clone();
        async move { ask(&a, 1, "echo").await }
    });
    fake.saw(|s| s.path == "/v1/server/challenge").await;
    *real.gate.lock().unwrap() = None;
    gate.notify_one();
    let answer = within(asked).await.unwrap();
    assert_eq!(answer["result"]["via"], "real", "{answer}");
    assert!(fake.never_saw_the_key());
    assert_eq!(real.calls("echo"), 1);
    // §5: the relay was told the pinned key, never the impostor's.
    let pinned_key = data_encoding::BASE64URL_NOPAD.encode(pinned().verifying_key().as_bytes());
    let keys = relay.hello_keys.lock().unwrap().clone();
    assert!(!keys.is_empty());
    assert!(keys.iter().all(|k| *k == pinned_key), "{keys:?}");
}

#[tokio::test]
async fn an_impostor_at_the_address_never_sees_the_key_and_the_relay_wins() {
    impostor_on_one_path(true).await;
}

#[tokio::test]
async fn an_impostor_behind_a_relay_never_sees_the_key_and_the_address_wins() {
    impostor_on_one_path(false).await;
}

#[tokio::test]
async fn an_impostor_everywhere_is_identity_failed_never_offline() {
    let fake = FakeServer::new("fake", impostor());
    let relay = FakeRelay::start("127.0.0.1", Hello::Ready, fake.clone()).await;
    let down = format!("ws://127.0.0.1:{}", closed_port());
    let a = adapter(options(
        Some(format!("http://127.0.0.1:{}", closed_port())),
        vec![relay.url.clone(), down],
    ));
    let answer = ask(&a, 1, "echo").await;
    assert_eq!(storm_error(&answer), "identity_failed", "{answer}");
    assert!(fake.never_saw_the_key());
    assert!(fake.seen().iter().all(|s| s.path == "/v1/server/challenge"));
}

#[tokio::test]
async fn a_fast_public_relay_loses_to_a_slow_self_hosted_one() {
    let slow = FakeServer::new("self-hosted", pinned());
    let gate = Arc::new(Notify::new());
    *slow.gate.lock().unwrap() = Some(gate.clone());
    let fast = FakeServer::new("public", pinned());
    let self_hosted = FakeRelay::start("127.0.0.1", Hello::Ready, slow.clone()).await;
    let public = FakeRelay::start("localhost", Hello::Ready, fast.clone()).await;
    let mut o = options(None, vec![public.url.clone(), self_hosted.url.clone()]);
    o.public_hosts = vec!["localhost".into()];
    let a = Arc::new(adapter(o));

    let asked = tokio::spawn({
        let a = a.clone();
        async move { ask(&a, 1, "echo").await }
    });
    slow.saw(|s| s.path == "/v1/server/challenge").await;
    assert_eq!(public.hellos(), 0, "the public relay was dialled");
    *slow.gate.lock().unwrap() = None;
    gate.notify_one();
    let answer = within(asked).await.unwrap();
    assert_eq!(answer["result"]["via"], "self-hosted", "{answer}");
    assert_eq!(public.hellos(), 0, "the public relay was dialled");
}

#[tokio::test]
async fn the_public_relay_is_used_only_when_the_address_and_self_hosted_relays_fail() {
    let server = FakeServer::new("public", pinned());
    let absent = FakeServer::new("never", pinned());
    let self_hosted =
        FakeRelay::start("127.0.0.1", Hello::Refuse("server_unreachable"), absent).await;
    let public = FakeRelay::start("localhost", Hello::Ready, server.clone()).await;
    let mut o = options(
        Some(format!("http://127.0.0.1:{}", closed_port())),
        vec![public.url.clone(), self_hosted.url.clone()],
    );
    o.public_hosts = vec!["localhost".into()];
    let a = adapter(o);
    let answer = ask(&a, 1, "echo").await;
    assert_eq!(answer["result"]["via"], "public", "{answer}");
    assert_eq!(self_hosted.hellos(), 1);
}

#[tokio::test]
async fn server_offline_is_a_relay_without_the_server_and_relay_down_is_no_relay() {
    let absent = FakeServer::new("never", pinned());
    let without = FakeRelay::start(
        "127.0.0.1",
        Hello::Refuse("server_unreachable"),
        absent.clone(),
    )
    .await;
    let down = format!("ws://127.0.0.1:{}", closed_port());

    let a = adapter(options(None, vec![without.url.clone()]));
    assert_eq!(storm_error(&ask(&a, 1, "echo").await), "server_offline");

    let a = adapter(options(None, vec![down.clone()]));
    assert_eq!(storm_error(&ask(&a, 1, "echo").await), "relay_down");

    // One relay down and one up without the server: the server is what is
    // missing, not the network.
    let a = adapter(options(None, vec![down, without.url.clone()]));
    let answer = ask(&a, 1, "echo").await;
    assert_eq!(storm_error(&answer), "server_offline", "{answer}");
    assert!(absent.seen().is_empty());
}

#[tokio::test]
async fn a_trunk_lost_mid_call_is_one_error_never_resent_and_the_next_call_re_races() {
    let server = FakeServer::new("relayed", pinned());
    let relay = FakeRelay::start("127.0.0.1", Hello::Ready, server.clone()).await;
    let a = adapter(options(None, vec![relay.url.clone()]));

    let answer = ask(&a, 1, "dies").await;
    assert_eq!(answer["id"], 1);
    assert_eq!(storm_error(&answer), "trunk_lost", "{answer}");
    assert_eq!(server.calls("dies"), 1, "the call was resent");
    assert_eq!(relay.hellos(), 1);

    let answer = ask(&a, 2, "echo").await;
    assert_eq!(answer["result"]["via"], "relayed", "{answer}");
    assert_eq!(server.calls("dies"), 1, "the call was resent");
    assert_eq!(relay.hellos(), 2, "the next call did not re-race");
}

#[tokio::test]
async fn a_relayed_call_with_nothing_back_is_server_silent_and_not_resent() {
    let server = FakeServer::new("relayed", pinned());
    let relay = FakeRelay::start("127.0.0.1", Hello::Ready, server.clone()).await;
    let mut o = options(None, vec![relay.url.clone()]);
    o.idle = Duration::from_millis(300);
    let a = adapter(o);

    let answer = ask(&a, 1, "hangs").await;
    assert_eq!(storm_error(&answer), "server_silent", "{answer}");
    assert_eq!(server.calls("hangs"), 1);

    let answer = ask(&a, 2, "echo").await;
    assert_eq!(answer["result"]["via"], "relayed", "{answer}");
    assert_eq!(server.calls("hangs"), 1, "the call was resent");
    assert_eq!(
        relay.hellos(),
        1,
        "a silent request is not a lost transport (§8.7)"
    );
}

#[tokio::test]
async fn a_stream_scoped_error_leaves_the_trunk_for_the_next_call() {
    let server = FakeServer::new("relayed", pinned());
    let relay = FakeRelay::start("127.0.0.1", Hello::Ready, server.clone()).await;
    let a = adapter(options(None, vec![relay.url.clone()]));

    for (id, tool, code) in [
        (1, "refused", "rate_limited"),
        (2, "aborted", "stream_aborted"),
    ] {
        let answer = ask(&a, id, tool).await;
        assert_eq!(storm_error(&answer), code, "{answer}");
        assert_eq!(server.calls(tool), 1, "the call was resent");
    }
    let answer = ask(&a, 3, "echo").await;
    assert_eq!(answer["result"]["via"], "relayed", "{answer}");
    assert_eq!(relay.hellos(), 1, "a stream-scoped error dropped the trunk");
    assert_eq!(a.connection().status(), Status::Relayed);
}

#[tokio::test]
async fn trunk_superseded_naming_a_stream_re_races_at_once() {
    let server = FakeServer::new("relayed", pinned());
    let relay = FakeRelay::start("127.0.0.1", Hello::Ready, server.clone()).await;
    let a = adapter(options(None, vec![relay.url.clone()]));
    let answer = ask(&a, 1, "superseded").await;
    assert_eq!(storm_error(&answer), "trunk_superseded", "{answer}");
    let answer = ask(&a, 2, "echo").await;
    assert_eq!(answer["result"]["via"], "relayed", "{answer}");
    assert_eq!(relay.hellos(), 2);
}

#[tokio::test]
async fn with_the_server_down_messages_fail_from_the_backoff_and_spend_one_hello_per_relay() {
    let absent = FakeServer::new("never", pinned());
    let one = FakeRelay::start(
        "127.0.0.1",
        Hello::Refuse("server_unreachable"),
        absent.clone(),
    )
    .await;
    let two = FakeRelay::start(
        "127.0.0.1",
        Hello::Refuse("server_unreachable"),
        absent.clone(),
    )
    .await;
    let a = adapter(options(
        Some(format!("http://127.0.0.1:{}", closed_port())),
        vec![one.url.clone(), two.url.clone()],
    ));
    for id in 0..10 {
        let answer = ask(&a, id, "echo").await;
        assert_eq!(storm_error(&answer), "server_offline", "{answer}");
    }
    assert_eq!((one.hellos(), two.hellos()), (1, 1), "a message re-raced");
    assert_eq!(a.connection().status(), Status::ServerOffline);
}

#[tokio::test]
async fn a_refused_challenge_is_identity_failed_unproven_and_not_re_raced() {
    for (status, body) in [
        (
            200,
            json!({"server_id": SERVER, "signature": "bm90IGEgc2lnbmF0dXJl"}),
        ),
        (
            200,
            json!({"server_id": SERVER, "signature": BASE64URL_NOPAD.encode(&[0u8; 64])}),
        ),
        (404, json!({"error": "not found"})),
        (500, json!({})),
    ] {
        let fake = FakeServer::new("fake", pinned());
        *fake.refuse_challenge.lock().unwrap() = Some((status, body.clone()));
        let relay = FakeRelay::start("127.0.0.1", Hello::Ready, fake.clone()).await;
        let down = format!("ws://127.0.0.1:{}", closed_port());
        let a = adapter(options(None, vec![relay.url.clone(), down]));
        for id in 0..3 {
            let answer = ask(&a, id, "echo").await;
            assert_eq!(storm_error(&answer), "identity_failed", "{body}: {answer}");
        }
        assert_eq!(a.connection().status(), Status::Unproven);
        assert_eq!(relay.hellos(), 1, "{body}: re-raced per message");
        assert!(fake.never_saw_the_key());
    }
}

#[tokio::test]
async fn the_relay_set_is_cached_after_a_connect_and_used_by_the_next() {
    let server = FakeServer::new("relayed", pinned());
    let seed = FakeRelay::start("127.0.0.1", Hello::Ready, server.clone()).await;
    let learned = FakeRelay::start("127.0.0.1", Hello::Ready, server.clone()).await;
    *server.relays.lock().unwrap() = vec![format!("{}/", learned.url)];
    let dir = tempfile::tempdir().unwrap();
    let cache = dir.path().join("state/relays.json");

    let mut o = options(None, vec![seed.url.clone()]);
    o.cache = Some(cache.clone());
    let a = adapter(o);
    assert_eq!(ask(&a, 1, "echo").await["result"]["via"], "relayed");
    assert_eq!(read_cache(&cache, SERVER), vec![learned.url.clone()]);
    assert!(
        server
            .seen()
            .iter()
            .any(|s| s.path == "/v1/server" && s.header("authorization").is_none())
    );

    // No flags at all: only the cache knows a way.
    let mut o = options(None, vec![]);
    o.cache = Some(cache.clone());
    let a = adapter(o);
    assert_eq!(ask(&a, 2, "echo").await["result"]["via"], "relayed");
    assert_eq!(learned.hellos(), 1);
    assert_eq!(seed.hellos(), 1);
}

/// §4.3: adopted only from the proven winner, only when a `relays_sig` it
/// carries verifies against the pinned key, and never with a plaintext relay
/// off a private network.
#[tokio::test]
async fn a_relay_set_is_adopted_only_when_its_signature_verifies() {
    let server = FakeServer::new("relayed", pinned());
    let seed = FakeRelay::start("127.0.0.1", Hello::Ready, server.clone()).await;
    let advertised = vec![
        "wss://relay.example".to_string(),
        "ws://100.64.0.1:8080".to_string(),
        "ws://192.168.1.5:8080".to_string(),
    ];
    *server.relays.lock().unwrap() = advertised.clone();
    let urls: Vec<&str> = advertised.iter().map(String::as_str).collect();
    let signed_by = |key: &SigningKey| {
        BASE64URL_NOPAD.encode(&key.sign(&relay_set_message(SERVER, &urls)).to_bytes())
    };
    let dir = tempfile::tempdir().unwrap();
    let cache = dir.path().join("relays.json");
    let run = |cache: std::path::PathBuf, id: i64| {
        let mut o = options(None, vec![seed.url.clone()]);
        o.cache = Some(cache);
        let a = adapter(o);
        async move { ask(&a, id, "echo").await }
    };

    *server.relays_sig.lock().unwrap() = Some(signed_by(&impostor()));
    assert_eq!(run(cache.clone(), 1).await["result"]["via"], "relayed");
    assert!(
        read_cache(&cache, SERVER).is_empty(),
        "a set signed by another key was adopted"
    );

    *server.relays_sig.lock().unwrap() = Some(signed_by(&pinned()));
    assert_eq!(run(cache.clone(), 2).await["result"]["via"], "relayed");
    assert_eq!(
        read_cache(&cache, SERVER),
        vec!["wss://relay.example", "ws://192.168.1.5:8080"],
        "a public ws:// relay was cached"
    );

    *server.relays.lock().unwrap() = vec!["wss://other.example".into()];
    *server.relays_sig.lock().unwrap() = None;
    assert_eq!(run(cache.clone(), 3).await["result"]["via"], "relayed");
    assert_eq!(read_cache(&cache, SERVER), vec!["wss://other.example"]);
}

#[tokio::test]
async fn a_relayed_request_carries_one_host_and_it_is_one_rmcp_accepts() {
    let server = FakeServer::new("relayed", pinned());
    let relay = FakeRelay::start("127.0.0.1", Hello::Ready, server.clone()).await;
    let a = adapter(options(None, vec![relay.url.clone()]));
    assert_eq!(ask(&a, 1, "echo").await["result"]["via"], "relayed");
    let heads = relay.heads.lock().unwrap().clone();
    assert!(heads.len() >= 2, "{heads:?}");
    for head in heads {
        let hosts: Vec<&String> = head["headers"]
            .as_object()
            .unwrap()
            .keys()
            .filter(|k| k.eq_ignore_ascii_case("host"))
            .collect();
        assert_eq!(hosts.len(), 1, "{head}");
        assert_eq!(head["headers"][hosts[0].as_str()], "localhost", "{head}");
    }
}
