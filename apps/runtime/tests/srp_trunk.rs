//! `SrpTrunk` against a fake relay that answers **on protocol events, never
//! on a timer**. A request is answered when its end-of-body frame arrives and
//! not before, so a client that forgets the frame hangs here exactly as it
//! hangs a real origin (P2, decision 92). A fake that replied a few
//! milliseconds after the head is how that defect hid for weeks.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use storm_runtime::srp::{SrpError, SrpTrunk, TrunkConfig};
use tokio::net::TcpListener;
use tokio::sync::Notify;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::server::{Request as WsRequest, Response};

const SERVER_ID: &str = "srv_test";
const SERVER_KEY: &str = "A6EHv_POEL4dcN0Y50vAmWfk1jCbpQ1fHdyGZBJVMbg";
const MAX_FRAME_PAYLOAD: usize = 256 * 1024;

/// Generous: nothing here waits on it unless the client under test is broken.
async fn within<T>(fut: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(20), fut)
        .await
        .expect("hung: the fake answers only on protocol events")
}

#[derive(Clone)]
enum Hello {
    Ready,
    Refuse(&'static str),
    Ignore,
}

#[derive(Clone)]
enum End {
    Close,
    /// What the real relay sends for an origin's `CLOSE {aborted: true}`.
    Error(&'static str),
    /// The flag itself, as a relay forwarding verbatim would.
    CloseAborted,
    /// The server's trunk died: `ERROR{trunk_lost, stream_id}`, then hang up.
    TrunkLost,
    /// Send nothing more, answer nothing, keep the socket open.
    Silent,
    /// Leave the stream open.
    Hang,
}

#[derive(Clone)]
struct Reply {
    /// 0 sends no head.
    status: u16,
    headers: Vec<(String, String)>,
    chunks: Vec<Vec<u8>>,
    end: End,
}

impl Reply {
    fn ok(body: &[u8]) -> Self {
        Self {
            status: 200,
            headers: vec![("content-type".into(), "application/json".into())],
            chunks: vec![body.to_vec()],
            end: End::Close,
        }
    }
}

#[derive(Debug, Clone)]
struct Req {
    stream_id: u32,
    method: String,
    path: String,
    head: Value,
    body: Vec<u8>,
}

type Handler = Arc<dyn Fn(&Req) -> Reply + Send + Sync>;

#[derive(Clone)]
struct Script {
    hello: Hello,
    refuse_opens: usize,
    /// Holds the first open until a second arrives, then refuses the
    /// second by its `attempt_id` (decision 106) before readying the first.
    refuse_second_by_id: bool,
    /// Requests are held until this many are complete, then answered with
    /// their frames interleaved — so cross-talk would show.
    batch: usize,
    /// `PING`s the relay sends before `READY` and again after it (§4.2: a
    /// relay may `PING` any trunk, and every `PING` is answered).
    pings: usize,
    /// Holds the first `STREAM_READY` until notified.
    late_ready: Option<Arc<Notify>>,
    handler: Handler,
}

impl Script {
    fn new(handler: impl Fn(&Req) -> Reply + Send + Sync + 'static) -> Self {
        Self {
            hello: Hello::Ready,
            refuse_opens: 0,
            refuse_second_by_id: false,
            batch: 1,
            pings: 0,
            late_ready: None,
            handler: Arc::new(handler),
        }
    }
}

#[derive(Default)]
struct Log {
    paths: Vec<String>,
    controls: Vec<Value>,
    /// Payload length of every `0x01` frame, per stream, in order.
    frames: HashMap<u32, Vec<usize>>,
    requests: Vec<Req>,
}

struct FakeRelay {
    url: String,
    log: Arc<Mutex<Log>>,
    changed: Arc<Notify>,
}

impl FakeRelay {
    async fn start(script: Script) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let log = Arc::new(Mutex::new(Log::default()));
        let changed = Arc::new(Notify::new());
        let (l, c) = (log.clone(), changed.clone());
        tokio::spawn(async move {
            while let Ok((tcp, _)) = listener.accept().await {
                tokio::spawn(serve(tcp, script.clone(), l.clone(), c.clone()));
            }
        });
        Self { url, log, changed }
    }

    fn log<T>(&self, f: impl FnOnce(&Log) -> T) -> T {
        f(&self.log.lock().unwrap())
    }

    /// Waits for a client control frame matching `pred`.
    async fn saw(&self, pred: impl Fn(&Value) -> bool) -> Value {
        within(async {
            loop {
                let notified = self.changed.notified();
                if let Some(v) = self.log(|l| l.controls.iter().find(|v| pred(v)).cloned()) {
                    return v;
                }
                notified.await;
            }
        })
        .await
    }
}

fn control(ty: &str, body: Value) -> Message {
    let mut v = body;
    v["v"] = json!(1);
    v["type"] = json!(ty);
    Message::text(v.to_string())
}

fn body_frame(stream_id: u32, payload: &[u8]) -> Message {
    let mut out = vec![0x02];
    out.extend_from_slice(&stream_id.to_be_bytes());
    out.extend_from_slice(payload);
    Message::Binary(out.into())
}

// Tungstenite's handshake callback fixes the closure's error type.
#[allow(clippy::result_large_err)]
async fn serve(
    tcp: tokio::net::TcpStream,
    script: Script,
    log: Arc<Mutex<Log>>,
    changed: Arc<Notify>,
) {
    let path_log = log.clone();
    let Ok(ws) =
        tokio_tungstenite::accept_hdr_async(tcp, move |req: &WsRequest, resp: Response| {
            path_log.lock().unwrap().paths.push(req.uri().to_string());
            Ok(resp)
        })
        .await
    else {
        return;
    };
    let (mut tx, mut rx) = ws.split();
    let mut silent = false;
    let mut refused = 0;
    let mut held: Option<Value> = None;
    let mut next_stream = 100u32;
    let mut heads: HashMap<u32, Value> = HashMap::new();
    let mut bodies: HashMap<u32, Vec<u8>> = HashMap::new();
    let mut complete: Vec<Req> = Vec::new();

    while let Some(Ok(message)) = rx.next().await {
        if silent {
            continue;
        }
        match message {
            Message::Text(text) => {
                let v: Value = serde_json::from_str(text.as_str()).unwrap();
                log.lock().unwrap().controls.push(v.clone());
                changed.notify_waiters();
                match v["type"].as_str().unwrap() {
                    "HELLO" => match &script.hello {
                        Hello::Ready => {
                            for _ in 0..script.pings {
                                let _ = tx.send(control("PING", json!({}))).await;
                            }
                            let _ = tx
                                .send(control("READY", json!({"client_trunk_id": "ctk_fake"})))
                                .await;
                            for _ in 0..script.pings {
                                let _ = tx.send(control("PING", json!({}))).await;
                            }
                        }
                        Hello::Refuse(code) => {
                            let _ = tx
                                .send(control("ERROR", json!({"code": code, "message": code})))
                                .await;
                            let _ = tx.close().await;
                            return;
                        }
                        Hello::Ignore => silent = true,
                    },
                    "OPEN_STREAM" if script.refuse_second_by_id && held.is_none() => {
                        held = Some(v["attempt_id"].clone());
                    }
                    "OPEN_STREAM" if script.refuse_second_by_id => {
                        let _ = tx
                            .send(control(
                                "ERROR",
                                json!({"code": "rate_limited", "message": "rate limited",
                                       "attempt_id": v["attempt_id"]}),
                            ))
                            .await;
                        next_stream += 1;
                        let _ = tx
                            .send(control(
                                "STREAM_READY",
                                json!({"attempt_id": held.take(), "stream_id": next_stream}),
                            ))
                            .await;
                    }
                    "OPEN_STREAM" if script.late_ready.is_some() && next_stream == 100 => {
                        script.late_ready.as_ref().unwrap().notified().await;
                        next_stream += 1;
                        let _ = tx
                            .send(control(
                                "STREAM_READY",
                                json!({"attempt_id": v["attempt_id"], "stream_id": next_stream}),
                            ))
                            .await;
                    }
                    "OPEN_STREAM" => {
                        if refused < script.refuse_opens {
                            refused += 1;
                            let _ = tx
                                .send(control(
                                    "ERROR",
                                    json!({"code": "rate_limited", "message": "rate limited"}),
                                ))
                                .await;
                        } else {
                            next_stream += 1;
                            let _ = tx
                                .send(control(
                                    "STREAM_READY",
                                    json!({"attempt_id": v["attempt_id"], "stream_id": next_stream}),
                                ))
                                .await;
                        }
                    }
                    "HTTP_REQUEST_HEAD" => {
                        heads.insert(v["stream_id"].as_u64().unwrap() as u32, v);
                    }
                    "PING" => {
                        let _ = tx.send(control("PONG", json!({}))).await;
                    }
                    _ => {}
                }
            }
            Message::Binary(frame) => {
                assert_eq!(frame[0], 0x01, "a client sends only request bodies");
                let stream_id = u32::from_be_bytes([frame[1], frame[2], frame[3], frame[4]]);
                let payload = &frame[5..];
                log.lock()
                    .unwrap()
                    .frames
                    .entry(stream_id)
                    .or_default()
                    .push(payload.len());
                changed.notify_waiters();
                if !payload.is_empty() {
                    bodies
                        .entry(stream_id)
                        .or_default()
                        .extend_from_slice(payload);
                    continue;
                }
                // End of body: the only event that answers a request.
                let head = heads
                    .remove(&stream_id)
                    .expect("end of body before its head");
                let req = Req {
                    stream_id,
                    method: head["method"].as_str().unwrap().to_string(),
                    path: head["path"].as_str().unwrap().to_string(),
                    head,
                    body: bodies.remove(&stream_id).unwrap_or_default(),
                };
                log.lock().unwrap().requests.push(req.clone());
                complete.push(req);
                if complete.len() < script.batch {
                    continue;
                }
                let batch: Vec<(Req, Reply)> = complete
                    .drain(..)
                    .rev()
                    .map(|r| {
                        let reply = (script.handler)(&r);
                        (r, reply)
                    })
                    .collect();
                if !answer(&mut tx, &batch).await {
                    silent = true;
                }
            }
            _ => {}
        }
    }
}

/// Heads, then chunks round-robin across streams, then the ends. Returns
/// false once the relay should go silent.
async fn answer(
    tx: &mut futures_util::stream::SplitSink<
        tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
        Message,
    >,
    batch: &[(Req, Reply)],
) -> bool {
    for (req, reply) in batch {
        if reply.status != 0 {
            let headers: serde_json::Map<String, Value> = reply
                .headers
                .iter()
                .map(|(k, v)| (k.clone(), json!(v)))
                .collect();
            let _ = tx
                .send(control(
                    "HTTP_RESPONSE_HEAD",
                    json!({"stream_id": req.stream_id, "status": reply.status, "headers": headers}),
                ))
                .await;
        }
    }
    let rounds = batch.iter().map(|(_, r)| r.chunks.len()).max().unwrap_or(0);
    for i in 0..rounds {
        for (req, reply) in batch {
            if let Some(chunk) = reply.chunks.get(i) {
                let _ = tx.send(body_frame(req.stream_id, chunk)).await;
            }
        }
    }
    for (req, reply) in batch {
        let id = req.stream_id;
        match reply.end {
            End::Close => {
                let _ = tx.send(control("CLOSE", json!({"stream_id": id}))).await;
            }
            End::CloseAborted => {
                let _ = tx
                    .send(control("CLOSE", json!({"stream_id": id, "aborted": true})))
                    .await;
            }
            End::Error(code) => {
                let _ = tx
                    .send(control(
                        "ERROR",
                        json!({"code": code, "message": code, "stream_id": id}),
                    ))
                    .await;
            }
            End::TrunkLost => {
                let _ = tx
                    .send(control(
                        "ERROR",
                        json!({"code": "trunk_lost", "message": "trunk lost", "stream_id": id}),
                    ))
                    .await;
                let _ = tx.close().await;
                return false;
            }
            End::Silent => return false,
            End::Hang => {}
        }
    }
    true
}

fn fast() -> TrunkConfig {
    TrunkConfig {
        handshake_timeout: Duration::from_secs(5),
        heartbeat: Duration::from_millis(50),
        silence_limit: Duration::from_millis(150),
    }
}

async fn connect(fake: &FakeRelay) -> SrpTrunk {
    within(SrpTrunk::connect_with(
        &fake.url,
        SERVER_ID,
        SERVER_KEY,
        TrunkConfig::default(),
    ))
    .await
    .expect("connect")
}

fn echo(req: &Req) -> Reply {
    Reply::ok(format!("{} {} {}", req.method, req.path, req.body.len()).as_bytes())
}

#[tokio::test]
async fn the_trunk_opens_with_hello_at_the_servers_connect_path() {
    let fake = FakeRelay::start(Script::new(echo)).await;
    let trunk = connect(&fake).await;
    assert_eq!(trunk.client_trunk_id(), "ctk_fake");
    // §4.3 and §5: the pinned key on the URL and in HELLO.
    assert_eq!(
        fake.log(|l| l.paths.clone()),
        vec![format!("/connect/srv_test?pubkey={SERVER_KEY}")]
    );
    let hello = fake.saw(|v| v["type"] == "HELLO").await;
    assert_eq!(hello["server_id"], SERVER_ID);
    assert_eq!(hello["pubkey"], SERVER_KEY);
    assert_eq!(hello["v"], 1);
}

#[tokio::test]
async fn every_request_ends_its_body_exactly_once_a_get_included() {
    let fake = FakeRelay::start(Script::new(echo)).await;
    let trunk = connect(&fake).await;

    let get = within(trunk.request("GET", "/v1/server?x=1", &[], Bytes::new()))
        .await
        .unwrap();
    assert_eq!(get.status, 200);
    assert_eq!(
        within(get.body.collect()).await.unwrap(),
        b"GET /v1/server?x=1 0"
    );

    let post = within(trunk.request(
        "POST",
        "/mcp",
        &[("authorization".into(), "Bearer stk_secret".into())],
        Bytes::from_static(b"{\"jsonrpc\":\"2.0\"}"),
    ))
    .await
    .unwrap();
    assert_eq!(within(post.body.collect()).await.unwrap(), b"POST /mcp 17");

    let (requests, frames) = fake.log(|l| (l.requests.clone(), l.frames.clone()));
    assert_eq!(
        frames[&requests[0].stream_id],
        vec![0],
        "a GET still ends its body"
    );
    assert_eq!(frames[&requests[1].stream_id], vec![17, 0]);
}

#[tokio::test]
async fn the_head_carries_the_credential_and_never_a_peer_ip() {
    let fake = FakeRelay::start(Script::new(echo)).await;
    let trunk = connect(&fake).await;
    let resp = within(trunk.request(
        "POST",
        "/mcp",
        &[
            ("authorization".into(), "Bearer stk_secret".into()),
            ("relay_peer_ip".into(), "6.6.6.6".into()),
        ],
        Bytes::from_static(b"{}"),
    ))
    .await
    .unwrap();
    assert_eq!(
        resp.headers,
        vec![("content-type".to_string(), "application/json".to_string())]
    );
    within(resp.body.collect()).await.unwrap();

    let head = fake.log(|l| l.requests[0].head.clone());
    assert!(head.get("relay_peer_ip").is_none());
    assert!(
        head["headers"].is_object(),
        "the relay forwards only an object"
    );
    assert_eq!(head["headers"]["authorization"], "Bearer stk_secret");
    assert!(head["headers"].get("relay_peer_ip").is_none());
    let hello = fake.saw(|v| v["type"] == "HELLO").await;
    assert!(
        !hello.to_string().contains("stk_secret"),
        "the relay is presented no credential (R12)"
    );
}

#[tokio::test]
async fn a_body_over_16_mib_goes_out_in_256_kib_frames_byte_exact() {
    let fake = FakeRelay::start(Script::new(|req: &Req| Reply {
        status: 200,
        headers: vec![],
        chunks: req
            .body
            .chunks(MAX_FRAME_PAYLOAD)
            .map(<[u8]>::to_vec)
            .collect(),
        end: End::Close,
    }))
    .await;
    let trunk = connect(&fake).await;
    let len: usize = 16 * 1024 * 1024 + 12_345;
    let body: Vec<u8> = (0..len).map(|i| (i * 31 % 251) as u8).collect();

    let resp = within(trunk.request("PUT", "/v1/attachments/x", &[], Bytes::from(body.clone())))
        .await
        .unwrap();
    let echoed = within(resp.body.collect()).await.unwrap();

    let (req, frames) = fake.log(|l| (l.requests[0].clone(), l.frames.clone()));
    let frames = &frames[&req.stream_id];
    assert_eq!(frames.len(), len.div_ceil(MAX_FRAME_PAYLOAD) + 1);
    assert!(frames.iter().all(|&n| n <= MAX_FRAME_PAYLOAD));
    assert_eq!(frames.iter().filter(|&&n| n == 0).count(), 1);
    assert_eq!(frames.last(), Some(&0), "end of body comes last");
    assert!(req.body == body, "the upload arrived byte-exact");
    assert!(echoed == body, "the download reassembled byte-exact");
}

#[tokio::test]
async fn concurrent_streams_do_not_cross_talk() {
    let mut script = Script::new(|req: &Req| {
        let n: u8 = req.path.trim_start_matches("/n/").parse().unwrap();
        Reply {
            status: 200 + u16::from(n),
            headers: vec![("x-n".into(), n.to_string())],
            chunks: (0..5).map(|i| vec![n; 1000 + i]).collect(),
            end: End::Close,
        }
    });
    script.batch = 8;
    let fake = FakeRelay::start(script).await;
    let trunk = connect(&fake).await;

    let tasks: Vec<_> = (0..8u8)
        .map(|n| {
            let trunk = trunk.clone();
            tokio::spawn(async move {
                let body = Bytes::from(vec![n; 3000]);
                let resp = trunk
                    .request("POST", &format!("/n/{n}"), &[], body)
                    .await
                    .unwrap();
                (
                    n,
                    resp.status,
                    resp.headers.clone(),
                    resp.body.collect().await.unwrap(),
                )
            })
        })
        .collect();
    for task in tasks {
        let (n, status, headers, body) = within(task).await.unwrap();
        assert_eq!(status, 200 + u16::from(n));
        assert_eq!(headers, vec![("x-n".to_string(), n.to_string())]);
        assert_eq!(body.len(), 1000 * 5 + 10);
        assert!(
            body.iter().all(|&b| b == n),
            "stream {n} got another stream's bytes"
        );
    }
    let requests = fake.log(|l| l.requests.clone());
    assert_eq!(requests.len(), 8);
    for req in requests {
        let n: u8 = req.path.trim_start_matches("/n/").parse().unwrap();
        assert!(req.body == vec![n; 3000], "request {n} arrived crossed");
    }
}

async fn an_aborted_response_fails_only_its_own_body(end: End, expected: SrpError) {
    let mut script = Script::new(move |req: &Req| match req.path.as_str() {
        "/abort" => Reply {
            status: 200,
            headers: vec![],
            chunks: vec![b"partial".to_vec()],
            end: end.clone(),
        },
        _ => Reply::ok(b"whole"),
    });
    script.batch = 2;
    let fake = FakeRelay::start(script).await;
    let trunk = connect(&fake).await;

    let (aborted, ok) = within(async {
        tokio::join!(
            trunk.request("GET", "/abort", &[], Bytes::new()),
            trunk.request("GET", "/ok", &[], Bytes::new()),
        )
    })
    .await;
    let mut aborted = aborted.unwrap();
    assert_eq!(
        aborted.status, 200,
        "the head came first; the failure is the body's"
    );
    assert_eq!(
        &within(aborted.body.chunk()).await.unwrap().unwrap()[..],
        b"partial"
    );
    assert_eq!(
        within(aborted.body.chunk()).await.unwrap().unwrap_err(),
        expected,
        "bytes already received are not a usable response"
    );
    assert!(within(aborted.body.chunk()).await.is_none());
    assert_eq!(within(ok.unwrap().body.collect()).await.unwrap(), b"whole");

    // The trunk carries on: a further pair is served.
    let (a, b) = within(async {
        tokio::join!(
            trunk.request("GET", "/ok", &[], Bytes::new()),
            trunk.request("GET", "/ok", &[], Bytes::new()),
        )
    })
    .await;
    assert_eq!(within(a.unwrap().body.collect()).await.unwrap(), b"whole");
    assert_eq!(within(b.unwrap().body.collect()).await.unwrap(), b"whole");
    assert!(!trunk.is_closed());
}

#[tokio::test]
async fn a_stream_aborted_error_fails_only_its_own_body() {
    an_aborted_response_fails_only_its_own_body(
        End::Error("stream_aborted"),
        SrpError::StreamAborted,
    )
    .await;
}

/// Decision 108c: a relay that cannot hold more for the origin may end one
/// stream `rate_limited`. Stream-scoped, so the trunk carries on.
#[tokio::test]
async fn a_stream_scoped_rate_limited_fails_only_its_own_body() {
    an_aborted_response_fails_only_its_own_body(End::Error("rate_limited"), SrpError::RateLimited)
        .await;
}

#[tokio::test]
async fn an_aborted_flag_on_close_is_a_body_error_too() {
    an_aborted_response_fails_only_its_own_body(End::CloseAborted, SrpError::StreamAborted).await;
}

#[tokio::test]
async fn a_silent_relay_is_relay_silent_and_its_streams_are_trunk_lost() {
    let mut script = Script::new(|req: &Req| match req.path.as_str() {
        "/mid-body" => Reply {
            status: 200,
            headers: vec![],
            chunks: vec![b"some".to_vec()],
            end: End::Hang,
        },
        _ => Reply {
            status: 0,
            headers: vec![],
            chunks: vec![],
            end: End::Silent,
        },
    });
    script.batch = 2;
    let fake = FakeRelay::start(script).await;
    let trunk = within(SrpTrunk::connect_with(
        &fake.url,
        SERVER_ID,
        SERVER_KEY,
        fast(),
    ))
    .await
    .unwrap();

    let (mid_body, no_head) = within(async {
        tokio::join!(
            trunk.request("GET", "/mid-body", &[], Bytes::new()),
            trunk.request("GET", "/no-head", &[], Bytes::new()),
        )
    })
    .await;
    assert_eq!(no_head.unwrap_err(), SrpError::TrunkLost);
    assert_eq!(
        within(mid_body.unwrap().body.collect()).await.unwrap_err(),
        SrpError::TrunkLost,
        "a body cut off by a dead trunk is an error, never a short body"
    );
    assert_eq!(within(trunk.closed()).await, SrpError::RelaySilent);
    assert_eq!(trunk.loss_reason().unwrap().code(), "relay_silent");
    assert_eq!(
        within(trunk.request("GET", "/after", &[], Bytes::new()))
            .await
            .unwrap_err(),
        SrpError::TrunkLost
    );
}

#[tokio::test]
async fn a_relay_that_answers_heartbeats_is_kept_past_the_silence_limit() {
    let fake = FakeRelay::start(Script::new(echo)).await;
    let trunk = within(SrpTrunk::connect_with(
        &fake.url,
        SERVER_ID,
        SERVER_KEY,
        fast(),
    ))
    .await
    .unwrap();
    // The test's own clock, not the fake's: the fake only ever answers PING.
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(!trunk.is_closed(), "{:?}", trunk.loss_reason());
    fake.saw(|v| v["type"] == "PING").await;
    let resp = within(trunk.request("GET", "/v1/health", &[], Bytes::new()))
        .await
        .unwrap();
    assert_eq!(
        within(resp.body.collect()).await.unwrap(),
        b"GET /v1/health 0"
    );
}

/// Decision 108c: a relay may `PING` a client trunk, before `READY` or after,
/// to learn how much it has read; the client answers every one.
#[tokio::test]
async fn every_relay_ping_is_answered_before_ready_and_after() {
    let mut script = Script::new(echo);
    script.pings = 3;
    let fake = FakeRelay::start(script).await;
    let _trunk = connect(&fake).await;
    within(async {
        loop {
            let notified = fake.changed.notified();
            if fake.log(|l| l.controls.iter().filter(|v| v["type"] == "PONG").count()) >= 6 {
                break;
            }
            notified.await;
        }
    })
    .await;
    assert_eq!(
        fake.log(|l| l.controls.iter().filter(|v| v["type"] == "PONG").count()),
        6
    );
}

#[tokio::test]
async fn server_unreachable_at_hello_is_server_offline() {
    let mut script = Script::new(echo);
    script.hello = Hello::Refuse("server_unreachable");
    let fake = FakeRelay::start(script).await;
    let err = within(SrpTrunk::connect(&fake.url, SERVER_ID, SERVER_KEY))
        .await
        .unwrap_err();
    assert_eq!(err, SrpError::ServerOffline);
    assert_eq!(err.code(), "server_offline");
}

#[tokio::test]
async fn a_rate_limited_hello_is_not_server_offline() {
    let mut script = Script::new(echo);
    script.hello = Hello::Refuse("rate_limited");
    let fake = FakeRelay::start(script).await;
    let err = within(SrpTrunk::connect(&fake.url, SERVER_ID, SERVER_KEY))
        .await
        .unwrap_err();
    assert_eq!(err.code(), "rate_limited");
}

#[tokio::test]
async fn a_relay_that_never_answers_hello_is_a_handshake_timeout() {
    let mut script = Script::new(echo);
    script.hello = Hello::Ignore;
    let fake = FakeRelay::start(script).await;
    let config = TrunkConfig {
        handshake_timeout: Duration::from_millis(300),
        ..TrunkConfig::default()
    };
    let err = within(SrpTrunk::connect_with(
        &fake.url, SERVER_ID, SERVER_KEY, config,
    ))
    .await
    .unwrap_err();
    assert_eq!(err.code(), "handshake_timeout");
}

#[tokio::test]
async fn a_relay_that_is_not_listening_is_relay_down() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    drop(listener);
    let err = within(SrpTrunk::connect(&url, SERVER_ID, SERVER_KEY))
        .await
        .unwrap_err();
    assert_eq!(err.code(), "relay_down");
}

#[tokio::test]
async fn a_public_relay_must_be_wss() {
    let err = SrpTrunk::connect("ws://relay.example.com", SERVER_ID, SERVER_KEY)
        .await
        .unwrap_err();
    assert_eq!(err.code(), "invalid_relay_target");
}

#[tokio::test]
async fn a_refused_open_is_rate_limited_and_the_trunk_survives() {
    let mut script = Script::new(echo);
    script.refuse_opens = 1;
    let fake = FakeRelay::start(script).await;
    let trunk = connect(&fake).await;

    let err = within(trunk.request("GET", "/one", &[], Bytes::new()))
        .await
        .unwrap_err();
    assert_eq!(err, SrpError::RateLimited);
    assert!(!trunk.is_closed());
    let resp = within(trunk.request("GET", "/two", &[], Bytes::new()))
        .await
        .unwrap();
    assert_eq!(within(resp.body.collect()).await.unwrap(), b"GET /two 0");
}

/// Decision 106: the refusal names its attempt, so it fails that open and
/// not the oldest one, which the FIFO inference would have.
#[tokio::test]
async fn a_refusal_naming_its_attempt_fails_that_open_only() {
    let mut script = Script::new(echo);
    script.refuse_second_by_id = true;
    let fake = FakeRelay::start(script).await;
    let trunk = connect(&fake).await;

    let (first, second) = within(async {
        tokio::join!(
            trunk.request("GET", "/first", &[], Bytes::new()),
            trunk.request("GET", "/second", &[], Bytes::new()),
        )
    })
    .await;
    assert_eq!(second.unwrap_err(), SrpError::RateLimited);
    let first = first.expect("the first open was failed for the second's refusal");
    assert_eq!(within(first.body.collect()).await.unwrap(), b"GET /first 0");
    assert!(!trunk.is_closed());
}

#[tokio::test]
async fn a_backlogged_client_gets_a_body_error_not_a_short_body() {
    an_aborted_response_fails_only_its_own_body(
        End::Error("client_backlogged"),
        SrpError::ClientBacklogged,
    )
    .await;
}

#[tokio::test]
async fn the_server_trunk_dying_ends_the_request_with_trunk_lost() {
    let fake = FakeRelay::start(Script::new(|_: &Req| Reply {
        status: 0,
        headers: vec![],
        chunks: vec![],
        end: End::TrunkLost,
    }))
    .await;
    let trunk = connect(&fake).await;
    let err = within(trunk.request("POST", "/mcp", &[], Bytes::from_static(b"{}")))
        .await
        .unwrap_err();
    assert_eq!(err.code(), "trunk_lost");
    assert_eq!(within(trunk.closed()).await, SrpError::TrunkLost);
}

/// §5.1: a `STREAM_READY` for an attempt nobody waits on any more is
/// answered with `CLOSE{stream_id}` at once, and the trunk carries on.
#[tokio::test]
async fn a_late_stream_ready_is_answered_with_close() {
    let release = Arc::new(Notify::new());
    let mut script = Script::new(echo);
    script.late_ready = Some(release.clone());
    let fake = FakeRelay::start(script).await;
    let trunk = connect(&fake).await;
    let t = trunk.clone();
    let abandoned =
        tokio::spawn(async move { t.request("GET", "/v1/server", &[], Bytes::new()).await });
    fake.saw(|v| v["type"] == "OPEN_STREAM").await;
    abandoned.abort();
    let _ = abandoned.await;
    release.notify_one();
    let close = fake.saw(|v| v["type"] == "CLOSE").await;
    assert_eq!(close["stream_id"], 101);
    let next = within(trunk.request("GET", "/v1/health", &[], Bytes::new()))
        .await
        .unwrap();
    assert_eq!(next.status, 200);
    assert!(!trunk.is_closed());
}

#[tokio::test]
async fn dropping_a_body_early_closes_its_stream_at_the_relay() {
    let fake = FakeRelay::start(Script::new(|_: &Req| Reply {
        status: 200,
        headers: vec![("content-type".into(), "text/event-stream".into())],
        chunks: vec![b"event: change\n\n".to_vec()],
        end: End::Hang,
    }))
    .await;
    let trunk = connect(&fake).await;
    let mut resp = within(trunk.request("GET", "/v1/stream", &[], Bytes::new()))
        .await
        .unwrap();
    assert_eq!(
        &within(resp.body.chunk()).await.unwrap().unwrap()[..],
        b"event: change\n\n",
        "a streamed body is readable before it ends"
    );
    let stream_id = fake.log(|l| l.requests[0].stream_id);
    drop(resp);
    fake.saw(|v| v["type"] == "CLOSE" && v["stream_id"] == stream_id)
        .await;
    assert!(!trunk.is_closed());
}

#[tokio::test]
async fn closing_the_trunk_says_close_and_fails_what_is_left() {
    let fake = FakeRelay::start(Script::new(echo)).await;
    let trunk = connect(&fake).await;
    trunk.close();
    fake.saw(|v| v["type"] == "CLOSE" && v.get("stream_id").is_none())
        .await;
    assert_eq!(trunk.loss_reason(), Some(SrpError::Closed));
    assert_eq!(
        trunk
            .request("GET", "/", &[], Bytes::new())
            .await
            .unwrap_err(),
        SrpError::TrunkLost
    );
}

/// A relay that answers `HELLO` and `OPEN_STREAM`, then stops reading the
/// socket until `resume` fires, and afterwards reports every frame it reads.
async fn stalling_relay(
    resume: Arc<Notify>,
) -> (String, tokio::sync::mpsc::UnboundedReceiver<Message>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let (seen, frames) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let ws = tokio_tungstenite::accept_async(tcp).await.unwrap();
        let (mut tx, mut rx) = ws.split();
        while let Some(Ok(message)) = rx.next().await {
            let Message::Text(text) = &message else {
                continue;
            };
            let v: Value = serde_json::from_str(text.as_str()).unwrap();
            match v["type"].as_str().unwrap() {
                "HELLO" => {
                    let _ = tx
                        .send(control("READY", json!({"client_trunk_id": "ctk_fake"})))
                        .await;
                }
                "OPEN_STREAM" => {
                    let _ = tx
                        .send(control(
                            "STREAM_READY",
                            json!({"attempt_id": v["attempt_id"], "stream_id": 7}),
                        ))
                        .await;
                    break;
                }
                _ => {}
            }
        }
        resume.notified().await;
        while let Some(Ok(message)) = rx.next().await {
            if let Message::Text(text) = &message
                && text.contains("\"PING\"")
            {
                let _ = tx.send(control("PONG", json!({}))).await;
            }
            if seen.send(message).is_err() {
                return;
            }
        }
    });
    (url, frames)
}

/// SRP §5.2 (decision 116b): a `PING` overtakes an upload already queued, and
/// the trunk is not judged silent while its own `PING` cannot be written. On
/// a slow uplink a 20 MiB upload used to end as `relay_silent`.
#[tokio::test]
async fn a_ping_overtakes_a_queued_upload_and_a_stalled_trunk_is_not_silent() {
    let resume = Arc::new(Notify::new());
    let (url, mut frames) = stalling_relay(resume.clone()).await;
    let config = TrunkConfig {
        handshake_timeout: Duration::from_secs(5),
        heartbeat: Duration::from_millis(200),
        silence_limit: Duration::from_millis(600),
    };
    let trunk = within(SrpTrunk::connect_with(&url, SERVER_ID, SERVER_KEY, config))
        .await
        .unwrap();

    let upload = tokio::spawn({
        let trunk = trunk.clone();
        async move {
            let _ = trunk
                .request(
                    "PUT",
                    "/v1/attachments/x",
                    &[],
                    Bytes::from(vec![7u8; 20 << 20]),
                )
                .await;
        }
    });

    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(
        !trunk.is_closed(),
        "judged silent while its own PING could not be written: {:?}",
        trunk.loss_reason()
    );

    resume.notify_one();
    let mut bodies = 0;
    loop {
        match within(frames.recv()).await.expect("relay gone") {
            Message::Text(text) if text.contains("\"PING\"") => break,
            Message::Binary(_) => bodies += 1,
            _ => {}
        }
    }
    assert!(bodies < 40, "the PING waited behind {bodies} body frames");
    assert!(!trunk.is_closed());
    upload.abort();
}
