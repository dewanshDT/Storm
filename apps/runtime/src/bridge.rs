//! `storm-runtime mcp-bridge <slug>`: the agent's stdio MCP server for one
//! gateway connection (spec §11, decision 81f).
//!
//! The agent CLI starts it, speaks MCP on its stdin and stdout, and each
//! message goes to the host daemon on its socket with the session handle. It
//! holds no credential: there is none on the host.
//!
//! **The six rules** (spec §11; R5, R7 and R8 were proved by breaking them in
//! the gates, and are tested here the same way):
//! 1. It keeps the agent's `initialize`.
//! 2. When the gateway answers `session_unknown` — after a server restart —
//!    the request was **not forwarded**. The bridge replays `initialize` and
//!    `notifications/initialized` under its own ids (`storm-reinit-N`),
//!    **swallows** that response so the agent sees exactly one initialize
//!    result (R5), then sends the original request once.
//! 3. Anything else that fails becomes one JSON-RPC error to the agent and is
//!    **never retried** (R7).
//! 4. When a request ends, every upstream-initiated request it opened toward
//!    the agent and the agent has not answered (an elicitation) is cancelled
//!    with `notifications/cancelled` (R8).
//! 5. Late answers to such a request are dropped, never sent upstream.
//! 6. Agent responses to upstream-initiated requests are never replayed.

use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;

/// What the bridge talks to: the host daemon, or a test's stand-in.
pub trait Daemon: Send + Sync + 'static {
    /// Sends one message; yields every line of the answer. A daemon that
    /// cannot be reached yields `{"storm_error": "storm_unreachable"}`.
    fn exchange(&self, message: Value) -> impl Future<Output = mpsc::Receiver<Value>> + Send;
    /// The session's unsolicited messages, while the daemon is reachable.
    fn subscribe(&self) -> impl Future<Output = Option<mpsc::Receiver<Value>>> + Send;
}

#[derive(Default)]
struct State {
    init: Option<Value>,
    reinit: u64,
    /// Upstream-initiated requests sent to the agent and not yet answered:
    /// their id (as JSON text) → the agent request whose stream carried them.
    open: HashMap<String, String>,
}

pub struct Bridge<D: Daemon> {
    daemon: D,
    out: mpsc::UnboundedSender<Value>,
    state: Mutex<State>,
}

fn key(id: &Value) -> String {
    id.to_string()
}

fn error_for(id: &Value, code: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {"code": -32001, "message": format!("storm: {code}"), "data": {"storm_error": code}},
    })
}

impl<D: Daemon> Bridge<D> {
    pub fn new(daemon: D, out: mpsc::UnboundedSender<Value>) -> Arc<Self> {
        Arc::new(Self {
            daemon,
            out,
            state: Mutex::new(State::default()),
        })
    }

    fn emit(&self, message: Value) {
        let _ = self.out.send(message);
    }

    /// Sends one message whose answer nobody reads (a notification, an
    /// agent's response), draining the answer so the exchange completes.
    async fn send_unanswered(&self, message: Value) {
        let mut rx = self.daemon.exchange(message).await;
        while rx.recv().await.is_some() {}
    }

    /// One line from the agent. Requests other than `initialize` run
    /// concurrently; `initialize` is answered before anything else is read.
    pub async fn on_agent_message(self: &Arc<Self>, message: Value) {
        let method = message
            .get("method")
            .and_then(|m| m.as_str())
            .map(str::to_string);
        let has_id = message.get("id").is_some();
        match (method.as_deref(), has_id) {
            (Some("initialize"), true) => {
                self.state.lock().unwrap().init = Some(message.clone());
                self.request(message).await;
            }
            (Some(_), true) => {
                let me = self.clone();
                tokio::spawn(async move { me.request(message).await });
            }
            // A notification: forwarded once.
            (Some(_), false) => self.send_unanswered(message).await,
            (None, _) => {
                let me = self.clone();
                tokio::spawn(async move { me.response(message).await });
            }
        }
    }

    /// The agent answering an upstream-initiated request (rules 5 and 6).
    async fn response(&self, message: Value) {
        let id = message.get("id").cloned().unwrap_or(Value::Null);
        let known = self.state.lock().unwrap().open.remove(&key(&id)).is_some();
        if !known {
            return; // late: dropped, never sent upstream
        }
        self.send_unanswered(message).await;
    }

    /// Rule 2: replays the kept `initialize` under the bridge's own id and
    /// swallows its answer. True when the gateway accepted it.
    async fn reinitialize(&self) -> bool {
        let (init, rid) = {
            let mut st = self.state.lock().unwrap();
            st.reinit += 1;
            (st.init.clone(), format!("storm-reinit-{}", st.reinit))
        };
        let Some(init) = init else { return false };
        let replay = json!({
            "jsonrpc": "2.0", "id": rid, "method": "initialize",
            "params": init.get("params").cloned().unwrap_or(json!({})),
        });
        let mut ok = false;
        let mut rx = self.daemon.exchange(replay).await;
        while let Some(line) = rx.recv().await {
            if let Some(m) = line.get("message")
                && m.get("id") == Some(&json!(rid))
                && m.get("result").is_some()
            {
                ok = true; // swallowed: the agent already has its one result
            }
        }
        if ok {
            self.send_unanswered(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))
                .await;
        }
        ok
    }

    async fn request(&self, req: Value) {
        let id = req.get("id").cloned().unwrap_or(Value::Null);
        let req_key = key(&id);
        let is_initialize = req.get("method") == Some(&json!("initialize"));
        let mut answered = false;
        for attempt in 1..=2 {
            let mut replay = false;
            let mut rx = self.daemon.exchange(req.clone()).await;
            while let Some(mut line) = rx.recv().await {
                if let Some(code) = line.get("storm_error").and_then(|c| c.as_str()) {
                    if code == "session_unknown" && attempt == 1 && !is_initialize {
                        replay = true;
                    } else {
                        self.emit(error_for(&id, code));
                        answered = true;
                    }
                    break;
                }
                let Some(m) = line.get_mut("message").map(Value::take) else {
                    continue;
                };
                if m.get("method").is_some() && m.get("id").is_some() {
                    // An upstream-initiated request riding this call's stream.
                    let up = key(m.get("id").unwrap());
                    self.state.lock().unwrap().open.insert(up, req_key.clone());
                } else if m.get("id") == Some(&id) {
                    answered = true;
                }
                self.emit(m);
            }
            if replay {
                if self.reinitialize().await {
                    continue;
                }
                self.emit(error_for(&id, "reinitialize_failed"));
                answered = true;
            }
            break;
        }
        if !answered {
            // The stream ended without a response: one error, never a retry.
            self.emit(error_for(&id, "no_response"));
        }
        // Rule 4: whatever this call opened and the agent has not answered.
        let stale: Vec<String> = {
            let mut st = self.state.lock().unwrap();
            let stale: Vec<String> = st
                .open
                .iter()
                .filter(|(_, owner)| **owner == req_key)
                .map(|(up, _)| up.clone())
                .collect();
            for up in &stale {
                st.open.remove(up);
            }
            stale
        };
        for up in stale {
            let request_id: Value = serde_json::from_str(&up).unwrap_or(Value::Null);
            self.emit(json!({
                "jsonrpc": "2.0",
                "method": "notifications/cancelled",
                "params": {"requestId": request_id, "reason": "the call that opened it ended"},
            }));
        }
    }

    /// Forwards the session's unsolicited messages (`list_changed`) for as
    /// long as the daemon is reachable, reconnecting with a backoff.
    pub async fn follow_unsolicited(self: Arc<Self>) {
        let mut backoff = std::time::Duration::from_millis(500);
        loop {
            if let Some(mut rx) = self.daemon.subscribe().await {
                backoff = std::time::Duration::from_millis(500);
                while let Some(mut line) = rx.recv().await {
                    if let Some(m) = line.get_mut("message") {
                        self.emit(m.take());
                    }
                }
            }
            tokio::time::sleep(backoff).await;
            backoff = (backoff * 2).min(std::time::Duration::from_secs(30));
        }
    }
}

/// The real daemon: the host's unix socket.
pub struct SocketDaemon {
    pub socket: PathBuf,
    pub handle: String,
    pub connection: String,
}

impl SocketDaemon {
    async fn open(&self, line: Value) -> Option<mpsc::Receiver<Value>> {
        let stream = tokio::net::UnixStream::connect(&self.socket).await.ok()?;
        let (read, mut write) = stream.into_split();
        let mut bytes = serde_json::to_vec(&line).ok()?;
        bytes.push(b'\n');
        write.write_all(&bytes).await.ok()?;
        let (tx, rx) = mpsc::channel(64);
        tokio::spawn(async move {
            let _keep = write;
            let mut lines = BufReader::new(read).lines();
            while let Ok(Some(l)) = lines.next_line().await {
                if let Ok(v) = serde_json::from_str::<Value>(&l)
                    && tx.send(v).await.is_err()
                {
                    break;
                }
            }
        });
        Some(rx)
    }
}

impl Daemon for SocketDaemon {
    async fn exchange(&self, message: Value) -> mpsc::Receiver<Value> {
        let line =
            json!({"handle": self.handle, "connection": self.connection, "message": message});
        match self.open(line).await {
            Some(rx) => rx,
            None => {
                let (tx, rx) = mpsc::channel(1);
                let _ = tx.send(json!({"storm_error": "storm_unreachable"})).await;
                rx
            }
        }
    }

    async fn subscribe(&self) -> Option<mpsc::Receiver<Value>> {
        self.open(json!({"handle": self.handle, "connection": self.connection, "subscribe": true}))
            .await
    }
}

/// `storm-runtime mcp-bridge <slug>`: stdin/stdout until the agent closes it.
pub async fn run(slug: String) -> anyhow::Result<()> {
    let handle = std::env::var(crate::mcp::HANDLE_ENV)
        .map_err(|_| anyhow::anyhow!("{} is not set", crate::mcp::HANDLE_ENV))?;
    let socket = std::env::var_os(crate::mcp::SOCKET_ENV)
        .map(PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("{} is not set", crate::mcp::SOCKET_ENV))?;
    let (out, mut out_rx) = mpsc::unbounded_channel::<Value>();
    let bridge = Bridge::new(
        SocketDaemon {
            socket,
            handle,
            connection: slug,
        },
        out,
    );
    // One writer, so lines never interleave.
    let writer = tokio::spawn(async move {
        let mut stdout = tokio::io::stdout();
        while let Some(m) = out_rx.recv().await {
            let mut bytes = serde_json::to_vec(&m).unwrap_or_default();
            bytes.push(b'\n');
            if stdout.write_all(&bytes).await.is_err() || stdout.flush().await.is_err() {
                break;
            }
        }
    });
    tokio::spawn(bridge.clone().follow_unsolicited());
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<Value>(&line) {
            Ok(message) => bridge.on_agent_message(message).await,
            Err(_) => continue,
        }
    }
    drop(bridge);
    writer.abort();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A scripted gateway: knows whether its upstream session exists, counts
    /// every message it receives, and plays one of a few call shapes.
    #[derive(Clone, Default)]
    struct Script {
        inner: Arc<ScriptState>,
    }

    #[derive(Default)]
    struct ScriptState {
        session: Mutex<bool>,
        received: Mutex<Vec<Value>>,
        executions: AtomicUsize,
    }

    impl Script {
        fn received(&self) -> Vec<Value> {
            self.inner.received.lock().unwrap().clone()
        }
        fn count(&self, method: &str) -> usize {
            self.received()
                .iter()
                .filter(|m| m.get("method") == Some(&json!(method)))
                .count()
        }
    }

    impl Daemon for Script {
        async fn exchange(&self, message: Value) -> mpsc::Receiver<Value> {
            self.inner.received.lock().unwrap().push(message.clone());
            let (tx, rx) = mpsc::channel(16);
            let id = message.get("id").cloned().unwrap_or(Value::Null);
            let method = message.get("method").and_then(|m| m.as_str()).unwrap_or("");
            let lines: Vec<Value> = match method {
                "initialize" => {
                    *self.inner.session.lock().unwrap() = true;
                    vec![
                        json!({"message": {"jsonrpc": "2.0", "id": id, "result": {"serverInfo": {"name": "up"}}}}),
                    ]
                }
                "" | "notifications/initialized" => vec![],
                _ if !*self.inner.session.lock().unwrap() => {
                    vec![json!({"storm_error": "session_unknown"})]
                }
                "tools/call" => {
                    self.inner.executions.fetch_add(1, Ordering::SeqCst);
                    match message.pointer("/params/name").and_then(|n| n.as_str()) {
                        // The stream breaks mid-call: no final response.
                        Some("dies") => vec![],
                        // An elicitation, then the stream breaks.
                        Some("asks_then_dies") => vec![json!({"message": {
                            "jsonrpc": "2.0", "id": "storm-elicit-1", "method": "elicitation/create",
                            "params": {"message": "lang?"}}})],
                        _ => vec![
                            json!({"message": {"jsonrpc": "2.0", "id": id, "result": {"content": []}}}),
                        ],
                    }
                }
                _ => vec![json!({"message": {"jsonrpc": "2.0", "id": id, "result": {}}})],
            };
            for l in lines {
                let _ = tx.send(l).await;
            }
            rx
        }

        async fn subscribe(&self) -> Option<mpsc::Receiver<Value>> {
            None
        }
    }

    fn bridge(script: &Script) -> (Arc<Bridge<Script>>, mpsc::UnboundedReceiver<Value>) {
        let (out, rx) = mpsc::unbounded_channel();
        (Bridge::new(script.clone(), out), rx)
    }

    fn drain(rx: &mut mpsc::UnboundedReceiver<Value>) -> Vec<Value> {
        let mut v = Vec::new();
        while let Ok(m) = rx.try_recv() {
            v.push(m);
        }
        v
    }

    async fn settle() {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    fn initialize() -> Value {
        json!({"jsonrpc": "2.0", "id": 0, "method": "initialize",
               "params": {"protocolVersion": "2025-11-25", "capabilities": {}}})
    }

    fn call(id: i64, name: &str) -> Value {
        json!({"jsonrpc": "2.0", "id": id, "method": "tools/call",
               "params": {"name": name, "arguments": {}}})
    }

    #[tokio::test]
    async fn r5_a_restart_replays_initialize_and_the_agent_sees_one_result() {
        let script = Script::default();
        let (b, mut rx) = bridge(&script);
        b.on_agent_message(initialize()).await;
        // The server restarts: the gateway forgets the upstream session.
        *script.inner.session.lock().unwrap() = false;
        b.on_agent_message(call(1, "echo")).await;
        settle().await;
        let out = drain(&mut rx);
        let init_results = out
            .iter()
            .filter(|m| m.pointer("/result/serverInfo").is_some())
            .count();
        assert_eq!(
            init_results, 1,
            "the agent saw a second initialize result: {out:?}"
        );
        assert!(
            out.iter()
                .any(|m| m["id"] == 1 && m.get("result").is_some()),
            "{out:?}"
        );
        // Replayed under the bridge's own id, then `initialized`, then the
        // original request exactly once more.
        assert!(
            script
                .received()
                .iter()
                .any(|m| m["id"] == "storm-reinit-1")
        );
        assert_eq!(script.count("notifications/initialized"), 1);
        assert_eq!(script.inner.executions.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn r7_a_call_that_fails_after_sending_is_never_retried() {
        let script = Script::default();
        let (b, mut rx) = bridge(&script);
        b.on_agent_message(initialize()).await;
        b.on_agent_message(call(2, "dies")).await;
        settle().await;
        let out = drain(&mut rx);
        let errors: Vec<&Value> = out.iter().filter(|m| m["id"] == 2).collect();
        assert_eq!(errors.len(), 1, "{out:?}");
        assert_eq!(errors[0]["error"]["data"]["storm_error"], "no_response");
        assert_eq!(
            script.inner.executions.load(Ordering::SeqCst),
            1,
            "the call executed twice"
        );
    }

    #[tokio::test]
    async fn an_unreachable_daemon_fails_the_call_once() {
        struct Down;
        impl Daemon for Down {
            async fn exchange(&self, _: Value) -> mpsc::Receiver<Value> {
                let (tx, rx) = mpsc::channel(1);
                let _ = tx.send(json!({"storm_error": "storm_unreachable"})).await;
                rx
            }
            async fn subscribe(&self) -> Option<mpsc::Receiver<Value>> {
                None
            }
        }
        let (out, mut rx) = mpsc::unbounded_channel();
        let b = Bridge::new(Down, out);
        b.on_agent_message(call(3, "echo")).await;
        settle().await;
        let out = drain(&mut rx);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["error"]["data"]["storm_error"], "storm_unreachable");
    }

    #[tokio::test]
    async fn r8_an_open_elicitation_is_cancelled_and_a_late_answer_is_dropped() {
        let script = Script::default();
        let (b, mut rx) = bridge(&script);
        b.on_agent_message(initialize()).await;
        b.on_agent_message(call(4, "asks_then_dies")).await;
        settle().await;
        let out = drain(&mut rx);
        assert!(out.iter().any(|m| m["method"] == "elicitation/create"));
        let cancelled: Vec<&Value> = out
            .iter()
            .filter(|m| m["method"] == "notifications/cancelled")
            .collect();
        assert_eq!(cancelled.len(), 1, "{out:?}");
        assert_eq!(cancelled[0]["params"]["requestId"], "storm-elicit-1");
        // The agent answers anyway: dropped, never sent toward upstream.
        let before = script.received().len();
        b.on_agent_message(json!({"jsonrpc": "2.0", "id": "storm-elicit-1",
            "result": {"action": "accept"}}))
            .await;
        settle().await;
        assert_eq!(
            script.received().len(),
            before,
            "a late answer was sent upstream"
        );
    }

    #[tokio::test]
    async fn an_answer_in_time_goes_through_once() {
        // The happy half of rules 5 and 6: answered while open, forwarded
        // once; answering again is late and dropped.
        let script = Script::default();
        let (b, _rx) = bridge(&script);
        b.state
            .lock()
            .unwrap()
            .open
            .insert(key(&json!("storm-elicit-9")), key(&json!(5)));
        let answer =
            json!({"jsonrpc": "2.0", "id": "storm-elicit-9", "result": {"action": "decline"}});
        b.on_agent_message(answer.clone()).await;
        b.on_agent_message(answer).await;
        settle().await;
        let sent = script
            .received()
            .iter()
            .filter(|m| m["id"] == "storm-elicit-9")
            .count();
        assert_eq!(sent, 1);
    }
}
