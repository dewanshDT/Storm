//! `storm-runtime mcp-connect`: the stdio MCP server an external MCP client
//! (Claude Code, Cursor, Claude Desktop) starts, which forwards each message
//! to the Storm Server's `/mcp` over a verified [`Transport`] (decision 105c).
//!
//! `/mcp` is stateless and answers plain JSON, so a line in is one `POST`,
//! and its answer is a line out; a notification is a `202` and writes
//! nothing.
//!
//! **The rules:**
//! 1. Nothing is sent before the server passes the identity challenge, the
//!    key included. An impostor is `identity_failed`, never "offline".
//! 2. A request that fails is one JSON-RPC error whose `data.storm_error`
//!    names the hop, and is **never retried**.
//! 3. After a failure the next request reconnects; nothing migrates mid-call.
//! 4. The key is never on argv and never logged, nor is any body. stdout is
//!    the protocol; logs go to stderr.
//! 5. It holds no authority: it sends the client's key, the server decides.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Context, anyhow, bail};
use data_encoding::BASE64;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;
use tokio::task::JoinSet;

use crate::transport::{
    Connector, DirectConnector, HttpRequest, HttpResponse, ServerPin, Transport, TransportError,
};

/// Where the `stk_` key comes from when `--key-file` is not given.
pub const KEY_ENV: &str = "STORM_MCP_KEY";

/// SEP-2575's per-request protocol version, which the header must repeat.
const META_PROTOCOL_VERSION: &str = "io.modelcontextprotocol/protocolVersion";

pub struct Adapter<C: Connector> {
    connector: C,
    bearer: String,
    current: tokio::sync::Mutex<Option<Arc<C::Transport>>>,
    /// The version `initialize` negotiated: the header every later request
    /// carries, as an HTTP client's would.
    protocol: Mutex<Option<String>>,
}

fn error_for(id: &Value, code: &str, detail: &str, status: Option<u16>) -> Value {
    let mut data = json!({ "storm_error": code });
    if let Some(status) = status {
        data["status"] = json!(status);
    }
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {"code": -32001, "message": format!("storm: {code}: {detail}"), "data": data},
    })
}

/// SEP-2243's `Mcp-Name`: which params field names a request's target.
fn target_name(method: &str, params: Option<&Value>) -> Option<String> {
    let key = match method {
        "tools/call" | "prompts/get" => "name",
        "resources/read" | "resources/subscribe" | "resources/unsubscribe" => "uri",
        "tasks/get" | "tasks/update" | "tasks/cancel" => "taskId",
        _ => return None,
    };
    params?.get(key)?.as_str().map(str::to_string)
}

fn header_safe(value: &str) -> bool {
    value
        .bytes()
        .all(|b| (0x21..=0x7e).contains(&b) || b == b' ')
        && !value.starts_with(' ')
        && !value.ends_with(' ')
}

/// rmcp's encoding for a header value that cannot travel bare.
fn header_value(value: &str) -> String {
    if header_safe(value) && !(value.starts_with("=?base64?") && value.ends_with("?=")) {
        value.to_string()
    } else {
        format!("=?base64?{}?=", BASE64.encode(value.as_bytes()))
    }
}

/// Every message an SSE body carries, for the rare answer rmcp streams even
/// in JSON mode (a handler that sends something before its result).
fn sse_messages(body: &[u8]) -> Vec<Value> {
    let text = String::from_utf8_lossy(body);
    let mut out = Vec::new();
    let mut data = String::new();
    for line in text.lines().chain(std::iter::once("")) {
        if line.is_empty() {
            if let Ok(v) = serde_json::from_str(&data) {
                out.push(v);
            }
            data.clear();
        } else if let Some(d) = line.strip_prefix("data:") {
            if !data.is_empty() {
                data.push('\n');
            }
            data.push_str(d.strip_prefix(' ').unwrap_or(d));
        }
    }
    out
}

/// A non-JSON-RPC answer to a request: the server (or its auth layer)
/// refused it before MCP saw it.
fn refusal(id: &Value, response: &HttpResponse) -> Value {
    let code = match response.status {
        401 => "unauthorized",
        403 => "forbidden",
        404 => "mcp_unavailable",
        429 => "rate_limited",
        s if (200..300).contains(&s) => "bad_response",
        _ => "http_error",
    };
    let said = serde_json::from_slice::<Value>(&response.body)
        .ok()
        .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(str::to_string))
        .unwrap_or_else(|| {
            String::from_utf8_lossy(&response.body)
                .chars()
                .take(200)
                .collect()
        });
    let detail = format!("HTTP {}: {said}", response.status);
    error_for(
        id,
        code,
        detail.trim_end_matches(": "),
        Some(response.status),
    )
}

impl<C: Connector> Adapter<C> {
    /// `key` is the client's `stk_` key, sent as its bearer.
    pub fn new(connector: C, key: &str) -> Self {
        Self {
            connector,
            bearer: format!("Bearer {key}"),
            current: tokio::sync::Mutex::new(None),
            protocol: Mutex::new(None),
        }
    }

    /// The verified transport, connecting (and verifying) if there is none.
    /// Held across the connect so concurrent requests share one.
    async fn transport(&self) -> Result<Arc<C::Transport>, TransportError> {
        let mut current = self.current.lock().await;
        if let Some(t) = current.as_ref() {
            return Ok(t.clone());
        }
        let t = Arc::new(self.connector.connect().await?);
        tracing::info!("connected to the server; its identity is verified");
        *current = Some(t.clone());
        Ok(t)
    }

    /// Rule 3: a failed transport is forgotten, so the next request
    /// reconnects; one that already replaced it is kept.
    async fn forget(&self, failed: &Arc<C::Transport>) {
        let mut current = self.current.lock().await;
        if current.as_ref().is_some_and(|t| Arc::ptr_eq(t, failed)) {
            *current = None;
        }
    }

    fn headers(&self, message: &Value) -> Vec<(String, String)> {
        let mut headers = vec![
            ("content-type".to_string(), "application/json".to_string()),
            (
                "accept".to_string(),
                "application/json, text/event-stream".to_string(),
            ),
            ("authorization".to_string(), self.bearer.clone()),
        ];
        let method = message.get("method").and_then(|m| m.as_str());
        let params = message.get("params");
        // `initialize` carries its version in the body, and rmcp refuses a
        // header that disagrees with it.
        if method != Some("initialize") {
            let version = params
                .and_then(|p| {
                    p.pointer(&format!(
                        "/_meta/{}",
                        META_PROTOCOL_VERSION.replace('/', "~1")
                    ))
                })
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .or_else(|| self.protocol.lock().unwrap().clone());
            if let Some(v) = version.filter(|v| header_safe(v)) {
                headers.push(("mcp-protocol-version".to_string(), v));
            }
        }
        if let Some(method) = method.filter(|m| header_safe(m)) {
            headers.push(("mcp-method".to_string(), method.to_string()));
            if let Some(name) = target_name(method, params) {
                headers.push(("mcp-name".to_string(), header_value(&name)));
            }
        }
        headers
    }

    async fn post(&self, message: &Value) -> Result<HttpResponse, TransportError> {
        let transport = self.transport().await?;
        let result = transport
            .send(HttpRequest {
                method: "POST".to_string(),
                path_and_query: "/mcp".to_string(),
                headers: self.headers(message),
                body: serde_json::to_vec(message).expect("a JSON value serializes"),
            })
            .await;
        if result.is_err() {
            self.forget(&transport).await;
        }
        result
    }

    /// One line from the client: what to write back, if anything. A batch
    /// is split, because rmcp's `/mcp` takes one message per `POST`, and its
    /// answers are written back as one array.
    pub async fn handle(&self, message: Value) -> Vec<Value> {
        match message {
            Value::Array(batch) if batch.is_empty() => vec![json!({
                "jsonrpc": "2.0", "id": null,
                "error": {"code": -32600, "message": "an empty batch"},
            })],
            Value::Array(batch) => {
                let answers: Vec<Value> =
                    futures_util::future::join_all(batch.into_iter().map(|m| self.handle_one(m)))
                        .await
                        .into_iter()
                        .flatten()
                        .collect();
                if answers.is_empty() {
                    vec![]
                } else {
                    vec![Value::Array(answers)]
                }
            }
            message => self.handle_one(message).await,
        }
    }

    async fn handle_one(&self, message: Value) -> Vec<Value> {
        let method = message.get("method").and_then(|m| m.as_str());
        let is_initialize = method == Some("initialize");
        // Only a request is answered: a notification, or the client's answer
        // to something the server asked, writes nothing back.
        let id = method.and(message.get("id")).cloned();
        let response = match self.post(&message).await {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(
                    storm_error = e.code,
                    "a message was not delivered: {}",
                    e.detail
                );
                return id
                    .map(|id| error_for(&id, e.code, &e.detail, None))
                    .into_iter()
                    .collect();
            }
        };
        let answers = self.answers(id.as_ref(), &response);
        if is_initialize
            && let Some(version) = answers
                .iter()
                .find(|a| a.get("id") == id.as_ref())
                .and_then(|a| a.pointer("/result/protocolVersion"))
                .and_then(|v| v.as_str())
        {
            *self.protocol.lock().unwrap() = Some(version.to_string());
        }
        answers
    }

    fn answers(&self, id: Option<&Value>, response: &HttpResponse) -> Vec<Value> {
        let content_type = response.header("content-type").unwrap_or("");
        let messages = if content_type.starts_with("text/event-stream") {
            sse_messages(&response.body)
        } else {
            match serde_json::from_slice::<Value>(&response.body) {
                Ok(v @ Value::Object(_)) if v.get("jsonrpc").is_some() => vec![v],
                Ok(Value::Array(all)) => all,
                _ => vec![],
            }
        };
        match id {
            Some(id) if !messages.iter().any(|m| m.get("id") == Some(id)) => {
                let mut messages = messages;
                messages.push(if messages.is_empty() {
                    refusal(id, response)
                } else {
                    error_for(id, "no_response", "the server sent no answer", None)
                });
                messages
            }
            Some(_) => messages,
            None => {
                if !(200..300).contains(&response.status) {
                    tracing::warn!(
                        status = response.status,
                        "the server refused a notification"
                    );
                }
                vec![]
            }
        }
    }
}

/// Reads the client's lines from `input` and writes answers to `output`
/// until `input` closes. Requests run concurrently. `initialize` and
/// notifications finish before the next line is read, so they reach the
/// server in the order the client sent them.
pub async fn serve<C, R, W>(
    adapter: Arc<Adapter<C>>,
    input: R,
    mut output: W,
) -> std::io::Result<()>
where
    C: Connector + 'static,
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let (tx, mut rx) = mpsc::unbounded_channel::<Value>();
    // One writer, so lines never interleave.
    let writer = tokio::spawn(async move {
        while let Some(m) = rx.recv().await {
            let mut bytes = serde_json::to_vec(&m).unwrap_or_default();
            bytes.push(b'\n');
            if output.write_all(&bytes).await.is_err() || output.flush().await.is_err() {
                break;
            }
        }
    });
    let mut tasks = JoinSet::new();
    let mut input = BufReader::new(input);
    let mut line = Vec::new();
    loop {
        line.clear();
        if input.read_until(b'\n', &mut line).await? == 0 {
            break;
        }
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let message = match serde_json::from_slice::<Value>(&line) {
            Ok(m) => m,
            Err(_) => {
                let _ = tx.send(json!({
                    "jsonrpc": "2.0", "id": null,
                    "error": {"code": -32700, "message": "not a JSON message"},
                }));
                continue;
            }
        };
        let in_order = message.get("method") == Some(&json!("initialize"))
            || (message.get("method").is_some() && message.get("id").is_none());
        let (adapter, tx) = (adapter.clone(), tx.clone());
        let job = async move {
            for answer in adapter.handle(message).await {
                let _ = tx.send(answer);
            }
        };
        if in_order {
            job.await;
        } else {
            tasks.spawn(job);
        }
        while tasks.try_join_next().is_some() {}
    }
    while tasks.join_next().await.is_some() {}
    drop(tx);
    let _ = writer.await;
    Ok(())
}

pub struct Options {
    pub server: String,
    pub pk: String,
    pub addr: Option<String>,
    pub relays: Vec<String>,
    pub key_file: Option<PathBuf>,
}

/// The key from `--key-file`, else from the environment. Errors never quote
/// it.
pub fn read_key(key_file: Option<&Path>, env: Option<String>) -> anyhow::Result<String> {
    let key = match key_file {
        Some(path) => std::fs::read_to_string(path)
            .with_context(|| format!("reading the key from {}", path.display()))?,
        None => env.ok_or_else(|| anyhow!("no key: set {KEY_ENV}, or pass --key-file"))?,
    };
    let key = key.trim().to_string();
    if key.is_empty() || !key.bytes().all(|b| (0x21..=0x7e).contains(&b)) {
        bail!("the key is empty or not a single printable token");
    }
    Ok(key)
}

/// `storm-runtime mcp-connect`: stdin and stdout until the client closes
/// stdin.
pub async fn run(options: Options) -> anyhow::Result<()> {
    let key = read_key(options.key_file.as_deref(), std::env::var(KEY_ENV).ok())?;
    let pin = ServerPin::new(&options.server, &options.pk)?;
    if !options.relays.is_empty() {
        tracing::warn!(
            "--relay is not used yet: this build reaches the server only at --addr \
             (relays arrive with mcp-connect S2)"
        );
    }
    let addr = options
        .addr
        .ok_or_else(|| anyhow!("--addr is required: this build cannot reach a server by relay"))?;
    let connector = DirectConnector::new(&addr, pin)?;
    serve(
        Arc::new(Adapter::new(connector, &key)),
        tokio::io::stdin(),
        tokio::io::stdout(),
    )
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::challenge_message;
    use crate::transport::verify_identity;
    use data_encoding::BASE64URL_NOPAD;
    use ed25519_dalek::{Signer, SigningKey};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    const SERVER: &str = "srv_pinned";

    type Received = (Vec<(String, String)>, Value);

    /// What reached the fake server: each request's headers and body.
    #[derive(Default)]
    struct Seen {
        requests: Mutex<Vec<Received>>,
    }

    impl Seen {
        fn mcp(&self) -> Vec<Received> {
            self.requests
                .lock()
                .unwrap()
                .iter()
                .filter(|(_, b)| b.get("jsonrpc").is_some())
                .cloned()
                .collect()
        }
        fn calls(&self, name: &str) -> usize {
            self.mcp()
                .iter()
                .filter(|(_, b)| b.pointer("/params/name") == Some(&json!(name)))
                .count()
        }
        fn bearers(&self) -> usize {
            self.requests
                .lock()
                .unwrap()
                .iter()
                .filter(|(h, _)| h.iter().any(|(n, _)| n == "authorization"))
                .count()
        }
    }

    fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
        headers
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }

    /// A fake path to a fake `/mcp`, holding `signing` as its identity.
    struct Fake {
        signing: SigningKey,
        seen: Arc<Seen>,
    }

    fn json_response(status: u16, body: Value) -> HttpResponse {
        HttpResponse {
            status,
            headers: vec![("Content-Type".into(), "application/json".into())],
            body: serde_json::to_vec(&body).unwrap(),
        }
    }

    impl Transport for Fake {
        async fn send(&self, req: HttpRequest) -> Result<HttpResponse, TransportError> {
            let body: Value = serde_json::from_slice(&req.body).unwrap();
            self.seen
                .requests
                .lock()
                .unwrap()
                .push((req.headers.clone(), body.clone()));
            if req.path_and_query == "/v1/server/challenge" {
                let nonce = body["nonce"].as_str().unwrap();
                let sig = self.signing.sign(&challenge_message(SERVER, nonce));
                return Ok(json_response(
                    200,
                    json!({"server_id": SERVER, "signature": BASE64URL_NOPAD.encode(&sig.to_bytes())}),
                ));
            }
            let id = body.get("id").cloned().unwrap_or(Value::Null);
            let ok = |result: Value| {
                json_response(200, json!({"jsonrpc": "2.0", "id": id, "result": result}))
            };
            match (
                body["method"].as_str().unwrap_or(""),
                body.pointer("/params/name").and_then(|n| n.as_str()),
            ) {
                ("initialize", _) => Ok(ok(json!({"protocolVersion": "2025-06-18"}))),
                (m, _) if m.starts_with("notifications/") => Ok(HttpResponse {
                    status: 202,
                    headers: vec![],
                    body: vec![],
                }),
                ("tools/call", Some("refused")) => Ok(ok(json!({
                    "content": [{"type": "text", "text": "no such vault"}], "isError": true
                }))),
                ("tools/call", Some("dies")) => Err(TransportError::new(
                    "stream_aborted",
                    "the connection ended mid-answer",
                )),
                ("tools/call", Some("slow")) => {
                    tokio::time::sleep(Duration::from_millis(150)).await;
                    Ok(ok(json!({"content": [], "who": "slow"})))
                }
                ("tools/call", Some("streams")) => Ok(HttpResponse {
                    status: 200,
                    headers: vec![("content-type".into(), "text/event-stream".into())],
                    body: format!(
                        "data: \n\ndata: {}\n\ndata: {}\n\n",
                        json!({"jsonrpc": "2.0", "method": "notifications/progress", "params": {}}),
                        json!({"jsonrpc": "2.0", "id": id, "result": {"content": []}})
                    )
                    .into_bytes(),
                }),
                ("tools/call", Some("locked")) => Ok(json_response(
                    401,
                    json!({"error": "a valid key is required"}),
                )),
                _ => Ok(ok(json!({"content": [], "who": "echo"}))),
            }
        }
    }

    /// A connector that verifies the fake exactly as the real one verifies
    /// a direct path.
    struct FakeConnector {
        signing: SigningKey,
        pin: ServerPin,
        seen: Arc<Seen>,
        connects: AtomicUsize,
    }

    impl Connector for FakeConnector {
        type Transport = Fake;
        async fn connect(&self) -> Result<Fake, TransportError> {
            self.connects.fetch_add(1, Ordering::SeqCst);
            let t = Fake {
                signing: self.signing.clone(),
                seen: self.seen.clone(),
            };
            verify_identity(&t, &self.pin).await?;
            Ok(t)
        }
    }

    fn pinned() -> SigningKey {
        SigningKey::from_bytes(&[7; 32])
    }

    fn adapter(signing: SigningKey) -> (Arc<Adapter<FakeConnector>>, Arc<Seen>) {
        let seen = Arc::new(Seen::default());
        let connector = FakeConnector {
            signing,
            pin: ServerPin {
                server_id: SERVER.into(),
                key: pinned().verifying_key(),
            },
            seen: seen.clone(),
            connects: AtomicUsize::new(0),
        };
        (Arc::new(Adapter::new(connector, "stk_secret")), seen)
    }

    fn call(id: i64, name: &str) -> Value {
        json!({"jsonrpc": "2.0", "id": id, "method": "tools/call",
               "params": {"name": name, "arguments": {}}})
    }

    fn initialize() -> Value {
        json!({"jsonrpc": "2.0", "id": 0, "method": "initialize",
               "params": {"protocolVersion": "2025-06-18", "capabilities": {}}})
    }

    /// Runs `serve` over in-memory pipes: writes `lines`, closes stdin, and
    /// returns everything written back.
    async fn session(adapter: Arc<Adapter<FakeConnector>>, lines: &[Value]) -> Vec<Value> {
        let (mut client_in, server_in) = tokio::io::duplex(1 << 16);
        let (server_out, client_out) = tokio::io::duplex(1 << 16);
        let served = tokio::spawn(serve(adapter, server_in, server_out));
        for l in lines {
            let mut bytes = serde_json::to_vec(l).unwrap();
            bytes.push(b'\n');
            client_in.write_all(&bytes).await.unwrap();
        }
        drop(client_in);
        tokio::time::timeout(Duration::from_secs(5), served)
            .await
            .expect("serve did not exit when stdin closed")
            .unwrap()
            .unwrap();
        let mut out = Vec::new();
        let mut reader = BufReader::new(client_out).lines();
        while let Some(l) = reader.next_line().await.unwrap() {
            out.push(serde_json::from_str(&l).unwrap());
        }
        out
    }

    #[tokio::test]
    async fn the_challenge_runs_before_the_key_is_sent() {
        let (a, seen) = adapter(pinned());
        let out = session(a, &[initialize(), call(1, "echo")]).await;
        assert_eq!(out.len(), 2, "{out:?}");
        let requests = seen.requests.lock().unwrap();
        assert!(
            requests[0].1.get("nonce").is_some(),
            "the challenge was not first"
        );
        assert!(header(&requests[0].0, "authorization").is_none());
        assert_eq!(
            header(&requests[1].0, "authorization"),
            Some("Bearer stk_secret")
        );
    }

    #[tokio::test]
    async fn an_impostor_is_identity_failed_and_never_sees_the_key() {
        let (a, seen) = adapter(SigningKey::from_bytes(&[9; 32]));
        let out = session(a.clone(), &[initialize(), call(1, "echo")]).await;
        assert_eq!(out.len(), 2, "{out:?}");
        for m in &out {
            assert_eq!(m["error"]["data"]["storm_error"], "identity_failed", "{m}");
        }
        assert_eq!(seen.bearers(), 0, "the impostor saw the key");
        assert!(seen.mcp().is_empty(), "an MCP message reached the impostor");
        // Every request tries again, and is refused again.
        assert_eq!(a.connector.connects.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn a_tool_error_passes_through_untouched() {
        let (a, _) = adapter(pinned());
        let out = session(a, &[call(3, "refused")]).await;
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["result"]["isError"], true);
        assert_eq!(out[0]["result"]["content"][0]["text"], "no such vault");
        assert!(out[0].get("error").is_none());
    }

    #[tokio::test]
    async fn a_transport_failure_is_one_error_never_resent_and_the_next_call_reconnects() {
        let (a, seen) = adapter(pinned());
        let out = session(a.clone(), &[call(4, "dies")]).await;
        assert_eq!(out.len(), 1, "{out:?}");
        assert_eq!(out[0]["id"], 4);
        assert_eq!(out[0]["error"]["data"]["storm_error"], "stream_aborted");
        assert_eq!(seen.calls("dies"), 1, "the failed call was resent");

        let out = a.handle(call(5, "echo")).await;
        assert_eq!(out[0]["result"]["who"], "echo");
        assert_eq!(a.connector.connects.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn notifications_write_nothing() {
        let (a, seen) = adapter(pinned());
        let out = session(
            a,
            &[
                json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
                json!({"jsonrpc": "2.0", "method": "notifications/cancelled", "params": {"requestId": 1}}),
            ],
        )
        .await;
        assert!(out.is_empty(), "{out:?}");
        assert_eq!(seen.mcp().len(), 2);
    }

    #[tokio::test]
    async fn concurrent_requests_are_answered_by_id() {
        let (a, _) = adapter(pinned());
        let out = session(a, &[call(10, "slow"), call(11, "echo")]).await;
        assert_eq!(out.len(), 2, "{out:?}");
        assert_eq!(out[0]["id"], 11, "the fast answer waited for the slow one");
        assert_eq!(out[0]["result"]["who"], "echo");
        assert_eq!(out[1]["id"], 10);
        assert_eq!(out[1]["result"]["who"], "slow");
    }

    #[tokio::test]
    async fn eof_exits_after_the_answers_in_flight() {
        let (a, _) = adapter(pinned());
        let out = session(a, &[call(12, "slow")]).await;
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["id"], 12);
    }

    #[tokio::test]
    async fn the_negotiated_version_and_the_standard_headers_go_with_each_request() {
        let (a, seen) = adapter(pinned());
        session(a, &[initialize(), call(1, "echo")]).await;
        let mcp = seen.mcp();
        assert!(header(&mcp[0].0, "mcp-protocol-version").is_none());
        assert_eq!(
            header(&mcp[1].0, "mcp-protocol-version"),
            Some("2025-06-18")
        );
        assert_eq!(header(&mcp[1].0, "mcp-method"), Some("tools/call"));
        assert_eq!(header(&mcp[1].0, "mcp-name"), Some("echo"));
        assert_eq!(
            header(&mcp[1].0, "accept"),
            Some("application/json, text/event-stream")
        );
    }

    #[tokio::test]
    async fn a_per_request_version_wins_over_the_negotiated_one() {
        let (a, seen) = adapter(pinned());
        let mut c = call(1, "echo");
        c["params"]["_meta"] = json!({META_PROTOCOL_VERSION: "2026-07-28"});
        session(a, &[initialize(), c]).await;
        assert_eq!(
            header(&seen.mcp()[1].0, "mcp-protocol-version"),
            Some("2026-07-28")
        );
    }

    #[tokio::test]
    async fn a_batch_is_split_and_answered_as_one_array() {
        let (a, seen) = adapter(pinned());
        let out = session(
            a,
            &[json!([
                call(20, "echo"),
                {"jsonrpc": "2.0", "method": "notifications/initialized"},
                call(21, "refused"),
            ])],
        )
        .await;
        assert_eq!(out.len(), 1, "{out:?}");
        let answers = out[0].as_array().unwrap();
        assert_eq!(answers.len(), 2);
        assert_eq!(seen.mcp().len(), 3, "a batch must be one POST per message");
        assert!(answers.iter().any(|m| m["id"] == 20));
        assert!(answers.iter().any(|m| m["id"] == 21));
    }

    #[tokio::test]
    async fn a_streamed_answer_is_written_message_by_message() {
        let (a, _) = adapter(pinned());
        let out = session(a, &[call(30, "streams")]).await;
        assert_eq!(out.len(), 2, "{out:?}");
        assert_eq!(out[0]["method"], "notifications/progress");
        assert_eq!(out[1]["id"], 30);
    }

    #[tokio::test]
    async fn a_refusal_before_mcp_is_one_error_with_the_status() {
        let (a, _) = adapter(pinned());
        let out = session(a, &[call(40, "locked")]).await;
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["error"]["data"]["storm_error"], "unauthorized");
        assert_eq!(out[0]["error"]["data"]["status"], 401);
        assert!(
            out[0]["error"]["message"]
                .as_str()
                .unwrap()
                .contains("a valid key is required")
        );
    }

    #[tokio::test]
    async fn a_line_that_is_not_json_is_a_parse_error() {
        let (a, _) = adapter(pinned());
        let (mut client_in, server_in) = tokio::io::duplex(1024);
        let (server_out, client_out) = tokio::io::duplex(1024);
        let served = tokio::spawn(serve(a, server_in, server_out));
        client_in.write_all(b"{not json\n").await.unwrap();
        drop(client_in);
        served.await.unwrap().unwrap();
        let line = BufReader::new(client_out)
            .lines()
            .next_line()
            .await
            .unwrap()
            .unwrap();
        let v: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["error"]["code"], -32700);
    }

    #[test]
    fn a_name_that_cannot_travel_bare_is_base64() {
        assert_eq!(header_value("get_vault"), "get_vault");
        assert_eq!(
            header_value("café"),
            format!("=?base64?{}?=", BASE64.encode("café".as_bytes()))
        );
        assert!(header_value("a\r\nEvil: 1").starts_with("=?base64?"));
    }

    #[test]
    fn the_key_comes_from_the_file_or_the_environment_and_is_never_quoted() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("key");
        std::fs::write(&file, "stk_fromfile\n").unwrap();
        assert_eq!(
            read_key(Some(&file), Some("stk_fromenv".into())).unwrap(),
            "stk_fromfile"
        );
        assert_eq!(
            read_key(None, Some("stk_fromenv".into())).unwrap(),
            "stk_fromenv"
        );
        assert!(read_key(None, None).is_err());
        let e = read_key(None, Some("stk_two words".into())).unwrap_err();
        assert!(!format!("{e:#}").contains("stk_two"));
    }
}
