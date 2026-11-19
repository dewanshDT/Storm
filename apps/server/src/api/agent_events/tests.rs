use std::path::Path as FsPath;

use axum::Router;
use tower::ServiceExt;

use super::*;
use crate::api::tests::{seed_owner, session_token, test_router_with_state};

fn put_json_with_auth(
    path: &str,
    body: serde_json::Value,
    auth: &str,
) -> axum::http::Request<axum::body::Body> {
    axum::http::Request::builder()
        .method("PUT")
        .uri(path)
        .header("content-type", "application/json")
        .header("authorization", auth)
        .body(axum::body::Body::from(body.to_string()))
        .unwrap()
}

fn post_json_with_auth(
    path: &str,
    body: serde_json::Value,
    auth: &str,
) -> axum::http::Request<axum::body::Body> {
    axum::http::Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", "application/json")
        .header("authorization", auth)
        .body(axum::body::Body::from(body.to_string()))
        .unwrap()
}

const HOST: &str = "hst_01HB6V3Z7Q2M4N8P0R5S9T1W3X";

/// Fast enough for a test to see both lanes in a few seconds.
///
/// The tests that time a lane run on paused time: the feed's waits are
/// tokio's, so they hold exactly however starved the runner is.
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
        let notify_changed = self.state.agent.watch_notifications();
        let (tx, rx) = mpsc::channel(queue);
        tokio::spawn(feed(self.state.clone(), changes, notify_changed, tx, pace));
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
#[tokio::test(start_paused = true)]
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

#[tokio::test(start_paused = true)]
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

#[tokio::test(start_paused = true)]
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

#[tokio::test(start_paused = true)]
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

/// A-155-4 at the shipped pace: a top-level session's program saying
/// `blocked kind=permission` reaches a subscriber on the state lane as
/// needs-you within 1.2 s, and its `idle` clears it the same way. The
/// record's `msg` and `title` are never kept or sent.
#[tokio::test(start_paused = true)]
async fn a_programs_blocked_is_pushed_at_once_and_its_words_never_are() {
    let dir = tempdir::TempDir::new("storm-155-events-program").unwrap();
    let f = fixture(dir.path()).await;
    let id = f.launch();
    let mut stream = f.open().await;
    let snapshot = stream.expect("snapshot").await;
    assert_eq!(
        snapshot.data["sessions"][0]["program_status"],
        serde_json::Value::Null
    );

    // "approve Bash: cat ~/.ssh/id_ed25519", base64 as OSC 7501 carries it.
    let words = "YXBwcm92ZSBCYXNoOiBjYXQgfi8uc3NoL2lkX2VkMjU1MTk=";
    let blocked = serde_json::json!({
        "status": "running", "title": "Claude Code", "activity": "idle",
        "activity_source": "program_status",
        "attention": "permission_prompt", "attention_source": "program_status",
        "program_status": {"state": "blocked", "kind": "permission", "app": "claude-code",
            "msg": words, "title": words, "progress": 40},
        "msg": words,
    });
    let sent = Instant::now();
    f.report(&id, blocked);
    let event = stream
        .next(Duration::from_millis(1200))
        .await
        .expect("blocked took over 1.2 s");
    assert!(sent.elapsed() <= Duration::from_millis(1200));
    assert_eq!(event.name, "session");
    assert_eq!(event.data["needs_attention"], "permission_prompt");
    assert_eq!(event.data["attention"]["kind"], "permission_prompt");
    assert_eq!(event.data["activity"], "idle");
    let program = &event.data["program_status"];
    assert_eq!(
        (&program["state"], &program["kind"]),
        (&"blocked".into(), &"permission".into())
    );
    assert_eq!(program["since"], event.data["attention"]["since"]);
    assert_eq!(program.as_object().unwrap().len(), 3, "{program}");

    let view = json(
        f.ask(get(&format!("/v1/agent/sessions/{id}"), Some(&f.bearer)))
            .await,
    )
    .await;
    assert_eq!(view["program_status"], *program);
    for text in [event.raw.clone(), view.to_string()] {
        assert!(!text.contains(words), "{text}");
        assert!(!text.contains("ssh"), "{text}");
    }

    f.report(
        &id,
        serde_json::json!({"status": "running", "title": "Claude Code", "activity": "idle",
            "activity_source": "program_status",
            "program_status": {"state": "idle", "app": "claude-code"}}),
    );
    let cleared = stream
        .next(Duration::from_millis(1200))
        .await
        .expect("idle took over 1.2 s");
    assert_eq!(cleared.data["needs_attention"], serde_json::Value::Null);
    assert_eq!(cleared.data["attention"], serde_json::Value::Null);
    assert_eq!(cleared.data["program_status"]["state"], "idle");
    assert!(cleared.data["program_status"].get("kind").is_none());
}

/// The same record reported again changes nothing a client sees.
#[tokio::test]
async fn an_unchanged_record_sends_nothing() {
    let dir = tempdir::TempDir::new("storm-155-events-dedup").unwrap();
    let f = fixture(dir.path()).await;
    let id = f.launch();
    let done = serde_json::json!({"status": "running", "activity": "idle",
        "program_status": {"state": "done", "app": "claude-code"}});
    f.report(&id, done.clone());
    let mut rx = f.feed(QUICK, QUEUE);
    let (_, snapshot) = next_frame(&mut rx, Duration::from_secs(5)).await.unwrap();
    assert_eq!(snapshot["sessions"][0]["program_status"]["state"], "done");

    tokio::time::sleep(Duration::from_millis(50)).await;
    for _ in 0..3 {
        f.report(&id, done.clone());
    }
    let sent = next_frame(&mut rx, QUICK.output + QUICK.state).await;
    assert!(sent.is_none(), "{sent:?}");
}

// ---- Decision 161, slice 161-1: notification/notification_read/prefs_changed ----

#[tokio::test]
async fn a_new_notice_reaches_a_live_subscriber() {
    let dir = tempdir::TempDir::new("storm-161-notify-live").unwrap();
    let f = fixture(dir.path()).await;
    let mut rx = f.feed(QUICK, QUEUE);
    next_frame(&mut rx, Duration::from_secs(5)).await.unwrap(); // snapshot

    let n = f.state.agent.send_test_notification(None).unwrap();
    let (name, data) = next_frame(&mut rx, Duration::from_secs(5))
        .await
        .expect("a notification frame");
    assert_eq!(name, "notification");
    assert_eq!(data["id"], n.id);
    assert_eq!(data["kind"], "test");
}

/// Read state is the inbox's, server-side (Q20): a read on one device
/// reaches every other subscriber on the stream.
#[tokio::test]
async fn a_read_syncs_to_a_second_subscriber() {
    let dir = tempdir::TempDir::new("storm-161-notify-read-sync").unwrap();
    let f = fixture(dir.path()).await;
    let mut a = f.feed(QUICK, QUEUE);
    let mut b = f.feed(QUICK, QUEUE);
    next_frame(&mut a, Duration::from_secs(5)).await.unwrap();
    next_frame(&mut b, Duration::from_secs(5)).await.unwrap();

    let n = f.state.agent.send_test_notification(None).unwrap();
    let (name_a, data_a) = next_frame(&mut a, Duration::from_secs(5)).await.unwrap();
    assert_eq!(name_a, "notification");
    assert_eq!(data_a["id"], n.id);
    let (name_b, _) = next_frame(&mut b, Duration::from_secs(5)).await.unwrap();
    assert_eq!(name_b, "notification");

    f.state
        .agent
        .mark_notifications_read(std::slice::from_ref(&n.id), None)
        .unwrap();

    let (name_a2, data_a2) = next_frame(&mut a, Duration::from_secs(5)).await.unwrap();
    assert_eq!(name_a2, "notification_read");
    assert_eq!(data_a2["ids"][0], n.id);
    let (name_b2, data_b2) = next_frame(&mut b, Duration::from_secs(5)).await.unwrap();
    assert_eq!(name_b2, "notification_read");
    assert_eq!(data_b2["ids"][0], n.id);
}

#[tokio::test]
async fn a_prefs_change_is_told_to_every_subscriber() {
    let dir = tempdir::TempDir::new("storm-161-notify-prefs").unwrap();
    let f = fixture(dir.path()).await;
    let mut rx = f.feed(QUICK, QUEUE);
    next_frame(&mut rx, Duration::from_secs(5)).await.unwrap();

    let before = f.state.agent.notification_prefs().unwrap();
    f.state
        .agent
        .update_notification_prefs(
            before.version,
            crate::agent::notifications::PrefsPatch {
                summary_on: Some(false),
                ..Default::default()
            },
        )
        .unwrap();

    let (name, data) = next_frame(&mut rx, Duration::from_secs(5))
        .await
        .expect("a prefs_changed frame");
    assert_eq!(name, "prefs_changed");
    assert_eq!(data["summary_on"], false);
}

/// Item 7: a row read before this subscriber's own scan ever saw it unread
/// (insert and read landing in the same wake) must still be
/// `notification_read`, never a `notification` frame carrying `read_at`.
#[tokio::test]
async fn a_read_before_ever_seen_unread_is_still_a_read() {
    let dir = tempdir::TempDir::new("storm-161-notify-read-before-seen").unwrap();
    let f = fixture(dir.path()).await;
    let mut rx = f.feed(QUICK, QUEUE);
    next_frame(&mut rx, Duration::from_secs(5)).await.unwrap();

    let n = f.state.agent.send_test_notification(None).unwrap();
    f.state
        .agent
        .mark_notifications_read(std::slice::from_ref(&n.id), None)
        .unwrap();

    let (name, data) = next_frame(&mut rx, Duration::from_secs(5))
        .await
        .expect("a notification_read frame");
    assert_eq!(name, "notification_read");
    assert_eq!(data["ids"][0], n.id);
    let extra = next_frame(&mut rx, Duration::from_millis(300)).await;
    assert!(extra.is_none(), "nothing else for this notice: {extra:?}");
}

fn agent_notify(id: &str, title: &str, key: &str) -> crate::agent::notifications::AgentNotify {
    crate::agent::notifications::AgentNotify {
        session_id: id.to_string(),
        provider: "fake".into(),
        session_title: "tidier".into(),
        title: title.into(),
        body: None,
        urgency: "normal",
        key: Some(key.to_string()),
        note_vault_id: None,
        note_id: None,
        due: None,
    }
}

/// A row this stream already sent as `notification` is never left for a
/// subscriber to keep once it is cancelled — by a `key` replace or by
/// `cancel_notify` — and a cancelled row is never in `GET /v1/notifications`
/// either, though it still exists for dedupe.
#[tokio::test]
async fn a_key_replace_tells_every_subscriber_the_old_notice_is_gone() {
    let dir = tempdir::TempDir::new("storm-161-notify-key-replace").unwrap();
    let f = fixture(dir.path()).await;
    let id = f.launch();
    let mut a = f.feed(QUICK, QUEUE);
    let mut b = f.feed(QUICK, QUEUE);
    next_frame(&mut a, Duration::from_secs(5)).await.unwrap();
    next_frame(&mut b, Duration::from_secs(5)).await.unwrap();

    let first = f
        .state
        .agent
        .agent_notify(agent_notify(&id, "first", "k"))
        .unwrap()
        .unwrap();
    let (name, data) = next_frame(&mut a, Duration::from_secs(5)).await.unwrap();
    assert_eq!(name, "notification");
    assert_eq!(data["id"], first.id);
    next_frame(&mut b, Duration::from_secs(5)).await.unwrap();

    let second = f
        .state
        .agent
        .agent_notify(agent_notify(&id, "second", "k"))
        .unwrap()
        .unwrap();
    assert_eq!(second.state, "replaced");

    for rx in [&mut a, &mut b] {
        let mut seen_new = false;
        let mut seen_removed = false;
        for _ in 0..2 {
            let (name, data) = next_frame(rx, Duration::from_secs(5))
                .await
                .expect("the new notice and the old one's removal");
            match name {
                "notification" => {
                    assert_eq!(data["id"], second.id);
                    seen_new = true;
                }
                "notification_removed" => {
                    assert_eq!(data["ids"], serde_json::json!([first.id]));
                    seen_removed = true;
                }
                other => panic!("unexpected frame: {other}"),
            }
        }
        assert!(seen_new && seen_removed);
    }

    let list = json(f.ask(get("/v1/notifications", Some(&f.bearer))).await).await;
    let ids: Vec<&str> = list["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["id"].as_str().unwrap())
        .collect();
    assert!(ids.contains(&second.id.as_str()), "{list}");
    assert!(!ids.contains(&first.id.as_str()), "{list}");
}

#[tokio::test]
async fn cancel_notify_of_a_live_notice_tells_every_subscriber_too() {
    let dir = tempdir::TempDir::new("storm-161-notify-cancel-live").unwrap();
    let f = fixture(dir.path()).await;
    let id = f.launch();
    let mut a = f.feed(QUICK, QUEUE);
    let mut b = f.feed(QUICK, QUEUE);
    next_frame(&mut a, Duration::from_secs(5)).await.unwrap();
    next_frame(&mut b, Duration::from_secs(5)).await.unwrap();

    let n = f
        .state
        .agent
        .agent_notify(agent_notify(&id, "hello", "k"))
        .unwrap()
        .unwrap();
    next_frame(&mut a, Duration::from_secs(5)).await.unwrap();
    next_frame(&mut b, Duration::from_secs(5)).await.unwrap();

    assert!(f.state.agent.cancel_agent_notify(&id, "k").unwrap());

    for rx in [&mut a, &mut b] {
        let (name, data) = next_frame(rx, Duration::from_secs(5))
            .await
            .expect("a notification_removed frame");
        assert_eq!(name, "notification_removed");
        assert_eq!(data["ids"], serde_json::json!([n.id]));
    }

    let list = json(f.ask(get("/v1/notifications", Some(&f.bearer))).await).await;
    let ids: Vec<&str> = list["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["id"].as_str().unwrap())
        .collect();
    assert!(!ids.contains(&n.id.as_str()), "{list}");
}

/// The "Notifies you" switch is a per-session write like any other
/// (161-2 F1): a flip reaches every client watching the session, not only
/// the one that made it, whether through `PUT …/notify` or the bulk
/// `POST …/sessions/notify-off`.
#[tokio::test]
async fn the_notify_switch_reaches_every_subscriber() {
    let dir = tempdir::TempDir::new("storm-161-notify-switch-push").unwrap();
    let f = fixture(dir.path()).await;
    let id = f.launch();
    let mut a = f.feed(QUICK, QUEUE);
    let mut b = f.feed(QUICK, QUEUE);
    next_frame(&mut a, Duration::from_secs(5)).await.unwrap();
    next_frame(&mut b, Duration::from_secs(5)).await.unwrap();

    // This fixture's launch grants no Storm connection, so notify starts
    // false (Q3: "never offered without Storm"); flip it on first.
    assert!(!f.state.agent.notify_enabled(&id));
    let response = f
        .ask(put_json_with_auth(
            &format!("/v1/agent/sessions/{id}/notify"),
            serde_json::json!({"enabled": true}),
            &f.bearer,
        ))
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    for rx in [&mut a, &mut b] {
        let (name, data) = next_frame(rx, Duration::from_secs(5))
            .await
            .expect("a session frame after PUT …/notify {enabled:true}");
        assert_eq!(name, "session");
        assert_eq!(data["id"], id);
        assert_eq!(data["notify"], true);
    }

    let response = f
        .ask(put_json_with_auth(
            &format!("/v1/agent/sessions/{id}/notify"),
            serde_json::json!({"enabled": false}),
            &f.bearer,
        ))
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    for rx in [&mut a, &mut b] {
        let (name, data) = next_frame(rx, Duration::from_secs(5))
            .await
            .expect("a session frame after PUT …/notify {enabled:false}");
        assert_eq!(name, "session");
        assert_eq!(data["id"], id);
        assert_eq!(data["notify"], false);
    }

    let response = f
        .ask(put_json_with_auth(
            &format!("/v1/agent/sessions/{id}/notify"),
            serde_json::json!({"enabled": true}),
            &f.bearer,
        ))
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    for rx in [&mut a, &mut b] {
        let (name, data) = next_frame(rx, Duration::from_secs(5))
            .await
            .expect("a session frame after PUT …/notify {enabled:true}, again");
        assert_eq!(name, "session");
        assert_eq!(data["id"], id);
        assert_eq!(data["notify"], true);
    }

    let response = f
        .ask(post_json_with_auth(
            "/v1/agent/sessions/notify-off",
            serde_json::json!({"scope": "all"}),
            &f.bearer,
        ))
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    for rx in [&mut a, &mut b] {
        let (name, data) = next_frame(rx, Duration::from_secs(5))
            .await
            .expect("a session frame after notify-off");
        assert_eq!(name, "session");
        assert_eq!(data["id"], id);
        assert_eq!(data["notify"], false);
    }
}

#[tokio::test]
async fn muting_is_told_to_every_subscriber() {
    let dir = tempdir::TempDir::new("storm-161-notify-mutes").unwrap();
    let f = fixture(dir.path()).await;
    let id = f.launch();
    let mut rx = f.feed(QUICK, QUEUE);
    next_frame(&mut rx, Duration::from_secs(5)).await.unwrap();

    f.state
        .agent
        .mute_notifications("session", &id, None, None)
        .unwrap();

    let (name, data) = next_frame(&mut rx, Duration::from_secs(5))
        .await
        .expect("a mutes_changed frame");
    assert_eq!(name, "mutes_changed");
    assert_eq!(data["mutes"][0]["target"], id);
}

#[tokio::test]
async fn clearing_the_inbox_is_told_to_every_subscriber() {
    let dir = tempdir::TempDir::new("storm-161-notify-cleared").unwrap();
    let f = fixture(dir.path()).await;
    let mut rx = f.feed(QUICK, QUEUE);
    next_frame(&mut rx, Duration::from_secs(5)).await.unwrap();

    f.state.agent.clear_notifications().unwrap();

    let (name, _) = next_frame(&mut rx, Duration::from_secs(5))
        .await
        .expect("a notifications_cleared frame");
    assert_eq!(name, "notifications_cleared");
}

/// 157g: the brightness a host says its client answered with reaches the
/// events, the session list and its view; a report without it keeps it, a
/// `hello` says it again, and anything but `light` or `dark` is ignored.
#[tokio::test]
async fn the_answered_brightness_round_trips_from_the_host() {
    let dir = tempdir::TempDir::new("storm-157g-answered").unwrap();
    let f = fixture(dir.path()).await;
    let id = f.launch();
    let mut stream = f.open().await;
    let snapshot = stream.expect("snapshot").await;
    assert!(
        snapshot.data["sessions"][0]
            .get("terminal_answered")
            .is_none()
    );

    f.report(
        &id,
        serde_json::json!({"status": "running", "terminal_answered": "light"}),
    );
    let event = stream
        .next(Duration::from_secs(5))
        .await
        .expect("no session event");
    assert_eq!(event.name, "session");
    assert_eq!(event.data["terminal_answered"], "light");

    f.report(
        &id,
        serde_json::json!({"status": "running", "terminal_answered": "mauve"}),
    );
    f.report(
        &id,
        serde_json::json!({"status": "running", "title": "Fix"}),
    );
    let list = json(f.ask(get("/v1/agent/sessions", Some(&f.bearer))).await).await;
    let listed = list["sessions"]
        .as_array()
        .or(list.as_array())
        .unwrap()
        .iter()
        .find(|s| s["id"] == id.as_str())
        .unwrap()
        .clone();
    assert_eq!(listed["terminal_answered"], "light");

    f.state
        .agent
        .hello(
            HOST,
            serde_json::from_value(serde_json::json!({
                "providers": [{"id": "fake", "kind": "fake", "interactions": ["terminal"], "available": true}],
                "workspaces": ["storm"],
                "max_sessions": 8,
                "sessions": [{"id": id, "status": "running", "terminal_answered": "dark",
                    "output_end_offset": 0}],
            }))
            .unwrap(),
        )
        .unwrap();
    let view = json(
        f.ask(get(&format!("/v1/agent/sessions/{id}"), Some(&f.bearer)))
            .await,
    )
    .await;
    assert_eq!(view["terminal_answered"], "dark");
}

// ---- Decision 161, slice 161-4: this device's own prefs ------------------

#[tokio::test]
async fn device_prefs_default_on_for_a_desktop_platform() {
    let dir = tempdir::TempDir::new("storm-161-4-device-default-desktop").unwrap();
    let f = fixture(dir.path()).await;
    let prefs = json(
        f.ask(get(
            "/v1/notifications/device?platform=linux",
            Some(&f.bearer),
        ))
        .await,
    )
    .await;
    assert_eq!(prefs["enabled"], true);
    assert_eq!(prefs["classes"], serde_json::json!([]));
    assert_eq!(prefs["preview"], "full");
}

#[tokio::test]
async fn device_prefs_default_off_for_web() {
    let dir = tempdir::TempDir::new("storm-161-4-device-default-web").unwrap();
    let f = fixture(dir.path()).await;
    let prefs = json(
        f.ask(get(
            "/v1/notifications/device?platform=web",
            Some(&f.bearer),
        ))
        .await,
    )
    .await;
    assert_eq!(prefs["enabled"], false);
}

#[tokio::test]
async fn device_prefs_put_patches_only_the_given_fields_and_persists() {
    let dir = tempdir::TempDir::new("storm-161-4-device-put").unwrap();
    let f = fixture(dir.path()).await;
    let created = json(
        f.ask(get(
            "/v1/notifications/device?platform=macos",
            Some(&f.bearer),
        ))
        .await,
    )
    .await;
    assert_eq!(created["enabled"], true);

    let response = f
        .ask(put_json_with_auth(
            "/v1/notifications/device",
            serde_json::json!({"enabled": false}),
            &f.bearer,
        ))
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    let patched = json(response).await;
    assert_eq!(patched["enabled"], false);
    assert_eq!(patched["preview"], "full", "untouched field stays as-is");

    // Persisted: a fresh GET (no platform — the row already exists) sees it.
    let reread = json(
        f.ask(get("/v1/notifications/device", Some(&f.bearer)))
            .await,
    )
    .await;
    assert_eq!(reread["enabled"], false);
}

#[tokio::test]
async fn device_prefs_put_rejects_an_unknown_class() {
    let dir = tempdir::TempDir::new("storm-161-4-device-bad-class").unwrap();
    let f = fixture(dir.path()).await;
    let response = f
        .ask(put_json_with_auth(
            "/v1/notifications/device",
            serde_json::json!({"classes": ["needs_you", "made_up"]}),
            &f.bearer,
        ))
        .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn device_prefs_put_rejects_an_unknown_preview_level() {
    let dir = tempdir::TempDir::new("storm-161-4-device-bad-preview").unwrap();
    let f = fixture(dir.path()).await;
    let response = f
        .ask(put_json_with_auth(
            "/v1/notifications/device",
            serde_json::json!({"preview": "everything"}),
            &f.bearer,
        ))
        .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn device_prefs_are_the_calling_devices_own() {
    let dir = tempdir::TempDir::new("storm-161-4-device-own").unwrap();
    let f = fixture(dir.path()).await;
    let (_device_b, bearer_b) = second_device(&f.state, &f.owner, "b").await;

    f.ask(put_json_with_auth(
        "/v1/notifications/device",
        serde_json::json!({"platform": "macos", "enabled": false}),
        &f.bearer,
    ))
    .await;
    // `platform` as a body field is ignored by this endpoint (it is a
    // query param); what matters here is that device B is untouched.
    let _ = f
        .ask(get(
            "/v1/notifications/device?platform=macos",
            Some(&f.bearer),
        ))
        .await;

    let b_prefs = json(
        f.ask(get(
            "/v1/notifications/device?platform=macos",
            Some(&bearer_b),
        ))
        .await,
    )
    .await;
    assert_eq!(
        b_prefs["enabled"], true,
        "device B's own first GET still gets the platform default"
    );
}

#[tokio::test]
async fn notification_devices_list_shows_every_paired_device() {
    let dir = tempdir::TempDir::new("storm-161-4-devices-list").unwrap();
    let f = fixture(dir.path()).await;
    let (device_b, bearer_b) = second_device(&f.state, &f.owner, "b").await;
    // Device A gets a row; device B never calls GET, so it has none yet.
    f.ask(get(
        "/v1/notifications/device?platform=macos",
        Some(&f.bearer),
    ))
    .await;

    let list = json(
        f.ask(get("/v1/notifications/devices", Some(&f.bearer)))
            .await,
    )
    .await;
    let rows = list.as_array().unwrap();
    let b_row = rows
        .iter()
        .find(|r| r["device_id"] == device_b)
        .expect("device B is listed even with no prefs row");
    assert!(b_row["enabled"].is_null());
    let _ = bearer_b;
}

// ---- Decision 161, slice 161-4: presence and the deliver hint -----------

/// A fresh device paired to the fixture's owner, for tests that need to
/// know a specific device's id — `session_token` mints one too, but never
/// hands the id back.
async fn second_device(state: &Shared, user_id: &str, name: &str) -> (String, String) {
    let now = crate::index::now_rfc3339();
    let mut auth_db = state.auth_db.lock().await;
    let device = crate::auth::devices::create_synthetic(&mut auth_db, name, &now).unwrap();
    let token = crate::auth::sessions::create(&mut auth_db, user_id, &device.id, &now)
        .unwrap()
        .access_token;
    (device.id, format!("Bearer {token}"))
}

async fn open_at(app: &Router, path: &str, bearer: &str) -> Reader {
    let response = app.clone().oneshot(get(path, Some(bearer))).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    Reader::new(response.into_body())
}

/// A device's own id in presence tests needs a unique connection id per
/// connection (review item 3): two tabs, or a reconnect's overlap, are
/// two different connections of the same device, and must stay that way.
async fn open_presence(
    app: &Router,
    bearer: &str,
    connection_id: &str,
    active: bool,
    open_session: Option<&str>,
) -> Reader {
    let presence = if active { "active" } else { "background" };
    let mut path = format!("/v1/agent/events?presence={presence}&connection_id={connection_id}");
    if let Some(id) = open_session {
        path.push_str(&format!("&open_session={id}"));
    }
    open_at(app, &path, bearer).await
}

#[tokio::test]
async fn deliver_hint_is_toast_for_the_one_active_device() {
    let dir = tempdir::TempDir::new("storm-161-4-deliver-toast").unwrap();
    let f = fixture(dir.path()).await;
    let id = f.launch();
    let (device_a, bearer_a) = second_device(&f.state, &f.owner, "a").await;
    let (_device_b, bearer_b) = second_device(&f.state, &f.owner, "b").await;

    let mut a = open_presence(&f.app, &bearer_a, "conn-a", true, None).await;
    a.expect("snapshot").await;
    let mut b = open_presence(&f.app, &bearer_b, "conn-b", false, None).await;
    b.expect("snapshot").await;

    let _ = f
        .state
        .agent
        .agent_notify(agent_notify(&id, "hi", "k"))
        .unwrap();

    let event_a = a.expect("notification").await;
    assert_eq!(event_a.data["deliver"], format!("toast:{device_a}"));
    let event_b = b.expect("notification").await;
    assert_eq!(
        event_b.data["deliver"], event_a.data["deliver"],
        "the same notice carries the same hint to every subscriber"
    );
}

#[tokio::test]
async fn deliver_hint_is_os_when_every_device_is_backgrounded() {
    let dir = tempdir::TempDir::new("storm-161-4-deliver-os").unwrap();
    let f = fixture(dir.path()).await;
    let id = f.launch();
    let (_device_a, bearer_a) = second_device(&f.state, &f.owner, "a").await;
    let (_device_b, bearer_b) = second_device(&f.state, &f.owner, "b").await;

    let mut a = open_presence(&f.app, &bearer_a, "conn-a", false, None).await;
    a.expect("snapshot").await;
    let mut b = open_presence(&f.app, &bearer_b, "conn-b", false, None).await;
    b.expect("snapshot").await;

    let _ = f
        .state
        .agent
        .agent_notify(agent_notify(&id, "hi", "k"))
        .unwrap();

    let event_a = a.expect("notification").await;
    assert_eq!(event_a.data["deliver"], "os");
    let event_b = b.expect("notification").await;
    assert_eq!(event_b.data["deliver"], "os");
}

#[tokio::test]
async fn deliver_hint_is_null_when_an_active_device_has_that_session_open() {
    let dir = tempdir::TempDir::new("storm-161-4-deliver-open").unwrap();
    let f = fixture(dir.path()).await;
    let id = f.launch();
    let (_device_a, bearer_a) = second_device(&f.state, &f.owner, "a").await;
    let (_device_b, bearer_b) = second_device(&f.state, &f.owner, "b").await;

    let mut a = open_presence(&f.app, &bearer_a, "conn-a", true, Some(&id)).await;
    a.expect("snapshot").await;
    let mut b = open_presence(&f.app, &bearer_b, "conn-b", false, None).await;
    b.expect("snapshot").await;

    let _ = f
        .state
        .agent
        .agent_notify(agent_notify(&id, "hi", "k"))
        .unwrap();

    // The key is always present — even here — so the client can tell a
    // 161-4 server's deliberate "nothing" from an older server that never
    // sent the field at all.
    let event_a = a.expect("notification").await;
    assert!(
        event_a.data.get("deliver").is_some_and(|v| v.is_null()),
        "the device already showing this session gets a null hint, not a missing key: {:?}",
        event_a.data
    );
    let event_b = b.expect("notification").await;
    assert!(
        event_b.data.get("deliver").is_some_and(|v| v.is_null()),
        "nobody else is told to alert either: {:?}",
        event_b.data
    );
}

#[tokio::test]
async fn deliver_hint_with_two_active_devices_picks_the_lower_id_deterministically() {
    let dir = tempdir::TempDir::new("storm-161-4-deliver-two-active").unwrap();
    let f = fixture(dir.path()).await;
    let id = f.launch();
    let (device_a, bearer_a) = second_device(&f.state, &f.owner, "a").await;
    let (device_b, bearer_b) = second_device(&f.state, &f.owner, "b").await;
    let expected = format!("toast:{}", device_a.min(device_b));

    let mut a = open_presence(&f.app, &bearer_a, "conn-a", true, None).await;
    a.expect("snapshot").await;
    let mut b = open_presence(&f.app, &bearer_b, "conn-b", true, None).await;
    b.expect("snapshot").await;

    let _ = f
        .state
        .agent
        .agent_notify(agent_notify(&id, "hi", "k"))
        .unwrap();

    let event_a = a.expect("notification").await;
    assert_eq!(event_a.data["deliver"], expected);
    let event_b = b.expect("notification").await;
    assert_eq!(event_b.data["deliver"], expected);
}

/// Review item 3: a device active on *any* one of its connections counts
/// as active, even while another of its own connections is backgrounded.
#[tokio::test]
async fn a_device_is_active_if_any_of_its_connections_is() {
    let dir = tempdir::TempDir::new("storm-161-4-deliver-multi-conn").unwrap();
    let f = fixture(dir.path()).await;
    let id = f.launch();
    let (device_a, bearer_a) = second_device(&f.state, &f.owner, "a").await;

    let mut tab1 = open_presence(&f.app, &bearer_a, "conn-1", false, None).await;
    tab1.expect("snapshot").await;
    let mut tab2 = open_presence(&f.app, &bearer_a, "conn-2", true, None).await;
    tab2.expect("snapshot").await;

    let _ = f
        .state
        .agent
        .agent_notify(agent_notify(&id, "hi", "k"))
        .unwrap();

    let event = tab1.expect("notification").await;
    assert_eq!(event.data["deliver"], format!("toast:{device_a}"));
}

/// Review item 3: an old connection's drop must never clear a newer
/// connection's presence (the overlap during a reconnect).
#[tokio::test]
async fn an_overlapping_reconnect_does_not_clear_the_newer_connection() {
    let dir = tempdir::TempDir::new("storm-161-4-deliver-overlap").unwrap();
    let f = fixture(dir.path()).await;
    let id = f.launch();
    let (device_a, bearer_a) = second_device(&f.state, &f.owner, "a").await;

    let mut old = open_presence(&f.app, &bearer_a, "conn-old", true, None).await;
    old.expect("snapshot").await;
    let mut fresh = open_presence(&f.app, &bearer_a, "conn-new", true, None).await;
    fresh.expect("snapshot").await;

    drop(old); // the stale connection from before a reconnect, ending late

    let _ = f
        .state
        .agent
        .agent_notify(agent_notify(&id, "hi", "k"))
        .unwrap();
    let event = fresh.expect("notification").await;
    assert_eq!(
        event.data["deliver"],
        format!("toast:{device_a}"),
        "conn-new's presence must survive conn-old's drop"
    );
}

#[tokio::test]
async fn presence_is_forgotten_when_the_stream_closes() {
    let dir = tempdir::TempDir::new("storm-161-4-deliver-forget").unwrap();
    let f = fixture(dir.path()).await;
    let (_device_a, bearer_a) = second_device(&f.state, &f.owner, "a").await;

    {
        let mut a = open_presence(&f.app, &bearer_a, "conn-a", true, None).await;
        a.expect("snapshot").await;
    } // dropped: the SSE body stream (and with it `feed`) ends here.

    // `feed`'s drop runs on its own task (dropping `rx` resolves its
    // `tx.closed()` branch almost at once); give it a beat to run
    // `PresenceGuard::drop`.
    for _ in 0..50 {
        if f.state.agent.deliver_hint_uncached(None) == Some("os".to_string()) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("presence was never cleared after the stream closed");
}

#[tokio::test]
async fn post_presence_updates_an_existing_connection() {
    let dir = tempdir::TempDir::new("storm-161-4-presence-post").unwrap();
    let f = fixture(dir.path()).await;
    let id = f.launch();
    let (device_a, bearer_a) = second_device(&f.state, &f.owner, "a").await;
    let (_device_b, bearer_b) = second_device(&f.state, &f.owner, "b").await;

    // Connects active, then goes background by POST — never by reconnecting.
    let mut a = open_presence(&f.app, &bearer_a, "conn-a", true, None).await;
    a.expect("snapshot").await;
    let mut b = open_presence(&f.app, &bearer_b, "conn-b", false, None).await;
    b.expect("snapshot").await;

    let response = f
        .ask(post_json_with_auth(
            "/v1/notifications/presence",
            serde_json::json!({"connection_id": "conn-a", "state": "background"}),
            &bearer_a,
        ))
        .await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    let _ = f
        .state
        .agent
        .agent_notify(agent_notify(&id, "hi", "k"))
        .unwrap();
    let event_a = a.expect("notification").await;
    assert_eq!(
        event_a.data["deliver"], "os",
        "conn-a is background now, so nobody is active: {:?}",
        event_a.data
    );
    let _ = device_a;
    let _ = b;
}

#[tokio::test]
async fn post_presence_404s_for_a_connection_that_never_connected() {
    let dir = tempdir::TempDir::new("storm-161-4-presence-post-404").unwrap();
    let f = fixture(dir.path()).await;
    let response = f
        .ask(post_json_with_auth(
            "/v1/notifications/presence",
            serde_json::json!({"connection_id": "never-opened", "state": "active"}),
            &f.bearer,
        ))
        .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn post_presence_404s_once_the_connection_has_closed() {
    let dir = tempdir::TempDir::new("storm-161-4-presence-post-closed").unwrap();
    let f = fixture(dir.path()).await;
    let (_device_a, bearer_a) = second_device(&f.state, &f.owner, "a").await;

    {
        let mut a = open_presence(&f.app, &bearer_a, "conn-a", true, None).await;
        a.expect("snapshot").await;
    } // dropped: conn-a's presence is cleared once `feed` notices.

    for _ in 0..50 {
        if f.state.agent.deliver_hint_uncached(None) == Some("os".to_string()) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let response = f
        .ask(post_json_with_auth(
            "/v1/notifications/presence",
            serde_json::json!({"connection_id": "conn-a", "state": "active"}),
            &bearer_a,
        ))
        .await;
    assert_eq!(
        response.status(),
        StatusCode::NOT_FOUND,
        "a stale POST must never resurrect a dead connection's presence"
    );
}

#[tokio::test]
async fn presence_requires_an_actual_session_actor() {
    let dir = tempdir::TempDir::new("storm-161-4-presence-mcp-refused").unwrap();
    let f = fixture(dir.path()).await;
    // No credential at all is refused before `crews_owner` is even
    // reached; `crews_owner` itself (exercised by every other
    // `/v1/notifications/*` route already) refuses a key or an agent
    // actor the same way.
    let request = axum::http::Request::builder()
        .method("POST")
        .uri("/v1/notifications/presence")
        .header("content-type", "application/json")
        .body(axum::body::Body::from(
            serde_json::json!({"connection_id": "conn-x", "state": "active"}).to_string(),
        ))
        .unwrap();
    let response = f.ask(request).await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

/// Review item 16: a key or an agent actor is refused by `crews_owner`
/// on every `/v1/notifications/*` route, device prefs and presence
/// included — called directly here since crafting an `stk_`/agent
/// bearer the real middleware accepts is this suite's job elsewhere,
/// not this one's.
#[tokio::test]
async fn device_prefs_and_presence_ops_refuse_a_key_or_agent_actor() {
    let dir = tempdir::TempDir::new("storm-161-4-device-presence-actor-refused").unwrap();
    let f = fixture(dir.path()).await;
    let bad_actors = [
        Actor::Key {
            key_id: "key_1".into(),
            user_id: f.owner.clone(),
        },
        Actor::Agent {
            session_id: "ags_1".into(),
            host_id: HOST.into(),
            user_id: f.owner.clone(),
            write_vault: None,
        },
    ];
    for actor in bad_actors {
        assert!(
            crate::ops::get_device_prefs(&f.state, &actor, Some("dev_x"), None)
                .await
                .is_err()
        );
        assert!(
            crate::ops::put_device_prefs(
                &f.state,
                &actor,
                Some("dev_x"),
                None,
                crate::ops::DevicePrefsPatch {
                    enabled: Some(false),
                    classes: None,
                    preview: None,
                },
            )
            .await
            .is_err(),
            "review item I: PUT must refuse a key or agent actor too"
        );
        assert!(
            crate::ops::list_device_notification_rows(&f.state, &actor)
                .await
                .is_err()
        );
        assert!(
            crate::ops::set_presence(&f.state, &actor, Some("dev_x"), "conn-x", "active", None)
                .is_err()
        );
    }
}

/// Review item 16: a notice with no session at all still gets a deliver
/// hint through the live frame (the `os`/`toast` branches never assumed
/// one; only the "showing that session" skip does).
#[tokio::test]
async fn a_sessionless_notice_still_carries_a_deliver_hint() {
    let dir = tempdir::TempDir::new("storm-161-4-deliver-no-session").unwrap();
    let f = fixture(dir.path()).await;
    let (_device_a, bearer_a) = second_device(&f.state, &f.owner, "a").await;

    let mut a = open_presence(&f.app, &bearer_a, "conn-a", false, None).await;
    a.expect("snapshot").await;

    let n = f.state.agent.send_test_notification(None).unwrap();
    let event = a.expect("notification").await;
    assert_eq!(event.data["id"], n.id);
    assert_eq!(event.data["deliver"], "os");
}

/// Review item D: revoking a device clears its presence at once, so it
/// never counts toward a deliver hint computed after — not only once its
/// own dead stream notices and runs `PresenceGuard::drop`.
#[tokio::test]
async fn revoking_a_device_clears_its_presence() {
    let dir = tempdir::TempDir::new("storm-161-4-revoke-clears-presence").unwrap();
    let f = fixture(dir.path()).await;
    let (device_a, bearer_a) = second_device(&f.state, &f.owner, "a").await;

    let mut a = open_presence(&f.app, &bearer_a, "conn-a", true, None).await;
    a.expect("snapshot").await;
    assert_eq!(
        f.state.agent.deliver_hint_uncached(None),
        Some(format!("toast:{device_a}")),
        "device a is active before it is revoked"
    );

    let response = f
        .ask(
            axum::http::Request::builder()
                .method("DELETE")
                .uri(format!("/v1/auth/devices/{device_a}"))
                .header("authorization", &f.bearer)
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    assert_eq!(
        f.state.agent.deliver_hint_uncached(None),
        Some("os".to_string()),
        "a's presence must not survive its own revoke"
    );

    // The device-prefs row (auth/devices.rs's own cascade) is still
    // covered separately; this is the API-level half review item D also
    // asks for — the device row itself is really gone.
    let devices = json(f.ask(get("/v1/auth/devices", Some(&f.bearer))).await).await;
    let row = devices
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["id"] == device_a)
        .expect("the revoked device is still listed, just marked revoked");
    assert!(row["revoked"].is_string());
}

// ---- 161h: delivery hardening (a bug-review audit of decision 161) -------

/// Item 1: a presence entry whose `last_seen` is past
/// `PRESENCE_IDLE_TIMEOUT` must be excluded from the deliver hint, even
/// though its `active` flag is still `true` — this is what actually fixes
/// a dead SSE connection, since nothing else ever notices one has gone
/// quiet.
#[tokio::test(start_paused = true)]
async fn a_stale_presence_entry_is_excluded_even_though_active_is_still_true() {
    let dir = tempdir::TempDir::new("storm-161h-presence-stale").unwrap();
    let f = fixture(dir.path()).await;
    let (device_a, bearer_a) = second_device(&f.state, &f.owner, "a").await;

    let mut a = open_presence(&f.app, &bearer_a, "conn-a", true, None).await;
    a.expect("snapshot").await;
    assert_eq!(
        f.state.agent.deliver_hint_uncached(None),
        Some(format!("toast:{device_a}")),
        "fresh presence still toasts"
    );

    // No heartbeat at all for longer than the TTL — an old client, or a
    // socket that is simply dead.
    tokio::time::advance(Duration::from_secs(46)).await;

    assert_eq!(
        f.state.agent.deliver_hint_uncached(None),
        Some("os".to_string()),
        "stale past the TTL must not be trusted, whatever `active` says"
    );
}

/// The same staleness check, but refreshed by the SSE heartbeat
/// (`POST …/presence`) just like a real client's periodic call would —
/// proving the fix is the staleness check itself, not merely the absence
/// of a heartbeat.
#[tokio::test(start_paused = true)]
async fn a_heartbeat_keeps_sse_presence_fresh_past_what_a_single_gap_would_allow() {
    let dir = tempdir::TempDir::new("storm-161h-presence-heartbeat").unwrap();
    let f = fixture(dir.path()).await;
    let (device_a, bearer_a) = second_device(&f.state, &f.owner, "a").await;

    let mut a = open_presence(&f.app, &bearer_a, "conn-a", true, None).await;
    a.expect("snapshot").await;

    tokio::time::advance(Duration::from_secs(20)).await;
    let response = f
        .ask(post_json_with_auth(
            "/v1/notifications/presence",
            serde_json::json!({"connection_id": "conn-a", "state": "active"}),
            &bearer_a,
        ))
        .await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    tokio::time::advance(Duration::from_secs(40)).await; // 60s since connect, 40s since the heartbeat
    assert_eq!(
        f.state.agent.deliver_hint_uncached(None),
        Some(format!("toast:{device_a}")),
        "the heartbeat 20s ago refreshed last_seen, so only 40s have really passed"
    );

    tokio::time::advance(Duration::from_secs(10)).await; // 50s since the heartbeat: past the TTL
    assert_eq!(
        f.state.agent.deliver_hint_uncached(None),
        Some("os".to_string())
    );
}

/// Item 1's WS half: `pump_ws` must close a connection that sends nothing
/// inbound (no frame of any kind — a dead socket never even answers a
/// `Ping` with a `Pong`) within its idle bound. Exercised over a real
/// WebSocket (through a minimal upgrade route, not the full
/// `/v1/agent/events` handler — the production bound is 45s, far too slow
/// for a unit test, so this drives `pump_ws` directly with a short one).
#[tokio::test]
async fn pump_ws_closes_a_connection_that_never_sends_anything_inbound() {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;

    async fn upgrade(ws: axum::extract::ws::WebSocketUpgrade) -> axum::response::Response {
        ws.on_upgrade(|socket| async move {
            let (_tx, rx) = mpsc::channel::<Frame>(1);
            pump_ws(socket, rx, Duration::from_millis(200)).await;
        })
    }
    let app = axum::Router::new().route("/ws", axum::routing::get(upgrade));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, app.into_make_service())
            .await
            .unwrap();
    });

    let request = format!("ws://{addr}/ws").into_client_request().unwrap();
    let (mut socket, handshake) = tokio_tungstenite::connect_async(request).await.unwrap();
    assert_eq!(handshake.status(), StatusCode::SWITCHING_PROTOCOLS);

    // The client never sends a thing back — not even a Pong, since the
    // server here never pings at all (no `feed`, no keepalive ticking).
    // Past its idle bound, the server must close the socket on its own.
    let closed = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match futures_util::StreamExt::next(&mut socket).await {
                Some(Ok(_)) => continue,
                Some(Err(_)) | None => return,
            }
        }
    })
    .await;
    assert!(
        closed.is_ok(),
        "the idle socket must be closed by the server itself within 5s"
    );
    server.abort();
}

/// Item 2(a): the toasted device disconnects at t=5s, well within the 15s
/// grace, while its notice is still unread — the hint flips to `os` and a
/// fresh `notification` frame reaches every subscriber, including one
/// that was already background the whole time.
#[tokio::test(start_paused = true)]
async fn a_toast_device_disconnecting_within_the_grace_flips_to_os() {
    let dir = tempdir::TempDir::new("storm-161h-fallback-in-grace").unwrap();
    let f = fixture(dir.path()).await;
    let id = f.launch();
    let (device_a, _bearer_a) = second_device(&f.state, &f.owner, "a").await;

    f.state.agent.set_presence("conn-a", &device_a, true, None);

    let mut toasted = f.feed(QUICK, QUEUE);
    let mut background = f.feed(QUICK, QUEUE);
    next_frame(&mut toasted, Duration::from_secs(5))
        .await
        .unwrap(); // snapshot
    next_frame(&mut background, Duration::from_secs(5))
        .await
        .unwrap(); // snapshot

    let n = f
        .state
        .agent
        .agent_notify(agent_notify(&id, "hi", "k"))
        .unwrap()
        .unwrap();
    let (name, data) = next_frame(&mut toasted, Duration::from_secs(5))
        .await
        .unwrap();
    assert_eq!(name, "notification");
    assert_eq!(data["id"], n.id);
    assert_eq!(data["deliver"], format!("toast:{device_a}"));
    let (_, data_bg) = next_frame(&mut background, Duration::from_secs(5))
        .await
        .unwrap();
    assert_eq!(data_bg["deliver"], format!("toast:{device_a}"));

    tokio::time::advance(Duration::from_secs(5)).await;
    f.state.agent.clear_presence("conn-a", &device_a);

    let (name2, data2) = next_frame(&mut background, Duration::from_secs(5))
        .await
        .expect("a fresh notification frame, re-delivered as os");
    assert_eq!(name2, "notification");
    assert_eq!(data2["id"], n.id);
    assert_eq!(data2["deliver"], "os");
}

/// Item 2(b): past the 15s grace, the same disconnect must not flip
/// anything — the device simply going background or away later is
/// ordinary, not a fallback trigger.
#[tokio::test(start_paused = true)]
async fn a_toast_device_disconnecting_past_the_grace_does_not_flip() {
    let dir = tempdir::TempDir::new("storm-161h-fallback-past-grace").unwrap();
    let f = fixture(dir.path()).await;
    let id = f.launch();
    let (device_a, _bearer_a) = second_device(&f.state, &f.owner, "a").await;

    f.state.agent.set_presence("conn-a", &device_a, true, None);
    let mut sub = f.feed(QUICK, QUEUE);
    next_frame(&mut sub, Duration::from_secs(5)).await.unwrap(); // snapshot

    let n = f
        .state
        .agent
        .agent_notify(agent_notify(&id, "hi", "k"))
        .unwrap()
        .unwrap();
    let (_, data) = next_frame(&mut sub, Duration::from_secs(5)).await.unwrap();
    assert_eq!(data["id"], n.id);
    assert_eq!(data["deliver"], format!("toast:{device_a}"));

    tokio::time::advance(Duration::from_secs(20)).await; // past the 15s grace
    f.state.agent.clear_presence("conn-a", &device_a);

    let extra = next_frame(&mut sub, Duration::from_millis(300)).await;
    assert!(extra.is_none(), "no re-delivery past the grace: {extra:?}");
}

/// Item 2(c): the notice was already read before the toast device goes
/// away at t=5s — nothing to re-show, so no flip.
#[tokio::test(start_paused = true)]
async fn an_already_read_toast_does_not_flip_when_its_device_goes_away() {
    let dir = tempdir::TempDir::new("storm-161h-fallback-already-read").unwrap();
    let f = fixture(dir.path()).await;
    let id = f.launch();
    let (device_a, _bearer_a) = second_device(&f.state, &f.owner, "a").await;

    f.state.agent.set_presence("conn-a", &device_a, true, None);
    let mut sub = f.feed(QUICK, QUEUE);
    next_frame(&mut sub, Duration::from_secs(5)).await.unwrap(); // snapshot

    let n = f
        .state
        .agent
        .agent_notify(agent_notify(&id, "hi", "k"))
        .unwrap()
        .unwrap();
    let (_, data) = next_frame(&mut sub, Duration::from_secs(5)).await.unwrap();
    assert_eq!(data["deliver"], format!("toast:{device_a}"));

    f.state
        .agent
        .mark_notifications_read(std::slice::from_ref(&n.id), None)
        .unwrap();
    let (name_r, data_r) = next_frame(&mut sub, Duration::from_secs(5)).await.unwrap();
    assert_eq!(name_r, "notification_read");
    assert_eq!(data_r["ids"][0], n.id);

    tokio::time::advance(Duration::from_secs(5)).await; // still within the grace
    f.state.agent.clear_presence("conn-a", &device_a);

    let extra = next_frame(&mut sub, Duration::from_millis(300)).await;
    assert!(
        extra.is_none(),
        "nothing to re-show once it is already read: {extra:?}"
    );
}

/// Item 2(d): a device with two live connections must not trigger the
/// fallback just because one of them disconnects — only once it has no
/// remaining active connection at all.
#[tokio::test(start_paused = true)]
async fn a_second_live_connection_keeps_the_device_active_so_nothing_flips() {
    let dir = tempdir::TempDir::new("storm-161h-fallback-multi-conn").unwrap();
    let f = fixture(dir.path()).await;
    let id = f.launch();
    let (device_a, _bearer_a) = second_device(&f.state, &f.owner, "a").await;

    f.state.agent.set_presence("conn-1", &device_a, true, None);
    f.state.agent.set_presence("conn-2", &device_a, true, None);
    let mut sub = f.feed(QUICK, QUEUE);
    next_frame(&mut sub, Duration::from_secs(5)).await.unwrap(); // snapshot

    let n = f
        .state
        .agent
        .agent_notify(agent_notify(&id, "hi", "k"))
        .unwrap()
        .unwrap();
    let (_, data) = next_frame(&mut sub, Duration::from_secs(5)).await.unwrap();
    assert_eq!(data["id"], n.id);
    assert_eq!(data["deliver"], format!("toast:{device_a}"));

    tokio::time::advance(Duration::from_secs(5)).await;
    f.state.agent.clear_presence("conn-1", &device_a); // only one of its two connections

    let extra = next_frame(&mut sub, Duration::from_millis(300)).await;
    assert!(
        extra.is_none(),
        "device a is still active on conn-2: {extra:?}"
    );
}

/// Item 4: the test notice targets the device that actually asked for
/// it — not whichever active device's id happens to sort lowest, as
/// `compute_deliver_hint`'s own arbitrary pick would give it.
#[tokio::test]
async fn send_test_notification_targets_the_requesting_device_not_the_lowest_id() {
    let dir = tempdir::TempDir::new("storm-161h-test-notice-targeted").unwrap();
    let f = fixture(dir.path()).await;
    let (device_a, _bearer_a) = second_device(&f.state, &f.owner, "a").await;
    let (device_b, _bearer_b) = second_device(&f.state, &f.owner, "b").await;
    f.state.agent.set_presence("conn-a", &device_a, true, None);
    f.state.agent.set_presence("conn-b", &device_b, true, None);
    // Whichever of the two does *not* sort lowest: if the fix were absent,
    // `compute_deliver_hint` would pick the other one instead.
    let requester = if device_a > device_b {
        &device_a
    } else {
        &device_b
    };

    let n = f
        .state
        .agent
        .send_test_notification(Some(requester))
        .unwrap();
    assert_eq!(
        f.state.agent.deliver_hint_for(&n.id, None),
        Some(format!("toast:{requester}")),
        "must target whoever asked, not whoever sorts lowest"
    );
}

/// Item 4: an `stk_`/agent key with no device (`device_id: None`) keeps
/// today's behaviour unchanged — `compute_deliver_hint` picks normally.
#[tokio::test]
async fn send_test_notification_with_no_device_id_keeps_the_old_fallback() {
    let dir = tempdir::TempDir::new("storm-161h-test-notice-fallback").unwrap();
    let f = fixture(dir.path()).await;
    let (device_a, _bearer_a) = second_device(&f.state, &f.owner, "a").await;
    f.state.agent.set_presence("conn-a", &device_a, true, None);

    let n = f.state.agent.send_test_notification(None).unwrap();
    assert_eq!(
        f.state.agent.deliver_hint_for(&n.id, None),
        Some(format!("toast:{device_a}")),
        "no device id: deliver_hint_for computes it normally, same as before"
    );
}
