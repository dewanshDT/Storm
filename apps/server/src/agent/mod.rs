//! The Agent Manager: the control plane of the Agent Runtime (freeze §3;
//! `PLAN.md` decisions 77 and 77c).
//!
//! **This is the only session authority.** It holds identity, ownership and
//! the lifecycle record; Runtime Hosts run agents and are told nothing about
//! who owns them. Clients talk only to this server, never to a host.
//!
//! It keeps two kinds of state:
//! - **Durable:** session records in `state/agent/agent.db` ([`store`]) and
//!   the default provider in `state/agent/config.json`.
//! - **Live, in memory:** which hosts are connected, the command channel to
//!   each, what each one offers, and a bounded output cache per session
//!   ([`cache`]). All of it is rebuilt from `hello` after a restart.
//!
//! Locks here are `std::sync::Mutex` held for a few map operations and never
//! across an `.await`, so no request can wedge another the way a held
//! `auth_db` guard once did.

pub mod cache;
pub mod store;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, watch};

use cache::{OutputCache, Read};
use store::{Fallback, LaunchRecord, McpGrant, SessionRecord, Store, WriteRecord};

/// The fallback order when a host lacks the requested provider (freeze §6).
pub const FALLBACK_ORDER: [&str; 3] = ["claude-code", "opencode", "shell"];
pub const DEFAULT_PROVIDER: &str = "claude-code";
/// Largest input POST (freeze §11.1).
pub const MAX_INPUT_BYTES: usize = 64 * 1024;

/// A command to a host, sent down its link (freeze §11.5).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "type")]
pub enum Command {
    #[serde(rename = "start")]
    Start {
        session: String,
        workspace: String,
        provider: String,
        interaction: String,
        terminal: TerminalSize,
        /// The connections this session may use (AM23, spec §6): ids and
        /// slugs only. Omitted when empty, so a session without grants sends
        /// exactly the bytes it always did — an old host ignores the field
        /// anyway, which is why a launch never relies on it (§6, old hosts).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        mcp: Vec<McpGrant>,
        /// The session was started from a note: the runtime adds its fixed
        /// opening prompt. A flag, never text, so no note data reaches argv.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
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
    /// An **unsolicited** upstream message for a session's bridge (spec §9:
    /// `list_changed`, a resource update). At most once: lost if the link is
    /// down. Request-scoped messages never come this way — they ride their
    /// call's own response stream.
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

/// A command with its sequence number, as it goes on the wire.
#[derive(Debug, Clone, Serialize)]
pub struct Envelope {
    pub cmd_seq: u64,
    #[serde(flatten)]
    pub command: Command,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct TerminalSize {
    pub cols: u16,
    pub rows: u16,
}

/// What a host offers (freeze §5.5). No resource metrics.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Capabilities {
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub providers: Vec<ProviderCap>,
    #[serde(default)]
    pub workspaces: Vec<String>,
    #[serde(default)]
    pub max_sessions: u32,
    /// The host runs `storm-runtime mcp-bridge` (spec §6). A host that does
    /// not say so is an old host: its sessions get no grants, and the launch
    /// says why.
    #[serde(default)]
    pub mcp_bridge: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderCap {
    pub id: String,
    pub kind: String,
    #[serde(default)]
    pub interactions: Vec<String>,
    pub available: bool,
}

/// One session in a host's `hello`.
#[derive(Debug, Clone, Deserialize)]
pub struct ReportedSession {
    pub id: String,
    #[serde(flatten)]
    pub status: StatusReport,
    #[serde(default)]
    pub output_end_offset: u64,
}

/// A status a host reports for one of its sessions.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct StatusReport {
    pub status: String,
    #[serde(default)]
    pub exit_code: Option<i32>,
    #[serde(default)]
    pub end_reason: Option<String>,
    #[serde(default)]
    pub signal: Option<i32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Hello {
    #[serde(flatten)]
    pub capabilities: Capabilities,
    #[serde(default)]
    pub sessions: Vec<ReportedSession>,
    #[serde(default)]
    pub last_cmd_seq: u64,
}

/// A launch request (freeze §11.1).
#[derive(Debug, Clone, Deserialize)]
pub struct Launch {
    pub host_id: String,
    pub workspace: String,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default = "terminal")]
    pub interaction: String,
    pub terminal: TerminalSize,
    /// The note the session starts from.
    #[serde(default)]
    pub context: Option<ContextRef>,
    /// The one vault the session may write to; absent is read only.
    #[serde(default)]
    pub write_vault_id: Option<String>,
    /// The pre-v2 toggle. Accepted from older clients, never honoured as
    /// writes: without a vault it launches read only, and the launch says so.
    #[serde(default)]
    pub allow_vault_writes: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ContextRef {
    pub vault_id: String,
    pub note_id: String,
}

/// What the launch resolved before the manager runs it.
#[derive(Debug, Clone, Default)]
pub struct LaunchMeta {
    pub context: Option<store::Context>,
    pub write_vault_id: Option<String>,
    /// Connection id → its display name now, kept with the grants.
    pub integration_names: HashMap<String, String>,
}

/// What a launch did about the MCP Gateway (spec §6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpOutcome {
    /// These connections were granted and sent in `start`.
    Granted(Vec<McpGrant>),
    /// The `shell` provider gets none (G-D9).
    Shell,
    /// The host has no `mcp_bridge`; the caller announces it.
    OldHost,
}

fn terminal() -> String {
    "terminal".into()
}

#[derive(Debug)]
pub enum AgentError {
    NotFound(&'static str),
    BadRequest(String),
    Conflict(String),
    /// The host's link is down: input and launches cannot reach it.
    HostOffline,
    /// `max_sessions` reached on the host.
    TooManySessions,
    /// The host offers no supported provider (422).
    NoProvider,
    /// A host posting for a session that is not its own.
    NotYours,
    Internal(anyhow::Error),
}

impl From<anyhow::Error> for AgentError {
    fn from(e: anyhow::Error) -> Self {
        Self::Internal(e)
    }
}

type AgentResult<T> = std::result::Result<T, AgentError>;

struct HostLink {
    generation: u64,
    tx: mpsc::UnboundedSender<Envelope>,
    next_seq: u64,
    capabilities: Option<Capabilities>,
}

struct SessionLive {
    cache: OutputCache,
    /// Bumped on every output and status change. Stream readers wait on it.
    version: watch::Sender<u64>,
}

impl SessionLive {
    fn new() -> Self {
        Self {
            cache: OutputCache::new(cache::DEFAULT_CAPACITY),
            version: watch::channel(0).0,
        }
    }

    fn bump(&self) {
        self.version.send_modify(|v| *v += 1);
    }
}

#[derive(Default)]
struct Live {
    hosts: HashMap<String, HostLink>,
    sessions: HashMap<String, SessionLive>,
    next_generation: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct AgentConfig {
    #[serde(default)]
    default_provider: Option<String>,
}

pub struct AgentManager {
    store: Mutex<Store>,
    live: Mutex<Live>,
    config_path: PathBuf,
    config: Mutex<AgentConfig>,
}

/// A host's live state, for the host list.
#[derive(Debug, Clone, Serialize)]
pub struct HostLive {
    pub online: bool,
    pub capabilities: Option<Capabilities>,
}

fn now() -> String {
    crate::index::now_rfc3339()
}

impl AgentManager {
    /// Opens `state/agent/`, and marks every session that had not ended
    /// `unknown` until its host reports in.
    pub fn open(state_dir: &Path) -> Result<Self> {
        let dir = state_dir.join("agent");
        std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        let store = Store::open(&dir.join("agent.db"))?;
        let unknown = store.mark_unknown_on_boot()?;
        if unknown > 0 {
            tracing::info!(
                sessions = unknown,
                "agent sessions are unknown until their hosts reconnect"
            );
        }
        let config_path = dir.join("config.json");
        let config = std::fs::read_to_string(&config_path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        Ok(Self {
            store: Mutex::new(store),
            live: Mutex::new(Live::default()),
            config_path,
            config: Mutex::new(config),
        })
    }

    // ---- configuration -------------------------------------------------

    pub fn default_provider(&self) -> String {
        self.config
            .lock()
            .unwrap()
            .default_provider
            .clone()
            .unwrap_or_else(|| DEFAULT_PROVIDER.into())
    }

    pub fn set_default_provider(&self, provider: &str) -> Result<()> {
        let mut config = self.config.lock().unwrap();
        config.default_provider = Some(provider.to_string());
        let tmp = self.config_path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(&*config)?)?;
        std::fs::rename(&tmp, &self.config_path)?;
        Ok(())
    }

    // ---- the host link -------------------------------------------------

    /// A host opened its link. Any older link for it is dropped, which ends
    /// that stream. Returns the generation that identifies this link.
    pub fn connect_host(&self, host_id: &str) -> (u64, mpsc::UnboundedReceiver<Envelope>) {
        let (tx, rx) = mpsc::unbounded_channel();
        let mut live = self.live.lock().unwrap();
        live.next_generation += 1;
        let generation = live.next_generation;
        let capabilities = live.hosts.remove(host_id).and_then(|h| h.capabilities);
        live.hosts.insert(
            host_id.to_string(),
            HostLink {
                generation,
                tx,
                next_seq: 1,
                capabilities,
            },
        );
        tracing::info!(host = host_id, generation, "runtime host connected");
        (generation, rx)
    }

    /// The link of `generation` closed. A newer link is left alone.
    pub fn disconnect_host(&self, host_id: &str, generation: u64) {
        {
            let mut live = self.live.lock().unwrap();
            match live.hosts.get(host_id) {
                Some(link) if link.generation == generation => {
                    live.hosts.remove(host_id);
                }
                _ => return,
            }
        }
        tracing::info!(host = host_id, generation, "runtime host disconnected");
        // Its sessions are unknown until it reconnects and reports them.
        let _ = self.transition_host_sessions(host_id, |r| {
            (!r.is_ended()).then(|| ("unknown".to_string(), None, None, None))
        });
    }

    #[cfg(test)]
    pub fn is_online(&self, host_id: &str) -> bool {
        self.live.lock().unwrap().hosts.contains_key(host_id)
    }

    pub fn host_live(&self, host_id: &str) -> HostLive {
        let live = self.live.lock().unwrap();
        match live.hosts.get(host_id) {
            Some(link) => HostLive {
                online: true,
                capabilities: link.capabilities.clone(),
            },
            None => HostLive {
                online: false,
                capabilities: None,
            },
        }
    }

    fn send(&self, host_id: &str, command: Command) -> AgentResult<()> {
        let mut live = self.live.lock().unwrap();
        let Some(link) = live.hosts.get_mut(host_id) else {
            tracing::info!(
                host = host_id,
                kind = command.kind(),
                session = command.session(),
                "command not sent: host offline"
            );
            return Err(AgentError::HostOffline);
        };
        // Input, resize and MCP messages are per keystroke or chatty: debug.
        match &command {
            Command::Input { .. } | Command::Resize { .. } | Command::McpMessage { .. } => {
                tracing::debug!(
                    host = host_id,
                    seq = link.next_seq,
                    kind = command.kind(),
                    session = command.session(),
                    "command sent"
                )
            }
            _ => tracing::info!(
                host = host_id,
                seq = link.next_seq,
                kind = command.kind(),
                session = command.session(),
                "command sent"
            ),
        }
        let envelope = Envelope {
            cmd_seq: link.next_seq,
            command,
        };
        link.next_seq += 1;
        link.tx.send(envelope).map_err(|_| AgentError::HostOffline)
    }

    /// Applies `change` to every session on `host_id`. `change` returns the
    /// new `(status, end_reason, exit_code, signal)`, or `None` to leave it.
    fn transition_host_sessions(
        &self,
        host_id: &str,
        change: impl Fn(&SessionRecord) -> Option<(String, Option<String>, Option<i32>, Option<i32>)>,
    ) -> Result<()> {
        let records = self.store.lock().unwrap().for_host(host_id)?;
        for mut r in records {
            if let Some((status, reason, exit, signal)) = change(&r) {
                let ending = matches!(status.as_str(), "completed" | "failed" | "stopped");
                r.status = status;
                if ending {
                    r.end_reason = reason;
                    r.exit_code = exit;
                    r.signal = signal;
                    r.ended_at.get_or_insert_with(now);
                }
                self.store.lock().unwrap().update(&r)?;
                self.bump(&r.id);
            }
        }
        Ok(())
    }

    fn bump(&self, session_id: &str) {
        let mut live = self.live.lock().unwrap();
        live.sessions
            .entry(session_id.to_string())
            .or_insert_with(SessionLive::new)
            .bump();
    }

    /// Reconciles from a host's `hello` (freeze §13):
    /// - Sessions it reports take their reported state.
    /// - Its sessions that are not ended and not reported become
    ///   `failed (lost)`.
    /// - Live sessions whose output cache is empty are asked to replay.
    pub fn hello(&self, host_id: &str, hello: Hello) -> AgentResult<()> {
        tracing::info!(
            host = host_id,
            sessions = hello.sessions.len(),
            last_cmd_seq = hello.last_cmd_seq,
            "runtime host hello"
        );
        {
            let mut live = self.live.lock().unwrap();
            let link = live.hosts.get_mut(host_id).ok_or(AgentError::HostOffline)?;
            link.capabilities = Some(hello.capabilities);
        }
        let reported: HashMap<String, ReportedSession> = hello
            .sessions
            .into_iter()
            .map(|s| (s.id.clone(), s))
            .collect();
        let mut replay = Vec::new();
        let records = self.store.lock().unwrap().for_host(host_id)?;
        for mut r in records {
            match reported.get(&r.id) {
                Some(rep) => {
                    if !r.is_ended() {
                        apply_report(&mut r, &rep.status)?;
                        self.store.lock().unwrap().update(&r)?;
                        self.bump(&r.id);
                    }
                    let empty = {
                        let live = self.live.lock().unwrap();
                        live.sessions
                            .get(&r.id)
                            .is_none_or(|s| !s.cache.initialized())
                    };
                    if empty && rep.output_end_offset > 0 {
                        replay.push(r.id.clone());
                    }
                }
                None if !r.is_ended() && r.status != "creating" => {
                    r.status = "failed".into();
                    r.end_reason = Some("lost".into());
                    r.ended_at.get_or_insert_with(now);
                    self.store.lock().unwrap().update(&r)?;
                    self.bump(&r.id);
                }
                None => {}
            }
        }
        for session in replay {
            self.send(host_id, Command::Replay { session, from: 0 })?;
        }
        Ok(())
    }

    /// Asks a host to re-report its inventory. Sent when a client asks for a
    /// host's workspaces, so a directory created since the last report shows
    /// up on the next look. Best-effort: an offline host is simply skipped.
    pub fn refresh(&self, host_id: &str) {
        let _ = self.send(host_id, Command::Refresh);
    }

    pub fn inventory(&self, host_id: &str, capabilities: Capabilities) -> AgentResult<()> {
        let mut live = self.live.lock().unwrap();
        let link = live.hosts.get_mut(host_id).ok_or(AgentError::HostOffline)?;
        link.capabilities = Some(capabilities);
        Ok(())
    }

    fn owned_by(&self, host_id: &str, session_id: &str) -> AgentResult<SessionRecord> {
        let r = self
            .store
            .lock()
            .unwrap()
            .get(session_id)?
            .ok_or(AgentError::NotFound("no such session"))?;
        // A host may post only for sessions on itself (freeze §11.5).
        if r.host_id != host_id {
            return Err(AgentError::NotYours);
        }
        Ok(r)
    }

    /// Output from a host, at `offset`.
    pub fn host_output(
        &self,
        host_id: &str,
        session_id: &str,
        offset: u64,
        bytes: &[u8],
    ) -> AgentResult<()> {
        let mut r = self.owned_by(host_id, session_id)?;
        {
            let mut live = self.live.lock().unwrap();
            let s = live
                .sessions
                .entry(session_id.to_string())
                .or_insert_with(SessionLive::new);
            s.cache.insert(offset, bytes);
            s.bump();
        }
        // Throttled: one activity write per second is plenty for a list view.
        let stamp = now();
        if r.last_activity.as_deref().map(|l| &l[..19.min(l.len())]) != Some(&stamp[..19]) {
            r.last_activity = Some(stamp);
            self.store.lock().unwrap().update(&r)?;
        }
        Ok(())
    }

    /// A status change from a host.
    pub fn host_status(
        &self,
        host_id: &str,
        session_id: &str,
        report: &StatusReport,
    ) -> AgentResult<()> {
        let mut r = self.owned_by(host_id, session_id)?;
        if r.is_ended() {
            // An ending is final; a late duplicate changes nothing.
            return Ok(());
        }
        apply_report(&mut r, report)?;
        tracing::info!(
            host = host_id,
            session = session_id,
            status = %r.status,
            end_reason = r.end_reason.as_deref(),
            "session status"
        );
        self.store.lock().unwrap().update(&r)?;
        self.bump(session_id);
        Ok(())
    }

    // ---- what clients do -------------------------------------------------

    /// Launches a session (freeze §6, §11.1). Returns the record, which
    /// carries any `provider_fallback`.
    pub fn launch(
        &self,
        owner: &str,
        req: Launch,
        meta: LaunchMeta,
        offered_grants: Vec<McpGrant>,
    ) -> AgentResult<(SessionRecord, LaunchRecord, McpOutcome)> {
        if req.interaction != "terminal" {
            return Err(AgentError::BadRequest(
                "V1 offers the terminal interaction only".into(),
            ));
        }
        if req.terminal.cols == 0 || req.terminal.rows == 0 {
            return Err(AgentError::BadRequest(
                "a terminal has at least one cell".into(),
            ));
        }
        let caps = {
            let live = self.live.lock().unwrap();
            let link = live
                .hosts
                .get(&req.host_id)
                .ok_or(AgentError::HostOffline)?;
            link.capabilities.clone().ok_or(AgentError::HostOffline)?
        };
        if !caps.workspaces.iter().any(|w| w == &req.workspace) {
            return Err(AgentError::BadRequest(format!(
                "the host has no workspace “{}”",
                req.workspace
            )));
        }
        let live_on_host = self
            .store
            .lock()
            .unwrap()
            .for_host(&req.host_id)?
            .iter()
            .filter(|r| !r.is_ended())
            .count();
        if caps.max_sessions > 0 && live_on_host >= caps.max_sessions as usize {
            return Err(AgentError::TooManySessions);
        }

        let requested = req
            .provider
            .clone()
            .unwrap_or_else(|| self.default_provider());
        let offered = |id: &str| {
            caps.providers
                .iter()
                .find(|p| {
                    p.id == id && p.available && p.interactions.iter().any(|i| i == "terminal")
                })
                .cloned()
        };
        let (provider, fallback) = match offered(&requested) {
            Some(p) => (p, None),
            None => {
                let p = FALLBACK_ORDER
                    .iter()
                    .find_map(|id| offered(id))
                    .ok_or(AgentError::NoProvider)?;
                let fallback = Fallback {
                    requested: requested.clone(),
                    used: p.id.clone(),
                    reason: "not_installed".into(),
                };
                (p, Some(fallback))
            }
        };

        let record = SessionRecord {
            id: crate::auth::identity::random_id("ags_"),
            owner_user_id: owner.to_string(),
            host_id: req.host_id.clone(),
            workspace: req.workspace.clone(),
            provider: provider.id.clone(),
            provider_kind: provider.kind.clone(),
            interaction: "terminal".into(),
            provider_fallback: fallback,
            status: "creating".into(),
            end_reason: None,
            signal: None,
            exit_code: None,
            created_at: now(),
            started_at: None,
            ended_at: None,
            last_activity: None,
            egress: "host".into(),
            cols: req.terminal.cols,
            rows: req.terminal.rows,
        };
        // The grants (spec §6): none for `shell` (G-D9), none for a host
        // that cannot bridge, and otherwise everything the caller offered.
        // Written before `start` goes out.
        let outcome = if provider.id == "shell" {
            McpOutcome::Shell
        } else if !caps.mcp_bridge {
            McpOutcome::OldHost
        } else {
            McpOutcome::Granted(offered_grants)
        };
        let grants = match &outcome {
            McpOutcome::Granted(g) => g.clone(),
            _ => Vec::new(),
        };
        let reads_storm = grants
            .iter()
            .any(|g| g.id == crate::gateway::connections::BUILTIN_ID);
        let launch = {
            let mut store = self.store.lock().unwrap();
            let launch = LaunchRecord {
                session_id: record.id.clone(),
                name: session_name(
                    meta.context.as_ref().map(|c| c.title.as_str()),
                    &record.workspace,
                    &store.live_names()?,
                ),
                context: meta.context,
                // A session that cannot reach Storm has nothing to write with.
                write_vault_id: meta.write_vault_id.filter(|_| reads_storm),
            };
            store.insert(&record)?;
            store.insert_grants(
                &record.id,
                launch.write_vault_id.is_some(),
                &grants,
                &record.created_at,
            )?;
            store.insert_launch(&launch, &record.created_at)?;
            let integrations: Vec<store::SessionIntegration> = grants
                .iter()
                .filter(|g| g.id != crate::gateway::connections::BUILTIN_ID)
                .map(|g| store::SessionIntegration {
                    id: g.id.clone(),
                    slug: g.slug.clone(),
                    display_name: meta
                        .integration_names
                        .get(&g.id)
                        .cloned()
                        .unwrap_or_else(|| g.slug.clone()),
                })
                .collect();
            store.insert_integrations(&record.id, &integrations)?;
            launch
        };
        self.live
            .lock()
            .unwrap()
            .sessions
            .insert(record.id.clone(), SessionLive::new());

        let sent = self.send(
            &req.host_id,
            Command::Start {
                session: record.id.clone(),
                workspace: record.workspace.clone(),
                provider: record.provider.clone(),
                interaction: "terminal".into(),
                terminal: req.terminal,
                mcp: grants,
                context: reads_storm && launch.context.is_some(),
            },
        );
        let mut record = record;
        match sent {
            Ok(()) => record.status = "starting".into(),
            Err(_) => {
                record.status = "failed".into();
                record.end_reason = Some("start_failure".into());
                record.ended_at = Some(now());
            }
        }
        self.store.lock().unwrap().update(&record)?;
        self.bump(&record.id);
        Ok((record, launch, outcome))
    }

    pub fn launch_of(&self, session_id: &str) -> Result<Option<LaunchRecord>> {
        self.store.lock().unwrap().launch(session_id)
    }

    /// What a session was granted at launch, as it was named then.
    pub fn integrations_of(&self, session_id: &str) -> Result<Vec<store::SessionIntegration>> {
        self.store
            .lock()
            .unwrap()
            .integrations_of(session_id, crate::gateway::connections::BUILTIN_ID)
    }

    pub fn record_write(&self, write: &WriteRecord) -> Result<()> {
        self.store.lock().unwrap().record_write(write)
    }

    pub fn writes_of(&self, session_id: &str) -> Result<Vec<WriteRecord>> {
        self.store.lock().unwrap().writes_of(session_id)
    }

    pub fn write_count(&self, session_id: &str) -> Result<i64> {
        self.store.lock().unwrap().write_count(session_id)
    }

    pub fn latest_write(&self, vault_id: &str, note_id: &str) -> Result<Option<WriteRecord>> {
        self.store.lock().unwrap().latest_write(vault_id, note_id)
    }

    pub fn latest_writes(&self, vault_id: &str) -> Result<Vec<WriteRecord>> {
        self.store.lock().unwrap().latest_writes(vault_id)
    }

    // ---- the MCP Gateway's view (decision 81e) ---------------------------

    /// A live grant for `(session, connection)`: its slug and the session's
    /// vault-write flag. `None` is "not granted", including a revoked grant.
    pub fn grant(&self, session_id: &str, connection_id: &str) -> Result<Option<(String, bool)>> {
        self.store.lock().unwrap().grant(session_id, connection_id)
    }

    #[cfg(test)]
    pub fn grants_of(&self, session_id: &str) -> Result<Vec<McpGrant>> {
        self.store.lock().unwrap().grants_of(session_id)
    }

    /// Disconnecting a connection revokes its grants (§13); the rows stay.
    pub fn revoke_grants_for(&self, connection_id: &str) -> Result<usize> {
        self.store
            .lock()
            .unwrap()
            .revoke_grants_for(connection_id, &now())
    }

    /// Sends an unsolicited upstream message to a session's host (spec §9).
    /// At most once; an offline host simply misses it.
    pub fn mcp_message(&self, session_id: &str, connection: &str, message: serde_json::Value) {
        let Ok(Some(r)) = self.store.lock().unwrap().get(session_id) else {
            return;
        };
        if r.is_ended() {
            return;
        }
        let _ = self.send(
            &r.host_id,
            Command::McpMessage {
                session: r.id,
                connection: connection.to_string(),
                message,
            },
        );
    }

    pub fn get(&self, id: &str) -> AgentResult<SessionRecord> {
        self.store
            .lock()
            .unwrap()
            .get(id)?
            .ok_or(AgentError::NotFound("no such session"))
    }

    pub fn list(&self) -> AgentResult<Vec<SessionRecord>> {
        Ok(self.store.lock().unwrap().list()?)
    }

    /// Live sessions per workspace on a host, for the shared-workspace warning.
    pub fn live_counts(&self, host_id: &str) -> AgentResult<HashMap<String, usize>> {
        let mut counts = HashMap::new();
        for r in self.store.lock().unwrap().for_host(host_id)? {
            if !r.is_ended() {
                *counts.entry(r.workspace).or_insert(0) += 1;
            }
        }
        Ok(counts)
    }

    fn live_record(&self, id: &str) -> AgentResult<SessionRecord> {
        let r = self.get(id)?;
        if r.is_ended() {
            return Err(AgentError::Conflict("the session has ended".into()));
        }
        Ok(r)
    }

    /// Raw input. At most once: refused while the host is offline, never
    /// queued (freeze §11.3).
    pub fn input(&self, id: &str, bytes: &[u8]) -> AgentResult<()> {
        if bytes.len() > MAX_INPUT_BYTES {
            return Err(AgentError::BadRequest("input is at most 64 KiB".into()));
        }
        let r = self.live_record(id)?;
        use data_encoding::BASE64;
        self.send(
            &r.host_id,
            Command::Input {
                session: r.id,
                data: BASE64.encode(bytes),
            },
        )
    }

    /// The PTY follows the most recently active client (freeze §11.4).
    pub fn resize(&self, id: &str, size: TerminalSize) -> AgentResult<()> {
        if size.cols == 0 || size.rows == 0 {
            return Err(AgentError::BadRequest(
                "a terminal has at least one cell".into(),
            ));
        }
        let mut r = self.live_record(id)?;
        self.send(
            &r.host_id,
            Command::Resize {
                session: r.id.clone(),
                cols: size.cols,
                rows: size.rows,
            },
        )?;
        r.cols = size.cols;
        r.rows = size.rows;
        self.store.lock().unwrap().update(&r)?;
        self.bump(id);
        Ok(())
    }

    /// Asks the host to end the session. Its status changes when the host
    /// reports it.
    pub fn end(&self, id: &str) -> AgentResult<()> {
        let r = self.live_record(id)?;
        self.send(&r.host_id, Command::End { session: r.id })
    }

    /// Removes an ended session from the list (freeze §7.4).
    pub fn dismiss(&self, id: &str) -> AgentResult<()> {
        let r = self.get(id)?;
        if !r.is_ended() {
            return Err(AgentError::Conflict(
                "only an ended session can be dismissed".into(),
            ));
        }
        self.store.lock().unwrap().delete(id)?;
        self.live.lock().unwrap().sessions.remove(id);
        Ok(())
    }

    /// Fails live and dismisses ended sessions not owned by `account_id`.
    pub fn retire_sessions_not_of(&self, account_id: &str) -> Result<(usize, usize)> {
        let records = self.store.lock().unwrap().list()?;
        let (mut failed, mut dismissed) = (0, 0);
        for mut r in records
            .into_iter()
            .filter(|r| r.owner_user_id != account_id)
        {
            if r.is_ended() {
                self.store.lock().unwrap().delete(&r.id)?;
                self.live.lock().unwrap().sessions.remove(&r.id);
                dismissed += 1;
            } else {
                r.status = "failed".into();
                r.end_reason = Some("owner_removed".into());
                r.ended_at.get_or_insert_with(now);
                self.store.lock().unwrap().update(&r)?;
                self.bump(&r.id);
                failed += 1;
            }
        }
        Ok((failed, dismissed))
    }

    /// A revoked host: its link is closed and its sessions are failed
    /// (freeze §5.6). The host ends them itself when next refused.
    pub fn revoke_host(&self, host_id: &str) -> Result<()> {
        let was_online = self.live.lock().unwrap().hosts.remove(host_id).is_some();
        tracing::info!(host = host_id, was_online, "runtime host revoked");
        self.transition_host_sessions(host_id, |r| {
            (!r.is_ended()).then(|| {
                (
                    "failed".to_string(),
                    Some("host_revoked".into()),
                    None,
                    None,
                )
            })
        })
    }

    // ---- the terminal stream ----------------------------------------------

    /// A receiver that changes whenever the session's output or status does.
    pub fn watch(&self, id: &str) -> watch::Receiver<u64> {
        self.live
            .lock()
            .unwrap()
            .sessions
            .entry(id.to_string())
            .or_insert_with(SessionLive::new)
            .version
            .subscribe()
    }

    pub fn read_output(&self, id: &str, from: u64, max: usize) -> Read {
        let live = self.live.lock().unwrap();
        match live.sessions.get(id) {
            Some(s) => s.cache.read(from, max),
            None if from == 0 => Read::UpToDate,
            None => Read::Ahead,
        }
    }
}

/// A session's name: the slug of its context note's title, else
/// `{workspace}-{n}`; a name a live session holds gets `-2`, `-3`, ….
pub fn session_name(title: Option<&str>, workspace: &str, live: &[String]) -> String {
    let taken = |name: &str| live.iter().any(|l| l == name);
    match title.map(slug).filter(|s| !s.is_empty()) {
        Some(base) => {
            if !taken(&base) {
                return base;
            }
            (2..)
                .map(|n| format!("{base}-{n}"))
                .find(|name| !taken(name))
                .expect("an unbounded range")
        }
        None => {
            let base = Some(slug(workspace))
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "session".into());
            (1..)
                .map(|n| format!("{base}-{n}"))
                .find(|name| !taken(name))
                .expect("an unbounded range")
        }
    }
}

fn slug(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars().flat_map(char::to_lowercase) {
        if c.is_alphanumeric() {
            out.push(c);
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
        if out.chars().count() >= 48 {
            break;
        }
    }
    out.trim_end_matches('-').to_string()
}

/// Applies a host's report to a record. Unknown statuses are refused rather
/// than stored: the vocabulary is fixed (freeze §7.2).
fn apply_report(r: &mut SessionRecord, report: &StatusReport) -> AgentResult<()> {
    match report.status.as_str() {
        "starting" | "running" => {
            if report.status == "running" {
                r.started_at.get_or_insert_with(now);
            }
            r.status = report.status.clone();
        }
        "completed" | "failed" | "stopped" => {
            r.status = report.status.clone();
            r.exit_code = report.exit_code;
            r.signal = report.signal;
            r.end_reason = match report.status.as_str() {
                "failed" => Some(
                    report
                        .end_reason
                        .clone()
                        .unwrap_or_else(|| "start_failure".into()),
                ),
                _ => None,
            };
            r.ended_at.get_or_insert_with(now);
        }
        other => {
            return Err(AgentError::BadRequest(format!("unknown status `{other}`")));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manager() -> (AgentManager, tempdir::TempDir) {
        let dir = tempdir::TempDir::new("storm-agent").unwrap();
        (AgentManager::open(dir.path()).unwrap(), dir)
    }

    fn caps(providers: &[(&str, bool)]) -> Capabilities {
        Capabilities {
            version: "test".into(),
            providers: providers
                .iter()
                .map(|(id, available)| ProviderCap {
                    id: id.to_string(),
                    kind: if *id == "fake" { "fake" } else { "cli" }.into(),
                    interactions: vec!["terminal".into()],
                    available: *available,
                })
                .collect(),
            workspaces: vec!["storm".into(), "site".into()],
            max_sessions: 2,
            mcp_bridge: true,
        }
    }

    fn online(m: &AgentManager, host: &str, c: Capabilities) -> mpsc::UnboundedReceiver<Envelope> {
        let (_, rx) = m.connect_host(host);
        m.hello(
            host,
            Hello {
                capabilities: c,
                sessions: vec![],
                last_cmd_seq: 0,
            },
        )
        .unwrap();
        rx
    }

    fn launch(m: &AgentManager, provider: Option<&str>) -> AgentResult<SessionRecord> {
        launch_with(m, provider, offered()).map(|(r, _)| r)
    }

    fn offered() -> Vec<McpGrant> {
        vec![
            McpGrant {
                id: "storm".into(),
                slug: "storm".into(),
            },
            McpGrant {
                id: "mcc_GH".into(),
                slug: "github".into(),
            },
        ]
    }

    fn launch_with(
        m: &AgentManager,
        provider: Option<&str>,
        grants: Vec<McpGrant>,
    ) -> AgentResult<(SessionRecord, McpOutcome)> {
        launch_meta(m, provider, grants, LaunchMeta::default()).map(|(r, _, o)| (r, o))
    }

    fn launch_meta(
        m: &AgentManager,
        provider: Option<&str>,
        grants: Vec<McpGrant>,
        meta: LaunchMeta,
    ) -> AgentResult<(SessionRecord, LaunchRecord, McpOutcome)> {
        m.launch(
            "usr_OWNER",
            Launch {
                host_id: "hst_A".into(),
                workspace: "storm".into(),
                provider: provider.map(Into::into),
                interaction: "terminal".into(),
                terminal: TerminalSize { cols: 80, rows: 24 },
                context: None,
                write_vault_id: None,
                allow_vault_writes: false,
            },
            meta,
            grants,
        )
    }

    fn with_context(title: &str, write_vault: Option<&str>) -> LaunchMeta {
        LaunchMeta {
            context: Some(store::Context {
                vault_id: "vlt_PERSONAL".into(),
                note_id: "0b5e1d2c-note".into(),
                title: title.into(),
            }),
            write_vault_id: write_vault.map(Into::into),
            ..LaunchMeta::default()
        }
    }

    fn end(m: &AgentManager, id: &str) {
        m.host_status(
            "hst_A",
            id,
            &StatusReport {
                status: "completed".into(),
                exit_code: Some(0),
                end_reason: None,
                signal: None,
            },
        )
        .unwrap();
    }

    #[test]
    fn the_launch_record_round_trips_through_a_restart() {
        let dir = tempdir::TempDir::new("storm-agent-launch").unwrap();
        let (id, launched) = {
            let m = AgentManager::open(dir.path()).unwrap();
            let _rx = online(&m, "hst_A", caps(&[("claude-code", true)]));
            let (r, launch, _) = launch_meta(
                &m,
                None,
                offered(),
                with_context("Gateway spec", Some("vlt_WORK")),
            )
            .unwrap();
            (r.id, launch)
        };
        let m = AgentManager::open(dir.path()).unwrap();
        let back = m.launch_of(&id).unwrap().unwrap();
        assert_eq!(back, launched);
        assert_eq!(back.name, "gateway-spec");
        assert_eq!(back.write_vault_id.as_deref(), Some("vlt_WORK"));
        let c = back.context.unwrap();
        assert_eq!(
            (c.vault_id.as_str(), c.note_id.as_str(), c.title.as_str()),
            ("vlt_PERSONAL", "0b5e1d2c-note", "Gateway spec")
        );
        assert_eq!(m.launch_of("ags_NOPE").unwrap(), None);
    }

    #[test]
    fn a_session_without_storm_has_no_write_vault_and_no_opening_prompt() {
        let (m, _d) = manager();
        let mut rx = online(&m, "hst_A", caps(&[("shell", true)]));
        let (_, launch, outcome) = launch_meta(
            &m,
            Some("shell"),
            offered(),
            with_context("Gateway spec", Some("vlt_WORK")),
        )
        .unwrap();
        assert_eq!(outcome, McpOutcome::Shell);
        assert_eq!(launch.write_vault_id, None);
        assert!(
            launch.context.is_some(),
            "the context is still the session's"
        );
        let wire = serde_json::to_value(rx.try_recv().unwrap()).unwrap();
        assert!(wire.get("context").is_none(), "{wire}");
    }

    #[test]
    fn no_note_data_reaches_the_start_command_only_the_context_flag() {
        let (m, _d) = manager();
        let mut rx = online(&m, "hst_A", caps(&[("claude-code", true)]));
        let marker = "MARKER-7f3a";
        let meta = LaunchMeta {
            context: Some(store::Context {
                vault_id: format!("vlt_{marker}"),
                note_id: format!("note-{marker}"),
                title: format!("Plan {marker}"),
            }),
            write_vault_id: Some(format!("vlt_{marker}")),
            ..LaunchMeta::default()
        };
        launch_meta(&m, None, offered(), meta).unwrap();
        let wire = serde_json::to_string(&rx.try_recv().unwrap()).unwrap();
        assert!(!wire.contains(marker), "note data reached the host: {wire}");
        assert!(!wire.to_lowercase().contains("marker"), "{wire}");
        let wire: serde_json::Value = serde_json::from_str(&wire).unwrap();
        assert_eq!(wire["context"], true);
        let keys: Vec<&str> = wire
            .as_object()
            .unwrap()
            .keys()
            .map(|k| k.as_str())
            .collect();
        assert_eq!(
            keys,
            [
                "cmd_seq",
                "context",
                "interaction",
                "mcp",
                "provider",
                "session",
                "terminal",
                "type",
                "workspace"
            ],
            "the start command grew a field"
        );

        let (_, _) = launch_with(&m, None, offered()).unwrap();
        let wire = serde_json::to_value(rx.try_recv().unwrap()).unwrap();
        assert!(wire.get("context").is_none(), "{wire}");
    }

    #[test]
    fn a_name_is_the_notes_slug_or_the_workspaces_and_a_live_duplicate_is_numbered() {
        assert_eq!(
            session_name(Some("Gateway spec"), "storm", &[]),
            "gateway-spec"
        );
        assert_eq!(
            session_name(Some("  Q3: Plan / Review!! "), "storm", &[]),
            "q3-plan-review"
        );
        assert_eq!(session_name(Some("Café notes"), "storm", &[]), "café-notes");
        assert_eq!(session_name(Some("???"), "storm", &[]), "storm-1");
        assert_eq!(session_name(None, "storm", &[]), "storm-1");
        assert_eq!(session_name(None, "My Site", &[]), "my-site-1");
        let live = vec!["gateway-spec".to_string(), "gateway-spec-2".to_string()];
        assert_eq!(
            session_name(Some("Gateway spec"), "storm", &live),
            "gateway-spec-3"
        );
        let live = vec!["storm-1".to_string(), "storm-3".to_string()];
        assert_eq!(session_name(None, "storm", &live), "storm-2");
        assert!(
            session_name(Some(&"word ".repeat(40)), "storm", &[])
                .chars()
                .count()
                <= 48
        );
    }

    #[test]
    fn only_a_live_session_holds_its_name() {
        let (m, _d) = manager();
        let _rx = online(&m, "hst_A", caps(&[("claude-code", true)]));
        let meta = || with_context("Gateway spec", None);
        let (a, la, _) = launch_meta(&m, None, offered(), meta()).unwrap();
        let (b, lb, _) = launch_meta(&m, None, offered(), meta()).unwrap();
        assert_eq!(
            (la.name.as_str(), lb.name.as_str()),
            ("gateway-spec", "gateway-spec-2")
        );
        end(&m, &a.id);
        let (_, lc, _) = launch_meta(&m, None, offered(), meta()).unwrap();
        assert_eq!(lc.name, "gateway-spec", "an ended session frees its name");
        end(&m, &b.id);
        let (_, ld, _) = launch_meta(&m, None, offered(), LaunchMeta::default()).unwrap();
        assert_eq!(ld.name, "storm-1");
    }

    #[test]
    fn writes_keep_created_follow_the_latest_version_and_survive_a_dismissal() {
        let (m, _d) = manager();
        let _rx = online(&m, "hst_A", caps(&[("claude-code", true)]));
        let a = launch(&m, None).unwrap();
        let b = launch(&m, None).unwrap();
        let w = |session: &str, note: &str, kind: &str, version: i64, at: &str| WriteRecord {
            session_id: session.into(),
            vault_id: "vlt_W".into(),
            note_id: Some(note.into()),
            path: None,
            kind: kind.into(),
            version: Some(version),
            at: at.into(),
        };
        let script = |session: &str, path: &str, kind: &str, at: &str| WriteRecord {
            session_id: session.into(),
            vault_id: "vlt_W".into(),
            note_id: None,
            path: Some(path.into()),
            kind: kind.into(),
            version: None,
            at: at.into(),
        };
        m.record_write(&w(&a.id, "n1", "created", 1, "2026-10-08T10:00:00.000Z"))
            .unwrap();
        m.record_write(&w(&a.id, "n1", "edited", 2, "2026-10-08T10:01:00.000Z"))
            .unwrap();
        m.record_write(&w(&a.id, "n2", "edited", 7, "2026-10-08T10:02:00.000Z"))
            .unwrap();
        let mine = m.writes_of(&a.id).unwrap();
        assert_eq!(
            mine.iter()
                .map(|w| (w.note_id.as_deref(), w.kind.as_str(), w.version))
                .collect::<Vec<_>>(),
            [
                (Some("n2"), "edited", Some(7)),
                (Some("n1"), "created", Some(2))
            ],
            "newest first, one row per note, created stays created"
        );
        assert_eq!(m.write_count(&a.id).unwrap(), 2);

        // A kit script is one row per path, beside the notes.
        m.record_write(&script(
            &a.id,
            "scripts/x.sh",
            "script_created",
            "2026-10-08T10:02:10.000Z",
        ))
        .unwrap();
        m.record_write(&script(
            &a.id,
            "scripts/x.sh",
            "script_edited",
            "2026-10-08T10:02:20.000Z",
        ))
        .unwrap();
        m.record_write(&script(
            &a.id,
            "scripts/y.sh",
            "script_edited",
            "2026-10-08T10:02:30.000Z",
        ))
        .unwrap();
        let mine = m.writes_of(&a.id).unwrap();
        assert_eq!(
            mine.iter()
                .take(2)
                .map(|w| (
                    w.path.as_deref(),
                    w.kind.as_str(),
                    w.note_id.as_deref(),
                    w.version
                ))
                .collect::<Vec<_>>(),
            [
                (Some("scripts/y.sh"), "script_edited", None, None),
                (Some("scripts/x.sh"), "script_created", None, None)
            ]
        );
        assert_eq!(m.write_count(&a.id).unwrap(), 4);

        // Session b edits n1 later: it is the latest writer; a still lists it.
        m.record_write(&w(&b.id, "n1", "edited", 3, "2026-10-08T10:03:00.000Z"))
            .unwrap();
        assert_eq!(
            m.latest_write("vlt_W", "n1").unwrap().unwrap().session_id,
            b.id
        );
        assert_eq!(
            m.latest_write("vlt_W", "n2").unwrap().unwrap().session_id,
            a.id
        );
        assert_eq!(m.latest_write("vlt_W", "n3").unwrap(), None);
        let latest = m.latest_writes("vlt_W").unwrap();
        assert_eq!(
            latest
                .iter()
                .map(|w| (w.note_id.as_deref(), w.session_id.as_str(), w.version))
                .collect::<Vec<_>>(),
            [
                (Some("n1"), b.id.as_str(), Some(3)),
                (Some("n2"), a.id.as_str(), Some(7))
            ],
            "scripts are not notes, so not provenance"
        );
        assert!(m.latest_writes("vlt_OTHER").unwrap().is_empty());
        assert_eq!(m.write_count(&a.id).unwrap(), 4);

        end(&m, &b.id);
        m.dismiss(&b.id).unwrap();
        assert_eq!(
            m.latest_write("vlt_W", "n1").unwrap().unwrap().session_id,
            b.id
        );
        assert!(
            m.launch_of(&b.id).unwrap().is_some(),
            "the name still resolves"
        );
    }

    #[test]
    fn a_launch_grants_the_offered_connections_and_tells_the_host_ids_and_slugs_only() {
        let (m, _d) = manager();
        let mut rx = online(&m, "hst_A", caps(&[("claude-code", true)]));
        let (r, outcome) = launch_with(&m, None, offered()).unwrap();
        assert_eq!(outcome, McpOutcome::Granted(offered()));
        let cmd = rx.try_recv().unwrap();
        let wire = serde_json::to_value(&cmd).unwrap();
        assert_eq!(
            wire["mcp"],
            serde_json::json!([{"id": "storm", "slug": "storm"}, {"id": "mcc_GH", "slug": "github"}])
        );
        // Persisted before `start`, and revocable.
        assert_eq!(
            m.grant(&r.id, "mcc_GH").unwrap(),
            Some(("github".into(), false))
        );
        assert_eq!(m.grant(&r.id, "mcc_OTHER").unwrap(), None);
        assert_eq!(m.revoke_grants_for("mcc_GH").unwrap(), 1);
        assert_eq!(m.grant(&r.id, "mcc_GH").unwrap(), None);
        assert_eq!(m.grants_of(&r.id).unwrap().len(), 1);
    }

    #[test]
    fn a_sessions_integrations_are_launch_history_named_as_at_launch() {
        let (m, _d) = manager();
        let _rx = online(&m, "hst_A", caps(&[("claude-code", true), ("shell", true)]));
        let meta = LaunchMeta {
            integration_names: [("mcc_GH".to_string(), "GitHub".to_string())].into(),
            ..LaunchMeta::default()
        };
        let (r, _, _) = launch_meta(&m, None, offered(), meta).unwrap();
        let want = vec![store::SessionIntegration {
            id: "mcc_GH".into(),
            slug: "github".into(),
            display_name: "GitHub".into(),
        }];
        assert_eq!(m.integrations_of(&r.id).unwrap(), want, "storm is not one");
        // A disconnect revokes the grant, not the history.
        m.revoke_grants_for("mcc_GH").unwrap();
        assert_eq!(m.integrations_of(&r.id).unwrap(), want);

        let (shell, _) = launch_with(&m, Some("shell"), offered()).unwrap();
        assert!(m.integrations_of(&shell.id).unwrap().is_empty());
        end(&m, &r.id);
        end(&m, &shell.id);

        // A session launched before names were kept shows its grants' slugs.
        let (old, _) = launch_with(&m, None, offered()).unwrap();
        m.store
            .lock()
            .unwrap()
            .conn
            .execute(
                "DELETE FROM session_integrations WHERE session_id = ?1",
                rusqlite::params![old.id],
            )
            .unwrap();
        assert_eq!(
            m.integrations_of(&old.id).unwrap()[0].display_name,
            "github"
        );
    }

    #[test]
    fn shell_and_old_hosts_get_no_grants_and_start_carries_none() {
        // G-D9 and spec §6: `shell` gets nothing; a host without
        // `mcp_bridge` gets nothing and the launch says so.
        let (m, _d) = manager();
        let mut rx = online(&m, "hst_A", caps(&[("shell", true)]));
        let (r, outcome) = launch_with(&m, Some("shell"), offered()).unwrap();
        assert_eq!(outcome, McpOutcome::Shell);
        assert!(m.grants_of(&r.id).unwrap().is_empty());
        let wire = serde_json::to_value(rx.try_recv().unwrap()).unwrap();
        assert!(wire.get("mcp").is_none(), "{wire}");

        let (m, _d) = manager();
        let mut old = caps(&[("claude-code", true)]);
        old.mcp_bridge = false;
        let mut rx = online(&m, "hst_A", old);
        let (r, outcome) = launch_with(&m, None, offered()).unwrap();
        assert_eq!(outcome, McpOutcome::OldHost);
        assert!(m.grants_of(&r.id).unwrap().is_empty());
        let wire = serde_json::to_value(rx.try_recv().unwrap()).unwrap();
        assert!(wire.get("mcp").is_none(), "{wire}");
    }

    #[test]
    fn an_unsolicited_message_goes_down_the_link_only_for_a_live_session() {
        let (m, _d) = manager();
        let mut rx = online(&m, "hst_A", caps(&[("claude-code", true)]));
        let r = launch(&m, None).unwrap();
        let _start = rx.try_recv().unwrap();
        let msg =
            serde_json::json!({"jsonrpc": "2.0", "method": "notifications/tools/list_changed"});
        m.mcp_message(&r.id, "mcc_GH", msg.clone());
        let wire = serde_json::to_value(rx.try_recv().unwrap()).unwrap();
        assert_eq!(wire["type"], "mcp.message");
        assert_eq!(wire["session"], r.id);
        assert_eq!(wire["connection"], "mcc_GH");
        assert_eq!(wire["message"], msg);
        m.mcp_message("ags_NOPE", "mcc_GH", msg);
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn a_launch_sends_start_and_the_host_drives_the_status() {
        let (m, _d) = manager();
        let mut rx = online(&m, "hst_A", caps(&[("claude-code", true)]));
        let r = launch(&m, None).unwrap();
        assert_eq!(r.status, "starting");
        assert_eq!(r.provider, "claude-code");
        let cmd = rx.try_recv().unwrap();
        assert_eq!(cmd.cmd_seq, 1);
        assert!(
            matches!(cmd.command, Command::Start { ref provider, .. } if provider == "claude-code")
        );

        m.host_status(
            "hst_A",
            &r.id,
            &StatusReport {
                status: "running".into(),
                exit_code: None,
                end_reason: None,
                signal: None,
            },
        )
        .unwrap();
        assert_eq!(m.get(&r.id).unwrap().status, "running");
        m.host_status(
            "hst_A",
            &r.id,
            &StatusReport {
                status: "completed".into(),
                exit_code: Some(0),
                end_reason: None,
                signal: None,
            },
        )
        .unwrap();
        let done = m.get(&r.id).unwrap();
        assert_eq!(
            (done.status.as_str(), done.exit_code),
            ("completed", Some(0))
        );
        // An ending is final.
        m.host_status(
            "hst_A",
            &r.id,
            &StatusReport {
                status: "running".into(),
                exit_code: None,
                end_reason: None,
                signal: None,
            },
        )
        .unwrap();
        assert_eq!(m.get(&r.id).unwrap().status, "completed");
    }

    #[test]
    fn a_missing_provider_falls_back_and_says_so() {
        let (m, _d) = manager();
        let _rx = online(
            &m,
            "hst_A",
            caps(&[("claude-code", false), ("opencode", true), ("shell", true)]),
        );
        let r = launch(&m, None).unwrap();
        assert_eq!(r.provider, "opencode");
        assert_eq!(
            r.provider_fallback,
            Some(Fallback {
                requested: "claude-code".into(),
                used: "opencode".into(),
                reason: "not_installed".into()
            })
        );
        // No supported provider at all is a 422, not a silent pick.
        let (m, _d) = manager();
        let _rx = online(&m, "hst_A", caps(&[("claude-code", false)]));
        assert!(matches!(launch(&m, None), Err(AgentError::NoProvider)));
    }

    #[test]
    fn launches_respect_the_hosts_limits() {
        let (m, _d) = manager();
        let _rx = online(&m, "hst_A", caps(&[("shell", true)]));
        launch(&m, Some("shell")).unwrap();
        launch(&m, Some("shell")).unwrap();
        assert!(matches!(
            launch(&m, Some("shell")),
            Err(AgentError::TooManySessions)
        ));
        let bad = m.launch(
            "u",
            Launch {
                host_id: "hst_A".into(),
                workspace: "../etc".into(),
                provider: None,
                interaction: "terminal".into(),
                terminal: TerminalSize { cols: 80, rows: 24 },
                context: None,
                write_vault_id: None,
                allow_vault_writes: false,
            },
            LaunchMeta::default(),
            Vec::new(),
        );
        assert!(matches!(bad, Err(AgentError::BadRequest(_))));
    }

    #[test]
    fn an_offline_host_refuses_input_and_launches() {
        let (m, _d) = manager();
        let (generation, _rx) = m.connect_host("hst_A");
        m.hello(
            "hst_A",
            Hello {
                capabilities: caps(&[("shell", true)]),
                sessions: vec![],
                last_cmd_seq: 0,
            },
        )
        .unwrap();
        let r = launch(&m, Some("shell")).unwrap();
        m.host_status(
            "hst_A",
            &r.id,
            &StatusReport {
                status: "running".into(),
                exit_code: None,
                end_reason: None,
                signal: None,
            },
        )
        .unwrap();

        m.disconnect_host("hst_A", generation);
        assert_eq!(m.get(&r.id).unwrap().status, "unknown");
        assert!(matches!(m.input(&r.id, b"x"), Err(AgentError::HostOffline)));
        assert!(matches!(
            launch(&m, Some("shell")),
            Err(AgentError::HostOffline)
        ));
    }

    #[test]
    fn a_stale_link_closing_does_not_take_a_newer_one_down() {
        let (m, _d) = manager();
        let (old, _rx1) = m.connect_host("hst_A");
        let (_new, _rx2) = m.connect_host("hst_A");
        m.disconnect_host("hst_A", old);
        assert!(m.is_online("hst_A"));
    }

    #[test]
    fn hello_reconciles_reported_lost_and_replays() {
        let (m, _d) = manager();
        let mut rx = online(&m, "hst_A", caps(&[("shell", true)]));
        let a = launch(&m, Some("shell")).unwrap();
        let b = launch(&m, Some("shell")).unwrap();
        while rx.try_recv().is_ok() {}

        // The host reconnects reporting only `a`, with output the server's
        // cache does not have: `b` is lost, `a` runs on and is replayed.
        let (_, mut rx) = m.connect_host("hst_A");
        m.hello(
            "hst_A",
            Hello {
                capabilities: caps(&[("shell", true)]),
                sessions: vec![ReportedSession {
                    id: a.id.clone(),
                    status: StatusReport {
                        status: "running".into(),
                        exit_code: None,
                        end_reason: None,
                        signal: None,
                    },
                    output_end_offset: 500,
                }],
                last_cmd_seq: 0,
            },
        )
        .unwrap();
        assert_eq!(m.get(&a.id).unwrap().status, "running");
        let lost = m.get(&b.id).unwrap();
        assert_eq!(
            (lost.status.as_str(), lost.end_reason.as_deref()),
            ("failed", Some("lost"))
        );
        let cmd = rx.try_recv().unwrap();
        assert_eq!(
            cmd.command,
            Command::Replay {
                session: a.id.clone(),
                from: 0
            }
        );
    }

    #[test]
    fn a_host_may_post_only_for_its_own_sessions() {
        let (m, _d) = manager();
        let _rx = online(&m, "hst_A", caps(&[("shell", true)]));
        let r = launch(&m, Some("shell")).unwrap();
        assert!(matches!(
            m.host_output("hst_B", &r.id, 0, b"x"),
            Err(AgentError::NotYours)
        ));
        m.host_output("hst_A", &r.id, 0, b"hi").unwrap();
        assert_eq!(
            m.read_output(&r.id, 0, 10),
            Read::Data {
                from: 0,
                bytes: b"hi".to_vec()
            }
        );
    }

    #[test]
    fn a_server_restart_makes_live_sessions_unknown() {
        let dir = tempdir::TempDir::new("storm-agent-restart").unwrap();
        let id = {
            let m = AgentManager::open(dir.path()).unwrap();
            let _rx = online(&m, "hst_A", caps(&[("shell", true)]));
            let r = launch(&m, Some("shell")).unwrap();
            m.host_status(
                "hst_A",
                &r.id,
                &StatusReport {
                    status: "running".into(),
                    exit_code: None,
                    end_reason: None,
                    signal: None,
                },
            )
            .unwrap();
            r.id
        };
        let m = AgentManager::open(dir.path()).unwrap();
        assert_eq!(m.get(&id).unwrap().status, "unknown");
    }

    #[test]
    fn revoking_fails_the_hosts_sessions_and_only_ended_ones_dismiss() {
        let (m, _d) = manager();
        let _rx = online(&m, "hst_A", caps(&[("shell", true)]));
        let r = launch(&m, Some("shell")).unwrap();
        assert!(matches!(m.dismiss(&r.id), Err(AgentError::Conflict(_))));
        m.revoke_host("hst_A").unwrap();
        let rec = m.get(&r.id).unwrap();
        assert_eq!(
            (rec.status.as_str(), rec.end_reason.as_deref()),
            ("failed", Some("host_revoked"))
        );
        assert!(!m.is_online("hst_A"));
        m.dismiss(&r.id).unwrap();
        assert!(matches!(m.get(&r.id), Err(AgentError::NotFound(_))));
    }

    #[test]
    fn the_wire_commands_are_the_freezes() {
        let e = Envelope {
            cmd_seq: 7,
            command: Command::Input {
                session: "ags_X".into(),
                data: "aGk=".into(),
            },
        };
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["type"], "terminal.input");
        assert_eq!(v["cmd_seq"], 7);
        let v = serde_json::to_value(Command::Replay {
            session: "s".into(),
            from: 3,
        })
        .unwrap();
        assert_eq!(v["type"], "terminal.replay");
    }
}
