use std::path::Path as FsPath;

use axum::Router;
use tower::ServiceExt;

use super::*;
use crate::api::tests::{seed_owner, session_token, test_router_with_state};

const HOST: &str = "hst_01HB6V3Z7Q2M4N8P0R5S9T1W3X";

/// Fast enough for a test to see both lanes in a few seconds.
const QUICK: Pace = Pace {
    coalesce: Duration::from_millis(20),
    state: Duration::from_millis(400),
    output: Duration::from_millis(1500),
    keepalive: Duration::from_secs(15),
};

struct Fixture {
    app: Router,
    state: Shared,
    owner: String,
    bearer: String,
    _link: tokio::sync::mpsc::UnboundedReceiver<crate::agent::Envelope>,
}

async fn fixture(dir: &FsPath) -> Fixture {
    let (app, _, state) = test_router_with_state(dir);
    let (_generation, link) = state.agent.connect_host(HOST);
    state
        .agent
        .hello(
            HOST,
            serde_json::from_value(serde_json::json!({
                "providers": [{"id": "fake", "kind": "fake", "interactions": ["terminal"], "available": true}],
                "workspaces": ["storm"],
                "max_sessions": 8,
            }))
            .unwrap(),
        )
        .unwrap();
    let owner = seed_owner(&state).await;
    let bearer = format!("Bearer {}", session_token(&state, &owner).await);
    Fixture {
        app,
        state,
        owner,
        bearer,
        _link: link,
    }
}

impl Fixture {
    fn launch(&self) -> String {
        let id = self
            .state
            .agent
            .launch(
                &self.owner,
                serde_json::from_value(serde_json::json!({
                    "host_id": HOST, "workspace": "storm", "provider": "fake",
                    "terminal": {"cols": 80, "rows": 24},
                }))
                .unwrap(),
                Default::default(),
                Vec::new(),
            )
            .unwrap()
            .0
            .id;
        self.report(&id, serde_json::json!({"status": "running"}));
        id
    }

    fn report(&self, id: &str, status: serde_json::Value) {
        self.state
            .agent
            .host_status(HOST, id, &serde_json::from_value(status).unwrap())
            .unwrap();
    }

    async fn ask(
        &self,
        request: axum::http::Request<axum::body::Body>,
    ) -> axum::http::Response<axum::body::Body> {
        self.app.clone().oneshot(request).await.unwrap()
    }

    async fn open(&self) -> Reader {
        let response = self.ask(get("/v1/agent/events", Some(&self.bearer))).await;
        assert_eq!(response.status(), StatusCode::OK);
        Reader::new(response.into_body())
    }

    /// The feed itself, at `pace`, into a queue of `queue`.
    fn feed(&self, pace: Pace, queue: usize) -> mpsc::Receiver<Frame> {
        let changes = self.state.agent.subscribe_changes();
        let (tx, rx) = mpsc::channel(queue);
        tokio::spawn(feed(self.state.clone(), changes, tx, pace));
        rx
    }
}

fn get(path: &str, auth: Option<&str>) -> axum::http::Request<axum::body::Body> {
    let mut request = axum::http::Request::builder().uri(path);
    if let Some(auth) = auth {
        request = request.header("authorization", auth);
    }
    request.body(axum::body::Body::empty()).unwrap()
}

async fn json(response: axum::http::Response<axum::body::Body>) -> serde_json::Value {
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

#[derive(Debug)]
struct Event {
    name: String,
    data: serde_json::Value,
    /// The block as it was on the wire.
    raw: String,
}

struct Reader {
    body: std::pin::Pin<
        Box<dyn futures_util::Stream<Item = Result<axum::body::Bytes, axum::Error>> + Send>,
    >,
    buffer: String,
}

impl Reader {
    fn new(body: axum::body::Body) -> Self {
        Self {
            body: Box::pin(body.into_data_stream()),
            buffer: String::new(),
        }
    }

    /// The next event within `within`, skipping keepalives; `None` if none.
    async fn next(&mut self, within: Duration) -> Option<Event> {
        let until = Instant::now() + within;
        loop {
            if let Some(at) = self.buffer.find("\n\n") {
                let raw: String = self.buffer.drain(..at + 2).collect();
                let (mut name, mut data) = (String::new(), String::new());
                for line in raw.lines() {
                    if let Some(v) = line.strip_prefix("event: ") {
                        name = v.into();
                    } else if let Some(v) = line.strip_prefix("data:") {
                        data.push_str(v.trim_start());
                    }
                }
                if name.is_empty() {
                    continue;
                }
                let data = serde_json::from_str(&data).unwrap_or(serde_json::Value::Null);
                return Some(Event { name, data, raw });
            }
            let chunk =
                tokio::time::timeout_at(until, futures_util::StreamExt::next(&mut self.body))
                    .await
                    .ok()?
                    .expect("the stream ended")
                    .unwrap();
            self.buffer.push_str(std::str::from_utf8(&chunk).unwrap());
        }
    }

    async fn expect(&mut self, name: &str) -> Event {
        let event = self
            .next(Duration::from_secs(5))
            .await
            .unwrap_or_else(|| panic!("no {name} event"));
        assert_eq!(event.name, name, "{event:?}");
        event
    }
}

/// The next event a feed queued within `within`, skipping keepalives.
async fn next_frame(
    rx: &mut mpsc::Receiver<Frame>,
    within: Duration,
) -> Option<(&'static str, serde_json::Value)> {
    let until = Instant::now() + within;
    loop {
        match tokio::time::timeout_at(until, rx.recv()).await.ok()?? {
            Frame::Event { name, data } => {
                return Some((
                    name,
                    serde_json::from_str(&data).unwrap_or(serde_json::Value::Null),
                ));
            }
            Frame::Keepalive => continue,
        }
    }
}

#[tokio::test]
async fn the_events_stream_is_the_owners_alone() {
    let dir = tempdir::TempDir::new("storm-154-events-auth").unwrap();
    let f = fixture(dir.path()).await;

    for auth in [
        None,
        Some("Bearer stk_0123456789abcdef"),
        Some("Bearer sht_0123456789abcdef"),
    ] {
        let response = f.ask(get("/v1/agent/events", auth)).await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{auth:?}");
    }

    // An agent's or a key's identity never gets a stream, whatever routed it.
    for actor in [
        Actor::Agent {
            session_id: "ags_1".into(),
            host_id: HOST.into(),
            user_id: f.owner.clone(),
            write_vault: None,
        },
        Actor::Key {
            key_id: "key_1".into(),
            user_id: f.owner.clone(),
        },
    ] {
        let mut request = get("/v1/agent/events", None);
        request.extensions_mut().insert(actor);
        let response = agent_events(State(f.state.clone()), request).await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    let mut stream = f.open().await;
    stream.expect("snapshot").await;

    // A browser's one-use ticket, as the change feed takes it.
    let ticket = json(
        f.ask(
            axum::http::Request::builder()
                .method("POST")
                .uri("/v1/auth/ws-ticket")
                .header("authorization", &f.bearer)
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await,
    )
    .await["ticket"]
        .as_str()
        .unwrap()
        .to_string();
    let response = f
        .ask(get(&format!("/v1/agent/events?ticket={ticket}"), None))
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        "text/event-stream"
    );
    Reader::new(response.into_body()).expect("snapshot").await;
}

#[tokio::test]
async fn the_snapshot_is_the_session_list_with_no_id() {
    let dir = tempdir::TempDir::new("storm-154-events-snapshot").unwrap();
    let f = fixture(dir.path()).await;
    let a = f.launch();
    f.launch();
    f.state.agent.host_output(HOST, &a, 0, b"hello").unwrap();

    let mut stream = f.open().await;
    let snapshot = stream.expect("snapshot").await;
    let list = json(f.ask(get("/v1/agent/sessions", Some(&f.bearer))).await).await;
    assert_eq!(snapshot.data["sessions"], list);
    assert_eq!(list.as_array().unwrap().len(), 2);
    assert_eq!(snapshot.data["epoch"], f.state.agent.epoch());
    assert!(
        !snapshot.raw.lines().any(|l| l.starts_with("id:")),
        "{}",
        snapshot.raw
    );
    let a_view = list
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["id"] == a.as_str())
        .unwrap();
    assert_eq!(
        a_view["output_end"], 5,
        "the client's renumber check reads it"
    );

    // A reconnect is a fresh snapshot of the same run; Last-Event-ID is not
    // a cursor.
    let mut request = get("/v1/agent/events", Some(&f.bearer));
    request
        .headers_mut()
        .insert("last-event-id", "anything".parse().unwrap());
    let again = Reader::new(f.ask(request).await.into_body())
        .expect("snapshot")
        .await;
    assert_eq!(again.data["epoch"], snapshot.data["epoch"]);
    assert_eq!(again.data["sessions"], list);
}

/// A-154-4, at the shipped pace: each host report reaches the subscriber
/// within 1.2 s, as one `session` event carrying the whole view.
#[tokio::test]
async fn working_then_idle_is_one_session_event_each_within_1_2_s() {
    let dir = tempdir::TempDir::new("storm-154-events-status").unwrap();
    let f = fixture(dir.path()).await;
    let id = f.launch();
    let mut stream = f.open().await;
    stream.expect("snapshot").await;

    for activity in ["working", "idle"] {
        let sent = Instant::now();
        f.report(
            &id,
            serde_json::json!({"status": "running", "activity": activity}),
        );
        let event = stream
            .next(Duration::from_millis(1200))
            .await
            .unwrap_or_else(|| panic!("{activity} took over 1.2 s"));
        assert_eq!(event.name, "session");
        assert_eq!(event.data["id"], id.as_str());
        assert_eq!(event.data["activity"], activity);
        assert_eq!(event.data["status"], "running");
        assert!(event.data.get("output_end").is_some(), "{}", event.data);
        assert!(sent.elapsed() <= Duration::from_millis(1200));
    }
    let extra = stream.next(Duration::from_millis(1500)).await;
    assert!(extra.is_none(), "one event per change: {extra:?}");
}

#[tokio::test]
async fn a_burst_of_uploads_is_one_slow_lane_event_per_window_with_the_final_end() {
    let dir = tempdir::TempDir::new("storm-154-events-output").unwrap();
    let f = fixture(dir.path()).await;
    let id = f.launch();
    let mut rx = f.feed(QUICK, QUEUE);
    assert_eq!(
        next_frame(&mut rx, Duration::from_secs(5)).await.unwrap().0,
        "snapshot"
    );

    let started = Instant::now();
    let piece = [b'x'; 1024];
    for n in 0..50u64 {
        f.state
            .agent
            .host_output(HOST, &id, n * 1024, &piece)
            .unwrap();
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let mut events = Vec::new();
    while let Some(frame) = next_frame(&mut rx, QUICK.output + Duration::from_millis(800)).await {
        events.push((started.elapsed(), frame));
        if events.len() > 3 {
            break;
        }
    }
    assert_eq!(events.len(), 1, "{events:?}");
    let (at, (name, view)) = &events[0];
    assert_eq!(*name, "session");
    assert_eq!(view["output_end"], 50 * 1024);
    assert!(
        *at >= QUICK.output - Duration::from_millis(100),
        "held for the window: {at:?}"
    );
}

#[tokio::test]
async fn a_state_change_goes_at_once_and_then_at_most_once_a_window() {
    let dir = tempdir::TempDir::new("storm-154-events-lanes").unwrap();
    let f = fixture(dir.path()).await;
    let id = f.launch();
    let mut rx = f.feed(QUICK, QUEUE);
    next_frame(&mut rx, Duration::from_secs(5)).await.unwrap();

    let started = Instant::now();
    for title in ["one", "two", "three"] {
        f.report(
            &id,
            serde_json::json!({"status": "running", "title": title}),
        );
        tokio::time::sleep(Duration::from_millis(60)).await;
    }
    let (_, first) = next_frame(&mut rx, Duration::from_secs(2)).await.unwrap();
    assert!(started.elapsed() < QUICK.state, "the leading edge");
    assert_eq!(first["title"], "one");
    let (_, last) = next_frame(&mut rx, Duration::from_secs(2)).await.unwrap();
    assert!(started.elapsed() >= QUICK.state, "then the window");
    assert_eq!(last["title"], "three");
    assert!(next_frame(&mut rx, QUICK.state * 2).await.is_none());
}

#[tokio::test]
async fn an_unchanged_report_sends_nothing() {
    let dir = tempdir::TempDir::new("storm-154-events-dedup").unwrap();
    let f = fixture(dir.path()).await;
    let id = f.launch();
    f.report(
        &id,
        serde_json::json!({"status": "running", "activity": "idle"}),
    );
    let mut rx = f.feed(QUICK, QUEUE);
    next_frame(&mut rx, Duration::from_secs(5)).await.unwrap();

    for _ in 0..3 {
        f.report(
            &id,
            serde_json::json!({"status": "running", "activity": "idle"}),
        );
    }
    let sent = next_frame(&mut rx, QUICK.output + QUICK.state).await;
    assert!(sent.is_none(), "{sent:?}");
}

#[tokio::test]
async fn a_dismissed_session_is_removed() {
    let dir = tempdir::TempDir::new("storm-154-events-removed").unwrap();
    let f = fixture(dir.path()).await;
    let id = f.launch();
    let mut stream = f.open().await;
    stream.expect("snapshot").await;

    f.report(
        &id,
        serde_json::json!({"status": "completed", "exit_code": 0}),
    );
    let ended = stream.expect("session").await;
    assert_eq!(ended.data["status"], "completed");

    let response = f
        .ask(
            axum::http::Request::builder()
                .method("DELETE")
                .uri(format!("/v1/agent/sessions/{id}"))
                .header("authorization", &f.bearer)
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    let removed = stream.expect("removed").await;
    assert_eq!(removed.data, serde_json::json!({ "id": id }));
}

#[tokio::test]
async fn a_crew_move_reaches_the_stream() {
    let dir = tempdir::TempDir::new("storm-154-events-crew").unwrap();
    let f = fixture(dir.path()).await;
    let id = f.launch();
    let mut stream = f.open().await;
    stream.expect("snapshot").await;

    let actor = Actor::Session {
        user_id: f.owner.clone(),
    };
    let crew = crate::ops::create_crew(
        &f.state,
        &actor,
        serde_json::from_value(serde_json::json!({"name": "Crew", "session_ids": [id]})).unwrap(),
    )
    .await
    .unwrap_or_else(|e| panic!("{}", e.1))
    .crew
    .unwrap();
    let moved = stream.expect("session").await;
    assert_eq!(moved.data["crew"]["id"], crew.id.as_str());
}

#[tokio::test]
async fn a_reader_that_fell_behind_is_sent_resync_then_a_snapshot() {
    let dir = tempdir::TempDir::new("storm-154-events-resync").unwrap();
    let f = fixture(dir.path()).await;
    let (a, b) = (f.launch(), f.launch());
    let mut rx = f.feed(QUICK, 1);
    // Unread, the snapshot fills the queue; two changes then overflow it.
    tokio::time::sleep(Duration::from_millis(100)).await;
    f.report(&a, serde_json::json!({"status": "running", "title": "a"}));
    f.report(&b, serde_json::json!({"status": "running", "title": "b"}));
    tokio::time::sleep(Duration::from_millis(200)).await;

    let mut names = Vec::new();
    while let Some((name, data)) = next_frame(&mut rx, Duration::from_millis(500)).await {
        if name == "snapshot" && !names.is_empty() {
            let titles: Vec<_> = data["sessions"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v["title"].clone())
                .collect();
            assert!(titles.contains(&"a".into()) && titles.contains(&"b".into()));
        }
        names.push(name);
        if names.len() >= 3 {
            break;
        }
    }
    assert_eq!(names, ["snapshot", "resync", "snapshot"]);
}

/// The WebSocket branch, over a real socket: `{event, data}` text frames.
#[tokio::test]
async fn a_direct_upgrade_gets_json_event_frames() {
    use tokio_tungstenite::tungstenite::{Message as WsMessage, client::IntoClientRequest};

    let dir = tempdir::TempDir::new("storm-154-events-ws").unwrap();
    let f = fixture(dir.path()).await;
    let id = f.launch();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = f.app.clone();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .unwrap();
    });

    let mut request = format!("ws://{addr}/v1/agent/events")
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("authorization", f.bearer.parse().unwrap());
    let (mut socket, handshake) = tokio_tungstenite::connect_async(request).await.unwrap();
    assert_eq!(handshake.status(), StatusCode::SWITCHING_PROTOCOLS);

    let mut frame = async || loop {
        let message = tokio::time::timeout(
            Duration::from_secs(5),
            futures_util::StreamExt::next(&mut socket),
        )
        .await
        .expect("the socket sent nothing")
        .expect("the socket closed")
        .unwrap();
        if let WsMessage::Text(text) = message {
            return serde_json::from_str::<serde_json::Value>(&text).unwrap();
        }
    };
    let snapshot = frame().await;
    assert_eq!(snapshot["event"], "snapshot");
    assert_eq!(snapshot["data"]["sessions"][0]["id"], id.as_str());
    assert_eq!(snapshot.as_object().unwrap().len(), 2, "{snapshot}");

    f.report(
        &id,
        serde_json::json!({"status": "running", "activity": "working"}),
    );
    let session = frame().await;
    assert_eq!(session["event"], "session");
    assert_eq!(session["data"]["activity"], "working");

    server.abort();
}

#[test]
fn the_shipped_pace_is_decision_154s() {
    assert_eq!(PACE.state, Duration::from_secs(1));
    assert_eq!(PACE.output, Duration::from_secs(5));
    assert_eq!(PACE.keepalive, Duration::from_secs(15));
    assert_eq!(Frame::resync().sse(), super::super::SSE_RESYNC);
}
