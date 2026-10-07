//! `storm-runtime serve`: the Runtime Host daemon (freeze §5, §11.5, §13;
//! decision 77c).
//!
//! **The link.** One loop: verify the server, prove the key, open the command
//! stream, send `hello`, handle commands until the stream drops, then do it
//! again with backoff from 1 s to 60 s.
//!
//! **Output.** One uploader task per session posts its ring from the last
//! offset the server acknowledged, then any pending status. A session's ending
//! is posted only after its output has drained. Output survives a link drop:
//! the ring holds it, and the uploader retries until the link is back.
//!
//! **Restarts.** `sessions.json` lists the sessions this host is running. A
//! host that restarts finds them dead, and reports each one as
//! `failed (host_restart)` in its first `hello`.
//!
//! **Revocation.** When the server refuses the key, every session is ended and
//! the daemon exits non-zero (freeze §5.6). An execution plane nobody can see
//! must not keep running agents.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use data_encoding::BASE64;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::{Notify, RwLock, watch};

use crate::cli::DEFAULT_GRACE;
use crate::client::{Refused, ServerClient};
use crate::config::{RuntimeConfig, list_workspaces, resolve_workspace};
use crate::identity::{HostConfig, HostKey};
use crate::provider::{
    Availability, Interaction, InteractionSpec, Provider, ProviderSession, SessionEvents,
    SessionSpec, TerminalSize,
};
use crate::scrollback::{Read, Scrollback};
use crate::status::{EndReason, SessionEnd};

const SESSIONS_FILE: &str = "sessions.json";
const BATCH: usize = 64 * 1024;
const COALESCE: Duration = Duration::from_millis(20);

/// A command from the server (freeze §11.5).
#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum Command {
    #[serde(rename = "start")]
    Start {
        session: String,
        workspace: String,
        provider: String,
        #[serde(default)]
        interaction: Option<String>,
        terminal: Size,
        /// The connections this session may use (AM23): ids and slugs only.
        #[serde(default)]
        mcp: Vec<crate::mcp::McpGrant>,
    },
    #[serde(rename = "end")]
    End { session: String },
    #[serde(rename = "terminal.input")]
    Input { session: String, data: String },
    #[serde(rename = "terminal.resize")]
    Resize {
        session: String,
        cols: u16,
        rows: u16,
    },
    #[serde(rename = "terminal.replay")]
    Replay { session: String, from: u64 },
    #[serde(rename = "refresh")]
    Refresh,
    /// An unsolicited upstream message for a session's bridge (spec §9).
    #[serde(rename = "mcp.message")]
    McpMessage {
        session: String,
        connection: String,
        message: serde_json::Value,
    },
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct Size {
    pub cols: u16,
    pub rows: u16,
}

#[derive(Debug, Deserialize)]
struct Envelope {
    #[serde(default)]
    cmd_seq: u64,
    #[serde(flatten)]
    command: Command,
}

/// The runtime's side of one session: where its output lands and how it ended.
struct Events {
    ring: Mutex<Scrollback>,
    ended: Mutex<Option<SessionEnd>>,
    wake: Notify,
}

impl SessionEvents for Events {
    fn output(&self, bytes: &[u8]) {
        self.ring.lock().unwrap().append(bytes);
        self.wake.notify_one();
    }

    fn ended(&self, end: SessionEnd) {
        *self.ended.lock().unwrap() = Some(end);
        self.wake.notify_one();
    }
}

struct HostSession {
    id: String,
    session: Mutex<Box<dyn ProviderSession>>,
    events: Arc<Events>,
    /// The offset the server has acknowledged.
    sent: Mutex<u64>,
    /// `running`, posted once.
    running_sent: AtomicBool,
    /// Input, in order, for this session's writer task. A queue so the link
    /// never waits on a PTY, and one writer so keystrokes never reorder.
    input: tokio::sync::mpsc::UnboundedSender<Vec<u8>>,
}

/// Bridges listening for unsolicited messages, by `(session, connection id)`.
type Subscribers = HashMap<(String, String), Vec<tokio::sync::mpsc::Sender<serde_json::Value>>>;

/// The table that survives a restart: which sessions this host was running.
#[derive(Debug, Default, Serialize, Deserialize)]
struct SessionTable {
    sessions: Vec<String>,
}

pub struct Host {
    state_dir: PathBuf,
    config: RuntimeConfig,
    identity: HostConfig,
    key: HostKey,
    client: ServerClient,
    providers: Vec<Arc<dyn Provider>>,
    sessions: Mutex<HashMap<String, Arc<HostSession>>>,
    token: RwLock<Option<String>>,
    link_up: watch::Sender<bool>,
    /// Sessions found dead at boot, reported in the first `hello`.
    died: Mutex<Vec<String>>,
    last_cmd_seq: Mutex<u64>,
    /// An uploader saw the token refused: reconnect and re-authenticate.
    reauth: Notify,
    /// The MCP Gateway's session handles (decision 81f).
    handles: crate::mcp::Handles,
    /// Bridges listening for a session's unsolicited messages, by
    /// `(session, connection id)`.
    subscribers: Mutex<Subscribers>,
    opencode_settings: Option<serde_json::Value>,
}

impl Host {
    pub fn new(state_dir: &Path, config: RuntimeConfig) -> Result<Arc<Self>> {
        let identity = HostConfig::load(state_dir)?;
        let key = HostKey::load(state_dir, &identity.key_id)?;
        let client = ServerClient::for_host(&identity)?;
        let providers = config.providers()?;
        let opencode_settings = config.opencode_settings();
        // Every session directory belongs to a session that died with the
        // previous process: its handle is gone, so its bridge config is too.
        let _ = std::fs::remove_dir_all(state_dir.join(crate::mcp::SESSIONS_DIR));
        // Every session the table lists died with the previous process.
        let died = read_table(state_dir).sessions;
        if !died.is_empty() {
            tracing::warn!(
                sessions = died.len(),
                "sessions from before this restart are gone; reporting them as host_restart"
            );
        }
        Ok(Arc::new(Self {
            state_dir: state_dir.to_path_buf(),
            config,
            identity,
            key,
            client,
            providers,
            sessions: Mutex::new(HashMap::new()),
            token: RwLock::new(None),
            link_up: watch::channel(false).0,
            died: Mutex::new(died),
            last_cmd_seq: Mutex::new(0),
            reauth: Notify::new(),
            handles: crate::mcp::Handles::default(),
            subscribers: Mutex::new(HashMap::new()),
            opencode_settings,
        }))
    }

    fn capabilities(&self) -> serde_json::Value {
        let providers: Vec<_> = self
            .providers
            .iter()
            .map(|p| {
                json!({
                    "id": p.id().as_str(),
                    "kind": p.kind().as_str(),
                    "interactions": p.interactions().iter().map(|i| i.as_str()).collect::<Vec<_>>(),
                    "available": p.available() == Availability::Available,
                })
            })
            .collect();
        json!({
            "version": env!("CARGO_PKG_VERSION"),
            "providers": providers,
            "workspaces": list_workspaces(&self.config.workspace_roots),
            "max_sessions": self.config.max_sessions,
            // This host runs `storm-runtime mcp-bridge` (spec §6). A host
            // that does not say so gets no grants.
            "mcp_bridge": true,
        })
    }

    fn write_table(&self) {
        let table = SessionTable {
            sessions: self.sessions.lock().unwrap().keys().cloned().collect(),
        };
        let path = self.state_dir.join(SESSIONS_FILE);
        let tmp = path.with_extension("json.tmp");
        let result = std::fs::write(&tmp, serde_json::to_vec(&table).unwrap_or_default())
            .and_then(|_| std::fs::rename(&tmp, &path));
        if let Err(e) = result {
            tracing::error!(error = %e, "could not write {}", path.display());
        }
    }

    async fn post(&self, path: &str, body: &serde_json::Value) -> Result<reqwest::StatusCode> {
        Ok(self.post_for_response(path, body, None).await?.status())
    }

    /// A POST on the host token. A `401` wakes re-authentication, whoever
    /// made the call: the token is the link's, not the caller's.
    async fn post_for_response(
        &self,
        path: &str,
        body: &(impl serde::Serialize + ?Sized),
        timeout: Option<Duration>,
    ) -> Result<reqwest::Response> {
        let token = self
            .token
            .read()
            .await
            .clone()
            .ok_or_else(|| anyhow!("no token yet"))?;
        let mut request = self
            .client
            .http()
            .post(format!("{}{path}", self.client.base()))
            .bearer_auth(token)
            .json(body);
        if let Some(timeout) = timeout {
            request = request.timeout(timeout);
        }
        let response = request.send().await?;
        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            self.reauth.notify_one();
        }
        Ok(response)
    }

    /// Runs until revoked. Returns the refusal that ended it.
    pub async fn run(self: Arc<Self>) -> Result<()> {
        let listener = self.bind_socket()?;
        tokio::spawn(self.clone().serve_socket(listener));
        let mut backoff = Duration::from_secs(1);
        loop {
            match self.clone().connect_once().await {
                Ok(()) => backoff = Duration::from_secs(1),
                Err(e) if e.downcast_ref::<Refused>().is_some() => {
                    tracing::error!(
                        "this host's key was refused: ending every session (freeze §5.6)"
                    );
                    self.end_all();
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    return Err(e);
                }
                Err(e) => tracing::warn!(error = %format!("{e:#}"), "link down"),
            }
            self.link_up.send_replace(false);
            tokio::time::sleep(backoff).await;
            backoff = (backoff * 2).min(Duration::from_secs(60));
        }
    }

    fn end_all(&self) {
        for session in self.sessions.lock().unwrap().values() {
            session
                .session
                .lock()
                .unwrap()
                .stop(Duration::from_millis(200));
        }
    }

    async fn connect_once(self: Arc<Self>) -> Result<()> {
        self.client.verify_server().await?;
        let token = self
            .client
            .authenticate(&self.identity.host_id, &self.key)
            .await?;
        *self.token.write().await = Some(token.token.clone());

        let response = self
            .client
            .http()
            .get(format!("{}/v1/runtime/link", self.client.base()))
            .bearer_auth(&token.token)
            .header("accept", "text/event-stream")
            // The command stream is long-lived; only the connect is bounded.
            .timeout(Duration::from_secs(60 * 60 * 24 * 365))
            .send()
            .await
            .context("opening the link")?
            .error_for_status()
            .context("the server refused the link")?;
        let mut stream = response.bytes_stream();

        self.hello().await?;
        self.link_up.send_replace(true);
        tracing::info!(host = %self.identity.host_id, "link up");
        for session in self.sessions.lock().unwrap().values() {
            session.events.wake.notify_one();
        }

        let mut buf: Vec<u8> = Vec::new();
        loop {
            tokio::select! {
                chunk = stream.next() => {
                    let Some(chunk) = chunk else { return Ok(()) };
                    buf.extend_from_slice(&chunk.context("reading the link")?);
                    while let Some((event, rest)) = split_event(&buf) {
                        if let Some(data) = event_data(&event) {
                            match serde_json::from_str::<Envelope>(&data) {
                                Ok(env) => {
                                    *self.last_cmd_seq.lock().unwrap() = env.cmd_seq;
                                    self.clone().handle(env.command).await;
                                }
                                Err(e) => tracing::warn!(error = %e, "unreadable command"),
                            }
                        }
                        buf = rest;
                    }
                }
                _ = self.reauth.notified() => {
                    tracing::info!("token refused; re-authenticating");
                    return Ok(());
                }
            }
        }
    }

    async fn hello(&self) -> Result<()> {
        let mut sessions = Vec::new();
        for s in self.sessions.lock().unwrap().values() {
            let end = *s.events.ended.lock().unwrap();
            let mut entry = status_json(end);
            entry["id"] = json!(s.id);
            entry["output_end_offset"] = json!(s.events.ring.lock().unwrap().end());
            sessions.push(entry);
        }
        let died: Vec<String> = self.died.lock().unwrap().clone();
        for id in &died {
            sessions.push(json!({
                "id": id,
                "status": "failed",
                "end_reason": EndReason::HostRestart.as_str(),
                "output_end_offset": 0,
            }));
        }
        let mut body = self.capabilities();
        body["sessions"] = json!(sessions);
        body["last_cmd_seq"] = json!(*self.last_cmd_seq.lock().unwrap());
        let status = self.post("/v1/runtime/hello", &body).await?;
        if !status.is_success() {
            anyhow::bail!("hello answered {status}");
        }
        // Reported: they need not be reported again.
        self.died.lock().unwrap().clear();
        self.write_table();
        Ok(())
    }

    async fn handle(self: Arc<Self>, command: Command) {
        match command {
            Command::Start {
                session,
                workspace,
                provider,
                interaction,
                terminal,
                mcp,
            } => {
                self.start(session, workspace, provider, interaction, terminal, mcp)
                    .await
            }
            Command::McpMessage {
                session,
                connection,
                message,
            } => {
                let senders = self
                    .subscribers
                    .lock()
                    .unwrap()
                    .get(&(session, connection))
                    .cloned()
                    .unwrap_or_default();
                for tx in senders {
                    let _ = tx.try_send(json!({ "message": message }));
                }
            }
            Command::End { session } => {
                if let Some(s) = self.session(&session) {
                    s.session.lock().unwrap().stop(DEFAULT_GRACE);
                }
            }
            Command::Input { session, data } => {
                if let (Some(s), Ok(bytes)) =
                    (self.session(&session), BASE64.decode(data.as_bytes()))
                {
                    let _ = s.input.send(bytes);
                }
            }
            Command::Resize {
                session,
                cols,
                rows,
            } => {
                if let (Some(s), Some(size)) =
                    (self.session(&session), TerminalSize::new(cols, rows))
                {
                    let mut guard = s.session.lock().unwrap();
                    let Interaction::Terminal(term) = guard.interaction();
                    let _ = term.resize(size);
                }
            }
            Command::Replay { session, from } => {
                if let Some(s) = self.session(&session) {
                    *s.sent.lock().unwrap() = from;
                    s.events.wake.notify_one();
                }
            }
            Command::Refresh => {
                let _ = self
                    .post("/v1/runtime/inventory", &self.capabilities())
                    .await;
            }
        }
    }

    fn session(&self, id: &str) -> Option<Arc<HostSession>> {
        self.sessions.lock().unwrap().get(id).cloned()
    }

    async fn start(
        self: Arc<Self>,
        id: String,
        workspace: String,
        provider_id: String,
        interaction: Option<String>,
        size: Size,
        mcp: Vec<crate::mcp::McpGrant>,
    ) {
        let refuse = |reason: &str| {
            tracing::warn!(session = %id, reason, "start refused");
        };
        if interaction.as_deref().is_some_and(|i| i != "terminal") {
            refuse("unsupported interaction");
            return self.post_start_failure(&id).await;
        }
        if self.sessions.lock().unwrap().len() >= self.config.max_sessions as usize {
            refuse("max_sessions reached");
            return self.post_start_failure(&id).await;
        }
        let Some(dir) = resolve_workspace(&self.config.workspace_roots, &workspace) else {
            refuse("no such workspace");
            return self.post_start_failure(&id).await;
        };
        let Some(provider) = self
            .providers
            .iter()
            .find(|p| p.id().as_str() == provider_id)
            .cloned()
        else {
            refuse("no such provider");
            return self.post_start_failure(&id).await;
        };
        let Some(size) = TerminalSize::new(size.cols, size.rows) else {
            refuse("bad size");
            return self.post_start_failure(&id).await;
        };
        // The provider's MCP config (AM32). Written before the agent starts;
        // a session whose config cannot be written does not start, rather
        // than start without `--strict-mcp-config` and load the host's own
        // MCP servers (G-D12).
        let launch = match self.mcp_launch(&id, &provider_id, &mcp) {
            Ok(extras) => extras,
            Err(e) => {
                tracing::warn!(session = %id, error = %format!("{e:#}"), "could not write the MCP config");
                return self.post_start_failure(&id).await;
            }
        };
        let events = Arc::new(Events {
            ring: Mutex::new(Scrollback::new(self.config.scrollback_bytes)),
            ended: Mutex::new(None),
            wake: Notify::new(),
        });
        let spec = SessionSpec {
            session_id: id.clone(),
            workspace: dir,
            interaction: InteractionSpec::Terminal(size),
            launch,
        };
        let started = {
            let events: Arc<dyn SessionEvents> = events.clone();
            tokio::task::spawn_blocking(move || provider.start(spec, events)).await
        };
        let session = match started {
            Ok(Ok(session)) => session,
            Ok(Err(e)) => {
                tracing::warn!(session = %id, error = %e, "start failed");
                return self.post_start_failure(&id).await;
            }
            Err(e) => {
                tracing::error!(session = %id, error = %e, "start panicked");
                return self.post_start_failure(&id).await;
            }
        };
        let (input, mut queue) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
        let host_session = Arc::new(HostSession {
            id: id.clone(),
            session: Mutex::new(session),
            events,
            sent: Mutex::new(0),
            running_sent: AtomicBool::new(false),
            input,
        });
        // The writer: one per session, in order. Each write is bounded by the
        // CLI provider's input deadline, so a hung agent costs this task
        // seconds, never the link.
        let writer = host_session.clone();
        tokio::spawn(async move {
            while let Some(bytes) = queue.recv().await {
                let s = writer.clone();
                let result = tokio::task::spawn_blocking(move || {
                    let mut guard = s.session.lock().unwrap();
                    let Interaction::Terminal(term) = guard.interaction();
                    term.write(&bytes)
                })
                .await;
                if let Ok(Err(e)) = result {
                    tracing::debug!(session = %writer.id, error = %e, "input not delivered");
                }
                if writer.events.ended.lock().unwrap().is_some() {
                    break;
                }
            }
        });
        self.sessions
            .lock()
            .unwrap()
            .insert(id.clone(), host_session.clone());
        self.write_table();
        tracing::info!(session = %id, provider = %provider_id, %workspace, "session started");
        tokio::spawn(self.clone().upload(host_session));
    }

    async fn post_start_failure(&self, id: &str) {
        self.forget_mcp(id);
        let _ = self
            .post(
                &format!("/v1/runtime/sessions/{id}/status"),
                &json!({ "status": "failed", "end_reason": EndReason::StartFailure.as_str() }),
            )
            .await;
    }

    /// Posts a session's output in order, then its status, until it has ended
    /// and everything has been acknowledged.
    async fn upload(self: Arc<Self>, s: Arc<HostSession>) {
        let mut retry = Duration::from_millis(250);
        loop {
            // Wait for the link before posting anything.
            let mut up = self.link_up.subscribe();
            while !*up.borrow_and_update() {
                if up.changed().await.is_err() {
                    return;
                }
            }

            if !s.running_sent.load(Ordering::SeqCst) {
                match self
                    .post(
                        &format!("/v1/runtime/sessions/{}/status", s.id),
                        &json!({"status": "running"}),
                    )
                    .await
                {
                    Ok(st) if st.is_success() => s.running_sent.store(true, Ordering::SeqCst),
                    _ => {
                        tokio::time::sleep(retry).await;
                        retry = (retry * 2).min(Duration::from_secs(5));
                        continue;
                    }
                }
            }

            let from = *s.sent.lock().unwrap();
            let read = s.events.ring.lock().unwrap().read(from, BATCH);
            match read {
                Read::Data { from, bytes } => {
                    let body = json!({ "offset": from, "data": BASE64.encode(&bytes) });
                    match self
                        .post(
                            &format!("/v1/runtime/sessions/{}/terminal/output", s.id),
                            &body,
                        )
                        .await
                    {
                        Ok(st) if st.is_success() => {
                            *s.sent.lock().unwrap() = from + bytes.len() as u64;
                            retry = Duration::from_millis(250);
                        }
                        _ => {
                            tokio::time::sleep(retry).await;
                            retry = (retry * 2).min(Duration::from_secs(5));
                        }
                    }
                    continue;
                }
                Read::Gap { to, .. } => {
                    *s.sent.lock().unwrap() = to;
                    continue;
                }
                Read::UpToDate | Read::Beyond { .. } => {}
            }

            let end = *s.events.ended.lock().unwrap();
            if let Some(end) = end {
                let body = status_json(Some(end));
                match self
                    .post(&format!("/v1/runtime/sessions/{}/status", s.id), &body)
                    .await
                {
                    Ok(st) if st.is_success() => {
                        self.sessions.lock().unwrap().remove(&s.id);
                        self.forget_mcp(&s.id);
                        self.write_table();
                        tracing::info!(session = %s.id, status = body["status"].as_str().unwrap_or(""), "session ended");
                        return;
                    }
                    _ => {
                        tokio::time::sleep(retry).await;
                        retry = (retry * 2).min(Duration::from_secs(5));
                        continue;
                    }
                }
            }

            // Nothing to send: wait for more, then let a burst gather.
            s.events.wake.notified().await;
            tokio::time::sleep(COALESCE).await;
        }
    }
}

// ---- the MCP Gateway (decision 81f) ---------------------------------------

/// The host-side bound on a forwarded call: the server's own deadline (60 s)
/// plus room for the stream to finish. Never a retry.
const MCP_FORWARD_TIMEOUT: Duration = Duration::from_secs(120);
/// The longest line a bridge may send.
const MCP_LINE_MAX: u64 = 4 * 1024 * 1024;

impl Host {
    /// Writes a session's provider config and registers its handle.
    fn mcp_launch(
        &self,
        session: &str,
        provider: &str,
        grants: &[crate::mcp::McpGrant],
    ) -> Result<crate::mcp::LaunchExtras> {
        if grants.is_empty() {
            return Ok(Default::default());
        }
        let handle = crate::mcp::new_handle();
        let exe = std::env::current_exe().context("finding this binary for the bridge")?;
        let extras = crate::mcp::write_provider_config(
            provider,
            &crate::mcp::session_dir(&self.state_dir, session),
            grants,
            &handle,
            &crate::mcp::socket_path(&self.state_dir),
            &exe,
            self.opencode_settings.as_ref(),
        )?;
        if extras != crate::mcp::LaunchExtras::default() {
            self.handles.register(&handle, session, grants);
        }
        Ok(extras)
    }

    /// A session ended: its handle stops working and its directory goes.
    fn forget_mcp(&self, session: &str) {
        self.handles.forget_session(session);
        self.subscribers
            .lock()
            .unwrap()
            .retain(|(s, _), _| s != session);
        let _ = std::fs::remove_dir_all(crate::mcp::session_dir(&self.state_dir, session));
    }

    /// `run/mcp.sock`, in a `0700` directory: only this account's processes
    /// (the bridges it launched) can connect.
    fn bind_socket(&self) -> Result<tokio::net::UnixListener> {
        let path = crate::mcp::socket_path(&self.state_dir);
        crate::mcp::private_dir(path.parent().expect("socket has a parent"))?;
        let _ = std::fs::remove_file(&path);
        let listener = tokio::net::UnixListener::bind(&path)
            .with_context(|| format!("binding {}", path.display()))?;
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        Ok(listener)
    }

    async fn serve_socket(self: Arc<Self>, listener: tokio::net::UnixListener) {
        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    tokio::spawn(self.clone().bridge_connection(stream));
                }
                Err(e) => {
                    tracing::warn!(error = %e, "mcp socket accept failed");
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
            }
        }
    }

    /// One bridge connection: a message to forward, or a subscription.
    async fn bridge_connection(self: Arc<Self>, stream: tokio::net::UnixStream) {
        use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};
        let (read, mut write) = stream.into_split();
        let mut line = String::new();
        let mut reader = tokio::io::BufReader::new(read.take(MCP_LINE_MAX));
        if reader.read_line(&mut line).await.is_err() {
            return;
        }
        let Ok(request) = serde_json::from_str::<crate::mcp::BridgeLine>(&line) else {
            return;
        };
        let send = |v: serde_json::Value| {
            let mut bytes = serde_json::to_vec(&v).unwrap_or_default();
            bytes.push(b'\n');
            bytes
        };
        let Some((session, connection)) =
            self.handles.resolve(&request.handle, &request.connection)
        else {
            let _ = write
                .write_all(&send(json!({"storm_error": "not_granted"})))
                .await;
            return;
        };
        if request.subscribe {
            let (tx, mut rx) = tokio::sync::mpsc::channel(64);
            self.subscribers
                .lock()
                .unwrap()
                .entry((session, connection))
                .or_default()
                .push(tx);
            while let Some(v) = rx.recv().await {
                if write.write_all(&send(v)).await.is_err() {
                    break;
                }
            }
            return;
        }
        let Some(message) = request.message else {
            return;
        };
        // At most once: no link, no call — never queued, never retried.
        let response = if *self.link_up.borrow() {
            self.post_for_response(
                &format!("/v1/runtime/sessions/{session}/mcp/{connection}"),
                &*message,
                Some(MCP_FORWARD_TIMEOUT),
            )
            .await
            .ok()
            .filter(|r| r.status().is_success())
        } else {
            None
        };
        let Some(response) = response else {
            let _ = write
                .write_all(&send(json!({"storm_error": "storm_unreachable"})))
                .await;
            return;
        };
        // The server's lines go through as they arrive. If the stream breaks,
        // the bridge sees no final response and fails the call once.
        let mut body = response.bytes_stream();
        while let Some(Ok(chunk)) = body.next().await {
            if write.write_all(&chunk).await.is_err() {
                return;
            }
        }
    }
}

/// The wire form of a session's status: running, or how it ended.
fn status_json(end: Option<SessionEnd>) -> serde_json::Value {
    match end {
        None => json!({ "status": "running" }),
        Some(SessionEnd::Completed { exit_code }) => {
            json!({ "status": "completed", "exit_code": exit_code })
        }
        Some(SessionEnd::Stopped) => json!({ "status": "stopped" }),
        Some(SessionEnd::Failed(reason)) => {
            let signal = match reason {
                EndReason::Signal(n) => Some(n),
                _ => None,
            };
            json!({ "status": "failed", "end_reason": reason.as_str(), "signal": signal })
        }
    }
}

fn read_table(state_dir: &Path) -> SessionTable {
    std::fs::read(state_dir.join(SESSIONS_FILE))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

/// Splits the first complete SSE event off `buf` (an event ends at a blank
/// line; `\r\n` is accepted). Returns the event and what remains.
fn split_event(buf: &[u8]) -> Option<(String, Vec<u8>)> {
    let text = std::str::from_utf8(buf).ok()?;
    let normalized = text.replace("\r\n", "\n");
    let at = normalized.find("\n\n")?;
    let event = normalized[..at].to_string();
    let rest = normalized.as_bytes()[at + 2..].to_vec();
    Some((event, rest))
}

/// The `data:` of an event, multi-line data joined with `\n`. Comments
/// (keepalives) and other fields are ignored.
fn event_data(event: &str) -> Option<String> {
    let lines: Vec<&str> = event
        .lines()
        .filter_map(|l| l.strip_prefix("data:"))
        .map(|d| d.strip_prefix(' ').unwrap_or(d))
        .collect();
    (!lines.is_empty()).then(|| lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sse_events_split_at_blank_lines_and_keepalives_carry_no_data() {
        let buf = b": keepalive\n\ndata: {\"a\":1}\n\ndata: partial";
        let (first, rest) = split_event(buf).unwrap();
        assert_eq!(event_data(&first), None);
        let (second, rest) = split_event(&rest).unwrap();
        assert_eq!(event_data(&second).as_deref(), Some("{\"a\":1}"));
        assert!(split_event(&rest).is_none(), "an incomplete event waits");
        let (crlf, _) = split_event(b"data: x\r\n\r\n").unwrap();
        assert_eq!(event_data(&crlf).as_deref(), Some("x"));
    }

    #[test]
    fn commands_parse_from_the_wire() {
        let env: Envelope = serde_json::from_str(
            r#"{"cmd_seq":3,"type":"start","session":"ags_X","workspace":"w","provider":"fake","interaction":"terminal","terminal":{"cols":80,"rows":24}}"#,
        )
        .unwrap();
        assert_eq!(env.cmd_seq, 3);
        assert!(matches!(env.command, Command::Start { .. }));
        let env: Envelope = serde_json::from_str(
            r#"{"cmd_seq":4,"type":"terminal.input","session":"s","data":"aGk="}"#,
        )
        .unwrap();
        assert!(matches!(env.command, Command::Input { .. }));
        let env: Envelope = serde_json::from_str(r#"{"cmd_seq":5,"type":"refresh"}"#).unwrap();
        assert!(matches!(env.command, Command::Refresh));
    }

    #[test]
    fn an_ending_has_one_wire_form() {
        assert_eq!(status_json(None)["status"], "running");
        let v = status_json(Some(SessionEnd::Failed(EndReason::Signal(9))));
        assert_eq!(
            (
                v["status"].as_str(),
                v["end_reason"].as_str(),
                v["signal"].as_i64()
            ),
            (Some("failed"), Some("signal"), Some(9))
        );
        let v = status_json(Some(SessionEnd::Completed { exit_code: Some(2) }));
        assert_eq!(v["exit_code"], 2);
    }
}
