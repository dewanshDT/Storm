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
//! **Revocation.** When the server refuses the key, every session is ended,
//! `host.json` becomes `host.json.revoked` (AM36) and the daemon exits
//! non-zero (freeze §5.6). An execution plane nobody can see must not keep
//! running agents.
//!
//! **Shutdown** (AM37). On SIGTERM or SIGINT, [`Host::shutdown`] ends every
//! session's process group and waits for them. Their endings are **not**
//! posted: `sessions.json` keeps them, so the next `hello` reports each as
//! `failed (host_restart)`, exactly as after a crash (§13). Nothing a session
//! spawned outlives the host, whichever service manager runs it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
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
use crate::terminal::{Title, TitleObserver};

const SESSIONS_FILE: &str = "sessions.json";
const BATCH: usize = 64 * 1024;
const COALESCE: Duration = Duration::from_millis(20);
/// At most one title report per session per this long (D15 AM43).
const TITLE_EVERY: Duration = Duration::from_secs(1);
/// How long a provider may take to start a session. A start that has not
/// returned by then is reported `failed (start_failure)`, and whatever it
/// later returns is stopped: a start that hangs costs its own session, never
/// the link (B-0).
/// How often the host tells the server it is alive (B-1), and how long the
/// link may carry nothing — the server's keepalives come every 15 s — before
/// the host takes it for dead and reconnects.
const HEARTBEAT: Duration = Duration::from_secs(15);
const LINK_IDLE: Duration = Duration::from_secs(45);
const START_DEADLINE: Duration = if cfg!(test) {
    Duration::from_secs(1)
} else {
    Duration::from_secs(30)
};

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
        /// Started from a note: the launch gains the fixed opening prompt.
        #[serde(default)]
        context: bool,
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

impl Command {
    /// The wire name, for the log.
    fn kind(&self) -> &'static str {
        match self {
            Self::Start { .. } => "start",
            Self::End { .. } => "end",
            Self::Input { .. } => "terminal.input",
            Self::Resize { .. } => "terminal.resize",
            Self::Replay { .. } => "terminal.replay",
            Self::Refresh => "refresh",
            Self::McpMessage { .. } => "mcp.message",
        }
    }

    /// The session a command is for, if it is for one.
    fn session(&self) -> Option<&str> {
        match self {
            Self::Start { session, .. }
            | Self::End { session }
            | Self::Input { session, .. }
            | Self::Resize { session, .. }
            | Self::Replay { session, .. }
            | Self::McpMessage { session, .. } => Some(session),
            Self::Refresh => None,
        }
    }
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
    title: Mutex<TitleObserver>,
}

impl SessionEvents for Events {
    fn output(&self, bytes: &[u8]) {
        self.title.lock().unwrap().feed(bytes);
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
    /// The title last acknowledged by the server, and when it was posted.
    title_sent: Mutex<(Title, Option<std::time::Instant>)>,
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
    /// Set once by [`Host::shutdown`]: from then on nothing is posted, so an
    /// ending the shutdown caused never reaches the server as `stopped`.
    shutting_down: AtomicBool,
    /// Sessions whose start is in flight, with the commands that arrived for
    /// them meanwhile, in order. They are applied once the session exists.
    starting: Mutex<HashMap<String, Vec<Command>>>,
}

/// How long a shutdown waits for its sessions beyond their two graces (the
/// group's, then any stragglers' in the session): output drains and the
/// waiter reaps them. With the 5 s default that is 13 s, inside launchd's
/// `ExitTimeOut` (20 s) and systemd's `TimeoutStopSec` (90 s).
const SHUTDOWN_DRAIN: Duration = Duration::from_secs(3);

impl Host {
    pub fn new(state_dir: &Path, config: RuntimeConfig) -> Result<Arc<Self>> {
        let providers = config.providers()?;
        Self::with_providers(state_dir, config, providers)
    }

    fn with_providers(
        state_dir: &Path,
        config: RuntimeConfig,
        providers: Vec<Arc<dyn Provider>>,
    ) -> Result<Arc<Self>> {
        let identity = HostConfig::load(state_dir)?;
        let key = HostKey::load(state_dir, &identity.key_id)?;
        let client = ServerClient::for_host(&identity)?;
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
            shutting_down: AtomicBool::new(false),
            starting: Mutex::new(HashMap::new()),
        }))
    }

    /// What this host offers, once, at start: the line to read first when a
    /// provider shows as missing in the app.
    pub fn log_inventory(&self) {
        tracing::info!(
            host = %self.identity.host_id,
            server = %self.identity.server_url,
            max_sessions = self.config.max_sessions,
            "identity"
        );
        for p in &self.providers {
            tracing::info!(
                provider = p.id().as_str(),
                kind = p.kind().as_str(),
                available = p.available() == Availability::Available,
                "provider"
            );
        }
        for root in &self.config.workspace_roots {
            tracing::info!(root = %root.display(), "workspace root");
        }
        tracing::info!(
            workspaces = list_workspaces(&self.config.workspace_roots).len(),
            "workspaces"
        );
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
            // It posts `/v1/runtime/heartbeat` while linked (B-1), so the
            // server may drop a link it has gone silent on.
            "heartbeat": true,
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
        if self.shutting_down.load(Ordering::SeqCst) {
            bail!("shutting down");
        }
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
                    // AM36: the identity is dead. Under launchd its absence is
                    // what stops the job being restarted.
                    if let Err(r) = HostConfig::mark_revoked(&self.state_dir) {
                        tracing::error!(error = %format!("{r:#}"), "could not mark the host revoked");
                    }
                    return Err(e);
                }
                Err(e) => tracing::warn!(error = %format!("{e:#}"), "link down"),
            }
            self.link_up.send_replace(false);
            tokio::time::sleep(backoff).await;
            backoff = (backoff * 2).min(Duration::from_secs(60));
        }
    }

    /// The orderly end of the host (AM37): every session's process group gets
    /// SIGHUP, then SIGKILL after the grace; then this waits, bounded, until
    /// each has drained and been reaped. Endings are not posted and
    /// `sessions.json` is left as it is (§13: `host_restart` on the next
    /// `hello`). The caller drops the link by dropping [`Host::run`].
    pub async fn shutdown(&self, grace: Duration) {
        self.shutting_down.store(true, Ordering::SeqCst);
        let sessions: Vec<Arc<HostSession>> =
            self.sessions.lock().unwrap().values().cloned().collect();
        if sessions.is_empty() {
            return;
        }
        tracing::info!(
            sessions = sessions.len(),
            "shutting down: ending every session"
        );
        for s in &sessions {
            s.session.lock().unwrap().stop(grace);
        }
        let deadline = tokio::time::Instant::now() + grace * 2 + SHUTDOWN_DRAIN;
        for s in &sessions {
            while s.events.ended.lock().unwrap().is_none() {
                if tokio::time::Instant::now() >= deadline {
                    tracing::warn!(session = %s.id, "did not end within the shutdown deadline");
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
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

        // The heartbeat: a `409` means the server has dropped this link.
        let relink = Arc::new(Notify::new());
        let _heartbeat = {
            let (host, relink) = (self.clone(), relink.clone());
            AbortOnDrop(tokio::spawn(async move {
                let mut tick = tokio::time::interval(HEARTBEAT);
                tick.tick().await;
                loop {
                    tick.tick().await;
                    let sent = host
                        .post_for_response("/v1/runtime/heartbeat", &json!({}), Some(HEARTBEAT))
                        .await;
                    if let Ok(r) = sent
                        && r.status() == reqwest::StatusCode::CONFLICT
                    {
                        relink.notify_one();
                        return;
                    }
                }
            }))
        };

        let mut buf: Vec<u8> = Vec::new();
        loop {
            tokio::select! {
                chunk = tokio::time::timeout(LINK_IDLE, stream.next()) => {
                    let Ok(chunk) = chunk else {
                        bail!("the link carried nothing for {} s", LINK_IDLE.as_secs());
                    };
                    let Some(chunk) = chunk else { return Ok(()) };
                    buf.extend_from_slice(&chunk.context("reading the link")?);
                    while let Some((event, rest)) = split_event(&buf) {
                        if let Some(data) = event_data(&event) {
                            match serde_json::from_str::<Envelope>(&data) {
                                Ok(env) => {
                                    *self.last_cmd_seq.lock().unwrap() = env.cmd_seq;
                                    self.dispatch(env.cmd_seq, env.command);
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
                _ = relink.notified() => {
                    tracing::warn!("the server has no link for this host; reconnecting");
                    return Ok(());
                }
            }
        }
    }

    async fn hello(&self) -> Result<()> {
        let mut sessions = Vec::new();
        for s in self.sessions.lock().unwrap().values() {
            let end = *s.events.ended.lock().unwrap();
            let mut entry = with_title(status_json(end), s.events.title.lock().unwrap().current());
            entry["id"] = json!(s.id);
            entry["output_end_offset"] = json!(s.events.ring.lock().unwrap().end());
            sessions.push(entry);
        }
        // A start still in flight is alive: without it here the server would
        // take it for lost, and the agent would run on with no session.
        for id in self.starting.lock().unwrap().keys() {
            if !sessions.iter().any(|s| s["id"] == json!(id)) {
                sessions.push(json!({
                    "id": id,
                    "status": "starting",
                    "output_end_offset": 0,
                }));
            }
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

    /// Takes one command off the link without ever waiting on a session
    /// (B-0). A start runs as its own task; until it returns, the commands
    /// for its session queue behind it, so they still apply in order. Every
    /// other command returns at once.
    fn dispatch(self: &Arc<Self>, cmd_seq: u64, command: Command) {
        match &command {
            Command::Input { .. } | Command::Resize { .. } | Command::McpMessage { .. } => {
                tracing::debug!(
                    seq = cmd_seq,
                    kind = command.kind(),
                    session = command.session(),
                    "command"
                )
            }
            _ => tracing::info!(
                seq = cmd_seq,
                kind = command.kind(),
                session = command.session(),
                "command"
            ),
        }
        match command {
            Command::Start { ref session, .. } => {
                let id = session.clone();
                {
                    let mut starting = self.starting.lock().unwrap();
                    let sessions = self.sessions.lock().unwrap();
                    if starting.contains_key(&id) || sessions.contains_key(&id) {
                        tracing::warn!(session = %id, "start for a session that exists; ignored");
                        return;
                    }
                    // In-flight starts hold their slot, or N concurrent
                    // starts would all pass the check in `start`.
                    if sessions.len() + starting.len() >= self.config.max_sessions as usize {
                        drop((starting, sessions));
                        tracing::warn!(session = %id, reason = "max_sessions reached", "start refused");
                        let host = self.clone();
                        tokio::spawn(async move { host.post_start_failure(&id).await });
                        return;
                    }
                    starting.insert(id.clone(), Vec::new());
                }
                let host = self.clone();
                tokio::spawn(async move {
                    host.clone().handle(command).await;
                    // A start that failed leaves its queue here: drop it.
                    host.starting.lock().unwrap().remove(&id);
                });
            }
            Command::Refresh => {
                tokio::spawn(self.clone().handle(command));
            }
            command => {
                if let Some(id) = command.session() {
                    let mut starting = self.starting.lock().unwrap();
                    if let Some(queue) = starting.get_mut(id) {
                        queue.push(command);
                        return;
                    }
                }
                self.handle_now(command);
            }
        }
    }

    /// Applies one command, a start to the end of its start.
    async fn handle(self: Arc<Self>, command: Command) {
        match command {
            Command::Start {
                session,
                workspace,
                provider,
                interaction,
                terminal,
                mcp,
                context,
            } => {
                self.start(
                    session,
                    workspace,
                    provider,
                    interaction,
                    terminal,
                    mcp,
                    context,
                )
                .await
            }
            Command::Refresh => {
                let _ = self
                    .post("/v1/runtime/inventory", &self.capabilities())
                    .await;
            }
            command => self.handle_now(command),
        }
    }

    /// Applies a command that is neither a start nor a refresh. It never
    /// waits: what can block (a stop, a resize, behind a PTY write that holds
    /// the session) runs on a blocking thread.
    fn handle_now(&self, command: Command) {
        match command {
            Command::Start { .. } | Command::Refresh => {
                debug_assert!(false, "{} handled inline", command.kind());
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
                    tokio::task::spawn_blocking(move || {
                        s.session.lock().unwrap().stop(DEFAULT_GRACE)
                    });
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
                    tokio::task::spawn_blocking(move || {
                        let mut guard = s.session.lock().unwrap();
                        let Interaction::Terminal(term) = guard.interaction();
                        let _ = term.resize(size);
                    });
                }
            }
            Command::Replay { session, from } => {
                if let Some(s) = self.session(&session) {
                    *s.sent.lock().unwrap() = from;
                    s.events.wake.notify_one();
                }
            }
        }
    }

    fn session(&self, id: &str) -> Option<Arc<HostSession>> {
        self.sessions.lock().unwrap().get(id).cloned()
    }

    #[allow(clippy::too_many_arguments)]
    async fn start(
        self: Arc<Self>,
        id: String,
        workspace: String,
        provider_id: String,
        interaction: Option<String>,
        size: Size,
        mcp: Vec<crate::mcp::McpGrant>,
        context: bool,
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
            Ok(extras) => crate::mcp::with_opening_prompt(extras, &provider_id, context),
            Err(e) => {
                tracing::warn!(session = %id, error = %format!("{e:#}"), "could not write the MCP config");
                return self.post_start_failure(&id).await;
            }
        };
        let events = Arc::new(Events {
            ring: Mutex::new(Scrollback::new(self.config.scrollback_bytes)),
            ended: Mutex::new(None),
            wake: Notify::new(),
            title: Mutex::new(TitleObserver::default()),
        });
        let spec = SessionSpec {
            session_id: id.clone(),
            workspace: dir,
            interaction: InteractionSpec::Terminal(size),
            launch,
        };
        tracing::info!(session = %id, provider = %provider_id, %workspace, "starting");
        let begun = std::time::Instant::now();
        let mut provider_start = {
            let events: Arc<dyn SessionEvents> = events.clone();
            tokio::task::spawn_blocking(move || provider.start(spec, events))
        };
        let started = match tokio::time::timeout(START_DEADLINE, &mut provider_start).await {
            Ok(started) => started,
            Err(_) => {
                tracing::warn!(
                    session = %id,
                    deadline_s = START_DEADLINE.as_secs(),
                    "start did not return in time; reporting it failed"
                );
                // Whatever it returns now has no session to belong to.
                tokio::spawn(async move {
                    if let Ok(Ok(mut late)) = provider_start.await {
                        tokio::task::spawn_blocking(move || late.stop(Duration::ZERO));
                    }
                });
                return self.post_start_failure(&id).await;
            }
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
            title_sent: Mutex::new((Title::default(), None)),
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
        tracing::info!(
            session = %id,
            provider = %provider_id,
            %workspace,
            took_ms = begun.elapsed().as_millis() as u64,
            "session started"
        );
        tokio::spawn(self.clone().upload(host_session));
        // What arrived while it started, in order. The lock is held
        // throughout, so a newer command waits for these rather than
        // overtaking them.
        let mut starting = self.starting.lock().unwrap();
        for command in starting.remove(&id).unwrap_or_default() {
            self.handle_now(command);
        }
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
            // A shutdown ends this session itself, and says so in the next
            // `hello`, not here (AM37).
            if self.shutting_down.load(Ordering::SeqCst) {
                return;
            }
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

            let title = s.events.title.lock().unwrap().current().clone();
            let end = *s.events.ended.lock().unwrap();
            let (sent, posted_at) = s.title_sent.lock().unwrap().clone();
            if end.is_none() && title != sent {
                let wait =
                    posted_at.map_or(Duration::ZERO, |t| TITLE_EVERY.saturating_sub(t.elapsed()));
                if !wait.is_zero() {
                    tokio::select! {
                        _ = s.events.wake.notified() => {}
                        _ = tokio::time::sleep(wait) => {}
                    }
                    continue;
                }
                let body = with_title(status_json(None), &title);
                match self
                    .post(&format!("/v1/runtime/sessions/{}/status", s.id), &body)
                    .await
                {
                    Ok(st) if st.is_success() => {
                        *s.title_sent.lock().unwrap() = (title, Some(std::time::Instant::now()));
                        retry = Duration::from_millis(250);
                    }
                    _ => {
                        tokio::time::sleep(retry).await;
                        retry = (retry * 2).min(Duration::from_secs(5));
                    }
                }
                continue;
            }
            if let Some(end) = end {
                let body = with_title(status_json(Some(end)), &title);
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

/// A status body with the session's title and activity (D15 AM43). A null
/// title changes nothing on the server; it keeps the last name it had.
fn with_title(mut body: serde_json::Value, title: &Title) -> serde_json::Value {
    body["title"] = json!(title.name);
    body["activity"] = json!(title.activity);
    body
}

/// Aborts a task when the scope that owns it ends.
struct AbortOnDrop(tokio::task::JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
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

    /// A stand-in server that answers every POST `200 {}` and records it.
    async fn recording_server() -> (String, Arc<Mutex<Vec<(String, serde_json::Value)>>>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let record = seen.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut conn, _)) = listener.accept().await else {
                    return;
                };
                let record = record.clone();
                tokio::spawn(async move {
                    let mut buf = Vec::new();
                    loop {
                        // One request: headers, then content-length bytes.
                        let head_end = loop {
                            if let Some(at) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                                break at + 4;
                            }
                            let mut chunk = [0u8; 4096];
                            match conn.read(&mut chunk).await {
                                Ok(0) | Err(_) => return,
                                Ok(n) => buf.extend_from_slice(&chunk[..n]),
                            }
                        };
                        let head = String::from_utf8_lossy(&buf[..head_end]).to_lowercase();
                        let path = head.split_whitespace().nth(1).unwrap_or("").to_string();
                        let len: usize = head
                            .lines()
                            .find_map(|l| l.strip_prefix("content-length:"))
                            .and_then(|v| v.trim().parse().ok())
                            .unwrap_or(0);
                        while buf.len() < head_end + len {
                            let mut chunk = [0u8; 4096];
                            match conn.read(&mut chunk).await {
                                Ok(0) | Err(_) => return,
                                Ok(n) => buf.extend_from_slice(&chunk[..n]),
                            }
                        }
                        let body = serde_json::from_slice(&buf[head_end..head_end + len])
                            .unwrap_or(serde_json::Value::Null);
                        buf.drain(..head_end + len);
                        record.lock().unwrap().push((path, body));
                        let reply = b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 2\r\n\r\n{}";
                        if conn.write_all(reply).await.is_err() {
                            return;
                        }
                    }
                });
            }
        });
        (url, seen)
    }

    fn alive(pid: i32) -> bool {
        rustix::process::test_kill_process(rustix::process::Pid::from_raw(pid).unwrap()).is_ok()
    }

    /// D15 AM43: the host reads the agent's title from its output and posts
    /// the name and activity, once per change, and `hello` carries them.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_agents_title_is_reported_once_per_change() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let (url, seen) = recording_server().await;
        let state = tempfile::tempdir().unwrap();
        let roots = tempfile::tempdir().unwrap();
        std::fs::create_dir(roots.path().join("w")).unwrap();
        let key = HostKey::generate();
        key.save(state.path()).unwrap();
        HostConfig {
            server_url: url,
            server_id: "srv_TEST".into(),
            server_pubkey: HostKey::generate().public_key_b64(),
            host_id: "hst_TEST".into(),
            key_id: key.key_id.clone(),
        }
        .save(state.path())
        .unwrap();
        let script = r#"t() { printf '\033]0;%s\007' "$1"; }
            t '✳ Claude Code'; t '◐ Fix the login'; t '◑ Fix the login'
            t '◐ Fix the login'; sleep 1.5; t '✳ Claude Code'; exec sleep 1000"#;
        let config: RuntimeConfig = toml::from_str(&format!(
            "workspace_roots = [{:?}]\n[[providers]]\nid = \"shell\"\ncommand = \"/bin/sh\"\nargs = [\"-c\", {:?}]\n",
            roots.path().display().to_string(),
            script,
        ))
        .unwrap();
        let host = Host::new(state.path(), config).unwrap();
        *host.token.write().await = Some("sht_test".into());
        host.link_up.send_replace(true);
        host.clone()
            .handle(Command::Start {
                session: "ags_TITLE".into(),
                workspace: "w".into(),
                provider: "shell".into(),
                interaction: Some("terminal".into()),
                terminal: Size { cols: 80, rows: 24 },
                mcp: Vec::new(),
                context: false,
            })
            .await;

        let titled = || -> Vec<serde_json::Value> {
            seen.lock()
                .unwrap()
                .iter()
                .filter(|(p, b)| p.ends_with("/ags_title/status") && b.get("title").is_some())
                .map(|(_, b)| json!({"title": b["title"], "activity": b["activity"]}))
                .collect()
        };
        let want = [
            json!({"title": null, "activity": "idle"}),
            json!({"title": "Fix the login", "activity": "working"}),
            json!({"title": "Fix the login", "activity": "idle"}),
        ];
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while titled().last() != want.last() {
            assert!(std::time::Instant::now() < deadline, "got {:?}", titled());
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let got = titled();
        assert!(
            got.len() <= want.len(),
            "a spinner frame was reported: {got:?}"
        );
        assert!(got.contains(&want[1]), "{got:?}");

        host.hello().await.unwrap();
        let hello = seen
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find(|(p, _)| p.ends_with("/hello"))
            .map(|(_, b)| b.clone())
            .unwrap();
        let entry = &hello["sessions"][0];
        assert_eq!(entry["title"], "Fix the login");
        assert_eq!(entry["activity"], "idle");
        host.shutdown(Duration::from_millis(200)).await;
    }

    /// AM37: a shutdown ends a session and everything it spawned — here a
    /// child that ignores SIGHUP, which only the group SIGKILL after the grace
    /// can end — posts no ending, and leaves `sessions.json` listing it, so
    /// the next `hello` reports `failed (host_restart)` (§13).
    #[tokio::test(flavor = "multi_thread")]
    async fn a_shutdown_ends_every_session_and_posts_no_ending() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let (url, seen) = recording_server().await;
        let state = tempfile::tempdir().unwrap();
        let roots = tempfile::tempdir().unwrap();
        std::fs::create_dir(roots.path().join("w")).unwrap();
        let key = HostKey::generate();
        key.save(state.path()).unwrap();
        HostConfig {
            server_url: url,
            server_id: "srv_TEST".into(),
            server_pubkey: HostKey::generate().public_key_b64(),
            host_id: "hst_TEST".into(),
            key_id: key.key_id.clone(),
        }
        .save(state.path())
        .unwrap();
        let config: RuntimeConfig = toml::from_str(&format!(
            r#"
workspace_roots = ["{}"]
[[providers]]
id = "shell"
command = "/bin/sh"
args = ["-c", "(trap '' HUP; exec sleep 1000) & echo bg:$!; wait"]
"#,
            roots.path().display()
        ))
        .unwrap();
        let host = Host::new(state.path(), config).unwrap();
        *host.token.write().await = Some("sht_test".into());
        host.link_up.send_replace(true);

        host.clone()
            .handle(Command::Start {
                session: "ags_SHUTDOWN".into(),
                workspace: "w".into(),
                provider: "shell".into(),
                interaction: Some("terminal".into()),
                terminal: Size { cols: 80, rows: 24 },
                mcp: Vec::new(),
                context: false,
            })
            .await;
        let s = host.session("ags_SHUTDOWN").expect("the session started");
        let pid = {
            let deadline = std::time::Instant::now() + Duration::from_secs(10);
            loop {
                let out = match s.events.ring.lock().unwrap().read(0, BATCH) {
                    Read::Data { bytes, .. } => String::from_utf8_lossy(&bytes).into_owned(),
                    _ => String::new(),
                };
                if let Some(pid) = out
                    .split("bg:")
                    .nth(1)
                    .and_then(|r| r.split_whitespace().next())
                    .and_then(|p| p.parse::<i32>().ok())
                {
                    break pid;
                }
                assert!(std::time::Instant::now() < deadline, "no pid in {out:?}");
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        };
        assert!(alive(pid));
        // The uploader is live: it has reported the session running.
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while !seen
            .lock()
            .unwrap()
            .iter()
            .any(|(_, b)| b["status"] == "running")
        {
            assert!(std::time::Instant::now() < deadline, "never posted running");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }

        host.shutdown(Duration::from_millis(300)).await;

        assert!(
            s.events.ended.lock().unwrap().is_some(),
            "the session ended"
        );
        assert!(
            !alive(pid),
            "the child that ignored SIGHUP survived the shutdown"
        );
        // Give a stray uploader every chance to post the ending.
        tokio::time::sleep(Duration::from_millis(500)).await;
        let endings: Vec<_> = seen
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, b)| b["status"].is_string() && b["status"] != "running")
            .cloned()
            .collect();
        assert!(
            endings.is_empty(),
            "a shutdown posted an ending: {endings:?}"
        );
        assert_eq!(
            read_table(state.path()).sessions,
            vec!["ags_SHUTDOWN".to_string()],
            "sessions.json must keep it for the next hello"
        );
    }

    /// A host on the recording server with `providers`, its link up.
    async fn test_host(
        providers: Vec<Arc<dyn Provider>>,
    ) -> (
        Arc<Host>,
        Arc<Mutex<Vec<(String, serde_json::Value)>>>,
        tempfile::TempDir,
        tempfile::TempDir,
    ) {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let (url, seen) = recording_server().await;
        let state = tempfile::tempdir().unwrap();
        let roots = tempfile::tempdir().unwrap();
        std::fs::create_dir(roots.path().join("w")).unwrap();
        let key = HostKey::generate();
        key.save(state.path()).unwrap();
        HostConfig {
            server_url: url,
            server_id: "srv_TEST".into(),
            server_pubkey: HostKey::generate().public_key_b64(),
            host_id: "hst_TEST".into(),
            key_id: key.key_id.clone(),
        }
        .save(state.path())
        .unwrap();
        let config: RuntimeConfig = toml::from_str(&format!(
            "workspace_roots = [\"{}\"]\nmax_sessions = 2",
            roots.path().display()
        ))
        .unwrap();
        let host = Host::with_providers(state.path(), config, providers).unwrap();
        *host.token.write().await = Some("sht_test".into());
        host.link_up.send_replace(true);
        (host, seen, state, roots)
    }

    fn start(session: &str, provider: &str) -> Command {
        Command::Start {
            session: session.into(),
            workspace: "w".into(),
            provider: provider.into(),
            interaction: Some("terminal".into()),
            terminal: Size { cols: 80, rows: 24 },
            mcp: Vec::new(),
            context: false,
        }
    }

    /// The fake provider under another id, whose `start` first waits for
    /// `gate` (or for `hold`, if the gate never opens).
    struct Gated {
        id: crate::provider::ProviderId,
        gate: Arc<(Mutex<bool>, std::sync::Condvar)>,
        hold: Duration,
        stopped: Arc<AtomicBool>,
    }

    impl Gated {
        fn new(id: &str, hold: Duration) -> Arc<Self> {
            Arc::new(Self {
                id: crate::provider::ProviderId::new(id).unwrap(),
                gate: Arc::new((Mutex::new(false), std::sync::Condvar::new())),
                hold,
                stopped: Arc::new(AtomicBool::new(false)),
            })
        }

        fn open(&self) {
            *self.gate.0.lock().unwrap() = true;
            self.gate.1.notify_all();
        }
    }

    struct Watched(Box<dyn ProviderSession>, Arc<AtomicBool>);

    impl ProviderSession for Watched {
        fn interaction(&mut self) -> Interaction<'_> {
            self.0.interaction()
        }
        fn stop(&mut self, grace: Duration) {
            self.1.store(true, Ordering::SeqCst);
            self.0.stop(grace)
        }
        fn status(&self) -> crate::status::SessionStatus {
            self.0.status()
        }
    }

    impl Provider for Gated {
        fn id(&self) -> &crate::provider::ProviderId {
            &self.id
        }
        fn kind(&self) -> crate::provider::ProviderKind {
            crate::provider::ProviderKind::Fake
        }
        fn interactions(&self) -> &[crate::provider::InteractionKind] {
            &[crate::provider::InteractionKind::Terminal]
        }
        fn available(&self) -> Availability {
            Availability::Available
        }
        fn start(
            &self,
            spec: SessionSpec,
            events: Arc<dyn SessionEvents>,
        ) -> Result<Box<dyn ProviderSession>, crate::provider::StartError> {
            let (open, cvar) = &*self.gate;
            let _ = cvar
                .wait_timeout_while(open.lock().unwrap(), self.hold, |open| !*open)
                .unwrap();
            let inner = crate::fake::FakeProvider::new().start(spec, events)?;
            Ok(Box::new(Watched(inner, self.stopped.clone())))
        }
    }

    fn output(host: &Host, id: &str) -> String {
        host.session(id)
            .map(|s| match s.events.ring.lock().unwrap().read(0, BATCH) {
                Read::Data { bytes, .. } => String::from_utf8_lossy(&bytes).into_owned(),
                _ => String::new(),
            })
            .unwrap_or_default()
    }

    async fn eventually(what: &str, mut cond: impl FnMut() -> bool) {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while !cond() {
            assert!(std::time::Instant::now() < deadline, "never: {what}");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    /// B-0: a start that never returns costs its own session, not the link.
    /// Another session starts and takes input meanwhile; the hung one is
    /// reported `start_failure` at the deadline; and what its start returns
    /// afterwards is stopped, not adopted.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_hung_start_blocks_neither_the_link_nor_other_sessions() {
        let hung = Gated::new("hung", Duration::from_secs(60));
        let (host, seen, _state, _roots) = test_host(vec![
            hung.clone(),
            Arc::new(crate::fake::FakeProvider::new()),
        ])
        .await;

        host.dispatch(1, start("ags_HUNG", "hung"));
        host.dispatch(2, start("ags_OK", "fake"));
        eventually("the second session started", || {
            host.session("ags_OK").is_some()
        })
        .await;
        host.dispatch(
            3,
            Command::Input {
                session: "ags_OK".into(),
                data: BASE64.encode(b"hi"),
            },
        );
        eventually("the second session echoed its input", || {
            output(&host, "ags_OK").contains("hi")
        })
        .await;
        // An End for the hung session neither blocks nor is lost.
        host.dispatch(
            4,
            Command::End {
                session: "ags_HUNG".into(),
            },
        );

        eventually("the hung start was reported failed", || {
            seen.lock()
                .unwrap()
                .iter()
                .any(|(path, b)| path.contains("ags_hung") && b["end_reason"] == "start_failure")
        })
        .await;
        assert!(host.session("ags_HUNG").is_none());
        // The server hears of the failure before the start's task has
        // dropped its queue.
        eventually("the hung start's queue was dropped", || {
            host.starting.lock().unwrap().is_empty()
        })
        .await;

        hung.open();
        eventually("the late session was stopped", || {
            hung.stopped.load(Ordering::SeqCst)
        })
        .await;
        assert!(
            host.session("ags_HUNG").is_none(),
            "a late start was adopted"
        );
    }

    /// A start in flight across a reconnect is reported in `hello`, so the
    /// server does not take it for lost; and in-flight starts hold their
    /// `max_sessions` slot, so concurrent starts cannot exceed it.
    #[tokio::test(flavor = "multi_thread")]
    async fn starts_in_flight_are_reported_and_hold_their_slot() {
        let slow = Gated::new("slow", Duration::from_secs(60));
        let (host, seen, _state, _roots) = test_host(vec![slow.clone()]).await;

        host.dispatch(1, start("ags_ONE", "slow"));
        host.dispatch(2, start("ags_TWO", "slow"));
        // A duplicate is ignored, not started twice.
        host.dispatch(3, start("ags_ONE", "slow"));
        // max_sessions = 2: a third is refused at once, not at the deadline.
        host.dispatch(4, start("ags_THREE", "slow"));
        eventually("the third start was refused", || {
            seen.lock()
                .unwrap()
                .iter()
                .any(|(path, b)| path.contains("ags_three") && b["end_reason"] == "start_failure")
        })
        .await;

        host.hello().await.unwrap();
        let hello = seen
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find(|(path, _)| path.ends_with("/hello"))
            .map(|(_, b)| b.clone())
            .expect("a hello");
        let mut reported: Vec<_> = hello["sessions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| (s["id"].as_str().unwrap().to_string(), s["status"].clone()))
            .collect();
        reported.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(
            reported,
            vec![
                ("ags_ONE".to_string(), json!("starting")),
                ("ags_TWO".to_string(), json!("starting")),
            ]
        );

        slow.open();
        eventually("both started", || {
            host.session("ags_ONE").is_some() && host.session("ags_TWO").is_some()
        })
        .await;
        assert!(host.session("ags_THREE").is_none());
    }

    /// B-0: commands for a session that is still starting wait for it, and
    /// then apply in the order they arrived.
    #[tokio::test(flavor = "multi_thread")]
    async fn commands_for_a_starting_session_apply_once_it_starts() {
        let slow = Gated::new("slow", Duration::from_secs(60));
        let ending = Gated::new("ending", Duration::from_secs(60));
        let (host, _seen, _state, _roots) = test_host(vec![slow.clone(), ending.clone()]).await;

        host.dispatch(1, start("ags_SLOW", "slow"));
        for (seq, chunk) in [(2, "a"), (3, "b"), (4, "c")] {
            host.dispatch(
                seq,
                Command::Input {
                    session: "ags_SLOW".into(),
                    data: BASE64.encode(chunk.as_bytes()),
                },
            );
        }
        host.dispatch(5, start("ags_ENDING", "ending"));
        host.dispatch(
            6,
            Command::End {
                session: "ags_ENDING".into(),
            },
        );
        assert!(host.session("ags_SLOW").is_none());

        slow.open();
        ending.open();
        eventually("the queued input arrived in order", || {
            output(&host, "ags_SLOW").contains("abc")
        })
        .await;
        eventually("the queued End stopped its session", || {
            ending.stopped.load(Ordering::SeqCst)
        })
        .await;
        assert!(!slow.stopped.load(Ordering::SeqCst));
    }

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
        assert!(matches!(env.command, Command::Start { context: false, .. }));
        let env: Envelope = serde_json::from_str(
            r#"{"cmd_seq":3,"type":"start","session":"ags_X","workspace":"w","provider":"claude-code","terminal":{"cols":80,"rows":24},"context":true}"#,
        )
        .unwrap();
        assert!(matches!(env.command, Command::Start { context: true, .. }));
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
