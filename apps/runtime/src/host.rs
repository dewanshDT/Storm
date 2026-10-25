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
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

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
/// Pasted images (D15 AM46): where they land, and the bounds on fetching one.
const INBOX: &str = "inbox";
const MAX_IMAGE_BYTES: u64 = 10 * 1024 * 1024;
const IMAGE_FETCH_TIMEOUT: Duration = Duration::from_secs(25);
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
/// How often a host looks for newer agent CLIs (133-5).
const AUTO_UPDATE_EVERY: Duration = Duration::from_secs(24 * 60 * 60);
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
        /// The agent's own session to resume (decision 89).
        #[serde(default)]
        resume: Option<String>,
        /// Token sync (AM69): the credential kind to pull before starting,
        /// never its value.
        #[serde(default)]
        provider_credential: Option<String>,
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
    /// Install, update or remove an agent CLI (decision 133-4): an agent and
    /// an operation, never a URL, version or command.
    #[serde(rename = "agent.install")]
    AgentInstall {
        job: String,
        agent: String,
        op: crate::install::Op,
    },
    /// A pasted image to fetch and stage in the session's inbox (D15 AM46).
    #[serde(rename = "terminal.image")]
    TerminalImage {
        session: String,
        image: String,
        size: u64,
        blake3: String,
        ext: String,
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
            Self::TerminalImage { .. } => "terminal.image",
            Self::AgentInstall { .. } => "agent.install",
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
            | Self::McpMessage { session, .. }
            | Self::TerminalImage { session, .. } => Some(session),
            Self::Refresh | Self::AgentInstall { .. } => None,
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
    /// The agent's terminal title names its conversation (decision 130);
    /// otherwise the session keeps its launch name.
    titles: bool,
    /// The agent's own session id, as last reported (decision 89).
    harness: Mutex<Option<String>>,
    /// The session's leader once the provider has started it (decision 135).
    leader: watch::Sender<Option<Arc<crate::platform::peer::Process>>>,
}

impl SessionEvents for Events {
    fn output(&self, bytes: &[u8]) {
        if self.titles {
            self.title.lock().unwrap().feed(bytes);
        }
        self.ring.lock().unwrap().append(bytes);
        self.wake.notify_one();
    }

    fn ended(&self, end: SessionEnd) {
        *self.ended.lock().unwrap() = Some(end);
        self.wake.notify_one();
    }

    fn leader(&self, leader: crate::platform::peer::Process) {
        self.leader.send_replace(Some(Arc::new(leader)));
    }
}

struct HostSession {
    id: String,
    /// The provider it runs, so a remove can refuse an agent in use (133-4).
    provider: String,
    session: Mutex<Box<dyn ProviderSession>>,
    events: Arc<Events>,
    /// The offset the server has acknowledged.
    sent: Mutex<u64>,
    /// `running`, posted once.
    running_sent: AtomicBool,
    /// Extra fields of that `running` report: the MCP servers the session
    /// skipped, and its plugins' local ones (decision 131a).
    start_report: serde_json::Value,
    /// The title last acknowledged by the server, and when it was posted.
    title_sent: Mutex<(Title, Option<std::time::Instant>)>,
    /// The agent's session id last acknowledged by the server.
    harness_sent: Mutex<Option<String>>,
    /// Input, in order, for this session's writer task. A queue so the link
    /// never waits on a PTY, and one writer so keystrokes never reorder.
    input: tokio::sync::mpsc::UnboundedSender<Vec<u8>>,
}

/// Bridges listening for unsolicited messages, by `(session, connection id)`.
type Subscribers = HashMap<(String, String), Vec<tokio::sync::mpsc::Sender<serde_json::Value>>>;

/// `harness-session` handles: handle → (session, provider, its events).
type HarnessHandles = HashMap<String, (String, String, Arc<Events>)>;

/// Each session's leader, by session id, from before it is started.
type Leaders = HashMap<String, watch::Receiver<Option<Arc<crate::platform::peer::Process>>>>;

/// How long a connection waits for its session's leader while the session
/// is still starting: the agent can run a hook before `start` returns.
const LEADER_WAIT: Duration = Duration::from_secs(5);

/// At most this many refusals a minute reach the server's audit (135-2).
const REFUSALS_PER_MINUTE: u32 = 30;
const REFUSAL_POST_TIMEOUT: Duration = Duration::from_secs(5);

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
    harness_handles: Mutex<HarnessHandles>,
    /// Bridge identity (decision 135): whether connections are checked, and
    /// each session's leader to check them against.
    peer_identity: crate::peer_identity::Identity,
    leaders: Mutex<Leaders>,
    /// Refusals told to the server in the current minute; past
    /// [`REFUSALS_PER_MINUTE`] only the journal has them.
    refusals_reported: Mutex<(Instant, u32)>,
    /// Refused connections, for the tests to see what the audit line said.
    #[cfg(test)]
    refused: Mutex<Vec<(String, &'static str, &'static str)>>,
    opencode_settings: Option<serde_json::Value>,
    /// Set once by [`Host::shutdown`]: from then on nothing is posted, so an
    /// ending the shutdown caused never reaches the server as `stopped`.
    shutting_down: AtomicBool,
    /// Sessions whose start is in flight, with the commands that arrived for
    /// them meanwhile, in order. They are applied once the session exists.
    starting: Mutex<HashMap<String, Vec<Command>>>,
    /// The remote servers this host's agents' plugins declare (131g): the
    /// account-wide listing, refreshed in the background so a `hello` never
    /// waits on it, and what launches found (project-enabled plugins).
    plugin_inventory: Arc<Mutex<Vec<serde_json::Value>>>,
    launch_remote: Mutex<Vec<serde_json::Value>>,
    /// The agents' versions and Node's, probed in the background (133-3).
    agents: Arc<crate::agents::Inventory>,
    /// Agents with a job running: one at a time each (133-4).
    agent_jobs: Mutex<std::collections::HashSet<String>>,
}

/// At most this many remote plugin servers in one inventory.
const PLUGIN_REMOTE_MAX: usize = 200;

/// How long a shutdown waits for its sessions beyond their two graces (the
/// group's, then any stragglers' in the session): output drains and the
/// waiter reaps them. With the 5 s default that is 13 s, inside launchd's
/// `ExitTimeOut` (20 s) and systemd's `TimeoutStopSec` (90 s).
const SHUTDOWN_DRAIN: Duration = Duration::from_secs(3);

impl Host {
    pub fn new(state_dir: &Path, config: RuntimeConfig) -> Result<Arc<Self>> {
        let providers = config.providers_with(&crate::agents::managed_bin(state_dir))?;
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
        crate::codex::forget_all(&crate::codex::home(state_dir));
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
            harness_handles: Mutex::new(HashMap::new()),
            peer_identity: crate::peer_identity::Identity::detect(),
            leaders: Mutex::new(HashMap::new()),
            refusals_reported: Mutex::new((Instant::now(), 0)),
            #[cfg(test)]
            refused: Mutex::new(Vec::new()),
            opencode_settings,
            shutting_down: AtomicBool::new(false),
            starting: Mutex::new(HashMap::new()),
            plugin_inventory: Arc::new(Mutex::new(Vec::new())),
            launch_remote: Mutex::new(Vec::new()),
            agents: Arc::default(),
            agent_jobs: Mutex::default(),
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
        let identity = self.peer_identity.report();
        tracing::info!(
            state = identity.state.as_str(),
            mechanism = identity.mechanism,
            reason = identity.reason,
            "bridge identity"
        );
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
                let mut cap = json!({
                    "id": p.id().as_str(),
                    "kind": p.kind().as_str(),
                    "interactions": p.interactions().iter().map(|i| i.as_str()).collect::<Vec<_>>(),
                    "available": p.available() == Availability::Available,
                });
                // Whether a session can have Storm's MCP (decision 130a). Said
                // only for a built-in: the server keeps its older behaviour
                // for an id this host knows nothing about.
                if let Some(mcp) = crate::profile::supports_mcp(p.id().as_str()) {
                    cap["mcp"] = json!(mcp);
                }
                self.agent_caps(p.as_ref(), &mut cap);
                cap
            })
            .collect();
        json!({
            "version": env!("CARGO_PKG_VERSION"),
            "providers": providers,
            "workspaces": list_workspaces(&self.config.workspace_roots),
            // Where a workspace goes, so an empty host can say where to put
            // one.
            "workspace_roots": self
                .config
                .workspace_roots
                .iter()
                .map(|r| r.display().to_string())
                .collect::<Vec<_>>(),
            "max_sessions": self.config.max_sessions,
            // This host runs `storm-runtime mcp-bridge` (spec §6). A host
            // that does not say so gets no grants.
            "mcp_bridge": true,
            // It posts `/v1/runtime/heartbeat` while linked (B-1), so the
            // server may drop a link it has gone silent on.
            "heartbeat": true,
            // It stages pasted images (`terminal.image`, 85e), so the server
            // may send it one instead of refusing the paste.
            "images": true,
            // It installs, updates and removes agent CLIs (`agent.install`,
            // decision 133-4).
            "agent_install": true,
            // The providers whose sessions it can resume (decision 89). A
            // host without this would start a resume as a new conversation.
            "resume": self
                .providers
                .iter()
                .map(|p| p.id().as_str())
                .filter(|id| crate::profile::resumable(id))
                .collect::<Vec<_>>(),
            // It pulls a synced login for a session whose `start` names one,
            // and fails the start if it cannot (AM69).
            "provider_credentials": [crate::credential::KIND],
            // Remote servers its agents' plugins declare, for the app's
            // suggestions (131g); never a header or env value.
            "plugin_mcp_remote": self.plugin_remote(),
            // Whether bridge and harness connections are bound to their
            // session (decision 135d). A host without it is `old_host`.
            "identity": self.peer_identity.report().to_json(),
        })
    }

    /// An installable agent's version, where it came from, and whether this
    /// host could install it now (133-3). Nothing for any other provider.
    fn agent_caps(&self, p: &dyn Provider, cap: &mut serde_json::Value) {
        let id = p.id().as_str();
        if !crate::agents::MANAGEABLE.contains(&id) {
            return;
        }
        if let Some(binary) = p.resolved() {
            cap["source"] = json!(crate::agents::source(&binary, &self.state_dir));
            if let Some(version) = self.agents.version(id) {
                cap["version"] = json!(version);
            }
        }
        if let Some(latest) = self.agents.latest(id) {
            cap["latest"] = json!(latest);
        }
        if !crate::install::verified(id) {
            cap["checksum_verified"] = json!(false);
        }
        let enabled = self.config.agent_install == crate::config::AgentInstall::On;
        match crate::agents::install_blocked(id, enabled, self.agents.node_major()) {
            None => cap["installable"] = json!(true),
            Some(reason) => {
                cap["installable"] = json!(false);
                cap["install_blocked"] = json!(reason);
            }
        }
    }

    /// Runs one `agent.install` job (133-4) and reports its steps and result
    /// on `/v1/runtime/agents/jobs/<job>`, then a fresh inventory.
    async fn agent_job(self: Arc<Self>, job: String, agent: String, op: crate::install::Op) {
        use crate::install::{Failure, Op};
        let job_ok = job.len() <= 64
            && job.starts_with("agj_")
            && job[4..].chars().all(|c| c.is_ascii_alphanumeric());
        if !job_ok {
            tracing::warn!(job = %job, "agent job: a malformed job id, ignored");
            return;
        }
        let report = format!("/v1/runtime/agents/jobs/{job}");
        let enabled = self.config.agent_install == crate::config::AgentInstall::On;
        let refused = if crate::install::recipe(&agent).is_none() {
            Some(Failure::Unknown)
        } else if let (false, _) | (_, Op::Install | Op::Update) = (enabled, op)
            && let Some(reason) =
                crate::agents::install_blocked(&agent, enabled, self.agents.node_major())
        {
            Some(Failure::Blocked(reason))
        } else if op == Op::Remove
            && self
                .sessions
                .lock()
                .unwrap()
                .values()
                .any(|s| s.provider == agent && s.events.ended.lock().unwrap().is_none())
        {
            Some(Failure::InUse)
        } else if !self.agent_jobs.lock().unwrap().insert(agent.clone()) {
            Some(Failure::Busy)
        } else {
            None
        };
        if let Some(failure) = refused {
            let _ = self
                .post(&report, &json!({ "failed": failure.code() }))
                .await;
            return;
        }
        self.run_job(job, agent, op).await;
    }

    /// Runs a job that passed its checks and holds its agent's slot in
    /// `agent_jobs`, which it releases.
    async fn run_job(self: Arc<Self>, job: String, agent: String, op: crate::install::Op) {
        use crate::install::Failure;
        let report = format!("/v1/runtime/agents/jobs/{job}");
        tracing::info!(agent = %agent, ?op, "agent job started");
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<&'static str>();
        let poster = {
            let host = self.clone();
            let report = report.clone();
            tokio::spawn(async move {
                while let Some(step) = rx.recv().await {
                    let _ = host.post(&report, &json!({ "step": step })).await;
                }
            })
        };
        let path = self
            .config
            .search_path_with(&crate::agents::managed_bin(&self.state_dir))
            .ok();
        let step = move |s: &'static str| {
            let _ = tx.send(s);
        };
        let result = tokio::time::timeout(
            Duration::from_secs(600),
            crate::install::run(&self.state_dir, &agent, op, path, &step),
        )
        .await
        .unwrap_or(Err(Failure::Timeout));
        drop(step);
        let _ = poster.await;
        self.agent_jobs.lock().unwrap().remove(&agent);
        let body = match &result {
            Ok(version) => json!({ "done": { "version": version } }),
            Err(failure) => {
                tracing::warn!(agent = %agent, code = failure.code(), "agent job failed");
                json!({ "failed": failure.code() })
            }
        };
        let _ = self.post(&report, &body).await;
        if op == crate::install::Op::Update {
            self.agents.record_update(&agent, result.is_ok());
        }
        self.probe_agents();
    }

    /// Once a day, from a random point in the first hour (133-5): learns
    /// each managed agent's newest version and, with `agent_updates =
    /// "auto"`, updates one that is behind while none of its sessions runs.
    async fn auto_updates(self: Arc<Self>) {
        let first = Duration::from_secs(rand::random_range(300..3600));
        tokio::time::sleep(first).await;
        loop {
            if *self.link_up.borrow() {
                self.clone().check_updates().await;
            }
            tokio::time::sleep(AUTO_UPDATE_EVERY).await;
        }
    }

    async fn check_updates(self: Arc<Self>) {
        let auto = self.config.agent_updates == crate::config::AgentUpdates::Auto
            && self.config.agent_install == crate::config::AgentInstall::On;
        let mut changed = false;
        for agent in crate::agents::MANAGEABLE {
            if crate::install::managed_target(&self.state_dir, agent).is_none() {
                continue;
            }
            let Ok(latest) = crate::install::latest(agent).await else {
                continue;
            };
            changed |= self.agents.set_latest(agent, &latest);
            let behind = self.agents.version(agent).is_some_and(|v| v != latest);
            let in_use = self
                .sessions
                .lock()
                .unwrap()
                .values()
                .any(|s| s.provider == agent && s.events.ended.lock().unwrap().is_none());
            if !auto || !behind || in_use || !self.agents.may_auto_update(agent) {
                continue;
            }
            if !self.agent_jobs.lock().unwrap().insert(agent.to_string()) {
                continue;
            }
            // The server creates the job, so an automatic update is audited and
            // shown like one started from the app.
            let created = self
                .post_for_response(
                    "/v1/runtime/agents/jobs",
                    &json!({ "agent": agent, "op": "update", "auto": true }),
                    None,
                )
                .await;
            let job = match created {
                Ok(r) if r.status().is_success() => r
                    .json::<serde_json::Value>()
                    .await
                    .ok()
                    .and_then(|v| v["job"].as_str().map(str::to_owned)),
                _ => None,
            };
            match job {
                Some(job) => {
                    self.clone()
                        .run_job(job, agent.to_string(), crate::install::Op::Update)
                        .await
                }
                None => {
                    self.agent_jobs.lock().unwrap().remove(agent);
                }
            }
        }
        if changed {
            let _ = self
                .post("/v1/runtime/inventory", &self.capabilities())
                .await;
        }
    }

    /// Probes the agents' binaries and Node in the background, and sends the
    /// server a fresh inventory when what the host reports changed.
    fn probe_agents(self: &Arc<Self>) {
        let agents: Vec<(String, Option<PathBuf>)> = self
            .providers
            .iter()
            .filter(|p| crate::agents::MANAGEABLE.contains(&p.id().as_str()))
            .map(|p| (p.id().as_str().to_string(), p.resolved()))
            .collect();
        let path = self
            .config
            .search_path_with(&crate::agents::managed_bin(&self.state_dir))
            .ok();
        let host = self.clone();
        tokio::spawn(async move {
            let inventory = host.agents.clone();
            let changed =
                tokio::task::spawn_blocking(move || inventory.refresh(&agents, path.as_deref()))
                    .await
                    .unwrap_or(false);
            if changed && *host.link_up.borrow() {
                let _ = host
                    .post("/v1/runtime/inventory", &host.capabilities())
                    .await;
            }
        });
    }

    fn plugin_remote(&self) -> Vec<serde_json::Value> {
        let listed = self.plugin_inventory.lock().unwrap().clone();
        let launched = self.launch_remote.lock().unwrap().clone();
        let mut out: Vec<serde_json::Value> = Vec::new();
        for e in listed.into_iter().chain(launched) {
            let same = |o: &serde_json::Value| {
                ["agent", "plugin", "server", "url"]
                    .iter()
                    .all(|k| o[k] == e[k])
            };
            if out.len() < PLUGIN_REMOTE_MAX && !out.iter().any(same) {
                out.push(e);
            }
        }
        out
    }

    /// Lists the account's plugins again in the background; the next
    /// inventory carries the result.
    fn refresh_plugin_inventory(&self) {
        use crate::profile::Mcp;
        let mcp_of =
            |p: &Arc<dyn Provider>| crate::profile::get(p.id().as_str()).and_then(|p| p.mcp);
        let claude: Vec<_> = self
            .providers
            .iter()
            .filter(|p| {
                mcp_of(p) == Some(Mcp::ClaudeCode) && p.available() == Availability::Available
            })
            .filter_map(|p| p.helper_command())
            .collect();
        let codex = self
            .providers
            .iter()
            .any(|p| mcp_of(p) == Some(Mcp::Codex))
            .then(|| crate::codex::home(&self.state_dir));
        if claude.is_empty() && codex.is_none() {
            return;
        }
        let dir = self.state_dir.clone();
        let slot = self.plugin_inventory.clone();
        tokio::task::spawn_blocking(move || {
            let mut out = Vec::new();
            for helper in claude {
                out.extend(crate::plugins::inventory(helper, &dir));
            }
            if let Some(home) = codex {
                out.extend(crate::codex::remote_inventory(&home));
            }
            *slot.lock().unwrap() = out;
        });
    }

    fn write_table(&self) {
        let table = SessionTable {
            sessions: self.sessions.lock().unwrap().keys().cloned().collect(),
        };
        let path = self.state_dir.join(SESSIONS_FILE);
        let result = crate::fsutil::write_atomic(
            &path,
            &serde_json::to_vec(&table).unwrap_or_default(),
            crate::fsutil::Access::Default,
        );
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
        tokio::spawn(self.clone().auto_updates());
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
        self.probe_agents();
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
        self.refresh_plugin_inventory();
        let mut sessions = Vec::new();
        for s in self.sessions.lock().unwrap().values() {
            let end = *s.events.ended.lock().unwrap();
            let mut entry = with_title(status_json(end), s.events.title.lock().unwrap().current());
            entry = with_harness(entry, s.events.harness.lock().unwrap().as_deref());
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
            Command::Refresh | Command::TerminalImage { .. } | Command::AgentInstall { .. } => {
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
                resume,
                provider_credential,
            } => {
                self.start(
                    session,
                    workspace,
                    provider,
                    interaction,
                    terminal,
                    mcp,
                    context,
                    resume,
                    provider_credential,
                )
                .await
            }
            Command::Refresh => {
                self.refresh_plugin_inventory();
                self.probe_agents();
                let _ = self
                    .post("/v1/runtime/inventory", &self.capabilities())
                    .await;
            }
            Command::TerminalImage {
                session,
                image,
                size,
                blake3,
                ext,
            } => {
                tokio::spawn(self.stage_image(session, image, size, blake3, ext));
            }
            Command::AgentInstall { job, agent, op } => self.agent_job(job, agent, op).await,
            command => self.handle_now(command),
        }
    }

    /// Applies a command that is neither a start nor a refresh. It never
    /// waits: what can block (a stop, a resize, behind a PTY write that holds
    /// the session) runs on a blocking thread.
    fn handle_now(&self, command: Command) {
        match command {
            Command::Start { .. }
            | Command::Refresh
            | Command::TerminalImage { .. }
            | Command::AgentInstall { .. } => {
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
        resume: Option<String>,
        provider_credential: Option<String>,
    ) {
        let refuse = |reason: &str| {
            tracing::warn!(session = %id, reason, "start refused");
        };
        if interaction.as_deref().is_some_and(|i| i != "terminal") {
            refuse("unsupported interaction");
            return self.post_start_failure(&id).await;
        }
        // 136a: on an isolated host no session runs as this account. Until
        // this host can start one in its own unit (136-2), it starts none.
        if self.config.isolation == crate::config::Isolation::PerSession {
            refuse("isolation is on and this host cannot isolate a session");
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
        if let Some(r) = &resume
            && !crate::harness::valid_id(&provider_id, r)
        {
            refuse("not a session this provider can resume");
            return self.post_start_failure(&id).await;
        }
        // Token sync (AM71): pulled before anything is written; any failure
        // ends the launch, never a fallback to this host's own login.
        let synced = match provider_credential.as_deref() {
            None => None,
            Some(kind) => match self.synced_login(&id, kind, &provider_id).await {
                Ok(synced) => Some(synced),
                Err(e) => {
                    tracing::warn!(session = %id, error = %e, "start refused");
                    return self.post_start_failure(&id).await;
                }
            },
        };
        let is_synced = synced.is_some();
        // What the provider gets with or without grants (AM66), then its MCP
        // config (AM32). Written before the agent starts; a session whose
        // config cannot be written does not start rather than start without
        // its integrations.
        let session_dir = crate::mcp::session_dir(&self.state_dir, &id);
        let mut launch = match crate::mcp::provider_baseline(&provider_id, &session_dir, &dir) {
            Ok(extras) => extras,
            Err(e) => {
                tracing::warn!(session = %id, error = %format!("{e:#}"), "could not write the instructions config");
                return self.post_start_failure(&id).await;
            }
        };
        let codex_plan = match self.codex_baseline(&provider_id, &dir) {
            Ok((extras, plan)) => {
                launch.args.extend(extras.args);
                launch.env.extend(extras.env);
                plan
            }
            Err(e) => {
                tracing::warn!(session = %id, error = %format!("{e:#}"), "could not write the Codex config");
                return self.post_start_failure(&id).await;
            }
        };
        // A session's plugins (decision 131): a Claude Code session's local
        // servers join its `mcp.json` (Codex's are switched in its home,
        // above), and everything it will not have is named. Only when the
        // host keeps sessions to Storm's servers: otherwise Claude Code loads
        // its plugins itself (decision 132).
        let storm_only = self.config.agent_mcp == crate::config::AgentMcp::StormOnly;
        let is_claude = crate::profile::get(&provider_id).and_then(|p| p.mcp)
            == Some(crate::profile::Mcp::ClaudeCode);
        let plugin_plan = if codex_plan.is_some() {
            codex_plan
        } else if storm_only && !mcp.is_empty() && is_claude {
            let helper = provider.helper_command();
            let allow = self.config.plugin_mcp == crate::config::PluginMcp::Allow;
            let workspace = dir.clone();
            tokio::task::spawn_blocking(move || {
                crate::plugins::session_plan(helper, allow, &workspace)
            })
            .await
            .ok()
        } else {
            None
        };
        let plugin_servers = plugin_plan
            .as_ref()
            .map(|p| p.local.clone())
            .unwrap_or_default();
        let taken = match crate::profile::get(&provider_id).and_then(|p| p.mcp) {
            _ if storm_only => Default::default(),
            Some(crate::profile::Mcp::ClaudeCode) => crate::plugins::claude_names(&dir),
            Some(crate::profile::Mcp::OpenCode) => crate::mcp::opencode_names(),
            _ => Default::default(),
        };
        let own = crate::mcp::AgentServers {
            storm_only,
            taken: &taken,
        };
        let plugin_plan = match self.mcp_launch(
            &id,
            &provider_id,
            &dir,
            &mcp,
            &plugin_servers,
            &own,
        ) {
            Ok((extras, plan)) => {
                launch.args.extend(extras.args);
                launch.env.extend(extras.env);
                plugin_plan.or(plan)
            }
            Err(e) => {
                tracing::warn!(session = %id, error = %format!("{e:#}"), "could not write the MCP config");
                return self.post_start_failure(&id).await;
            }
        };
        let start_report = match &plugin_plan {
            Some(plan) => {
                {
                    let mut found = self.launch_remote.lock().unwrap();
                    for e in &plan.remote {
                        if !found.contains(e) && found.len() < PLUGIN_REMOTE_MAX {
                            found.push(e.clone());
                        }
                    }
                }
                if let Some(text) = plan.notice() {
                    let path = session_dir.join(crate::plugins::NOTICE_FILE);
                    match crate::mcp::write_private(&path, text.as_bytes()) {
                        Ok(()) => launch
                            .env
                            .push((crate::plugins::NOTICE_ENV.into(), path.into_os_string())),
                        Err(e) => {
                            tracing::warn!(session = %id, error = %format!("{e:#}"), "could not write the MCP notice")
                        }
                    }
                }
                plan.report()
            }
            None => serde_json::Value::Null,
        };
        let events = Arc::new(Events {
            ring: Mutex::new(Scrollback::new(self.config.scrollback_bytes)),
            ended: Mutex::new(None),
            wake: Notify::new(),
            title: Mutex::new(TitleObserver::default()),
            titles: crate::profile::get(&provider_id).is_none_or(|p| p.titles),
            harness: Mutex::new(None),
            leader: watch::channel(None).0,
        });
        self.leaders
            .lock()
            .unwrap()
            .insert(id.clone(), events.leader.subscribe());
        let pins = synced.as_ref().map(|(pins, _)| pins);
        match self.harness_launch(&id, &provider_id, resume.as_deref(), &events, pins) {
            Ok(extras) => {
                launch.args.extend(extras.args);
                launch.env.extend(extras.env);
            }
            // A session that cannot report its id still runs; it only cannot
            // be resumed. A resume still names its conversation, and a synced
            // session's pins live in the same file.
            Err(e) => {
                tracing::warn!(session = %id, error = %format!("{e:#}"), "could not set up resume");
                if resume.is_some() || is_synced {
                    return self.post_start_failure(&id).await;
                }
            }
        }
        if let Some((_, token)) = synced {
            launch
                .env_remove
                .extend(crate::credential::ENV_REMOVE.map(Into::into));
            launch
                .secret_env
                .push((crate::credential::TOKEN_ENV.into(), token));
        }
        // Storm is the only thing that updates its own copy (133-2).
        if provider_id == "claude-code"
            && provider
                .resolved()
                .is_some_and(|b| crate::agents::source(&b, &self.state_dir) == "storm")
        {
            launch.env.push(("DISABLE_AUTOUPDATER".into(), "1".into()));
        }
        // A resumed conversation has read its note already.
        let launch =
            crate::mcp::with_opening_prompt(launch, &provider_id, context && resume.is_none());
        let spec = SessionSpec {
            session_id: id.clone(),
            workspace: dir,
            interaction: InteractionSpec::Terminal(size),
            launch,
        };
        tracing::info!(session = %id, provider = %provider_id, %workspace, synced = is_synced, "starting");
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
            provider: provider_id.clone(),
            session: Mutex::new(session),
            events,
            sent: Mutex::new(0),
            running_sent: AtomicBool::new(false),
            start_report,
            title_sent: Mutex::new((Title::default(), None)),
            harness_sent: Mutex::new(None),
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
                        &running_report(&s.start_report),
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
            let harness = s.events.harness.lock().unwrap().clone();
            let end = *s.events.ended.lock().unwrap();
            let (sent, posted_at) = s.title_sent.lock().unwrap().clone();
            let harness_changed = harness != *s.harness_sent.lock().unwrap();
            if end.is_none() && (title != sent || harness_changed) {
                let wait =
                    posted_at.map_or(Duration::ZERO, |t| TITLE_EVERY.saturating_sub(t.elapsed()));
                if !wait.is_zero() {
                    tokio::select! {
                        _ = s.events.wake.notified() => {}
                        _ = tokio::time::sleep(wait) => {}
                    }
                    continue;
                }
                let body = with_harness(with_title(status_json(None), &title), harness.as_deref());
                match self
                    .post(&format!("/v1/runtime/sessions/{}/status", s.id), &body)
                    .await
                {
                    Ok(st) if st.is_success() => {
                        *s.title_sent.lock().unwrap() = (title, Some(std::time::Instant::now()));
                        *s.harness_sent.lock().unwrap() = harness;
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
                let body = with_harness(
                    with_title(status_json(Some(end)), &title),
                    harness.as_deref(),
                );
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
        workspace: &Path,
        grants: &[crate::mcp::McpGrant],
        plugin_servers: &serde_json::Map<String, serde_json::Value>,
        own: &crate::mcp::AgentServers,
    ) -> Result<(crate::mcp::LaunchExtras, Option<crate::plugins::Plan>)> {
        use crate::profile::Mcp;
        if grants.is_empty() && !own.storm_only {
            return Ok(Default::default());
        }
        let handle = crate::mcp::new_handle();
        let exe = std::env::current_exe().context("finding this binary for the bridge")?;
        let socket = self.socket_address();
        let mcp = crate::profile::get(provider).and_then(|p| p.mcp);
        let (extras, plan) = if matches!(mcp, Some(Mcp::Codex | Mcp::Cursor | Mcp::CommandCode)) {
            let grants: Vec<_> = grants
                .iter()
                .filter(|g| crate::mcp::slug_ok(&g.slug))
                .collect();
            match mcp {
                _ if grants.is_empty() => Default::default(),
                Some(Mcp::Codex) => (
                    crate::codex::write_profile(
                        &crate::codex::home(&self.state_dir),
                        session,
                        &grants,
                        &handle,
                        &socket,
                        &exe,
                        &crate::codex::own_servers(&crate::codex::home(&self.state_dir)),
                    )?,
                    None,
                ),
                Some(Mcp::Cursor) => {
                    let home = account_home()?;
                    let (extras, plan) = crate::cursor::write_session(
                        &crate::mcp::session_dir(&self.state_dir, session),
                        workspace,
                        &home,
                        &crate::cursor::data_dir(&home),
                        &grants,
                        &handle,
                        &socket,
                        &exe,
                        own.storm_only,
                    )?;
                    (extras, Some(plan))
                }
                _ => {
                    let home = account_home()?;
                    let (extras, plan) = crate::command_code::write_session(
                        workspace,
                        &home,
                        &grants,
                        &handle,
                        &socket,
                        &exe,
                        own.storm_only,
                    )?;
                    (extras, Some(plan))
                }
            }
        } else {
            (
                crate::mcp::write_provider_config(
                    provider,
                    &crate::mcp::session_dir(&self.state_dir, session),
                    grants,
                    &handle,
                    &socket,
                    &exe,
                    self.opencode_settings.as_ref(),
                    plugin_servers,
                    own,
                )?,
                None,
            )
        };
        if !grants.is_empty() && extras != crate::mcp::LaunchExtras::default() {
            self.handles.register(&handle, session, grants);
        }
        Ok((extras, plan))
    }

    /// What every Codex session gains, grants or not (decision 130b): Storm's
    /// home, with its own keys written for this workspace, and under
    /// `storm-only` what keeps the account's servers out and its plugin plan
    /// (decisions 131i, 132c). Nothing for any other provider.
    fn codex_baseline(
        &self,
        provider: &str,
        workspace: &Path,
    ) -> Result<(crate::mcp::LaunchExtras, Option<crate::plugins::Plan>)> {
        if crate::profile::get(provider).and_then(|p| p.mcp) != Some(crate::profile::Mcp::Codex) {
            return Ok(Default::default());
        }
        let exe = std::env::current_exe().context("finding this binary for the hook")?;
        let allow = self.config.plugin_mcp == crate::config::PluginMcp::Allow;
        let storm_only = self.config.agent_mcp == crate::config::AgentMcp::StormOnly;
        let (extras, plan) = crate::codex::baseline(
            &crate::codex::home(&self.state_dir),
            workspace,
            &exe,
            allow,
            storm_only,
        )?;
        Ok((extras, storm_only.then_some(plan)))
    }

    /// What a session's launch gains so the agent's own session id reaches
    /// this host, with its handle registered (decision 89).
    fn harness_launch(
        &self,
        session: &str,
        provider: &str,
        resume: Option<&str>,
        events: &Arc<Events>,
        synced: Option<&serde_json::Map<String, serde_json::Value>>,
    ) -> Result<crate::mcp::LaunchExtras> {
        if !crate::profile::resumable(provider) {
            return Ok(Default::default());
        }
        let handle = crate::mcp::new_handle();
        let exe = std::env::current_exe().context("finding this binary for the hook")?;
        let (extras, known) = crate::harness::launch(
            provider,
            &crate::mcp::session_dir(&self.state_dir, session),
            &handle,
            &self.socket_address(),
            &exe,
            resume,
            synced,
        )?;
        *events.harness.lock().unwrap() = known;
        self.harness_handles.lock().unwrap().insert(
            handle,
            (session.to_string(), provider.to_string(), events.clone()),
        );
        Ok(extras)
    }

    /// A synced session's settings pins and its token, pulled for this one
    /// launch while the session is `starting` (AM71).
    async fn synced_login(
        &self,
        session: &str,
        kind: &str,
        provider: &str,
    ) -> Result<
        (
            serde_json::Map<String, serde_json::Value>,
            crate::credential::SecretOsString,
        ),
        crate::credential::SyncError,
    > {
        use crate::credential::{SyncError, check_launch, managed_dir, pull, settings};
        check_launch(kind, provider, &managed_dir())?;
        let pins = settings(|name| std::env::var_os(name))?;
        let token = self
            .token
            .read()
            .await
            .clone()
            .ok_or_else(|| SyncError::Unreachable("no host token yet".into()))?;
        let value = pull(
            self.client.http(),
            self.client.base(),
            &token,
            session,
            || self.reauth.notify_one(),
        )
        .await?;
        Ok((pins, value))
    }

    /// A session ended: its handle stops working and its directory goes.
    /// Fetches a pasted image, checks it, and writes it into the session's
    /// inbox; the server is told the path, or why not. Never retried.
    async fn stage_image(
        self: Arc<Self>,
        session: String,
        image: String,
        size: u64,
        hash: String,
        ext: String,
    ) {
        let staged = self.fetch_image(&session, &image, size, &hash, &ext).await;
        if let Err(e) = &staged {
            tracing::warn!(%session, %image, error = %format!("{e:#}"), "image not staged");
        }
        let body = match staged {
            Ok(path) => json!({ "staged": path }),
            Err(e) => json!({ "failed": format!("{e:#}") }),
        };
        let _ = self
            .post(
                &format!("/v1/runtime/sessions/{session}/terminal/images/{image}"),
                &body,
            )
            .await;
    }

    async fn fetch_image(
        &self,
        session: &str,
        image: &str,
        size: u64,
        hash: &str,
        ext: &str,
    ) -> Result<String> {
        let id_ok = image.strip_prefix("img_").is_some_and(|rest| {
            !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_alphanumeric())
        });
        if !id_ok || !matches!(ext, "png" | "jpg" | "gif" | "webp") {
            bail!("not an image this host stages");
        }
        if self.session(session).is_none() {
            bail!("no such session on this host");
        }
        if size > MAX_IMAGE_BYTES {
            bail!("too large");
        }
        let token = self
            .token
            .read()
            .await
            .clone()
            .ok_or_else(|| anyhow!("no token yet"))?;
        let bytes = self
            .client
            .http()
            .get(format!(
                "{}/v1/runtime/sessions/{session}/terminal/images/{image}",
                self.client.base()
            ))
            .bearer_auth(token)
            .timeout(IMAGE_FETCH_TIMEOUT)
            .send()
            .await
            .context("fetching the image")?
            .error_for_status()
            .context("the server refused the image")?
            .bytes()
            .await
            .context("reading the image")?;
        if bytes.len() as u64 != size || blake3::hash(&bytes).to_hex().as_str() != hash {
            bail!("the image arrived damaged");
        }
        let inbox =
            std::path::absolute(crate::mcp::session_dir(&self.state_dir, session).join(INBOX))?;
        write_private(&inbox, &format!("{image}.{ext}"), &bytes)
    }

    fn forget_mcp(&self, session: &str) {
        self.handles.forget_session(session);
        self.leaders.lock().unwrap().remove(session);
        self.harness_handles
            .lock()
            .unwrap()
            .retain(|_, (s, _, _)| s != session);
        self.subscribers
            .lock()
            .unwrap()
            .retain(|(s, _), _| s != session);
        let _ = std::fs::remove_dir_all(crate::mcp::session_dir(&self.state_dir, session));
        crate::codex::forget(&crate::codex::home(&self.state_dir), session);
    }

    /// `run/mcp.sock`, in a `0700` directory (a named pipe on Windows,
    /// AM55): only this account's processes (the bridges it launched) can
    /// connect.
    fn bind_socket(&self) -> Result<crate::platform::ipc::Listener> {
        let path = crate::mcp::socket_path(&self.state_dir);
        crate::platform::ipc::Listener::bind(&path)
            .with_context(|| format!("binding {}", path.display()))
    }

    /// What a session's bridges and hooks are told to connect to.
    fn socket_address(&self) -> PathBuf {
        crate::platform::ipc::address(&crate::mcp::socket_path(&self.state_dir)).into()
    }

    async fn serve_socket(self: Arc<Self>, mut listener: crate::platform::ipc::Listener) {
        loop {
            match listener.accept().await {
                Ok(stream) => {
                    tokio::spawn(self.clone().bridge_connection(stream));
                }
                Err(e) => {
                    tracing::warn!(error = %e, "mcp socket accept failed");
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
            }
        }
    }

    /// 135a: whether a connection that named `session` may act for it. On a
    /// host that is not `verified` it is handle-only, as before. Otherwise
    /// the connector must descend from the session's leader, and every
    /// failure — no leader, an error, a walk that does not arrive — refuses,
    /// audited on the session.
    async fn admit(
        &self,
        peer: Option<io::Result<crate::platform::peer::Process>>,
        session: &str,
        kind: &'static str,
    ) -> bool {
        let Some(peer) = peer else {
            return true;
        };
        let leader = self.leader_of(session).await;
        match verdict(peer, leader).await {
            Ok(()) => true,
            Err(why) => {
                self.refuse(session, kind, why).await;
                false
            }
        }
    }

    /// Audits a refusal on its session, never its handle. A precondition
    /// that stopped holding drops the host to `unavailable` there and then,
    /// and the server hears it at once (135d).
    async fn refuse(&self, session: &str, kind: &'static str, why: crate::peer_identity::Mismatch) {
        tracing::warn!(
            target: "storm_runtime::audit",
            %session,
            connection = kind,
            reason = why.as_str(),
            "identity_mismatch: connection refused"
        );
        #[cfg(test)]
        self.refused
            .lock()
            .unwrap()
            .push((session.to_string(), kind, why.as_str()));
        if let crate::peer_identity::Mismatch::Precondition(reason) = why
            && self.peer_identity.downgrade(reason)
        {
            tracing::error!(reason, "bridge identity is now unavailable");
            if *self.link_up.borrow() {
                let _ = self
                    .post("/v1/runtime/inventory", &self.capabilities())
                    .await;
            }
        }
        // The server's audit row (135-2). An older server answers 404, and a
        // flood of refusals is bounded here: the journal line above has each.
        if *self.link_up.borrow() && self.may_report_refusal() {
            let _ = self
                .post_for_response(
                    &format!("/v1/runtime/sessions/{session}/identity-refused"),
                    &json!({"connection": kind, "reason": why.as_str()}),
                    Some(REFUSAL_POST_TIMEOUT),
                )
                .await;
        }
    }

    fn may_report_refusal(&self) -> bool {
        let mut window = self.refusals_reported.lock().unwrap();
        if window.0.elapsed() >= Duration::from_secs(60) {
            *window = (Instant::now(), 0);
        }
        window.1 += 1;
        window.1 <= REFUSALS_PER_MINUTE
    }

    /// The session's leader, waiting a little for one still starting.
    async fn leader_of(&self, session: &str) -> Option<Arc<crate::platform::peer::Process>> {
        let mut rx = self.leaders.lock().unwrap().get(session)?.clone();
        let found = tokio::time::timeout(LEADER_WAIT, rx.wait_for(Option::is_some)).await;
        found.ok()?.ok()?.clone()
    }

    /// The agent's hook or plugin saying which conversation it is on. Only
    /// a handle this host gave out, and only an id its provider could have
    /// made, is taken.
    async fn harness_reported(
        &self,
        report: crate::harness::HarnessLine,
        peer: Option<io::Result<crate::platform::peer::Process>>,
    ) {
        let Some(session) = self
            .harness_handles
            .lock()
            .unwrap()
            .get(&report.handle)
            .map(|(s, _, _)| s.clone())
        else {
            return;
        };
        if !self.admit(peer, &session, "harness").await {
            return;
        }
        let handles = self.harness_handles.lock().unwrap();
        let Some((session, provider, events)) = handles.get(&report.handle) else {
            return;
        };
        if !crate::harness::valid_id(provider, &report.harness_session) {
            tracing::warn!(%session, "a harness session id that is not one; ignored");
            return;
        }
        let mut current = events.harness.lock().unwrap();
        if current.as_deref() != Some(&report.harness_session) {
            tracing::info!(%session, harness = %report.harness_session, "harness session");
            *current = Some(report.harness_session);
            events.wake.notify_one();
        }
    }

    /// One bridge connection: a message to forward, or a subscription.
    async fn bridge_connection(self: Arc<Self>, stream: crate::platform::ipc::Stream) {
        use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};
        // Who connected, asked before anything is read: the connector, even
        // if it hands the socket on (135b).
        let peer = self
            .peer_identity
            .verified()
            .then(|| crate::platform::peer::peer(&stream));
        let (read, mut write) = tokio::io::split(stream);
        let mut line = String::new();
        let mut reader = tokio::io::BufReader::new(read.take(MCP_LINE_MAX));
        if reader.read_line(&mut line).await.is_err() {
            return;
        }
        if let Ok(report) = serde_json::from_str::<crate::harness::HarnessLine>(&line) {
            return self.harness_reported(report, peer).await;
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
        if !self.admit(peer, &session, "bridge").await {
            let _ = write
                .write_all(&send(json!({"storm_error": "identity_mismatch"})))
                .await;
            return;
        }
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
/// 135a's answer for one connection: any error, or no leader, refuses.
async fn verdict(
    peer: io::Result<crate::platform::peer::Process>,
    leader: Option<Arc<crate::platform::peer::Process>>,
) -> Result<(), crate::peer_identity::Mismatch> {
    use crate::peer_identity::Mismatch;
    match (peer, leader) {
        (Err(e), _) => Err(Mismatch::from(e)),
        (_, None) => Err(Mismatch::LeaderGone),
        (Ok(peer), Some(leader)) => {
            tokio::task::spawn_blocking(move || crate::peer_identity::check(&peer, &leader))
                .await
                .unwrap_or(Err(Mismatch::Error(io::ErrorKind::Other)))
        }
    }
}

/// The host account's home, where Cursor and Command Code keep their own
/// config.
fn account_home() -> Result<PathBuf> {
    std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
        .context("HOME is not set")
}

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

/// Writes `name` into `dir` as `0600`, through a temporary file and a rename,
/// with `dir` and its parent `0700`. Returns the file's path.
fn write_private(dir: &Path, name: &str, bytes: &[u8]) -> Result<String> {
    use std::io::Write;
    std::fs::create_dir_all(dir).context("creating the inbox")?;
    for d in [dir.parent(), Some(dir)].into_iter().flatten() {
        crate::platform::tighten_private_dir(d)?;
    }
    let tmp = dir.join(format!(".{name}.tmp"));
    let _ = std::fs::remove_file(&tmp);
    let mut f = crate::platform::open_private(
        &tmp,
        std::fs::OpenOptions::new().write(true).create_new(true),
    )
    .context("creating the image file")?;
    f.write_all(bytes)?;
    f.sync_all()?;
    let path = dir.join(name);
    std::fs::rename(&tmp, &path)?;
    Ok(path.to_string_lossy().into_owned())
}

/// The first `running` report, with the session's start report merged in.
fn running_report(extra: &serde_json::Value) -> serde_json::Value {
    let mut body = json!({"status": "running"});
    if let (Some(body), Some(extra)) = (body.as_object_mut(), extra.as_object()) {
        for (k, v) in extra {
            body.insert(k.clone(), v.clone());
        }
    }
    body
}

/// A status body with the agent's own session id, once there is one.
fn with_harness(mut body: serde_json::Value, harness: Option<&str>) -> serde_json::Value {
    if let Some(id) = harness {
        body["harness_session"] = json!(id);
    }
    body
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

    #[cfg(unix)]
    fn alive(pid: i32) -> bool {
        rustix::process::test_kill_process(rustix::process::Pid::from_raw(pid).unwrap()).is_ok()
    }

    #[test]
    fn a_staged_image_is_private_and_whole() {
        let state = tempfile::tempdir().unwrap();
        let inbox = state.path().join("sessions/ags_X/inbox");
        let path = write_private(&inbox, "img_A.png", b"\x89PNG").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"\x89PNG");
        crate::platform::assert_private(std::path::Path::new(&path), 0o600);
        crate::platform::assert_private(&inbox, 0o700);
        crate::platform::assert_private(inbox.parent().unwrap(), 0o700);
        let left: Vec<_> = std::fs::read_dir(&inbox).unwrap().collect();
        assert_eq!(left.len(), 1, "no temporary file left");
    }

    /// D15 AM46: an image that does not match what the server announced is
    /// not staged, and the server is told why.
    #[tokio::test(flavor = "multi_thread")]
    #[cfg(unix)]
    async fn a_damaged_image_is_reported_not_staged() {
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
            "workspace_roots = [{:?}]\n[[providers]]\nid = \"shell\"\ncommand = \"/bin/sh\"\nargs = [\"-c\", \"exec sleep 1000\"]\n",
            roots.path().display().to_string(),
        ))
        .unwrap();
        let host = Host::new(state.path(), config).unwrap();
        *host.token.write().await = Some("sht_test".into());
        host.link_up.send_replace(true);
        host.clone()
            .handle(Command::Start {
                session: "ags_IMG".into(),
                workspace: "w".into(),
                provider: "shell".into(),
                interaction: Some("terminal".into()),
                terminal: Size { cols: 80, rows: 24 },
                mcp: Vec::new(),
                context: false,
                resume: None,
                provider_credential: None,
            })
            .await;
        // The recording server answers every request with `{}`: two bytes
        // that are not the image announced.
        host.clone()
            .handle(Command::TerminalImage {
                session: "ags_IMG".into(),
                image: "img_ABC".into(),
                size: 4,
                blake3: blake3::hash(b"\x89PNG").to_hex().to_string(),
                ext: "png".into(),
            })
            .await;
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let report = loop {
            let found = seen
                .lock()
                .unwrap()
                .iter()
                .find(|(p, b)| {
                    p.ends_with("/terminal/images/img_abc")
                        && b.is_object()
                        && !b.as_object().unwrap().is_empty()
                })
                .map(|(_, b)| b.clone());
            if let Some(b) = found {
                break b;
            }
            assert!(std::time::Instant::now() < deadline, "no report");
            tokio::time::sleep(Duration::from_millis(20)).await;
        };
        assert!(
            report["failed"].as_str().unwrap().contains("damaged"),
            "{report}"
        );
        assert!(
            !crate::mcp::session_dir(state.path(), "ags_IMG")
                .join(INBOX)
                .join("img_ABC.png")
                .exists()
        );

        // An id that could leave the inbox is refused before any fetch.
        let err = host
            .fetch_image("ags_IMG", "img_../../x", 4, "h", "png")
            .await
            .unwrap_err();
        assert!(format!("{err}").contains("not an image"), "{err}");
        host.shutdown(Duration::from_millis(200)).await;
    }

    /// A host whose `claude-code` and `opencode` are `/bin/sh` running
    /// `script` (the launch's own arguments follow as `$0`, `$1`, …).
    #[cfg(unix)]
    async fn harness_host(
        script: &str,
    ) -> (
        Arc<Host>,
        Arc<Mutex<Vec<(String, serde_json::Value)>>>,
        tempfile::TempDir,
        tempfile::TempDir,
    ) {
        harness_host_with(script, "").await
    }

    /// [`harness_host`] with `settings` at the top of its `runtime.toml`.
    #[cfg(unix)]
    async fn harness_host_with(
        script: &str,
        settings: &str,
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
        let mut toml = format!(
            "{settings}\nworkspace_roots = [{:?}]\n",
            roots.path().display().to_string()
        );
        for id in ["claude-code", "opencode", "codex"] {
            toml.push_str(&format!(
                "[[providers]]\nid = {id:?}\ncommand = \"/bin/sh\"\nargs = [\"-c\", {script:?}]\n"
            ));
        }
        let host = Host::new(state.path(), toml::from_str(&toml).unwrap()).unwrap();
        let listener = host.bind_socket().unwrap();
        tokio::spawn(host.clone().serve_socket(listener));
        *host.token.write().await = Some("sht_test".into());
        host.link_up.send_replace(true);
        (host, seen, state, roots)
    }

    #[cfg(unix)]
    fn harness_start(
        session: &str,
        provider: &str,
        resume: Option<&str>,
        context: bool,
    ) -> Command {
        Command::Start {
            session: session.into(),
            workspace: "w".into(),
            provider: provider.into(),
            interaction: Some("terminal".into()),
            terminal: Size {
                cols: 200,
                rows: 24,
            },
            mcp: Vec::new(),
            context,
            resume: resume.map(str::to_string),
            provider_credential: None,
        }
    }

    #[cfg(unix)]
    async fn wait_for(what: &str, mut ok: impl FnMut() -> bool) {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while !ok() {
            assert!(std::time::Instant::now() < deadline, "never saw {what}");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    #[cfg(unix)]
    fn harness_posts(seen: &Mutex<Vec<(String, serde_json::Value)>>, session: &str) -> Vec<String> {
        seen.lock()
            .unwrap()
            .iter()
            .filter(|(p, _)| p.ends_with(&format!("/{session}/status")))
            .filter_map(|(_, b)| b["harness_session"].as_str().map(str::to_string))
            .collect()
    }

    /// A command for a session's script that runs [`identity_probe`] as one
    /// of the session's own processes: `exec` makes it the leader itself.
    #[cfg(unix)]
    fn probe(mode: &str, exec: bool) -> String {
        format!(
            "STORM_PROBE='{mode}' {}'{}' --exact host::tests::identity_probe --nocapture -q --test-threads=1 >/dev/null 2>&1",
            if exec { "exec " } else { "" },
            std::env::current_exe().unwrap().display()
        )
    }

    #[cfg(unix)]
    fn rerun_probe(mode: String) -> std::process::Child {
        std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "host::tests::identity_probe",
                "--nocapture",
                "-q",
                "--test-threads=1",
            ])
            .env("STORM_PROBE", mode)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap()
    }

    #[cfg(unix)]
    fn wait_for_new_parent(was: u32) {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while std::os::unix::process::parent_id() == was {
            assert!(std::time::Instant::now() < deadline, "never re-parented");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Not a test: the process a session's script starts to reach its host
    /// the way a hook, a tool command or a stray would (decision 135). It
    /// does nothing unless `STORM_PROBE` names what to do.
    #[cfg(unix)]
    #[test]
    fn identity_probe() {
        use std::io::{Read, Write};
        use std::os::fd::{AsRawFd, FromRawFd};
        let Ok(mode) = std::env::var("STORM_PROBE") else {
            return;
        };
        let (verb, arg) = mode.split_once(' ').unwrap_or((&mode, ""));
        let socket = || std::env::var_os(crate::mcp::SOCKET_ENV).unwrap();
        let handle = || std::env::var(crate::harness::HANDLE_ENV).unwrap();
        let line = |handle: String, id: &str| {
            let mut l = serde_json::to_vec(&crate::harness::HarnessLine {
                handle,
                harness_session: id.into(),
            })
            .unwrap();
            l.push(b'\n');
            l
        };
        match verb {
            // A hook: the session's own process, with its own handle.
            "send" => crate::harness::send(socket(), handle(), arg.into()).unwrap(),
            // With the handle in file `arg` (another session's, say), or a
            // literal one when there is no such file.
            "as" => {
                let (h, id) = arg.split_once(' ').unwrap();
                let h = std::fs::read_to_string(h).unwrap_or_else(|_| h.to_string());
                crate::harness::send(socket(), h, id.into()).unwrap()
            }
            // A tool command: a new process session, still a descendant.
            "setsid" => {
                rustix::process::setsid().unwrap();
                crate::harness::send(socket(), handle(), arg.into()).unwrap()
            }
            // A detached shell: leaves the tree, then reports.
            "detach" => {
                rerun_probe(format!("orphan {} {arg}", std::process::id()));
                std::process::exit(0);
            }
            "orphan" => {
                let (parent, id) = arg.split_once(' ').unwrap();
                wait_for_new_parent(parent.parse().unwrap());
                crate::harness::send(socket(), handle(), id.into()).unwrap()
            }
            // An exited connector: connect, hand the socket to a child that
            // outlives this process, and let the child write.
            "handoff" => {
                let stream = std::os::unix::net::UnixStream::connect(socket()).unwrap();
                rustix::io::fcntl_setfd(&stream, rustix::io::FdFlags::empty()).unwrap();
                rerun_probe(format!(
                    "heir {} {} {arg}",
                    stream.as_raw_fd(),
                    std::process::id()
                ));
                std::process::exit(0);
            }
            "heir" => {
                let mut parts = arg.splitn(3, ' ');
                let fd: i32 = parts.next().unwrap().parse().unwrap();
                let parent: u32 = parts.next().unwrap().parse().unwrap();
                let id = parts.next().unwrap();
                wait_for_new_parent(parent);
                // SAFETY: the parent left this descriptor open for us.
                let mut stream = unsafe { std::os::unix::net::UnixStream::from_raw_fd(fd) };
                stream.write_all(&line(handle(), id)).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(10)))
                    .unwrap();
                let _ = stream.read(&mut [0u8; 64]);
            }
            // The session's bridge: one call through its own MCP config,
            // the answer written to a file.
            "bridge" => {
                let config = std::env::var("STORM_PROBE_CONFIG").unwrap();
                let config: serde_json::Value =
                    serde_json::from_slice(&std::fs::read(config).unwrap()).unwrap();
                let (_, server) = config["mcpServers"]
                    .as_object()
                    .unwrap()
                    .iter()
                    .next()
                    .unwrap();
                let env = &server["env"];
                let mut stream = std::os::unix::net::UnixStream::connect(
                    env[crate::mcp::SOCKET_ENV].as_str().unwrap(),
                )
                .unwrap();
                let mut request = serde_json::to_vec(&json!({
                    "handle": env[crate::mcp::HANDLE_ENV],
                    "connection": server["args"][1],
                    "message": {"jsonrpc": "2.0", "id": 1, "method": "ping"},
                }))
                .unwrap();
                request.push(b'\n');
                stream.write_all(&request).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(10)))
                    .unwrap();
                let mut answer = String::new();
                let _ = stream.read_to_string(&mut answer);
                std::fs::write("answer", answer).unwrap();
            }
            other => panic!("no probe {other:?}"),
        }
    }

    #[cfg(target_os = "linux")]
    fn refusals(host: &Host, session: &str) -> Vec<(&'static str, &'static str)> {
        host.refused
            .lock()
            .unwrap()
            .iter()
            .filter(|(s, _, _)| s == session)
            .map(|(_, kind, why)| (*kind, *why))
            .collect()
    }

    #[cfg(target_os = "linux")]
    fn harness_id(host: &Host, session: &str) -> Option<String> {
        host.session(session)?
            .events
            .harness
            .lock()
            .unwrap()
            .clone()
    }

    #[cfg(target_os = "linux")]
    fn handle_of(host: &Host, session: &str) -> String {
        host.harness_handles
            .lock()
            .unwrap()
            .iter()
            .find(|(_, (s, _, _))| s == session)
            .map(|(h, _)| h.clone())
            .unwrap()
    }

    /// A script that does `cases` by session id, then stays up.
    #[cfg(target_os = "linux")]
    fn by_session(cases: &[(&str, String)]) -> String {
        let arms: String = cases
            .iter()
            .map(|(session, cmd)| format!("{session}) {cmd};; "))
            .collect();
        format!(
            "while [ ! -e go ]; do sleep 0.05; done; \
             case \"$STORM_RUNTIME_SESSION\" in {arms}esac; exec sleep 1000"
        )
    }

    /// G-135-1, accepted: the session's own processes — its leader, a hook
    /// under it, and a tool command in a process session of its own.
    #[tokio::test(flavor = "multi_thread")]
    #[cfg(target_os = "linux")]
    async fn g_135_1_the_sessions_own_processes_are_accepted() {
        let ids = [
            ("ags_LEAD", "11111111-1111-4111-8111-111111111111"),
            ("ags_HOOK", "22222222-2222-4222-8222-222222222222"),
            ("ags_TOOL", "33333333-3333-4333-8333-333333333333"),
        ];
        let script = by_session(&[
            (ids[0].0, probe(&format!("send {}", ids[0].1), true)),
            (ids[1].0, probe(&format!("send {}", ids[1].1), false)),
            (
                ids[2].0,
                format!(
                    "/bin/sh -c \"{}\"",
                    probe(&format!("setsid {}", ids[2].1), false)
                ),
            ),
        ]);
        let (host, seen, _state, roots) = harness_host(&script).await;
        assert!(
            host.peer_identity.verified(),
            "{:?}",
            host.peer_identity.report()
        );
        assert_eq!(host.capabilities()["identity"]["state"], "verified");
        for (session, _) in ids {
            host.clone()
                .handle(harness_start(session, "claude-code", None, false))
                .await;
        }
        std::fs::write(roots.path().join("w").join("go"), "").unwrap();
        for (session, id) in ids {
            // The leader's session ends with it, so its report is read
            // from what was posted.
            wait_for(&format!("{session}'s report"), || {
                harness_posts(&seen, &session.to_lowercase()).contains(&id.to_string())
            })
            .await;
            assert_eq!(refusals(&host, session), [], "{session}");
        }
        host.shutdown(Duration::from_millis(200)).await;
    }

    /// G-135-1, refused and audited: a real handle from outside the session
    /// — the host's own process, another session's, a detached shell's — and
    /// a connector that exited and left its socket to a child. None of them
    /// changes a session's id.
    #[tokio::test(flavor = "multi_thread")]
    #[cfg(target_os = "linux")]
    async fn g_135_1_a_real_handle_from_outside_the_session_is_refused() {
        let id = "44444444-4444-4444-8444-444444444444";
        let script = by_session(&[
            ("ags_OTHER", probe(&format!("as victim {id}"), false)),
            ("ags_DETACH", probe(&format!("detach {id}"), false)),
            ("ags_HANDOFF", probe(&format!("handoff {id}"), false)),
        ]);
        let (host, _seen, state, roots) = harness_host(&script).await;
        assert!(host.peer_identity.verified());
        let sessions = ["ags_VICTIM", "ags_OTHER", "ags_DETACH", "ags_HANDOFF"];
        for session in sessions {
            host.clone()
                .handle(harness_start(session, "claude-code", None, false))
                .await;
        }
        let before: Vec<_> = sessions.iter().map(|s| harness_id(&host, s)).collect();
        let victim = handle_of(&host, "ags_VICTIM");
        std::fs::write(roots.path().join("w").join("victim"), &victim).unwrap();

        // The host's own process — no session's — with the real handle.
        let socket = crate::mcp::socket_path(state.path());
        let h = victim.clone();
        tokio::task::spawn_blocking(move || crate::harness::send(&socket, h, id.into()))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(refusals(&host, "ags_VICTIM"), [("harness", "reached_host")]);

        std::fs::write(roots.path().join("w").join("go"), "").unwrap();
        wait_for("another session's process refused", || {
            refusals(&host, "ags_VICTIM").len() == 2
        })
        .await;
        assert_eq!(
            refusals(&host, "ags_VICTIM")[1],
            ("harness", "reached_host")
        );
        for (session, why) in [
            ("ags_DETACH", "reached_init"),
            ("ags_HANDOFF", "peer_exited"),
        ] {
            wait_for(&format!("{session} refused"), || {
                !refusals(&host, session).is_empty()
            })
            .await;
            assert_eq!(refusals(&host, session), [("harness", why)], "{session}");
        }
        let after: Vec<_> = sessions.iter().map(|s| harness_id(&host, s)).collect();
        assert_eq!(after, before);
        host.shutdown(Duration::from_millis(200)).await;
    }

    /// 135a: a handle whose session has no leader on record — none was ever
    /// started, or it has gone — takes nothing, whoever sends it.
    #[tokio::test(flavor = "multi_thread")]
    #[cfg(target_os = "linux")]
    async fn a_session_without_a_leader_takes_no_connection() {
        let (host, _seen, state, _roots) = harness_host("exec sleep 1000").await;
        let events = Arc::new(Events {
            ring: Mutex::new(Scrollback::new(64)),
            ended: Mutex::new(None),
            wake: Notify::new(),
            title: Mutex::new(TitleObserver::default()),
            titles: false,
            harness: Mutex::new(None),
            leader: watch::channel(None).0,
        });
        host.harness_handles.lock().unwrap().insert(
            "h_ORPHAN".into(),
            ("ags_NOLEAD".into(), "claude-code".into(), events.clone()),
        );
        let socket = crate::mcp::socket_path(state.path());
        let id = "66666666-6666-4666-8666-666666666666";
        tokio::task::spawn_blocking(move || {
            crate::harness::send(&socket, "h_ORPHAN".into(), id.into())
        })
        .await
        .unwrap()
        .unwrap();
        assert_eq!(refusals(&host, "ags_NOLEAD"), [("harness", "leader_gone")]);
        assert_eq!(*events.harness.lock().unwrap(), None);
        host.shutdown(Duration::from_millis(200)).await;
    }

    /// 135a: a connector the kernel would not name is refused like any other
    /// failure, never waved through.
    #[tokio::test]
    #[cfg(target_os = "linux")]
    async fn a_peer_that_cannot_be_read_is_refused() {
        let me = Arc::new(crate::platform::peer::Process::open(std::process::id() as i32).unwrap());
        let r = verdict(Err(io::ErrorKind::InvalidInput.into()), Some(me)).await;
        assert_eq!(r.unwrap_err().as_str(), "error");
    }

    /// 135d: a precondition that stops holding while the host runs drops it
    /// to `unavailable` at once, and the server is told straight away.
    #[tokio::test(flavor = "multi_thread")]
    #[cfg(target_os = "linux")]
    async fn a_lost_precondition_downgrades_the_host_at_once() {
        let (host, seen, _state, _roots) = harness_host("exec sleep 1000").await;
        assert!(host.peer_identity.verified());
        host.refuse(
            "ags_X",
            "bridge",
            crate::peer_identity::Mismatch::Precondition("ptrace_scope"),
        )
        .await;
        assert!(!host.peer_identity.verified());
        let inventory = seen
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find(|(p, _)| p.ends_with("/inventory"))
            .map(|(_, b)| b.clone())
            .expect("an inventory was posted");
        assert_eq!(
            inventory["identity"],
            json!({"state": "unavailable", "mechanism": "so_peerpidfd", "reason": "ptrace_scope"})
        );
        host.shutdown(Duration::from_millis(200)).await;
    }

    /// G-135-1 for a bridge line: the session's own call is forwarded; the
    /// same handle from outside is answered `identity_mismatch`, audited,
    /// and never reaches the server.
    #[tokio::test(flavor = "multi_thread")]
    #[cfg(target_os = "linux")]
    async fn g_135_1_a_bridge_line_from_outside_is_never_forwarded() {
        use std::io::{Read, Write};
        // The probe finds the session's MCP config in its own arguments.
        let script =
            "for a in \"$0\" \"$@\"; do [ \"$prev\" = --mcp-config ] && cfg=$a; prev=$a; done; \
             while [ ! -e go ]; do sleep 0.05; done; "
                .to_string()
                + &probe("bridge", false)
                    .replace("STORM_PROBE=", "STORM_PROBE_CONFIG=\"$cfg\" STORM_PROBE=")
                + "; exec sleep 1000";
        let (host, seen, _state, roots) = harness_host(&script).await;
        let mut start = harness_start("ags_BRIDGE", "claude-code", None, false);
        if let Command::Start { mcp, .. } = &mut start {
            mcp.push(crate::mcp::McpGrant {
                id: "con_NOTES".into(),
                slug: "notes".into(),
                plugin_origin: None,
            });
        }
        host.clone().handle(start).await;
        let forwarded = || {
            seen.lock()
                .unwrap()
                .iter()
                .filter(|(p, _)| p.ends_with("/sessions/ags_bridge/mcp/con_notes"))
                .count()
        };
        std::fs::write(roots.path().join("w").join("go"), "").unwrap();
        let answer = roots.path().join("w").join("answer");
        wait_for("the session's own call", || answer.exists()).await;
        assert_eq!(std::fs::read_to_string(&answer).unwrap(), "{}");
        assert_eq!(forwarded(), 1);

        // Its handle, read from its config, presented from outside.
        let config_path = crate::mcp::session_dir(&host.state_dir, "ags_BRIDGE").join("mcp.json");
        let config: serde_json::Value =
            serde_json::from_slice(&std::fs::read(config_path).unwrap()).unwrap();
        let env = config["mcpServers"]["notes"]["env"].clone();
        let reply = tokio::task::spawn_blocking(move || {
            let mut stream = std::os::unix::net::UnixStream::connect(
                env[crate::mcp::SOCKET_ENV].as_str().unwrap(),
            )
            .unwrap();
            let line = json!({
                "handle": env[crate::mcp::HANDLE_ENV],
                "connection": "notes",
                "message": {"jsonrpc": "2.0", "id": 2, "method": "ping"},
            });
            stream.write_all(format!("{line}\n").as_bytes()).unwrap();
            let mut reply = String::new();
            stream.read_to_string(&mut reply).unwrap();
            reply
        })
        .await
        .unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&reply).unwrap(),
            json!({"storm_error": "identity_mismatch"})
        );
        assert_eq!(forwarded(), 1);
        assert_eq!(refusals(&host, "ags_BRIDGE"), [("bridge", "reached_host")]);
        // The server is told, for its audit row (135-2); never the handle.
        let told: Vec<serde_json::Value> = seen
            .lock()
            .unwrap()
            .iter()
            .filter(|(p, _)| p.ends_with("/sessions/ags_bridge/identity-refused"))
            .map(|(_, b)| b.clone())
            .collect();
        assert_eq!(
            told,
            [json!({"connection": "bridge", "reason": "reached_host"})]
        );
        host.shutdown(Duration::from_millis(200)).await;
    }

    /// G-135-3: a host that is not `verified` is handle-only, as before —
    /// it says so, and a line from outside still goes through.
    #[tokio::test(flavor = "multi_thread")]
    #[cfg(unix)]
    async fn an_unverified_host_is_handle_only_and_says_why() {
        let (host, _seen, state, _roots) = harness_host("exec sleep 1000").await;
        host.peer_identity.downgrade("ptrace_scope");
        let caps = host.capabilities();
        assert_eq!(caps["identity"]["state"], "unavailable");
        assert!(caps["identity"]["reason"].is_string());
        host.clone()
            .handle(harness_start("ags_OPEN", "claude-code", None, false))
            .await;
        let handle = host
            .harness_handles
            .lock()
            .unwrap()
            .iter()
            .find(|(_, (s, _, _))| s == "ags_OPEN")
            .map(|(h, _)| h.clone())
            .unwrap();
        let id = "55555555-5555-4555-8555-555555555555";
        let socket = crate::mcp::socket_path(state.path());
        tokio::task::spawn_blocking(move || crate::harness::send(&socket, handle, id.into()))
            .await
            .unwrap()
            .unwrap();
        wait_for("the outsider's report", || {
            host.session("ags_OPEN")
                .and_then(|s| s.events.harness.lock().unwrap().clone())
                .as_deref()
                == Some(id)
        })
        .await;
        host.shutdown(Duration::from_millis(200)).await;
    }

    /// Decision 89: the id a fresh Claude Code session starts with is
    /// reported, then whatever its hook says, but only through its own
    /// handle and only as an id Claude Code could have made. The reports
    /// come from the session's own processes, as a real hook's do: on a
    /// Linux host they are checked (decision 135).
    #[tokio::test(flavor = "multi_thread")]
    #[cfg(unix)]
    async fn the_agents_session_id_is_reported_as_it_changes() {
        let cleared = "0c2bf366-2db2-42da-9c3c-9498392b1f63";
        let script = format!(
            "while [ ! -e go ]; do sleep 0.05; done; {}; {}; {}; exec sleep 1000",
            probe(&format!("as not-a-handle {cleared}"), false),
            probe("send ses_notclaude", false),
            probe(&format!("send {cleared}"), false),
        );
        let (host, seen, _state, roots) = harness_host(&script).await;
        host.clone()
            .handle(harness_start("ags_HARN", "claude-code", None, false))
            .await;
        let first = host
            .session("ags_HARN")
            .unwrap()
            .events
            .harness
            .lock()
            .unwrap()
            .clone()
            .unwrap();
        assert!(crate::harness::valid_id("claude-code", &first));
        wait_for("the first id", || {
            harness_posts(&seen, "ags_harn").contains(&first)
        })
        .await;
        std::fs::write(roots.path().join("w").join("go"), "").unwrap();
        wait_for("the cleared id", || {
            harness_posts(&seen, "ags_harn").last().map(String::as_str) == Some(cleared)
        })
        .await;
        assert_eq!(
            harness_posts(&seen, "ags_harn"),
            [first, cleared.to_string()]
        );

        host.hello().await.unwrap();
        let hello = seen
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find(|(p, _)| p.ends_with("/hello"))
            .map(|(_, b)| b.clone())
            .unwrap();
        assert_eq!(hello["sessions"][0]["harness_session"], cleared);
        assert_eq!(hello["resume"], json!(["claude-code", "opencode", "codex"]));
        host.shutdown(Duration::from_millis(200)).await;
    }

    /// Decision 130a: the hello says which providers can have Storm's MCP —
    /// for built-ins only, whatever command runs them — and only those with
    /// conversations are offered for Resume.
    #[test]
    fn the_hello_says_which_providers_can_have_mcp() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let state = tempfile::tempdir().unwrap();
        let roots = tempfile::tempdir().unwrap();
        let key = HostKey::generate();
        key.save(state.path()).unwrap();
        HostConfig {
            server_url: "https://127.0.0.1:1".into(),
            server_id: "srv_TEST".into(),
            server_pubkey: HostKey::generate().public_key_b64(),
            host_id: "hst_TEST".into(),
            key_id: key.key_id.clone(),
        }
        .save(state.path())
        .unwrap();
        let mut toml = format!(
            "workspace_roots = [{:?}]\n",
            roots.path().display().to_string()
        );
        for id in [
            "claude-code",
            "opencode",
            "cursor",
            "command-code",
            "shell",
            "my-agent",
        ] {
            toml.push_str(&format!(
                "[[providers]]\nid = {id:?}\ncommand = \"/bin/true\"\n"
            ));
        }
        let host = Host::new(state.path(), toml::from_str(&toml).unwrap()).unwrap();
        let caps = host.capabilities();
        let mcp: Vec<(String, serde_json::Value)> = caps["providers"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| (p["id"].as_str().unwrap().to_string(), p["mcp"].clone()))
            .collect();
        assert_eq!(
            mcp,
            [
                ("claude-code".to_string(), json!(true)),
                ("opencode".to_string(), json!(true)),
                ("cursor".to_string(), json!(false)),
                ("command-code".to_string(), json!(false)),
                ("shell".to_string(), json!(false)),
                ("my-agent".to_string(), serde_json::Value::Null),
            ]
        );
        assert_eq!(caps["resume"], json!(["claude-code", "opencode"]));

        // Decision 133-3: an installable agent says where its binary came
        // from and whether this host could install it; `shell` and an
        // unknown id say neither.
        let by_id = |id: &str| {
            caps["providers"]
                .as_array()
                .unwrap()
                .iter()
                .find(|p| p["id"] == id)
                .unwrap()
                .clone()
        };
        let claude = by_id("claude-code");
        // `/bin/true` exists on Linux only; elsewhere the agent is simply
        // not installed, and says nothing of where it came from.
        if claude["available"] == json!(true) {
            assert_eq!(claude["source"], "system", "{claude}");
        } else {
            assert!(claude.get("source").is_none(), "{claude}");
        }
        assert_eq!(claude["installable"], json!(cfg!(unix)), "{claude}");
        for id in ["shell", "my-agent"] {
            let p = by_id(id);
            assert!(
                p.get("installable").is_none() && p.get("source").is_none(),
                "{p}"
            );
        }
        let off = Host::new(
            state.path(),
            toml::from_str(&format!("agent_install = \"off\"\n{toml}")).unwrap(),
        )
        .unwrap();
        let claude = off.capabilities()["providers"][0].clone();
        assert_eq!(claude["install_blocked"], "disabled", "{claude}");
    }

    /// 131g: the inventory lists the account's plugins in the background
    /// with the provider's own program, and reports their remote servers
    /// once each, beside what launches found.
    #[cfg(unix)]
    #[tokio::test]
    async fn the_inventory_reports_plugins_remote_servers() {
        use std::os::unix::fs::PermissionsExt;
        let state = tempfile::tempdir().unwrap();
        let key = HostKey::generate();
        key.save(state.path()).unwrap();
        HostConfig {
            server_url: "https://127.0.0.1:1".into(),
            server_id: "srv_TEST".into(),
            server_pubkey: HostKey::generate().public_key_b64(),
            host_id: "hst_TEST".into(),
            key_id: key.key_id.clone(),
        }
        .save(state.path())
        .unwrap();
        let bin = state.path().join("fake-claude");
        std::fs::write(
            &bin,
            r#"#!/bin/sh
[ "$1 $2 $3" = "plugin list --json" ] || exit 1
echo '[{"id":"slack@m","enabled":true,"installPath":"/nowhere","version":"1.0.0","mcpServers":{"slack":{"type":"http","url":"https://mcp.slack.com/mcp","oauth":{"clientId":"SECRET-ID"}}}},{"id":"proj@m","enabled":false,"projectEnabled":true,"installPath":"/nowhere","mcpServers":{"p":{"url":"https://p.example.com/mcp"}}}]'
"#,
        )
        .unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        let toml = format!(
            "[[providers]]\nid = \"claude-code\"\ncommand = {:?}\n",
            bin.display().to_string()
        );
        let host = Host::new(state.path(), toml::from_str(&toml).unwrap()).unwrap();
        assert_eq!(host.capabilities()["plugin_mcp_remote"], json!([]));
        host.refresh_plugin_inventory();
        let mut remote = json!([]);
        for _ in 0..200 {
            remote = host.capabilities()["plugin_mcp_remote"].clone();
            if remote.as_array().is_some_and(|r| !r.is_empty()) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        let list = remote.as_array().unwrap();
        assert_eq!(list.len(), 1, "only the account's own plugins: {remote}");
        assert_eq!(list[0]["plugin"], "slack");
        assert_eq!(list[0]["plugin_version"], "1.0.0");
        assert_eq!(list[0]["auth_hint"], "vendor_client");
        assert!(!remote.to_string().contains("SECRET-ID"));
        host.launch_remote.lock().unwrap().extend([list[0].clone(), json!({"agent": "claude-code", "plugin": "proj", "server": "p", "url": "https://p.example.com/mcp"})]);
        assert_eq!(
            host.capabilities()["plugin_mcp_remote"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
    }

    /// A resume names the conversation and skips the opening prompt; an id
    /// the provider could not have made never reaches a command line.
    #[tokio::test(flavor = "multi_thread")]
    #[cfg(unix)]
    async fn a_resume_reopens_the_conversation_or_fails_to_start() {
        let (host, seen, _state, _roots) =
            harness_host(r#"echo "ARGS $0 $*"; exec sleep 1000"#).await;
        let ses = "ses_ed5303b0dffeRnwXZdkDih2uxK";
        host.clone()
            .handle(harness_start("ags_RES", "opencode", Some(ses), true))
            .await;
        wait_for("the resumed id", || {
            harness_posts(&seen, "ags_res").contains(&ses.to_string())
        })
        .await;
        let output = || -> String {
            seen.lock()
                .unwrap()
                .iter()
                .filter(|(p, _)| p.ends_with("/ags_res/terminal/output"))
                .map(|(_, b)| {
                    String::from_utf8_lossy(
                        &BASE64
                            .decode(b["data"].as_str().unwrap().as_bytes())
                            .unwrap(),
                    )
                    .into_owned()
                })
                .collect()
        };
        wait_for("the arguments", || output().contains("ARGS")).await;
        let out = output();
        assert!(out.contains(&format!("-s {ses}")), "{out}");
        assert!(!out.contains("--prompt"), "{out}");

        host.clone()
            .handle(harness_start(
                "ags_BAD",
                "claude-code",
                Some("--dangerously-skip-permissions"),
                false,
            ))
            .await;
        assert!(host.session("ags_BAD").is_none());
        assert!(
            seen.lock()
                .unwrap()
                .iter()
                .any(|(p, b)| p.ends_with("/ags_bad/status") && b["end_reason"] == "start_failure")
        );
        host.shutdown(Duration::from_millis(200)).await;
    }

    /// Decision 133-4: a job the host must refuse is reported failed with
    /// its stable code, and never touches the managed folder.
    #[tokio::test(flavor = "multi_thread")]
    #[cfg(unix)]
    async fn an_agent_job_is_refused_with_a_stable_code() {
        let failed = |seen: &Arc<Mutex<Vec<(String, serde_json::Value)>>>, job: &str| {
            seen.lock()
                .unwrap()
                .iter()
                .find(|(p, b)| {
                    p.ends_with(&format!("/agents/jobs/{job}")) && b.get("failed").is_some()
                })
                .map(|(_, b)| b["failed"].as_str().unwrap().to_string())
        };
        let install = |job: &str, agent: &str, op: crate::install::Op| Command::AgentInstall {
            job: job.into(),
            agent: agent.into(),
            op,
        };
        let (host, seen, state, _roots) =
            harness_host(r#"echo "ARGS $0 $*"; exec sleep 1000"#).await;
        assert_eq!(host.capabilities()["agent_install"], json!(true));

        host.clone()
            .handle(install("agj_a1", "shell", crate::install::Op::Install))
            .await;
        assert_eq!(failed(&seen, "agj_a1").as_deref(), Some("unknown_agent"));

        host.clone()
            .handle(install("agj_a2", "codex", crate::install::Op::Remove))
            .await;
        assert_eq!(
            failed(&seen, "agj_a2").as_deref(),
            Some("not_managed"),
            "nothing of Storm's to remove"
        );

        host.clone()
            .handle(harness_start("ags_LIVE", "opencode", None, false))
            .await;
        host.clone()
            .handle(install("agj_a3", "opencode", crate::install::Op::Remove))
            .await;
        assert_eq!(failed(&seen, "agj_a3").as_deref(), Some("in_use"));

        host.clone()
            .handle(install("../evil", "codex", crate::install::Op::Install))
            .await;
        assert!(
            !seen.lock().unwrap().iter().any(|(p, _)| p.contains("evil")),
            "a malformed job id is never put in a URL"
        );
        assert!(
            !crate::agents::managed_dir(state.path())
                .join("opencode")
                .exists()
        );

        let (off, seen, _state, _roots) =
            harness_host_with(r#"exec sleep 1000"#, "agent_install = \"off\"").await;
        off.clone()
            .handle(install(
                "agj_b1",
                "claude-code",
                crate::install::Op::Install,
            ))
            .await;
        assert_eq!(failed(&seen, "agj_b1").as_deref(), Some("disabled"));
    }

    /// Decision 132: a Claude Code session keeps the agent's own MCP servers
    /// (no `--strict-mcp-config`) unless the host says `storm-only`, which
    /// holds even for a session without Storm's servers.
    #[tokio::test(flavor = "multi_thread")]
    #[cfg(unix)]
    async fn claude_code_keeps_its_own_servers_unless_the_host_is_storm_only() {
        let script = r#"echo "ARGS $0 $*"; exec sleep 1000"#;
        let args_of = |seen: &Arc<Mutex<Vec<(String, serde_json::Value)>>>, session: &str| {
            seen.lock()
                .unwrap()
                .iter()
                .filter(|(p, _)| p.ends_with(&format!("/{session}/terminal/output")))
                .map(|(_, b)| {
                    String::from_utf8_lossy(
                        &BASE64
                            .decode(b["data"].as_str().unwrap().as_bytes())
                            .unwrap(),
                    )
                    .into_owned()
                })
                .collect::<String>()
        };
        let grant = vec![crate::mcp::McpGrant {
            id: "storm".into(),
            slug: "storm".into(),
            plugin_origin: None,
        }];

        let (host, seen, _state, _roots) = harness_host(script).await;
        let mut start = harness_start("ags_OWN", "claude-code", None, false);
        if let Command::Start { mcp, .. } = &mut start {
            *mcp = grant.clone();
        }
        host.clone().handle(start).await;
        wait_for("the arguments", || {
            args_of(&seen, "ags_own").contains("ARGS")
        })
        .await;
        let out = args_of(&seen, "ags_own");
        assert!(out.contains("--mcp-config"), "{out}");
        assert!(!out.contains("--strict-mcp-config"), "{out}");

        let (host, seen, _state, _roots) =
            harness_host_with(script, "agent_mcp = \"storm-only\"").await;
        host.clone()
            .handle(harness_start("ags_ONLY", "claude-code", None, false))
            .await;
        wait_for("the arguments", || {
            args_of(&seen, "ags_only").contains("ARGS")
        })
        .await;
        let out = args_of(&seen, "ags_only");
        assert!(
            out.contains("--strict-mcp-config"),
            "no grants, still strict: {out}"
        );
    }

    /// Decision 130b: a Codex session runs in Storm's own home, with its
    /// servers in its own profile that asks for each tool, the prompt last,
    /// and the profile gone when the session ends; a resume names the
    /// conversation and has no prompt.
    #[tokio::test(flavor = "multi_thread")]
    #[cfg(unix)]
    async fn a_codex_session_gets_its_own_profile_in_storms_home() {
        let (host, seen, state, _roots) =
            harness_host(r#"echo "ARGS $0 $*"; echo "HOME=$CODEX_HOME"; exec sleep 1000"#).await;
        let output = |session: &str| -> String {
            seen.lock()
                .unwrap()
                .iter()
                .filter(|(p, _)| p.ends_with(&format!("/{session}/terminal/output")))
                .map(|(_, b)| {
                    String::from_utf8_lossy(
                        &BASE64
                            .decode(b["data"].as_str().unwrap().as_bytes())
                            .unwrap(),
                    )
                    .into_owned()
                })
                .collect()
        };
        let home = crate::codex::home(state.path());
        let mut start = harness_start("ags_CDX", "codex", None, true);
        if let Command::Start { mcp, .. } = &mut start {
            *mcp = vec![
                crate::mcp::McpGrant {
                    id: "mcc_storm".into(),
                    slug: "storm".into(),
                    plugin_origin: None,
                },
                crate::mcp::McpGrant {
                    id: "mcc_bad".into(),
                    slug: "Bad Slug".into(),
                    plugin_origin: None,
                },
            ];
        }
        host.clone().handle(start).await;
        wait_for("the arguments", || output("ags_cdx").contains("HOME=")).await;
        let out = output("ags_cdx");
        assert!(out.contains("-p storm-ags_CDX"), "{out}");
        assert!(
            out.contains(&format!("-p storm-ags_CDX {}", crate::mcp::OPENING_PROMPT)),
            "the prompt comes last: {out}"
        );
        assert!(out.contains(&format!("HOME={}", home.display())), "{out}");
        let profile: toml::Table = std::fs::read_to_string(home.join("storm-ags_CDX.config.toml"))
            .unwrap()
            .parse()
            .unwrap();
        let servers = profile["mcp_servers"].as_table().unwrap();
        assert_eq!(
            servers.keys().collect::<Vec<_>>(),
            ["storm"],
            "a bad slug is dropped"
        );
        assert_eq!(
            servers["storm"]["default_tools_approval_mode"].as_str(),
            Some("prompt")
        );
        let config: toml::Table = std::fs::read_to_string(home.join("config.toml"))
            .unwrap()
            .parse()
            .unwrap();
        // Decision 132c: the workspace's trust is Codex's own question, and
        // Storm's hook carries its trust record.
        assert!(config.get("projects").is_none(), "{config}");
        assert!(!config["hooks"]["state"].as_table().unwrap().is_empty());

        host.clone()
            .handle(Command::End {
                session: "ags_CDX".into(),
            })
            .await;
        wait_for("the profile to go", || {
            !home.join("storm-ags_CDX.config.toml").exists()
        })
        .await;

        let id = "01a15d7b-580b-7e12-87d8-0ba9ab803093";
        host.clone()
            .handle(harness_start("ags_CDR", "codex", Some(id), true))
            .await;
        wait_for("the resume", || output("ags_cdr").contains("HOME=")).await;
        let out = output("ags_cdr");
        assert!(out.contains(&format!("resume {id}")), "{out}");
        assert!(!out.contains(crate::mcp::OPENING_PROMPT), "{out}");
        wait_for("the resumed id", || {
            harness_posts(&seen, "ags_cdr").contains(&id.to_string())
        })
        .await;
        host.shutdown(Duration::from_millis(200)).await;
    }

    /// D15 AM43: the host reads the agent's title from its output and posts
    /// the name and activity, once per change, and `hello` carries them.
    #[tokio::test(flavor = "multi_thread")]
    #[cfg(unix)]
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
                resume: None,
                provider_credential: None,
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
    #[cfg(unix)]
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
                resume: None,
                provider_credential: None,
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
        test_host_with(providers, "").await
    }

    async fn test_host_with(
        providers: Vec<Arc<dyn Provider>>,
        extra_config: &str,
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
            "workspace_roots = [{:?}]\nmax_sessions = 2\n{extra_config}",
            roots.path().display().to_string()
        ))
        .unwrap();
        let host = Host::with_providers(state.path(), config, providers).unwrap();
        *host.token.write().await = Some("sht_test".into());
        host.link_up.send_replace(true);
        (host, seen, state, roots)
    }

    /// 136a: with isolation on, a host that cannot isolate a session
    /// starts none, rather than run it as its own account.
    #[tokio::test]
    async fn with_isolation_on_no_session_starts_as_the_host_account() {
        let (host, seen, _state, _roots) = test_host_with(
            vec![Arc::new(crate::fake::FakeProvider::new())],
            "isolation = \"per-session\"",
        )
        .await;
        host.dispatch(1, start("ags_ISO", "fake"));
        eventually("the start was reported failed", || {
            seen.lock()
                .unwrap()
                .iter()
                .any(|(path, b)| path.contains("ags_iso") && b["end_reason"] == "start_failure")
        })
        .await;
        assert!(host.session("ags_ISO").is_none());

        let (host, _seen, _state, _roots) =
            test_host(vec![Arc::new(crate::fake::FakeProvider::new())]).await;
        host.dispatch(1, start("ags_OFF", "fake"));
        eventually("with isolation off it starts", || {
            host.session("ags_OFF").is_some()
        })
        .await;
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
            resume: None,
            provider_credential: None,
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

    /// A host whose `claude-code` and `opencode` are `/bin/sh` recording
    /// their environment and arguments under `out`, with a provider env file
    /// holding competing credentials, on a server that answers the pull
    /// route with `answers`.
    #[cfg(unix)]
    async fn synced_host(
        answers: &[(u16, String)],
    ) -> (
        Arc<Host>,
        crate::credential::tests::Seen,
        tempfile::TempDir,
        tempfile::TempDir,
    ) {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let (url, seen) =
            crate::credential::tests::stub_server(Arc::new(Mutex::new(answers.to_vec()))).await;
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
        let env_file = roots.path().join("claude.env");
        std::fs::write(
            &env_file,
            "ANTHROPIC_API_KEY=sk-ant-api03-ENVFILE\nCLAUDE_CODE_OAUTH_TOKEN=sk-ant-oat01-ENVFILE\n\
             ANTHROPIC_BASE_URL=http://127.0.0.1:9\nCLAUDE_CODE_USE_BEDROCK=1\nDISABLE_TELEMETRY=1\n",
        )
        .unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&env_file, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        let out = roots.path().join("out");
        std::fs::create_dir(&out).unwrap();
        let script = format!(
            "env > {o}/env.$STORM_RUNTIME_SESSION; printf '%s\\n' \"$@\" > {o}/args.$STORM_RUNTIME_SESSION; exec sleep 1000",
            o = out.display()
        );
        let mut toml = format!(
            "workspace_roots = [{:?}]\n",
            roots.path().display().to_string()
        );
        for id in ["claude-code", "opencode"] {
            toml.push_str(&format!(
                "[[providers]]\nid = {id:?}\ncommand = \"/bin/sh\"\nargs = [\"-c\", {script:?}]\nenv_file = {:?}\n",
                env_file.display().to_string()
            ));
        }
        let host = Host::new(state.path(), toml::from_str(&toml).unwrap()).unwrap();
        *host.token.write().await = Some("sht_test".into());
        host.link_up.send_replace(true);
        (host, seen, state, roots)
    }

    #[cfg(unix)]
    fn synced_start(session: &str, provider: &str, kind: Option<&str>) -> Command {
        Command::Start {
            session: session.into(),
            workspace: "w".into(),
            provider: provider.into(),
            interaction: Some("terminal".into()),
            terminal: Size { cols: 80, rows: 24 },
            mcp: Vec::new(),
            context: false,
            resume: None,
            provider_credential: kind.map(str::to_string),
        }
    }

    #[cfg(unix)]
    fn recorded(roots: &Path, what: &str, session: &str) -> Option<String> {
        std::fs::read_to_string(roots.join("out").join(format!("{what}.{session}"))).ok()
    }

    #[cfg(unix)]
    fn start_failed(seen: &crate::credential::tests::Seen, session: &str) -> bool {
        seen.lock().unwrap().iter().any(|(m, p, _, b)| {
            m == "POST" && p.ends_with(&format!("/{session}/status")) && b.contains("start_failure")
        })
    }

    #[cfg(unix)]
    fn files_holding(dir: &Path, needle: &str) -> Vec<PathBuf> {
        let mut hits = Vec::new();
        let mut stack = vec![dir.to_path_buf()];
        while let Some(d) = stack.pop() {
            for entry in std::fs::read_dir(&d).into_iter().flatten().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else if std::fs::read(&path)
                    .is_ok_and(|b| String::from_utf8_lossy(&b).contains(needle))
                {
                    hits.push(path);
                }
            }
        }
        hits
    }

    /// AC-TS7, AC-TS11, AC-TS18 (runtime half): a synced start pulls the
    /// token on the host token, gives it to the CLI as its env only, strips
    /// the competing credentials, pins the settings, and writes it nowhere.
    #[tokio::test(flavor = "multi_thread")]
    #[cfg(unix)]
    async fn a_synced_start_runs_on_the_pulled_token_and_nothing_else() {
        use crate::credential::tests::{CANARY, ok_body};
        let (host, seen, state, roots) = synced_host(&[(200, ok_body(CANARY))]).await;
        assert_eq!(
            host.capabilities()["provider_credentials"],
            serde_json::json!([crate::credential::KIND])
        );
        host.clone()
            .handle(synced_start(
                "ags_SYNC",
                "claude-code",
                Some(crate::credential::KIND),
            ))
            .await;
        wait_for("the CLI's env", || {
            recorded(roots.path(), "args", "ags_SYNC").is_some_and(|a| a.contains("--settings"))
        })
        .await;
        let env = recorded(roots.path(), "env", "ags_SYNC").unwrap();
        let lines: Vec<&str> = env.lines().collect();
        assert!(lines.contains(&format!("CLAUDE_CODE_OAUTH_TOKEN={CANARY}").as_str()));
        assert!(
            lines.contains(&"DISABLE_TELEMETRY=1"),
            "non-credential env-file values stay"
        );
        for name in crate::credential::ENV_REMOVE {
            assert!(
                !lines.iter().any(|l| l.starts_with(&format!("{name}="))),
                "{name} reached the CLI"
            );
        }
        assert!(!env.contains("ENVFILE"));

        let pull: Vec<_> = seen
            .lock()
            .unwrap()
            .iter()
            .filter(|(m, ..)| m == "GET")
            .cloned()
            .collect();
        assert_eq!(pull.len(), 1);
        assert_eq!(
            pull[0].1,
            "/v1/runtime/sessions/ags_SYNC/provider-credential"
        );
        assert_eq!(pull[0].2, "bearer sht_test");

        let args = recorded(roots.path(), "args", "ags_SYNC").unwrap();
        assert!(!args.contains(CANARY), "never argv");
        let lines: Vec<&str> = args.lines().collect();
        let at = lines.iter().position(|l| *l == "--settings").unwrap();
        let settings: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(lines[at + 1]).unwrap()).unwrap();
        assert_eq!(settings["apiKeyHelper"], "");
        assert_eq!(
            settings["env"]["ANTHROPIC_BASE_URL"],
            "https://api.anthropic.com"
        );
        assert_eq!(settings["env"]["ANTHROPIC_API_KEY"], "");
        assert!(settings["hooks"]["SessionStart"].is_array());

        assert_eq!(
            files_holding(state.path(), "STORMCANARY"),
            Vec::<PathBuf>::new()
        );
        let session_dir = crate::mcp::session_dir(state.path(), "ags_SYNC");
        host.handle_now(Command::End {
            session: "ags_SYNC".into(),
        });
        wait_for("the session's directory to go", || !session_dir.exists()).await;
        host.shutdown(Duration::from_millis(200)).await;
    }

    /// AM70: a start that names no credential is the native path,
    /// byte-for-byte as before.
    #[tokio::test(flavor = "multi_thread")]
    #[cfg(unix)]
    async fn an_unsynced_start_keeps_the_env_file_and_pulls_nothing() {
        let (host, seen, _state, roots) = synced_host(&[(500, "{}".into())]).await;
        host.clone()
            .handle(synced_start("ags_NATIVE", "claude-code", None))
            .await;
        wait_for("the CLI's env", || {
            recorded(roots.path(), "args", "ags_NATIVE").is_some_and(|a| a.contains("--settings"))
        })
        .await;
        let env = recorded(roots.path(), "env", "ags_NATIVE").unwrap();
        assert!(env.contains("CLAUDE_CODE_OAUTH_TOKEN=sk-ant-oat01-ENVFILE"));
        assert!(env.contains("ANTHROPIC_API_KEY=sk-ant-api03-ENVFILE"));
        assert!(!seen.lock().unwrap().iter().any(|(m, ..)| m == "GET"));
        let args = recorded(roots.path(), "args", "ags_NATIVE").unwrap();
        let lines: Vec<&str> = args.lines().collect();
        let at = lines.iter().position(|l| *l == "--settings").unwrap();
        let settings: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(lines[at + 1]).unwrap()).unwrap();
        assert!(settings.get("env").is_none() && settings.get("apiKeyHelper").is_none());
        host.shutdown(Duration::from_millis(200)).await;
    }

    /// AC-TS11, AC-TS17: a synced start that cannot get its token, names a
    /// kind or provider it cannot sync, or cannot write its pins, fails and
    /// never spawns the CLI on the host's own login.
    #[tokio::test(flavor = "multi_thread")]
    #[cfg(unix)]
    async fn a_synced_start_fails_closed() {
        use crate::credential::KIND;
        for (status, code) in [
            (403, "host_not_in_scope"),
            (404, "no_credential"),
            (409, "session_not_starting"),
            (503, "unavailable"),
        ] {
            let body = serde_json::json!({ "error": code }).to_string();
            let (host, seen, state, roots) = synced_host(&[(status, body)]).await;
            host.clone()
                .handle(synced_start("ags_FAIL", "claude-code", Some(KIND)))
                .await;
            assert!(host.session("ags_FAIL").is_none(), "{code}");
            assert!(start_failed(&seen, "ags_FAIL"), "{code}");
            assert_eq!(
                recorded(roots.path(), "env", "ags_FAIL"),
                None,
                "{code}: spawned"
            );
            assert!(!crate::mcp::session_dir(state.path(), "ags_FAIL").exists());
            host.shutdown(Duration::from_millis(200)).await;
        }

        let (host, seen, state, roots) = synced_host(&[(
            200,
            crate::credential::tests::ok_body(crate::credential::tests::CANARY),
        )])
        .await;
        for (session, provider, kind) in [
            ("ags_KIND", "claude-code", "openai_token"),
            ("ags_PROV", "opencode", KIND),
        ] {
            host.clone()
                .handle(synced_start(session, provider, Some(kind)))
                .await;
            assert!(host.session(session).is_none(), "{session}");
            assert!(start_failed(&seen, session), "{session}");
            assert_eq!(recorded(roots.path(), "env", session), None, "{session}");
        }
        assert!(
            !seen.lock().unwrap().iter().any(|(m, ..)| m == "GET"),
            "nothing is pulled for a launch that cannot use it"
        );

        // The settings file cannot be written: fatal when synced only.
        for session in ["ags_SETS", "ags_SETN"] {
            std::fs::create_dir_all(
                crate::mcp::session_dir(state.path(), session).join("claude-settings.json"),
            )
            .unwrap();
        }
        host.clone()
            .handle(synced_start("ags_SETS", "claude-code", Some(KIND)))
            .await;
        assert!(host.session("ags_SETS").is_none());
        assert!(start_failed(&seen, "ags_SETS"));
        assert_eq!(recorded(roots.path(), "env", "ags_SETS"), None);
        host.clone()
            .handle(synced_start("ags_SETN", "claude-code", None))
            .await;
        assert!(host.session("ags_SETN").is_some(), "unsynced still runs");
        host.shutdown(Duration::from_millis(200)).await;
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
        assert!(matches!(
            env.command,
            Command::Start {
                provider_credential: None,
                ..
            }
        ));
        let env: Envelope = serde_json::from_str(
            r#"{"cmd_seq":3,"type":"start","session":"ags_X","workspace":"w","provider":"claude-code","terminal":{"cols":80,"rows":24},"provider_credential":"claude_subscription_token"}"#,
        )
        .unwrap();
        let Command::Start {
            provider_credential,
            ..
        } = &env.command
        else {
            panic!("a start");
        };
        assert_eq!(
            provider_credential.as_deref(),
            Some("claude_subscription_token")
        );
        assert!(!format!("{:?}", env.command).contains("sk-ant-"));
        let env: Envelope = serde_json::from_str(
            r#"{"cmd_seq":4,"type":"terminal.input","session":"s","data":"aGk="}"#,
        )
        .unwrap();
        assert!(matches!(env.command, Command::Input { .. }));
        let env: Envelope = serde_json::from_str(r#"{"cmd_seq":5,"type":"refresh"}"#).unwrap();
        assert!(matches!(env.command, Command::Refresh));
    }

    /// Decision 131a: the first `running` report carries what the session
    /// skipped; a session with nothing to report sends the plain one.
    #[test]
    fn the_running_report_carries_the_start_report() {
        assert_eq!(
            running_report(&serde_json::Value::Null),
            json!({"status": "running"})
        );
        let extra = json!({"mcp_skipped": [{"name": "slack"}], "mcp_plugin_local": []});
        let body = running_report(&extra);
        assert_eq!(body["status"], "running");
        assert_eq!(body["mcp_skipped"][0]["name"], "slack");
        assert_eq!(body["mcp_plugin_local"], json!([]));
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
