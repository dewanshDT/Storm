//! HTTP + WebSocket surface.
//!
//! Deliberately small. The interesting logic lives in [`crate::index`]; this
//! module only translates it to and from JSON, and enforces the bearer token.

use std::collections::HashMap;
use std::path::{Path as FsPath, PathBuf};
use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{
        Extension, FromRequestParts, Path, Query, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{delete, get, patch, post, put},
};
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, RwLock, broadcast};

use crate::auth::authz::{Access, Actor, Decision, VaultPolicy};
use crate::db::{Change, Db};
use crate::index::Indexer;
use crate::registry::Registry;
use crate::vault::Vault;

/// One open vault's index.
///
/// The `Mutex` is per vault rather than global, so two vaults can be worked on
/// at once while each keeps the single-writer discipline the indexer relies on.
///
/// Deliberately holds no copy of the [`VaultEntry`]: the registry is the only
/// source of truth for a vault's name and directory, and a second copy here
/// would need keeping in step on every rename.
pub struct VaultHandle {
    pub indexer: Mutex<Indexer>,
}

/// Every open vault, plus the root they live under.
pub struct VaultSet {
    pub registry: Registry,
    pub open: HashMap<String, Arc<VaultHandle>>,
}

impl VaultSet {
    pub fn get(&self, id: &str) -> Option<Arc<VaultHandle>> {
        self.open.get(id).cloned()
    }
}

pub struct AppState {
    pub vaults: RwLock<VaultSet>,
    pub events: broadcast::Sender<Change>,
    pub state_dir: PathBuf,
    /// Who this server is, loaded from `state/auth.db` at boot.
    ///
    /// Held in memory rather than read per request: signing a challenge needs
    /// the private key, and the file is read once so its bytes are not walking
    /// through the filesystem on every unauthenticated call.
    pub identity: Arc<crate::auth::ServerIdentity>,
    /// Notifies the watcher that the root moved, so it can be respawned
    /// against the new one. Sends the new root.
    pub root_changed: broadcast::Sender<PathBuf>,
    /// The configured relay list, sent on every save so the tunnels follow it
    /// without a restart (decision 74). `relay::manage` is the receiver.
    pub relays_changed: tokio::sync::watch::Sender<Vec<String>>,
    /// Whether `/mcp` answers. Mirrors `Registry::mcp_enabled`, which is the
    /// persisted copy; this one exists so the gate can read it without taking
    /// the vault lock on every request.
    pub mcp_enabled: std::sync::atomic::AtomicBool,
    /// Whether MCP may change the vault. Read per request, so the switch takes
    /// effect on the next call rather than the next restart.
    pub mcp_writable: std::sync::atomic::AtomicBool,
    /// Whether agent sessions may write to their launch's vault. Mirrors
    /// `Registry::agent_writes`; read per call, like `mcp_writable`.
    pub agent_writes: std::sync::atomic::AtomicBool,
    /// The auth database, held open for request-time use (device lookup,
    /// session authenticate, login, refresh, ws-ticket).
    pub auth_db: Arc<tokio::sync::Mutex<crate::auth::AuthDb>>,
    /// The server's listen address, used as the `addr` hint in QR payloads.
    pub listen_addr: String,
    /// Decides whether an actor may reach a vault.
    ///
    /// Behind a trait object so the RBAC slice can replace it without touching
    /// a handler — that is the whole reason the boundary exists. Today it is
    /// `StormPolicy`: everything, except an agent writing outside its vault.
    pub vault_policy: Arc<dyn VaultPolicy>,
    /// **The one Argon2id gate for the whole process.**
    ///
    /// [`crate::auth::Hasher`] bounds concurrent hashes with a semaphore, and
    /// its own documentation states the condition that makes that bound real:
    /// *the bound is only a bound if every caller goes through the same one.*
    /// Every handler called `Hasher::new()`, which mints a **fresh** pair of
    /// permits — so the limit applied within a single request, which never
    /// makes more than one hash anyway, and to nothing across requests. The
    /// semaphore was decorative on every path an HTTP client can reach.
    ///
    /// **This was latent rather than live, and the reason is worth knowing.**
    /// Each of these handlers takes the `auth_db` mutex and holds it across the
    /// `.await` on the KDF, so Argon2id calls were already serialized at
    /// concurrency 1 — by a global lock, accidentally, not by the mechanism
    /// built for it. The exposure is the next person to narrow that lock scope,
    /// which is an obvious thing to want (holding one mutex over a ~170 ms KDF
    /// serializes all authentication) and would remove the only real bound
    /// while leaving the one that looks like a bound in place. `spawn_blocking`
    /// runs 512 threads and each hash takes 192 MiB.
    ///
    /// Lives here so there is exactly one, for the process's whole life.
    pub hasher: crate::auth::Hasher,
    /// **The one login rate limiter for the whole process.**
    ///
    /// Same discipline as [`AppState::hasher`], for the same reason: a
    /// per-handler limiter would be decorative in exactly the way a
    /// per-handler `Hasher::new()` was — each request would mint a fresh pair
    /// of full buckets and no limit would exist across requests. See
    /// [`crate::auth::ratelimit`] for the two-bucket shape and why the global
    /// ceiling is strict while the per-caller one is generous.
    pub login_limiter: crate::auth::ratelimit::LoginLimiter,
    /// The rate limit on the unauthenticated host routes (`/v1/runtime/enroll`
    /// and `/auth*`, decision 77b). **Its own budget, not the login one**: a
    /// misbehaving host retrying its link must not lock people out of logging
    /// in, and a login flood must not stop hosts reconnecting.
    pub host_limiter: crate::auth::ratelimit::LoginLimiter,
    /// The Agent Manager: the only session authority (decision 77c).
    pub agent: Arc<crate::agent::AgentManager>,
    /// The MCP Gateway's store and data key (decision 81b). Opened at boot so
    /// the key exists before the first backup.
    pub gateway: Arc<crate::gateway::Gateway>,
    /// LAN discovery's switch (decision 118): the advertisement follows it, and
    /// password pairing is refused while it is off.
    pub discovery: crate::discovery::Discovery,
}

pub type Shared = Arc<AppState>;

/// Opens every registered vault under `registry.root`.
///
/// A vault whose directory has vanished is skipped rather than created: the
/// registry entry survives and the API reports it as `missing`. Creating the
/// directory here would quietly resurrect a vault whose disk went away, which
/// is exactly the "where did my notes go" failure this design refuses to have.
pub fn open_vaults(registry: &Registry, state_dir: &FsPath) -> anyhow::Result<VaultSet> {
    let mut open = HashMap::new();
    for entry in &registry.vaults {
        if registry.is_missing(entry) {
            tracing::warn!(
                vault = %entry.name,
                dir = %registry.path_of(entry).display(),
                "vault directory is missing — keeping it registered, serving it as missing"
            );
            continue;
        }
        let vault = Vault::new(registry.path_of(entry))?;
        let path = vault.root().to_path_buf();
        let db = Db::open(&state_dir.join(&entry.id).join("index.db"), &entry.id)?;
        let mut indexer = Indexer::new(vault, db);
        let report = indexer.reconcile(false)?;
        tracing::info!(
            vault = %entry.name,
            path = %path.display(),
            scanned = report.scanned,
            indexed = report.indexed,
            updated = report.updated,
            "vault reconciled"
        );
        open.insert(
            entry.id.clone(),
            Arc::new(VaultHandle {
                indexer: Mutex::new(indexer),
            }),
        );
    }
    Ok(VaultSet {
        registry: registry.clone(),
        open,
    })
}

/// Upload ceiling. Generous for a scanned PDF, small enough that a stray
/// request can't take down a 4 GB VM.
pub const MAX_ATTACHMENT_BYTES: usize = 64 * 1024 * 1024;

/// A note's body ceiling (128-D8). Every other route keeps axum's 2 MiB
/// default; a router-wide attachment limit once gave all of them 64 MiB.
pub const MAX_NOTE_BYTES: usize = 16 * 1024 * 1024;

/// Errors rendered as JSON rather than bare status codes, so the Flutter
/// client can show something useful.
pub struct ApiError(pub StatusCode, pub String);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(serde_json::json!({ "error": self.1 }))).into_response()
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> Self {
        ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
    }
}

/// The one place a vault operation's failure becomes a status (decision 128a).
///
/// A 404 is reserved for a note or folder that does not exist: the client's
/// outbox discards a queued edit on 404, so a write that failed for any other
/// reason answers 500, which it keeps queued and retries.
impl From<crate::index::IndexError> for ApiError {
    fn from(e: crate::index::IndexError) -> Self {
        use crate::index::IndexError as E;
        match e {
            E::NotFound(m) => not_found(m),
            E::Exists(m) | E::Conflict(m) => conflict(m),
            E::Invalid(m) => bad_request(m),
            E::Other(e) => {
                tracing::error!(error = format!("{e:#}"), "a vault operation failed");
                ApiError(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "the server could not complete that change".into(),
                )
            }
        }
    }
}

pub fn bad_request(msg: impl Into<String>) -> ApiError {
    ApiError(StatusCode::BAD_REQUEST, msg.into())
}

pub fn not_found(msg: impl Into<String>) -> ApiError {
    ApiError(StatusCode::NOT_FOUND, msg.into())
}

pub fn conflict(msg: impl Into<String>) -> ApiError {
    ApiError(StatusCode::CONFLICT, msg.into())
}

pub type ApiResult<T> = Result<T, ApiError>;

/// Resolves a vault id to its open handle — **the authorization boundary**.
///
/// One place, so no handler repeats the distinction: unknown is a 404, while a
/// registered vault whose directory has gone is a 409 — the difference between
/// "never existed" and "is not where it should be" is the whole point of
/// keeping missing vaults in the registry.
///
/// It also takes an [`Actor`] and an [`Access`], and that is the load-bearing
/// part. **This is the only way to obtain a `VaultHandle` for a named vault**,
/// so a handler cannot forget to ask whether the caller is allowed — it has
/// nothing to operate on until it has said who is asking and what for. The
/// check is not a rule people follow; it is a parameter they cannot omit.
///
/// REST handlers and MCP tools both arrive here, because both go through
/// `ops.rs` or call this directly. A middleware layer could not do the same
/// job: MCP's vault id lives in the JSON-RPC body, not the URL, so a
/// URL-matching layer sees one `POST /mcp` and cannot tell which vault is
/// being asked for.
///
/// The policy is consulted **before** existence is checked, so a refusal never
/// doubles as a probe for which vault ids are real.
pub async fn vault_of(
    state: &Shared,
    actor: &Actor,
    access: Access,
    id: &str,
) -> ApiResult<Arc<VaultHandle>> {
    if let Decision::Deny(reason) = state.vault_policy.decide(actor, id, access) {
        tracing::info!(
            actor = actor.describe(),
            key = actor.key_id(),
            host = actor.host_id(),
            vault = id,
            ?access,
            reason,
            "vault access refused"
        );
        // 403, never 404 and never an empty list: "you may not see this" has
        // to be distinguishable from "your notes are gone" (decision 25).
        // An agent's refusal is its stable code, which `mcp.rs` hands it.
        let message = if reason == crate::auth::authz::AGENT_WRITE_REFUSED {
            reason
        } else {
            "you do not have access to this vault"
        };
        return Err(ApiError(StatusCode::FORBIDDEN, message.into()));
    }

    let vaults = state.vaults.read().await;
    if let Some(handle) = vaults.get(id) {
        return Ok(handle);
    }
    match vaults.registry.get(id) {
        Some(entry) => Err(conflict(format!(
            "the directory for “{}” is missing from the storage root",
            entry.name
        ))),
        None => Err(not_found("no such vault")),
    }
}

/// Whether a vault belongs in a *collection* this actor is reading.
///
/// The other half of the rule, and it is not the same answer. `403` is right
/// for a named vault and wrong for a list: you cannot refuse a list, so
/// `GET /v1/vaults` and `/v1/recents` **filter** instead. Converting these to
/// refusals would mean one ungranted vault blanking the whole dashboard.
pub fn may_see_vault(state: &Shared, actor: &Actor, id: &str) -> bool {
    state
        .vault_policy
        .decide(actor, id, Access::Read)
        .is_allowed()
}

/// Builds the HTTP surface.
///
/// `mcp` mounts the Model Context Protocol endpoint. It is a parameter rather
/// than something bolted on afterwards for a load-bearing reason: axum applies
/// a layer only to the routes registered *above* it, so `/mcp` has to be nested
/// before the session auth layer or it would be the one unauthenticated route on
/// the server. `mcp_requires_the_bearer_token` is the test that holds this.
pub fn router(state: Shared, mcp: crate::mcp::McpOptions) -> Router {
    // Always mounted, never conditionally: whether MCP answers is a runtime
    // setting the app can toggle, and a route that only exists when a flag was
    // passed at boot could not be turned on without a restart the client has no
    // way to perform.
    let mcp_router = Router::new()
        .fallback_service(crate::mcp::service(state.clone(), mcp))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            require_mcp_enabled,
        ));

    // Each tier is a self-contained Router with its own auth layer.
    // `merge` combines routes without leaking layers across tiers.

    // ---- none tier: no credential required --------------------------------
    let none_router = Router::new()
        .route("/v1/health", get(health))
        .route("/v1/server", get(server_info))
        .route("/v1/server/challenge", post(server_challenge))
        .route("/v1/pair", post(pair_handler))
        // LAN discovery pairing (decision 118). Unauthenticated like
        // `/v1/pair`; the account password is the credential, and the op
        // refuses anything relayed or not from the LAN.
        .route("/v1/pairings/password", post(pair_password_handler))
        // A Runtime Host's way in (decision 77b). Unauthenticated by nature —
        // the enrollment token and the host key *are* the credentials — and
        // rate-limited by `host_limiter`.
        .route("/v1/runtime/enroll", post(runtime_enroll))
        .route("/v1/runtime/auth/challenge", post(runtime_challenge))
        .route("/v1/runtime/auth", post(runtime_auth))
        .layer(Extension(RequiredTier::None));

    // ---- host tier: `Bearer sht_…`, `/v1/runtime/*` only ------------------
    //
    // Its own tier, not a flag on another, for the reason `Mcp` is: the set of
    // credentials that may reach it is disjoint from every other route's. A
    // host token is refused everywhere else, and nothing else is accepted
    // here (decision 77b).
    let host_router = Router::new()
        .route("/v1/runtime/whoami", get(runtime_whoami))
        .route("/v1/runtime/link", get(runtime_link))
        .route("/v1/runtime/hello", post(runtime_hello))
        .route("/v1/runtime/heartbeat", post(runtime_heartbeat))
        .route("/v1/runtime/inventory", post(runtime_inventory))
        .route(
            "/v1/runtime/sessions/{id}/terminal/output",
            post(runtime_output),
        )
        .route("/v1/runtime/sessions/{id}/status", post(runtime_status))
        .route(
            "/v1/runtime/sessions/{id}/terminal/images/{image}",
            get(runtime_image).post(runtime_image_staged),
        )
        // The MCP Gateway (decision 81e): a session's bridge traffic, carried
        // by the host's existing token — no new credential (G-D3).
        .route(
            "/v1/runtime/sessions/{id}/mcp/{connection}",
            post(runtime_mcp),
        )
        // Bridge identity (decision 135a): a connection the host refused.
        .route(
            "/v1/runtime/sessions/{id}/identity-refused",
            post(runtime_identity_refused),
        )
        // Token sync (decision 124): the synced login for a starting session.
        .route(
            "/v1/runtime/sessions/{id}/provider-credential",
            get(runtime_provider_credential),
        )
        // Agent installs (decision 133): a job's progress, and a host's own
        // automatic update (133-5).
        .route("/v1/runtime/agents/jobs", post(runtime_start_agent_job))
        .route("/v1/runtime/agents/jobs/{job}", post(runtime_agent_job))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            require_auth,
        ))
        .layer(Extension(RequiredTier::Host));

    // ---- device tier: `StormDevice <id>:<secret>` -------------------------
    let device_router = Router::new()
        .route("/v1/account", get(account_state))
        .route("/v1/users/first", post(create_first_user))
        .route("/v1/auth/login", post(login_handler))
        .route("/v1/auth/refresh", post(refresh_handler))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            require_auth,
        ))
        .layer(Extension(RequiredTier::Device));

    // ---- session tier: `Bearer <token>` -----------------------------------
    let session_router = Router::new()
        .route("/v1/vaults", get(list_vaults).post(create_vault))
        .route("/v1/vaults/{vault}", patch(rename_vault))
        .route("/v1/vaults/{vault}", delete(remove_vault))
        .route("/v1/config", get(get_config).put(put_config))
        .route("/v1/config/mcp", put(put_mcp))
        .route("/v1/config/relays", put(put_relays))
        .route("/v1/config/discovery", put(put_discovery))
        // MCP keys (A14). **Session tier**: minting a key costs a real
        // sign-in on a paired device, which is also what makes "a key cannot
        // mint a key" true without a special case — a key never reaches here.
        .route("/v1/keys", get(list_keys).post(create_key))
        .route("/v1/keys/{id}", delete(revoke_key))
        .route("/v1/recents", get(recents))
        .route("/v1/vaults/{vault}/tree", get(tree))
        .route("/v1/vaults/{vault}/sync", get(sync))
        .route(
            "/v1/vaults/{vault}/notes",
            post(create_note).layer(axum::extract::DefaultBodyLimit::max(MAX_NOTE_BYTES)),
        )
        .route("/v1/vaults/{vault}/notes/{id}", get(get_note))
        .route(
            "/v1/vaults/{vault}/notes/{id}",
            put(put_note).layer(axum::extract::DefaultBodyLimit::max(MAX_NOTE_BYTES)),
        )
        .route("/v1/vaults/{vault}/notes/{id}", delete(delete_note))
        .route("/v1/vaults/{vault}/notes/{id}/move", post(move_note))
        .route("/v1/vaults/{vault}/notes/{id}/opened", post(mark_opened))
        .route("/v1/vaults/{vault}/notes/{id}/backlinks", get(backlinks))
        .route("/v1/vaults/{vault}/notes/{id}/versions", get(note_versions))
        .route(
            "/v1/vaults/{vault}/notes/{id}/versions/{version}",
            get(note_version),
        )
        .route("/v1/vaults/{vault}/folders", post(create_folder))
        .route("/v1/vaults/{vault}/folders/rename", post(rename_folder))
        .route("/v1/vaults/{vault}/folders/{*path}", delete(delete_folder))
        .route("/v1/vaults/{vault}/search", get(search))
        .route("/v1/vaults/{vault}/tags", get(tags))
        .route("/v1/vaults/{vault}/tags/{tag}", get(notes_by_tag))
        .route("/v1/vaults/{vault}/agent-writes", get(vault_agent_writes))
        .route("/v1/vaults/{vault}/attachments", get(list_attachments))
        .route(
            "/v1/vaults/{vault}/attachments/{*path}",
            get(get_attachment)
                .put(put_attachment)
                .delete(delete_attachment)
                .layer(axum::extract::DefaultBodyLimit::max(MAX_ATTACHMENT_BYTES)),
        )
        .route("/v1/auth/logout", post(logout_handler))
        .route("/v1/auth/sessions", get(list_sessions_handler))
        .route("/v1/auth/sessions/{id}", delete(revoke_session_handler))
        .route("/v1/auth/devices", get(list_devices_handler))
        .route("/v1/auth/devices/{id}", delete(revoke_device_handler))
        .route("/v1/auth/password", post(change_password_handler))
        .route("/v1/auth/ws-ticket", post(ws_ticket_handler))
        // Agent Runtime host administration — owner only, checked in ops.
        .route("/v1/agent/hosts", get(agent_list_hosts))
        .route("/v1/agent/hosts/enrollments", post(agent_issue_enrollment))
        .route(
            "/v1/agent/hosts/{id}",
            patch(agent_rename_host).delete(agent_revoke_host),
        )
        .route(
            "/v1/agent/hosts/{id}/workspaces",
            get(agent_host_workspaces),
        )
        // Agent installs (decision 133): owner only, never an `stk_` key.
        .route("/v1/agent/hosts/{id}/agents/{agent}", post(agent_start_job))
        .route("/v1/agent/hosts/{id}/agents/jobs/{job}", get(agent_get_job))
        .route(
            "/v1/agent/sessions",
            get(agent_list_sessions).post(agent_launch),
        )
        .route(
            "/v1/agent/sessions/{id}",
            get(agent_get_session).delete(agent_dismiss_session),
        )
        .route("/v1/agent/sessions/{id}/end", post(agent_end_session))
        // Crews and role labels (decision 138): owner only, checked in ops.
        .route("/v1/agent/crews", get(crews_list).post(crews_create))
        .route("/v1/agent/crews/order", put(crews_reorder))
        .route(
            "/v1/agent/crews/{id}",
            patch(crews_update).delete(crews_delete),
        )
        .route(
            "/v1/agent/crews/{id}/members/order",
            put(crews_reorder_members),
        )
        .route("/v1/agent/sessions/{id}/crew", put(agent_session_crew))
        .route(
            "/v1/agent/sessions/{id}/role",
            put(agent_session_role).delete(agent_clear_session_role),
        )
        // The spawn switch (decision 134-2): owner only, checked in ops.
        .route("/v1/agent/sessions/{id}/spawn", put(agent_session_spawn))
        // The approval engine (decision 134-3): a device decides, owner only.
        .route(
            "/v1/agent/requests/{id}/approve",
            post(agent_request_approve),
        )
        .route("/v1/agent/requests/{id}/deny", post(agent_request_deny))
        .route("/v1/agent/roles", get(agent_roles))
        .route("/v1/agent/sessions/{id}/writes", get(agent_session_writes))
        .route(
            "/v1/agent/sessions/{id}/terminal/stream",
            get(agent_terminal_stream),
        )
        .route("/v1/agent/sessions/{id}/terminal/input", post(agent_input))
        .route(
            "/v1/agent/sessions/{id}/terminal/images",
            post(agent_image).layer(axum::extract::DefaultBodyLimit::max(
                crate::agent::images::MAX_IMAGE_BYTES + 1,
            )),
        )
        .route(
            "/v1/agent/sessions/{id}/terminal/resize",
            post(agent_resize),
        )
        .route(
            "/v1/config/agent",
            get(agent_get_config).put(agent_put_config),
        )
        // MCP Gateway integrations (decision 81c) — owner only, checked in
        // ops. **Session tier**, so an `stk_` key can never manage one (§14).
        .route(
            "/v1/integrations/connections",
            get(integrations_list).post(integrations_create),
        )
        .route(
            "/v1/integrations/connections/{id}",
            get(integrations_get)
                .patch(integrations_patch)
                .delete(integrations_delete),
        )
        .route(
            "/v1/integrations/connections/{id}/test",
            post(integrations_test),
        )
        .route(
            "/v1/integrations/connections/{id}/tools",
            get(integrations_tools),
        )
        .route(
            "/v1/integrations/connections/{id}/authorize",
            post(integrations_authorize),
        )
        .route(
            "/v1/integrations/oauth/callback",
            post(integrations_callback),
        )
        // Remote servers the owner's agents' plugins declare (decision 131g).
        .route(
            "/v1/integrations/suggestions",
            get(integrations_suggestions),
        )
        .route(
            "/v1/integrations/suggestions/dismiss",
            post(integrations_dismiss_suggestion),
        )
        // Token sync (decision 124n): owner only, session tier. No route here
        // returns the token.
        .route(
            "/v1/provider-credentials/claude-subscription",
            get(provider_credential_get)
                .put(provider_credential_put)
                .delete(provider_credential_delete)
                .layer(axum::extract::DefaultBodyLimit::max(
                    crate::gateway::provider::MAX_BODY_BYTES,
                )),
        )
        .route(
            "/v1/provider-credentials/claude-subscription/hosts",
            patch(provider_credential_hosts).layer(axum::extract::DefaultBodyLimit::max(64 * 1024)),
        )
        .route("/v1/pairings", post(issue_pairing_handler))
        .route("/v1/stream", get(stream))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            require_auth,
        ))
        .layer(Extension(RequiredTier::Session));

    // MCP needs authentication + the mcp_enabled gate. It is nested under /mcp
    // rather than merged, so its paths don't collide with REST routes.
    //
    // **`RequiredTier::Mcp`, not `Session`** (A14): this is the one surface
    // that accepts an `stk_` key, and it still accepts everything it accepted
    // before — a session token. There is no shared-token path any more.
    let mcp_with_auth = Router::new()
        .nest("/mcp", mcp_router)
        // Inner to `require_auth`, so `Extension<Actor>` is already set when it
        // runs. Layers apply outermost-last, so listing it *first* here is what
        // puts it after authentication. It exists because rmcp's handler is
        // built by a factory that receives no request — see `mcp::scope_actor`.
        .layer(axum::middleware::from_fn(crate::mcp::scope_actor))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            require_auth,
        ))
        .layer(Extension(RequiredTier::Mcp));

    // Merge all tiers: none is checked first, then device, then session, then
    // MCP. Each carries its own auth layer; merge does not leak them.
    none_router
        .merge(host_router)
        .merge(device_router)
        .merge(session_router)
        .merge(mcp_with_auth)
        .with_state(state)
}

// ---- server identity (unauthenticated) ---------------------------------

/// Answered flat rather than in an envelope: every field is the client's, and
/// there is no list here for a key to name.
async fn server_info(State(state): State<Shared>) -> ApiResult<Json<crate::ops::ServerInfo>> {
    Ok(Json(crate::ops::server_info(&state).await?))
}

#[derive(Deserialize)]
struct ChallengeRequest {
    nonce: String,
}

async fn server_challenge(
    State(state): State<Shared>,
    Json(body): Json<ChallengeRequest>,
) -> ApiResult<Json<crate::ops::ChallengeAnswer>> {
    Ok(Json(crate::ops::sign_challenge(&state, &body.nonce).await?))
}

// ---- device-tier endpoints ------------------------------------------------

/// POST /v1/keys — mint an MCP key (A14).
///
/// **The response carries the plaintext, and this is the only time it exists.**
/// Nothing stores it: not the database (which holds a blake3 hash), not the
/// server, not the client. A caller who loses it revokes and mints another.
async fn create_key(
    State(state): State<Shared>,
    Extension(actor): Extension<Actor>,
    // Required again since the cutover. It was `Option` because the shared
    // token satisfied this tier while inserting no `SessionAuth`, which made a
    // required extractor answer `500` where `403` was the truth. With that
    // credential gone, every caller who reaches here has a real session.
    Extension(session): Extension<SessionAuth>,
    Json(body): Json<CreateKeyRequest>,
) -> ApiResult<Json<crate::ops::CreatedApiKey>> {
    // Which device minted it, for the audit trail — a key that turns up in a
    // log is easier to place when you know where it was born.
    let via = session.authenticated.device.id.clone();
    Ok(Json(
        crate::ops::create_api_key(
            &state,
            &actor,
            &body.name,
            body.expires.as_deref(),
            Some(&via),
        )
        .await?,
    ))
}

#[derive(Deserialize)]
struct CreateKeyRequest {
    name: String,
    /// Optional absolute RFC3339 instant. `None` means the key does not expire
    /// — the design's default, revisited when there is evidence for another.
    #[serde(default)]
    expires: Option<String>,
}

/// GET /v1/keys — the account's access keys.
async fn list_keys(
    State(state): State<Shared>,
    Extension(actor): Extension<Actor>,
) -> ApiResult<Json<Vec<crate::auth::keys::ApiKey>>> {
    Ok(Json(crate::ops::list_api_keys(&state, &actor).await?))
}

/// DELETE /v1/keys/{id} — revoke a key, effective on the next request.
async fn revoke_key(
    State(state): State<Shared>,
    Extension(actor): Extension<Actor>,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    crate::ops::revoke_api_key(&state, &actor, &id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// GET /v1/account — whether this Storm is set up. Device tier: pairing asks
/// it to choose between setup and sign-in.
async fn account_state(State(state): State<Shared>) -> ApiResult<Json<serde_json::Value>> {
    let exists = state
        .auth_db
        .lock()
        .await
        .has_account()
        .map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(serde_json::json!({ "exists": exists })))
}

/// POST /v1/users/first — set up the account, once. Device tier (A8).
async fn create_first_user(
    State(state): State<Shared>,
    Json(body): Json<SetupRequest>,
) -> ApiResult<StatusCode> {
    if let Err(msg) = crate::auth::password::validate_password(&body.password) {
        return Err(ApiError(StatusCode::UNPROCESSABLE_ENTITY, msg));
    }

    let now = crate::index::now_rfc3339();
    let mut auth_db = state.auth_db.lock().await;

    // Before the hash, so a refusal never takes a `Hasher` permit.
    if auth_db
        .has_account()
        .map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "an account already exists".into(),
        ));
    }

    let hash = state
        .hasher
        .hash(body.password.clone())
        .await
        .map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    crate::auth::account::create_account(&mut auth_db, &hash, &now)
        .map_err(|e| ApiError(StatusCode::CONFLICT, e.to_string()))?;
    // `claimed=1` in the advertisement.
    state.discovery.poke();
    Ok(StatusCode::CREATED)
}

/// v0.3.x clients still send a username; it is ignored.
#[derive(Deserialize)]
pub struct SetupRequest {
    #[serde(default, rename = "username")]
    _username: Option<String>,
    password: String,
}

/// The socket peer, where this request has one.
///
/// A hand-written extractor rather than `Option<ConnectInfo<SocketAddr>>`,
/// which **does not compile on axum 0.8**: there is no blanket `Option<E>`
/// impl, `E` has to implement `OptionalFromRequestParts`, and `ConnectInfo`
/// does not — so the obvious signature fails the `Handler` bound with an error
/// that names neither `ConnectInfo` nor the missing trait. Same shape as
/// `Option<WebSocketUpgrade>`, which the relay design already had to work
/// around on `stream`.
///
/// The property that matters is that this **never rejects**. `main.rs` serves
/// with `into_make_service_with_connect_info`, so a real socket always carries
/// the extension; the relay's in-process dispatch reconstructs an
/// `http::Request` and calls the router as a tower service, so it never does.
/// A required extractor would turn every relayed login into an extractor
/// rejection — a refusal that never reaches the handler and reads as a
/// malformed request rather than a rate limit. `pair_handler` has the bare
/// form and will need this same treatment when the tunnel lands.
struct MaybePeer(Option<std::net::SocketAddr>);

impl<S: Send + Sync> axum::extract::FromRequestParts<S> for MaybePeer {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        _state: &S,
    ) -> Result<Self, Self::Rejection> {
        Ok(MaybePeer(
            parts
                .extensions
                .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
                .map(|c| c.0),
        ))
    }
}

/// POST /v1/auth/login — exchange the password for a token pair.
///
/// The caller must present a `StormDevice` header (device tier). The device
/// must be paired; if not the login fails with 401. After authentication the
/// caller receives access + refresh tokens bound to this device.
async fn login_handler(
    State(state): State<Shared>,
    MaybePeer(peer): MaybePeer,
    headers: HeaderMap,
    Json(body): Json<LoginRequest>,
) -> Result<Json<crate::auth::sessions::IssuedSession>, Response> {
    let now = crate::index::now_rfc3339();

    // The device must be present and paired — require_auth already checked
    // this, but we need the device_id for session binding.
    let device_id = extract_device_id(&headers).ok_or_else(|| {
        ApiError(StatusCode::UNAUTHORIZED, "device header required".into()).into_response()
    })?;

    // Charge the budget *before* acquiring the Argon2 permit: the point is to
    // stop a flood from ever reaching the KDF. A successful login refunds,
    // because a server under attack must still let real users in — see
    // `ratelimit.rs`. The socket peer is the only identity used here;
    // `X-Forwarded-For` is client-forgeable and never a security input.
    use crate::auth::ratelimit::CallerKey;
    let caller = peer
        .map(|c| CallerKey::Ip(c.ip()))
        .unwrap_or(CallerKey::Unattributed);
    if let Err(retry_after_secs) = state.login_limiter.check(&caller) {
        let remote = match &caller {
            CallerKey::Ip(ip) => Some(ip.to_string()),
            CallerKey::Unattributed => None,
        };
        let auth_db = state.auth_db.lock().await;
        if let Err(e) = auth_db.record_event_from(
            crate::auth::sessions::EVENT_LOGIN_THROTTLED,
            None,
            Some(&device_id),
            remote.as_deref(),
            &now,
            &format!(r#"{{"retry_after_secs":{retry_after_secs}}}"#),
        ) {
            tracing::warn!(error = %e, "could not record login_throttled event");
        }
        drop(auth_db);
        return Err(rate_limited(retry_after_secs));
    }

    let hasher = &state.hasher;
    let mut auth_db = state.auth_db.lock().await;
    match crate::auth::sessions::login(
        &mut auth_db,
        hasher,
        body.password.clone(),
        &device_id,
        &now,
    )
    .await
    {
        Ok(issued) => {
            state.login_limiter.refund(&caller);
            Ok(Json(issued))
        }
        // Rate limiting is the one refusal that is not a 401, because the
        // client's remedy is "wait", not "try different credentials" — and it
        // cannot say *how long* to wait without the number. Hence a `Response`
        // return type for this handler: `ApiError` is a status and a string,
        // with nowhere to put a header.
        Err(crate::auth::sessions::LoginError::Refused(
            crate::auth::sessions::LoginFailure::RateLimited { retry_after_secs },
        )) => Err(rate_limited(retry_after_secs)),
        Err(crate::auth::sessions::LoginError::Refused(failure)) => {
            Err(ApiError(StatusCode::UNAUTHORIZED, failure.code().to_string()).into_response())
        }
        Err(crate::auth::sessions::LoginError::Internal(e)) => {
            Err(ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response())
        }
    }
}

/// `429` with `Retry-After`, per the error table in *Storm Auth Protocol*.
///
/// The seconds go in the body as well as the header: the header is the correct
/// HTTP answer, and the body is what the client actually renders, since a
/// message that says "too many attempts" without saying for how long invites
/// exactly the retry it is trying to stop.
fn rate_limited(retry_after_secs: i64) -> Response {
    (
        StatusCode::TOO_MANY_REQUESTS,
        [(
            axum::http::header::RETRY_AFTER,
            retry_after_secs.to_string(),
        )],
        Json(serde_json::json!({
            "error": "rate_limited",
            "retry_after": retry_after_secs,
        })),
    )
        .into_response()
}

/// Extracts the device id from a `StormDevice <id>:<secret>` header.
fn extract_device_id(headers: &HeaderMap) -> Option<String> {
    let val = headers.get("authorization")?.to_str().ok()?;
    let rest = val.strip_prefix("StormDevice ")?;
    let (id, _) = rest.split_once(':')?;
    Some(id.to_string())
}

/// v0.3.x clients still send a username; it is ignored.
#[derive(Deserialize)]
pub struct LoginRequest {
    #[serde(default, rename = "username")]
    _username: Option<String>,
    password: String,
}

/// POST /v1/auth/refresh — exchange a refresh token for a new token pair.
async fn refresh_handler(
    State(state): State<Shared>,
    Json(body): Json<RefreshRequest>,
) -> ApiResult<Json<crate::auth::sessions::IssuedSession>> {
    let now = crate::index::now_rfc3339();
    let mut auth_db = state.auth_db.lock().await;
    match crate::auth::sessions::refresh(&mut auth_db, &body.refresh_token, &now) {
        Ok(issued) => Ok(Json(issued)),
        Err(crate::auth::sessions::SessionError::Refused(
            crate::auth::sessions::AuthFailure::Unknown,
        )) => Err(ApiError(
            StatusCode::UNAUTHORIZED,
            "invalid refresh token".into(),
        )),
        Err(crate::auth::sessions::SessionError::Refused(
            crate::auth::sessions::AuthFailure::Revoked,
        )) => Err(ApiError(StatusCode::UNAUTHORIZED, "session revoked".into())),
        Err(crate::auth::sessions::SessionError::Refused(_)) => {
            Err(ApiError(StatusCode::UNAUTHORIZED, "token rejected".into()))
        }
        Err(crate::auth::sessions::SessionError::Internal(e)) => {
            Err(ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
        }
    }
}

#[derive(Deserialize)]
pub struct RefreshRequest {
    refresh_token: String,
}

// ---- session-tier auth management endpoints -------------------------------

/// POST /v1/auth/logout — revoke the current session.
async fn logout_handler(
    State(state): State<Shared>,
    Extension(auth): Extension<SessionAuth>,
) -> ApiResult<StatusCode> {
    let now = crate::index::now_rfc3339();
    let mut auth_db = state.auth_db.lock().await;
    crate::auth::sessions::revoke(
        &mut auth_db,
        &auth.authenticated.session.id,
        "user logout",
        &now,
    )
    .map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(StatusCode::NO_CONTENT)
}

/// GET /v1/auth/sessions — list all sessions for the current user.
async fn list_sessions_handler(
    State(state): State<Shared>,
    Extension(auth): Extension<SessionAuth>,
) -> ApiResult<Json<Vec<crate::auth::sessions::Session>>> {
    let auth_db = state.auth_db.lock().await;
    let sessions = auth_db
        .list_sessions(Some(&auth.authenticated.account.id))
        .map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(sessions))
}

/// DELETE /v1/auth/sessions/{id} — revoke a session (own sessions only).
async fn revoke_session_handler(
    State(state): State<Shared>,
    Extension(auth): Extension<SessionAuth>,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    let now = crate::index::now_rfc3339();
    let mut auth_db = state.auth_db.lock().await;

    // Verify ownership.
    let session = auth_db
        .session_by_id(&id)
        .map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "session not found".into()))?;

    if session.user_id != auth.authenticated.account.id {
        return Err(ApiError(StatusCode::FORBIDDEN, "not your session".into()));
    }

    crate::auth::sessions::revoke(&mut auth_db, &id, "user revoked", &now)
        .map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(StatusCode::NO_CONTENT)
}

/// GET /v1/auth/devices — list all paired devices.
async fn list_devices_handler(
    State(state): State<Shared>,
) -> ApiResult<Json<Vec<crate::auth::devices::Device>>> {
    let auth_db = state.auth_db.lock().await;
    let devices = auth_db
        .list_devices()
        .map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(devices))
}

/// DELETE /v1/auth/devices/{id} — revoke a paired device.
async fn revoke_device_handler(
    State(state): State<Shared>,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    let now = crate::index::now_rfc3339();
    let mut auth_db = state.auth_db.lock().await;
    crate::auth::devices::revoke(&mut auth_db, &id, "user revoked", &now)
        .map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(StatusCode::NO_CONTENT)
}

/// POST /v1/auth/password — change the current user's password.
///
/// Verifies the current password, hashes the new one, and stores it. The
/// caller must be authenticated.
async fn change_password_handler(
    State(state): State<Shared>,
    Extension(auth): Extension<SessionAuth>,
    Json(body): Json<ChangePasswordRequest>,
) -> ApiResult<StatusCode> {
    if let Err(msg) = crate::auth::password::validate_password(&body.new_password) {
        return Err(ApiError(StatusCode::UNPROCESSABLE_ENTITY, msg));
    }

    let now = crate::index::now_rfc3339();
    let mut auth_db = state.auth_db.lock().await;

    // Verify the current password before allowing the change.
    let stored_hash = auth_db
        .password_hash_of(&auth.authenticated.account.id)
        .map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| {
            ApiError(
                StatusCode::INTERNAL_SERVER_ERROR,
                "account has no password".into(),
            )
        })?;

    let hasher = &state.hasher;
    let ok = hasher
        .verify(body.current_password.clone(), stored_hash)
        .await
        .map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    if !ok {
        return Err(ApiError(StatusCode::UNAUTHORIZED, "wrong password".into()));
    }

    let new_hash = hasher
        .hash(body.new_password.clone())
        .await
        .map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    crate::auth::account::set_password(&mut auth_db, &new_hash, &now)
        .map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct ChangePasswordRequest {
    current_password: String,
    new_password: String,
}

/// POST /v1/auth/ws-ticket — mint a short-lived ticket for WebSocket auth.
async fn ws_ticket_handler(
    State(state): State<Shared>,
    Extension(auth): Extension<SessionAuth>,
) -> ApiResult<Json<serde_json::Value>> {
    let now = crate::index::now_rfc3339();
    let mut auth_db = state.auth_db.lock().await;
    let issued =
        crate::auth::sessions::create_ws_ticket(&mut auth_db, &auth.authenticated.session.id, &now)
            .map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(serde_json::json!({
        "ticket": issued.ticket,
        "expires_in": issued.expires_in,
    })))
}

// ---- pairing endpoints ---------------------------------------------------

/// How many web-bootstrap nonces one peer may be issued per minute.
pub const WEB_BOOTSTRAP_PER_MINUTE: i64 = 12;

/// Default ceiling on *outstanding* (unconsumed, unexpired) web-bootstrap
/// nonces, across every peer.
///
/// A bound on the pairing table, not on how many devices may exist — a
/// permanent cap on devices would be a different and unjustified restriction.
/// Reachable only by a client fetching the page far faster than a person does.
pub const WEB_BOOTSTRAP_MAX_OUTSTANDING: i64 = 256;

/// Headers whose presence means the peer address belongs to a proxy.
///
/// Also stripped from every relayed request before dispatch: they are
/// client-supplied over the tunnel, and `web_bootstrap_nonce` below treats
/// their presence as a fact about the *network*. See `relay/dispatch.rs`.
pub(crate) const FORWARDING_HEADERS: [&str; 4] = [
    "x-forwarded-for",
    "x-forwarded-host",
    "x-real-ip",
    "forwarded",
];

/// Mints a web bootstrap nonce for `peer`, or `None` if it should not have one.
///
/// `None` is never an error to the caller: the index document is served either
/// way, and a client that does not get a nonce simply sees the pairing screen.
/// Serving the app is not the thing being rationed.
async fn web_bootstrap_nonce(
    state: &Shared,
    peer: std::net::IpAddr,
    headers: &HeaderMap,
) -> Option<(String, String)> {
    // **Behind a proxy, nobody gets one.** The peer address would be the
    // proxy's, so binding would bind the entire LAN to a single address —
    // the exact opposite of the intent. `X-Forwarded-For` is not consulted as
    // a substitute because it is set by the client. A deployment that wants
    // this behind a proxy needs explicit trusted-proxy configuration, which is
    // deferred rather than guessed at.
    if FORWARDING_HEADERS.iter().any(|h| headers.contains_key(*h)) {
        tracing::debug!("no web bootstrap: a forwarding header hides the real peer");
        return None;
    }

    let peer_ip = peer.to_string();
    let now = crate::index::now_rfc3339();

    {
        let auth_db = state.auth_db.lock().await;

        // Sweep first: every page load mints one of these and almost none are
        // consumed, so without this the table grows with traffic rather than
        // with devices — and the outstanding count below would be measuring
        // litter.
        if let Err(e) = auth_db.sweep_expired_pairings(&now) {
            tracing::warn!(error = %e, "sweeping expired pairing sessions");
        }

        let minute_ago = crate::index::rfc3339_minus_secs(&now, 60);
        match auth_db.web_bootstrap_issued_since(&peer_ip, &minute_ago) {
            Ok(n) if n >= WEB_BOOTSTRAP_PER_MINUTE => {
                tracing::warn!(peer = %peer_ip, issued = n, "web bootstrap rate limit");
                return None;
            }
            Err(e) => {
                tracing::warn!(error = %e, "counting web bootstrap issuance");
                return None;
            }
            _ => {}
        }

        match auth_db.web_bootstrap_outstanding(&now) {
            Ok(n) if n >= WEB_BOOTSTRAP_MAX_OUTSTANDING => {
                tracing::warn!(outstanding = n, "web bootstrap ceiling reached");
                return None;
            }
            Err(e) => {
                tracing::warn!(error = %e, "counting outstanding web bootstraps");
                return None;
            }
            _ => {}
        }
    }

    match crate::ops::issue_pairing_qr(state, "web_bootstrap", None, Some(&peer_ip)).await {
        Ok(qr) => Some((qr.n, qr.exp)),
        Err(e) => {
            tracing::warn!(error = %e.1, "minting a web bootstrap nonce");
            None
        }
    }
}

/// Puts the bootstrap nonce into the document Storm serves its web client from.
///
/// **This is the only place a web bootstrap nonce is issued.** A dedicated
/// endpoint would be a second front door; the entry point of the app is the one
/// moment where bootstrapping means anything.
fn inject_bootstrap(html: &str, nonce: &str, expires: &str) -> String {
    let tag = format!(
        r#"<meta name="storm-bootstrap" content="{}" data-expires="{}">"#,
        html_escape(nonce),
        html_escape(expires)
    );
    // Before `</head>` where there is one, and at the top otherwise — a
    // document we cannot find a head in still has to work.
    match html.find("</head>") {
        Some(i) => format!("{}{}{}", &html[..i], tag, &html[i..]),
        None => format!("{tag}{html}"),
    }
}

/// Minimal attribute escaping for the injected values.
///
/// The nonce is base64url and the expiry is RFC3339, so neither can contain
/// these today. Escaped anyway: "the input cannot contain a quote" is a
/// property of code somewhere else, and this is the line where that assumption
/// would become an injected attribute.
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Serves the SPA document, carrying a bootstrap nonce.
///
/// Paths that look like files go to `ServeDir`; everything else is a client
/// route and gets the document. That split is deliberate rather than clever: it
/// means the injected document is served for `/` and for every deep link,
/// without having to ask `ServeDir` after the fact whether what it returned was
/// the fallback index.
pub async fn serve_web_index(
    state: Shared,
    index: std::path::PathBuf,
    peer: std::net::SocketAddr,
    headers: HeaderMap,
) -> Response {
    let html = match tokio::fs::read_to_string(&index).await {
        Ok(h) => h,
        Err(e) => {
            tracing::error!(error = %e, path = %index.display(), "reading the web index");
            return (StatusCode::NOT_FOUND, "no web client installed").into_response();
        }
    };

    let body = match web_bootstrap_nonce(&state, peer.ip(), &headers).await {
        Some((nonce, expires)) => inject_bootstrap(&html, &nonce, &expires),
        None => html,
    };

    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            // **Never cached.** This document carries a single-use credential;
            // a cached copy is that credential with an unbounded lifetime,
            // sitting wherever the cache is.
            (header::CACHE_CONTROL, "no-store"),
        ],
        body,
    )
        .into_response()
}

/// The web document with no bootstrap nonce in it.
///
/// For the case where the peer address is unavailable, which means the nonce
/// could not be bound to anyone. An unbound web nonce is exactly what this
/// design refuses to mint, so the document is served plain and the client falls
/// back to the pairing screen — the pre-slice-15 behaviour, which still works.
pub async fn serve_web_index_without_bootstrap(index: std::path::PathBuf) -> Response {
    match tokio::fs::read_to_string(&index).await {
        Ok(html) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
            html,
        )
            .into_response(),
        Err(e) => {
            tracing::error!(error = %e, path = %index.display(), "reading the web index");
            (StatusCode::NOT_FOUND, "no web client installed").into_response()
        }
    }
}

/// POST /v1/pair — consume a pairing nonce and receive device credentials.
///
/// This is a `none`-tier endpoint: the nonce itself is the trust anchor. The
/// client has already verified the server's identity via the challenge step
/// before reaching here.
/// The peer is [`MaybePeer`] rather than a bare `ConnectInfo` because
/// `/v1/pair` is the *first* route a remote client touches, and over the relay
/// it arrives by in-process dispatch with no socket behind it. A required
/// extractor would reject before this handler ran, so the client would meet a
/// malformed-request error where a pairing answer belongs.
///
/// The refusal that falls out of `None` is the correct one, and it is worth
/// stating because it looks like a gap: a **web-bootstrap** nonce is
/// peer-bound at issuance, so it is refused over a relayed connection — it was
/// bound to a browser that reached the server directly. An **unbound QR**
/// nonce still works, which is the whole point of a QR. Neither behaviour is
/// new; `consume` has always taken `Option`.
///
/// Do **not** substitute a synthesized address here. A fabricated peer either
/// always fails the binding check or, worse, sometimes coincidentally passes.
async fn pair_handler(
    State(state): State<Shared>,
    MaybePeer(peer): MaybePeer,
    Json(body): Json<PairRequest>,
) -> ApiResult<Json<crate::auth::pairing::ConsumeResult>> {
    let now = crate::index::now_rfc3339();
    let peer_ip = peer.map(|p| p.ip().to_string());
    let mut auth_db = state.auth_db.lock().await;
    // The peer is passed for every purpose; only a nonce that recorded one
    // (web bootstrap) is checked against it. A QR nonce stays unbound, because
    // it is meant to be carried to a different machine.
    crate::auth::pairing::consume(
        &mut auth_db,
        &body.n,
        &body.name,
        body.platform.as_deref(),
        body.version.as_deref(),
        peer_ip.as_deref(),
        &now,
    )
    .map_err(|e| {
        let msg = e.to_string();
        if msg.contains("already used") {
            ApiError(StatusCode::CONFLICT, "pairing_consumed".into())
        } else if msg.contains("expired") {
            ApiError(StatusCode::GONE, "pairing_expired".into())
        } else if msg.contains("different client") {
            // Distinct from "invalid": the nonce was real, but it belongs to
            // another peer. Says so, because a bootstrap nonce replayed from
            // elsewhere is the thing the binding exists to catch.
            ApiError(StatusCode::FORBIDDEN, "pairing_wrong_peer".into())
        } else if msg.contains("too many") {
            ApiError(StatusCode::TOO_MANY_REQUESTS, "rate_limited".into())
        } else {
            ApiError(StatusCode::UNAUTHORIZED, msg)
        }
    })
    .map(Json)
}

/// POST /v1/pairings/password — pair a device found by LAN discovery with the
/// account password (decision 118). The logic, and every refusal, is
/// `ops::pair_with_password`; this maps them onto the wire. A wrong password
/// answers exactly as `/v1/auth/login` does.
async fn pair_password_handler(
    State(state): State<Shared>,
    MaybePeer(peer): MaybePeer,
    relayed: Option<Extension<crate::relay::Relayed>>,
    Json(body): Json<crate::ops::PasswordPairing>,
) -> Result<Json<crate::auth::pairing::ConsumeResult>, Response> {
    use crate::ops::PasswordPairError as E;
    let origin = crate::ops::Origin {
        peer: peer.map(|p| p.ip()),
        relayed: relayed.is_some(),
    };
    match crate::ops::pair_with_password(&state, body, origin).await {
        Ok(result) => Ok(Json(result)),
        Err(E::LanOnly) => Err(ApiError(StatusCode::FORBIDDEN, "lan_only".into()).into_response()),
        Err(E::SetupRequired) => {
            Err(ApiError(StatusCode::CONFLICT, "setup_required".into()).into_response())
        }
        Err(E::BadRequest(msg)) => Err(bad_request(msg).into_response()),
        Err(E::Refused(crate::auth::sessions::LoginFailure::RateLimited { retry_after_secs })) => {
            Err(rate_limited(retry_after_secs))
        }
        Err(E::Refused(failure)) => {
            Err(ApiError(StatusCode::UNAUTHORIZED, failure.code().to_string()).into_response())
        }
        Err(E::Internal(e)) => {
            Err(ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response())
        }
    }
}

#[derive(Deserialize)]
pub struct PairRequest {
    /// The pairing nonce from the QR.
    n: String,
    /// A human-readable device name, e.g. "Pixel 10".
    name: String,
    /// Platform, e.g. "android", "ios", "macos", "linux", "web".
    platform: Option<String>,
    /// Client version string.
    version: Option<String>,
}

/// POST /v1/pairings — issue a new pairing QR for another device.
///
/// Session tier: the caller must be logged in. Returns the QR payload as JSON
/// so the client can render it as a QR image.
async fn issue_pairing_handler(
    State(state): State<Shared>,
    Extension(auth): Extension<SessionAuth>,
    Json(body): Json<IssuePairingRequest>,
) -> ApiResult<Json<crate::ops::PairingQrPayload>> {
    let purpose = body.purpose.as_deref().unwrap_or("add_device");
    Ok(Json(
        crate::ops::issue_pairing_qr(&state, purpose, Some(&auth.authenticated.account.id), None)
            .await?,
    ))
}

#[derive(Deserialize)]
pub struct IssuePairingRequest {
    /// The pairing purpose: "first_user" or "add_device" (default).
    purpose: Option<String>,
}

// ---- Agent Runtime: hosts (decision 77b) ----------------------------------

fn peer_string(peer: Option<std::net::SocketAddr>) -> Option<String> {
    peer.map(|p| p.ip().to_string())
}

/// Charges the host limiter for an unauthenticated host call, by socket peer
/// (never a header). `Err` is the `429` to return.
fn charge_host_limiter(state: &Shared, peer: Option<std::net::SocketAddr>) -> Result<(), Response> {
    use crate::auth::ratelimit::CallerKey;
    let caller = peer
        .map(|c| CallerKey::Ip(c.ip()))
        .unwrap_or(CallerKey::Unattributed);
    state.host_limiter.check(&caller).map_err(|retry_after| {
        (
            StatusCode::TOO_MANY_REQUESTS,
            [(header::RETRY_AFTER, retry_after.to_string())],
            Json(serde_json::json!({ "error": "rate_limited" })),
        )
            .into_response()
    })
}

#[derive(Deserialize)]
struct RuntimeEnrollRequest {
    token: String,
    public_key: String,
    key_id: String,
    name: String,
    signature: String,
}

/// POST /v1/runtime/enroll — a host consumes its enrollment token.
async fn runtime_enroll(
    State(state): State<Shared>,
    MaybePeer(peer): MaybePeer,
    Json(body): Json<RuntimeEnrollRequest>,
) -> Result<Json<crate::ops::EnrolledHost>, Response> {
    charge_host_limiter(&state, peer)?;
    let remote = peer_string(peer);
    crate::ops::enroll_host(
        &state,
        crate::ops::EnrollHost {
            token: &body.token,
            public_key: &body.public_key,
            key_id: &body.key_id,
            name: &body.name,
            signature: &body.signature,
        },
        remote.as_deref(),
    )
    .await
    .map(Json)
    .map_err(IntoResponse::into_response)
}

#[derive(Deserialize)]
struct RuntimeChallengeRequest {
    host_id: String,
}

/// POST /v1/runtime/auth/challenge — a single-use nonce for one connect.
async fn runtime_challenge(
    State(state): State<Shared>,
    MaybePeer(peer): MaybePeer,
    Json(body): Json<RuntimeChallengeRequest>,
) -> Result<Json<crate::ops::HostChallenge>, Response> {
    charge_host_limiter(&state, peer)?;
    let remote = peer_string(peer);
    crate::ops::host_challenge(&state, &body.host_id, remote.as_deref())
        .await
        .map(Json)
        .map_err(IntoResponse::into_response)
}

#[derive(Deserialize)]
struct RuntimeAuthRequest {
    host_id: String,
    nonce: String,
    signature: String,
}

/// POST /v1/runtime/auth — prove the host key, get a host token.
async fn runtime_auth(
    State(state): State<Shared>,
    MaybePeer(peer): MaybePeer,
    Json(body): Json<RuntimeAuthRequest>,
) -> Result<Json<crate::auth::hosts::IssuedHostToken>, Response> {
    charge_host_limiter(&state, peer)?;
    let remote = peer_string(peer);
    let issued = crate::ops::host_authenticate(
        &state,
        &body.host_id,
        &body.nonce,
        &body.signature,
        remote.as_deref(),
    )
    .await
    .map_err(IntoResponse::into_response)?;
    // A host that proved its key gets its budget back, so a healthy host
    // reconnecting through a flaky network is never throttled out.
    if let Some(p) = peer {
        state
            .host_limiter
            .refund(&crate::auth::ratelimit::CallerKey::Ip(p.ip()));
    }
    Ok(Json(issued))
}

/// GET /v1/runtime/whoami — which host this token belongs to.
async fn runtime_whoami(Extension(auth): Extension<HostAuth>) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "host_id": auth.host.id, "name": auth.host.name }))
}

async fn agent_list_hosts(
    State(state): State<Shared>,
) -> ApiResult<Json<Vec<crate::ops::HostView>>> {
    Ok(Json(crate::ops::list_hosts(&state).await?))
}

#[derive(Deserialize)]
struct IssueEnrollmentRequest {
    /// How *this client* reaches the server. The server cannot know it — it
    /// may be behind NAT, a name, a port forward — and the host must dial it.
    server_url: String,
}

async fn agent_issue_enrollment(
    State(state): State<Shared>,
    Extension(actor): Extension<Actor>,
    Json(body): Json<IssueEnrollmentRequest>,
) -> ApiResult<Json<crate::ops::IssuedEnrollment>> {
    Ok(Json(
        crate::ops::issue_host_enrollment(&state, &actor, &body.server_url).await?,
    ))
}

#[derive(Deserialize)]
struct RenameHostRequest {
    name: String,
}

async fn agent_rename_host(
    State(state): State<Shared>,
    Extension(actor): Extension<Actor>,
    Path(id): Path<String>,
    Json(body): Json<RenameHostRequest>,
) -> ApiResult<Json<crate::ops::HostView>> {
    Ok(Json(
        crate::ops::rename_host(&state, &actor, &id, &body.name).await?,
    ))
}

async fn agent_revoke_host(
    State(state): State<Shared>,
    Extension(actor): Extension<Actor>,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    crate::ops::revoke_host(&state, &actor, &id).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---- MCP Gateway: integrations (decision 81c) — owner only, in ops ---------

async fn integrations_list(
    State(state): State<Shared>,
    Extension(actor): Extension<Actor>,
) -> ApiResult<Json<Vec<crate::ops::IntegrationView>>> {
    Ok(Json(crate::ops::list_integrations(&state, &actor).await?))
}

async fn integrations_create(
    State(state): State<Shared>,
    Extension(actor): Extension<Actor>,
    Json(body): Json<crate::ops::NewIntegration>,
) -> ApiResult<(StatusCode, Json<crate::ops::IntegrationView>)> {
    let view = crate::ops::create_integration(&state, &actor, body).await?;
    Ok((StatusCode::CREATED, Json(view)))
}

async fn integrations_suggestions(
    State(state): State<Shared>,
    Extension(actor): Extension<Actor>,
) -> ApiResult<Json<Vec<crate::ops::PluginSuggestion>>> {
    Ok(Json(crate::ops::plugin_suggestions(&state, &actor).await?))
}

async fn integrations_dismiss_suggestion(
    State(state): State<Shared>,
    Extension(actor): Extension<Actor>,
    Json(body): Json<crate::ops::DismissSuggestion>,
) -> ApiResult<StatusCode> {
    crate::ops::dismiss_suggestion(&state, &actor, body).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn integrations_get(
    State(state): State<Shared>,
    Extension(actor): Extension<Actor>,
    Path(id): Path<String>,
) -> ApiResult<Json<crate::ops::IntegrationView>> {
    Ok(Json(
        crate::ops::get_integration(&state, &actor, &id).await?,
    ))
}

async fn integrations_patch(
    State(state): State<Shared>,
    Extension(actor): Extension<Actor>,
    Path(id): Path<String>,
    Json(body): Json<crate::ops::IntegrationPatch>,
) -> ApiResult<Json<crate::ops::IntegrationView>> {
    Ok(Json(
        crate::ops::update_integration(&state, &actor, &id, body).await?,
    ))
}

async fn integrations_test(
    State(state): State<Shared>,
    Extension(actor): Extension<Actor>,
    Path(id): Path<String>,
) -> ApiResult<Json<crate::ops::IntegrationTest>> {
    Ok(Json(
        crate::ops::test_integration(&state, &actor, &id).await?,
    ))
}

async fn integrations_tools(
    State(state): State<Shared>,
    Extension(actor): Extension<Actor>,
    Path(id): Path<String>,
) -> ApiResult<Json<Vec<crate::ops::IntegrationTool>>> {
    Ok(Json(
        crate::ops::integration_tools(&state, &actor, &id).await?,
    ))
}

async fn integrations_authorize(
    State(state): State<Shared>,
    Extension(actor): Extension<Actor>,
    Path(id): Path<String>,
    Json(body): Json<crate::ops::AuthorizeIntegration>,
) -> ApiResult<Json<crate::ops::AuthorizationStarted>> {
    Ok(Json(
        crate::ops::authorize_integration(&state, &actor, &id, body).await?,
    ))
}

async fn integrations_callback(
    State(state): State<Shared>,
    Extension(actor): Extension<Actor>,
    Json(body): Json<crate::ops::OAuthCallback>,
) -> ApiResult<Json<crate::ops::IntegrationTest>> {
    Ok(Json(
        crate::ops::oauth_callback(&state, &actor, body).await?,
    ))
}

async fn integrations_delete(
    State(state): State<Shared>,
    Extension(actor): Extension<Actor>,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    crate::ops::delete_integration(&state, &actor, &id).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---- Token sync (decision 124n) — owner only, in ops ------------------------

type ProviderCredentialResult =
    Result<Json<crate::gateway::provider::ProviderCredentialView>, crate::gateway::provider::Code>;

async fn provider_credential_get(State(state): State<Shared>) -> ProviderCredentialResult {
    Ok(Json(crate::ops::provider_credential_status(&state).await?))
}

/// The body is read as bytes and parsed into a redacting `Secret` by hand, so
/// no extractor rejection can quote it.
async fn provider_credential_put(
    State(state): State<Shared>,
    Extension(actor): Extension<Actor>,
    body: axum::body::Bytes,
) -> ProviderCredentialResult {
    let body = crate::gateway::provider::PutBody::parse(&body)?;
    Ok(Json(
        crate::ops::put_provider_credential(&state, &actor, body.token, body.minted_at).await?,
    ))
}

async fn provider_credential_delete(
    State(state): State<Shared>,
    Extension(actor): Extension<Actor>,
) -> Result<StatusCode, crate::gateway::provider::Code> {
    crate::ops::delete_provider_credential(&state, &actor).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn provider_credential_hosts(
    State(state): State<Shared>,
    Extension(actor): Extension<Actor>,
    body: axum::body::Bytes,
) -> ProviderCredentialResult {
    let body: crate::gateway::provider::HostsBody = serde_json::from_slice(&body)
        .map_err(|_| crate::gateway::provider::Code::bad_request("bad_request_body"))?;
    Ok(Json(
        crate::ops::set_provider_credential_hosts(&state, &actor, body.hosts).await?,
    ))
}

// ---- Agent Runtime: the host link and the agent routes (decision 77c) ------

/// Marks the host offline when its link stream ends — dropped by the
/// connection closing, or by a newer link or a revocation replacing it. The
/// generation is what keeps an old stream's end from taking a newer link down.
struct LinkGuard {
    agent: Arc<crate::agent::AgentManager>,
    host_id: String,
    generation: u64,
}

impl Drop for LinkGuard {
    fn drop(&mut self) {
        self.agent.disconnect_host(&self.host_id, self.generation);
    }
}

/// GET /v1/runtime/link — the server→host command stream (freeze §11.5).
///
/// SSE rather than a WebSocket, so it crosses the relay unchanged (SRP §3).
/// Keepalives every 15 s are also how a dead connection is noticed: the write
/// fails and the guard runs.
async fn runtime_link(
    State(state): State<Shared>,
    Extension(auth): Extension<HostAuth>,
) -> axum::response::sse::Sse<
    impl futures_util::Stream<Item = Result<axum::response::sse::Event, std::convert::Infallible>>,
> {
    use axum::response::sse::{Event, KeepAlive, Sse};
    use futures_util::StreamExt;
    let (generation, rx) = state.agent.connect_host(&auth.host.id);
    let guard = LinkGuard {
        agent: state.agent.clone(),
        host_id: auth.host.id.clone(),
        generation,
    };
    let stream = tokio_stream::wrappers::UnboundedReceiverStream::new(rx).map(move |envelope| {
        let _held = &guard;
        Ok(Event::default().data(serde_json::to_string(&envelope).unwrap_or_default()))
    });
    Sse::new(stream).keep_alive(KeepAlive::new().interval(std::time::Duration::from_secs(15)))
}

/// POST /v1/runtime/heartbeat — the host is alive (B-1). Every host request
/// counts as one (the auth layer records it); this is the one a quiet host
/// sends. `409` says the server has no link for it: reconnect.
async fn runtime_heartbeat(
    State(state): State<Shared>,
    Extension(auth): Extension<HostAuth>,
) -> StatusCode {
    if state.agent.host_live(&auth.host.id).online {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::CONFLICT
    }
}

async fn runtime_hello(
    State(state): State<Shared>,
    Extension(auth): Extension<HostAuth>,
    Json(hello): Json<crate::agent::Hello>,
) -> ApiResult<StatusCode> {
    crate::ops::runtime_hello(&state, &auth.host.id, hello).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn runtime_inventory(
    State(state): State<Shared>,
    Extension(auth): Extension<HostAuth>,
    Json(caps): Json<crate::agent::Capabilities>,
) -> ApiResult<StatusCode> {
    crate::ops::runtime_inventory(&state, &auth.host.id, caps)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct OutputBatch {
    offset: u64,
    /// Standard base64, padded (RFC 4648 §4), both directions (77c).
    data: String,
}

async fn runtime_output(
    State(state): State<Shared>,
    Extension(auth): Extension<HostAuth>,
    Path(id): Path<String>,
    Json(batch): Json<OutputBatch>,
) -> ApiResult<StatusCode> {
    crate::ops::runtime_output(&state, &auth.host.id, &id, batch.offset, &batch.data)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn runtime_status(
    State(state): State<Shared>,
    Extension(auth): Extension<HostAuth>,
    Path(id): Path<String>,
    Json(report): Json<crate::agent::StatusReport>,
) -> ApiResult<StatusCode> {
    crate::ops::runtime_status(&state, &auth.host.id, &id, &report)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn runtime_identity_refused(
    State(state): State<Shared>,
    Extension(auth): Extension<HostAuth>,
    Path(id): Path<String>,
    Json(refusal): Json<crate::ops::IdentityRefusal>,
) -> ApiResult<StatusCode> {
    crate::ops::runtime_identity_refused(&state, &auth.host.id, &id, &refusal).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// One JSON-RPC message from a session's bridge; the answer streams back as
/// newline-delimited JSON lines (`{"message": …}` or `{"storm_error": …}`).
async fn runtime_mcp(
    State(state): State<Shared>,
    Extension(auth): Extension<HostAuth>,
    Path((id, connection)): Path<(String, String)>,
    Json(message): Json<serde_json::Value>,
) -> ApiResult<axum::response::Response> {
    use futures_util::StreamExt;
    let rx = crate::ops::integration_call(&state, &auth.host.id, &id, &connection, message).await;
    let lines = tokio_stream::wrappers::ReceiverStream::new(rx).map(|line| {
        let mut bytes = serde_json::to_vec(&line).unwrap_or_default();
        bytes.push(b'\n');
        Ok::<_, std::convert::Infallible>(bytes)
    });
    axum::response::Response::builder()
        .header(axum::http::header::CONTENT_TYPE, "application/x-ndjson")
        .header(axum::http::header::CACHE_CONTROL, "no-store")
        .body(axum::body::Body::from_stream(lines))
        .map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

async fn agent_host_workspaces(
    State(state): State<Shared>,
    Path(id): Path<String>,
) -> ApiResult<Json<Vec<crate::ops::WorkspaceView>>> {
    Ok(Json(crate::ops::host_workspaces(&state, &id).await?))
}

#[derive(Deserialize)]
struct AgentJobRequest {
    op: String,
}

async fn agent_start_job(
    State(state): State<Shared>,
    Extension(actor): Extension<Actor>,
    Path((id, agent)): Path<(String, String)>,
    Json(body): Json<AgentJobRequest>,
) -> ApiResult<(StatusCode, Json<crate::agent::jobs::AgentJob>)> {
    let job = crate::ops::start_agent_job(&state, &actor, &id, &agent, &body.op).await?;
    Ok((StatusCode::ACCEPTED, Json(job)))
}

async fn agent_get_job(
    State(state): State<Shared>,
    Path((id, job)): Path<(String, String)>,
) -> ApiResult<Json<crate::agent::jobs::AgentJob>> {
    Ok(Json(crate::ops::get_agent_job(&state, &id, &job).await?))
}

#[derive(Deserialize)]
struct AutoJobRequest {
    agent: String,
    op: String,
}

async fn runtime_start_agent_job(
    State(state): State<Shared>,
    Extension(auth): Extension<HostAuth>,
    Json(body): Json<AutoJobRequest>,
) -> ApiResult<(StatusCode, Json<serde_json::Value>)> {
    let job =
        crate::ops::runtime_start_agent_job(&state, &auth.host.id, &body.agent, &body.op).await?;
    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({ "job": job.id })),
    ))
}

async fn runtime_agent_job(
    State(state): State<Shared>,
    Extension(auth): Extension<HostAuth>,
    Path(job): Path<String>,
    Json(body): Json<crate::agent::jobs::ReportBody>,
) -> ApiResult<StatusCode> {
    crate::ops::runtime_agent_job(&state, &auth.host.id, &job, body).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn agent_launch(
    State(state): State<Shared>,
    Extension(actor): Extension<Actor>,
    Json(req): Json<crate::agent::Launch>,
) -> ApiResult<Json<crate::ops::LaunchedSession>> {
    Ok(Json(crate::ops::launch_session(&state, &actor, req).await?))
}

async fn agent_list_sessions(
    State(state): State<Shared>,
) -> ApiResult<Json<Vec<crate::ops::SessionView>>> {
    Ok(Json(crate::ops::list_sessions(&state).await?))
}

async fn agent_get_session(
    State(state): State<Shared>,
    Path(id): Path<String>,
) -> ApiResult<Json<crate::ops::SessionView>> {
    Ok(Json(crate::ops::get_session(&state, &id).await?))
}

async fn agent_session_writes(
    State(state): State<Shared>,
    Extension(actor): Extension<Actor>,
    Path(id): Path<String>,
) -> ApiResult<Json<Vec<crate::ops::SessionWrite>>> {
    Ok(Json(crate::ops::session_writes(&state, &actor, &id).await?))
}

async fn vault_agent_writes(
    State(state): State<Shared>,
    Extension(actor): Extension<Actor>,
    Path(vault): Path<String>,
) -> ApiResult<Json<std::collections::BTreeMap<String, crate::ops::LatestAgentWrite>>> {
    Ok(Json(
        crate::ops::vault_agent_writes(&state, &actor, &vault).await?,
    ))
}

async fn agent_end_session(
    State(state): State<Shared>,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    crate::ops::end_session(&state, &id).await?;
    Ok(StatusCode::ACCEPTED)
}

/// DELETE a session: 200 with what else it removed (decision 138c). Older
/// clients ignore the body.
async fn agent_dismiss_session(
    State(state): State<Shared>,
    Path(id): Path<String>,
) -> ApiResult<Json<crate::ops::Dismissed>> {
    Ok(Json(crate::ops::dismiss_session(&state, &id).await?))
}

async fn crews_list(
    State(state): State<Shared>,
    Extension(actor): Extension<Actor>,
) -> ApiResult<Json<Vec<crate::agent::Crew>>> {
    Ok(Json(crate::ops::list_crews(&state, &actor).await?))
}

async fn crews_create(
    State(state): State<Shared>,
    Extension(actor): Extension<Actor>,
    Json(req): Json<crate::ops::CreateCrew>,
) -> ApiResult<(StatusCode, Json<crate::ops::CrewChange>)> {
    let change = crate::ops::create_crew(&state, &actor, req).await?;
    Ok((StatusCode::CREATED, Json(change)))
}

async fn crews_update(
    State(state): State<Shared>,
    Extension(actor): Extension<Actor>,
    Path(id): Path<String>,
    Json(req): Json<crate::ops::UpdateCrew>,
) -> ApiResult<Json<crate::agent::Crew>> {
    Ok(Json(
        crate::ops::update_crew(&state, &actor, &id, req).await?,
    ))
}

async fn crews_reorder(
    State(state): State<Shared>,
    Extension(actor): Extension<Actor>,
    Json(req): Json<crate::ops::Order>,
) -> ApiResult<StatusCode> {
    crate::ops::reorder_crews(&state, &actor, req).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn crews_reorder_members(
    State(state): State<Shared>,
    Extension(actor): Extension<Actor>,
    Path(id): Path<String>,
    Json(req): Json<crate::ops::Order>,
) -> ApiResult<StatusCode> {
    crate::ops::reorder_crew_members(&state, &actor, &id, req).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn crews_delete(
    State(state): State<Shared>,
    Extension(actor): Extension<Actor>,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    crate::ops::delete_crew(&state, &actor, &id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn agent_session_spawn(
    State(state): State<Shared>,
    Extension(actor): Extension<Actor>,
    auth: Option<Extension<SessionAuth>>,
    Path(id): Path<String>,
    Json(req): Json<crate::ops::SetSpawn>,
) -> ApiResult<Json<crate::ops::SpawnSwitch>> {
    let device = auth.as_ref().map(|a| a.authenticated.device.id.as_str());
    Ok(Json(
        crate::ops::set_session_spawn(&state, &actor, device, &id, req).await?,
    ))
}

async fn agent_request_approve(
    State(state): State<Shared>,
    Extension(actor): Extension<Actor>,
    Extension(auth): Extension<SessionAuth>,
    Path(id): Path<String>,
    Json(req): Json<crate::ops::ApproveRequest>,
) -> ApiResult<Json<crate::ops::RequestView>> {
    Ok(Json(
        crate::ops::approve_spawn_request(&state, &actor, &auth.authenticated.device.id, &id, req)
            .await?,
    ))
}

async fn agent_request_deny(
    State(state): State<Shared>,
    Extension(actor): Extension<Actor>,
    Extension(auth): Extension<SessionAuth>,
    Path(id): Path<String>,
) -> ApiResult<Json<crate::ops::RequestView>> {
    Ok(Json(
        crate::ops::deny_spawn_request(&state, &actor, &auth.authenticated.device.id, &id).await?,
    ))
}

async fn agent_session_crew(
    State(state): State<Shared>,
    Extension(actor): Extension<Actor>,
    Path(id): Path<String>,
    Json(req): Json<crate::ops::MoveToCrew>,
) -> ApiResult<Json<crate::ops::CrewChange>> {
    Ok(Json(
        crate::ops::move_session_to_crew(&state, &actor, &id, req).await?,
    ))
}

async fn agent_session_role(
    State(state): State<Shared>,
    Extension(actor): Extension<Actor>,
    Path(id): Path<String>,
    Json(req): Json<crate::ops::RoleRef>,
) -> ApiResult<StatusCode> {
    crate::ops::set_session_role(&state, &actor, &id, Some(req)).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn agent_clear_session_role(
    State(state): State<Shared>,
    Extension(actor): Extension<Actor>,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    crate::ops::set_session_role(&state, &actor, &id, None).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn agent_roles(
    State(state): State<Shared>,
    Extension(actor): Extension<Actor>,
) -> ApiResult<Json<Vec<crate::ops::RoleNote>>> {
    Ok(Json(crate::ops::list_roles(&state, &actor).await?))
}

/// POST …/terminal/input — raw bytes, at most 64 KiB, at most once.
async fn agent_input(
    State(state): State<Shared>,
    Path(id): Path<String>,
    body: axum::body::Bytes,
) -> ApiResult<StatusCode> {
    crate::ops::session_input(&state, &id, &body).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// A pasted image: staged on the session's host, answered with its path
/// (D15 AM46).
async fn agent_image(
    State(state): State<Shared>,
    Path(id): Path<String>,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> ApiResult<Json<crate::agent::StagedImage>> {
    let content_type = headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok());
    let staged = crate::ops::stage_image(&state, &id, content_type, body.to_vec()).await?;
    Ok(Json(staged))
}

async fn runtime_image(
    State(state): State<Shared>,
    Extension(auth): Extension<HostAuth>,
    Path((id, image)): Path<(String, String)>,
) -> ApiResult<axum::response::Response> {
    let bytes = crate::ops::runtime_image(&state, &auth.host.id, &id, &image)?;
    axum::response::Response::builder()
        .header(axum::http::header::CONTENT_TYPE, "application/octet-stream")
        .header(axum::http::header::CACHE_CONTROL, "no-store")
        .body(axum::body::Body::from(bytes.as_ref().clone()))
        .map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

async fn runtime_provider_credential(
    State(state): State<Shared>,
    Extension(auth): Extension<HostAuth>,
    Path(id): Path<String>,
) -> ApiResult<Response> {
    let pulled = crate::ops::runtime_provider_credential(&state, &auth.host, &id).await?;
    Ok((
        [(axum::http::header::CACHE_CONTROL, "no-store")],
        Json(pulled),
    )
        .into_response())
}

#[derive(Deserialize)]
struct StagedReport {
    #[serde(default)]
    staged: Option<String>,
    #[serde(default)]
    failed: Option<String>,
}

async fn runtime_image_staged(
    State(state): State<Shared>,
    Extension(auth): Extension<HostAuth>,
    Path((id, image)): Path<(String, String)>,
    Json(report): Json<StagedReport>,
) -> ApiResult<StatusCode> {
    let outcome = match report {
        StagedReport {
            staged: Some(path), ..
        } => Ok(path),
        StagedReport { failed, .. } => Err(failed.unwrap_or_else(|| "no reason".into())),
    };
    crate::ops::runtime_image_staged(&state, &auth.host.id, &id, &image, outcome)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct ResizeRequest {
    cols: u16,
    rows: u16,
    // Clients also send `focus` (freeze §11.4). Every resize is applied, so
    // the PTY follows the most recently active client without reading it.
}

async fn agent_resize(
    State(state): State<Shared>,
    Path(id): Path<String>,
    Json(req): Json<ResizeRequest>,
) -> ApiResult<StatusCode> {
    crate::ops::session_resize(
        &state,
        &id,
        crate::agent::TerminalSize {
            cols: req.cols,
            rows: req.rows,
        },
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct StreamQuery {
    #[serde(default)]
    offset: u64,
}

struct TerminalStream {
    agent: Arc<crate::agent::AgentManager>,
    id: String,
    cursor: u64,
    last_status: Option<String>,
    rx: tokio::sync::watch::Receiver<u64>,
    done: bool,
    live_sent: bool,
}

/// GET …/terminal/stream?offset=N — the terminal as SSE (freeze §11.2).
///
/// Every stream starts with a `status` event. Output is `event: output` with
/// `id:` the end offset and base64 `data:`; a range no longer retained is an
/// explicit `event: gap`. Resume is by offset, never `Last-Event-ID`, so it
/// is exact and crosses the relay (SRP §5.3). The stream ends once the session
/// has ended and everything retained has been sent.
///
/// `event: live` (data: the offset) marks the first time a stream has sent
/// everything retained: what came before it is replay, and a client's
/// terminal must not answer the queries in it again (decision 132, fix b).
/// Older clients ignore it.
async fn agent_terminal_stream(
    State(state): State<Shared>,
    Path(id): Path<String>,
    Query(q): Query<StreamQuery>,
) -> ApiResult<
    axum::response::sse::Sse<
        impl futures_util::Stream<Item = Result<axum::response::sse::Event, std::convert::Infallible>>,
    >,
> {
    use axum::response::sse::{Event, KeepAlive, Sse};
    crate::ops::get_session(&state, &id).await?;
    let start = TerminalStream {
        agent: state.agent.clone(),
        rx: state.agent.watch(&id),
        id,
        cursor: q.offset,
        last_status: None,
        done: false,
        live_sent: false,
    };
    let stream = futures_util::stream::unfold(start, |mut st| async move {
        loop {
            if st.done {
                return None;
            }
            // Mark the current version seen *before* reading, so a change
            // landing between the read and the wait still wakes us.
            st.rx.borrow_and_update();
            let record = match st.agent.get(&st.id) {
                Ok(r) => r,
                Err(_) => return None,
            };
            let json = serde_json::to_string(&record).unwrap_or_default();
            if st.last_status.as_ref() != Some(&json) {
                st.last_status = Some(json.clone());
                return Some((Ok(Event::default().event("status").data(json)), st));
            }
            match st.agent.read_output(&st.id, st.cursor, 32 * 1024) {
                crate::agent::cache::Read::Data { from, bytes } => {
                    st.cursor = from + bytes.len() as u64;
                    let event = Event::default()
                        .event("output")
                        .id(st.cursor.to_string())
                        .data(data_encoding::BASE64.encode(&bytes));
                    return Some((Ok(event), st));
                }
                crate::agent::cache::Read::Gap { from, to } => {
                    st.cursor = to;
                    let event = Event::default()
                        .event("gap")
                        .data(serde_json::json!({ "from": from, "to": to }).to_string());
                    return Some((Ok(event), st));
                }
                crate::agent::cache::Read::UpToDate | crate::agent::cache::Read::Ahead => {
                    if !st.live_sent && !record.is_ended() {
                        st.live_sent = true;
                        let event = Event::default().event("live").data(st.cursor.to_string());
                        return Some((Ok(event), st));
                    }
                    if record.is_ended() {
                        return None;
                    }
                    match tokio::time::timeout(std::time::Duration::from_secs(15), st.rx.changed())
                        .await
                    {
                        Ok(Ok(())) => continue,
                        Ok(Err(_)) => return None,
                        Err(_) => return Some((Ok(Event::default().comment("keepalive")), st)),
                    }
                }
            }
        }
    });
    Ok(Sse::new(stream).keep_alive(KeepAlive::new().interval(std::time::Duration::from_secs(15))))
}

async fn agent_get_config(
    State(state): State<Shared>,
) -> ApiResult<Json<crate::ops::AgentConfigView>> {
    Ok(Json(crate::ops::agent_config(&state).await?))
}

#[derive(Deserialize)]
struct AgentConfigRequest {
    default_provider: String,
}

async fn agent_put_config(
    State(state): State<Shared>,
    Extension(actor): Extension<Actor>,
    Json(req): Json<AgentConfigRequest>,
) -> ApiResult<Json<crate::ops::AgentConfigView>> {
    Ok(Json(
        crate::ops::set_agent_config(&state, &actor, &req.default_provider).await?,
    ))
}

/// Three credential tiers: `none` (unauthenticated), `device` (paired
/// installation), `session` (logged-in user). Set per-route via axum
/// `Extension<RequiredTier>`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RequiredTier {
    None,
    Device,
    Session,
    /// The MCP surface (A14). Accepts a session token or an `stk_` MCP key.
    ///
    /// **A separate tier rather than a flag on `Session`**, because the set of
    /// credentials that may reach `/mcp` is genuinely different from the set
    /// that may reach the REST API: a key is accepted here and refused
    /// everywhere else (A14.2). Encoding that as "session tier, but also keys"
    /// would put the exception in the middleware instead of on the route,
    /// which is where it would be forgotten.
    Mcp,
    /// `/v1/runtime/*` (decision 77b): a Runtime Host's `sht_` token, and
    /// nothing else. No user credential reaches here; no host token reaches
    /// anywhere else.
    Host,
}

/// A Runtime Host that proved its token. Set on `Host`-tier routes.
#[derive(Clone)]
pub struct HostAuth {
    pub host: crate::auth::hosts::Host,
}

/// A caller that has proven who it is (user + device + session).
///
/// Set as a request extension by `require_auth` on session-tier routes when the
/// credential is a `Bearer` token.
#[derive(Clone)]
pub struct SessionAuth {
    pub authenticated: crate::auth::sessions::Authenticated,
}

/// Tier-aware authentication middleware.
///
/// Reads `RequiredTier` from the request extensions (set by the router layer)
/// and checks the appropriate credential:
///
/// - **none**: always passes.
/// - **device**: `Authorization: StormDevice <id>:<secret>` → verify secret →
///   reject revoked. Sets `DeviceAuth` extension.
/// - **session**: `Authorization: Bearer <token>` → `sessions::authenticate()` →
///   reject expired/revoked/disabled. Sets `SessionAuth` extension.
///
/// - **mcp**: `Bearer stk_…` (A14), plus everything the session tier takes.
///
/// **There is no shared-token path.** It was removed in the cutover: the only
/// ways in are a paired device, a session, and an MCP key. A server nobody has
/// paired with has no network route to authentication at all — bootstrapping
/// is the console pairing nonce or `storm-server user add`, both of which need
/// shell access, which is the intended bar (A8).
async fn require_auth(
    State(state): State<Shared>,
    headers: HeaderMap,
    mut request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let tier = request
        .extensions()
        .get::<RequiredTier>()
        .copied()
        .unwrap_or(RequiredTier::Session);

    if tier == RequiredTier::None {
        return next.run(request).await;
    }

    let presented = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
        .or_else(|| {
            // Browsers can't set headers on a WebSocket handshake, and image
            // widgets can't set them on a GET, so the credential may also
            // arrive as a query parameter. It is the *same* string the header
            // would carry, scheme included.
            //
            // **Percent-decoded, which it was not.** Every credential Storm
            // has left contains a space (`Bearer …`, `StormDevice …`), so a
            // conforming client sends `token=Bearer%20…` — and comparing that
            // raw against `"Bearer "` matches nothing. This path therefore
            // could not authenticate anyone at all once the shared token was
            // removed: that value was compared whole and had no space in it,
            // so it was the only credential the raw form ever fit. Nothing
            // failed loudly; the WebSocket change feed just stopped
            // authenticating and fell back to reconnect-driven polling.
            query_param(request.uri(), "token")
        });

    // --- `?ticket=` — a WebSocket handshake's own credential ---
    //
    // Its own parameter rather than another shape inside `?token=`, because a
    // ticket names no scheme and guessing at an unschemed value is precisely
    // the ambiguity decision 57 was about. Distinct name, unambiguous parse.
    //
    // This exists so a **thirty-day session token does not travel in a URL**,
    // where it lands in proxy logs, browser history and referrers. A ticket
    // lives sixty seconds and dies on first use, so the exposure is bounded
    // to a window in which it has already been spent.
    if let Some(raw) = query_param(request.uri(), "ticket") {
        let now = crate::index::now_rfc3339();
        let mut auth_db = state.auth_db.lock().await;
        return match crate::auth::sessions::consume_ws_ticket(&mut auth_db, &raw, &now) {
            Ok(authenticated) => {
                drop(auth_db);
                if tier != RequiredTier::Session {
                    // A ticket buys the session tier and nothing else. It is
                    // minted *from* a session, so it can carry no more.
                    return tier_error("ticket_not_accepted_here", StatusCode::UNAUTHORIZED);
                }
                request.extensions_mut().insert(Actor::Session {
                    user_id: authenticated.account.id.clone(),
                });
                request
                    .extensions_mut()
                    .insert(SessionAuth { authenticated });
                next.run(request).await
            }
            // Answered generically on purpose: unknown, expired, already used
            // and revoked-session are one message, so a caller holding a
            // captured ticket learns nothing about why it failed.
            Err(crate::auth::sessions::SessionError::Refused(_)) => {
                unauthorized("invalid or missing token")
            }
            Err(crate::auth::sessions::SessionError::Internal(e)) => {
                tracing::error!(error = %e, "ws ticket authentication failed");
                internal_error()
            }
        };
    }

    let Some(credential) = presented else {
        return unauthorized("invalid or missing token");
    };

    // --- StormDevice: `StormDevice <id>:<secret>` ---
    if let Some(rest) = credential.strip_prefix("StormDevice ") {
        // **A device credential is not a session, and the tier check has to
        // say so here.** This branch used to run whatever tier the route
        // asked for, so a `StormDevice` header satisfied `require_auth` on a
        // *session* route and the request reached the handler. Nothing was
        // stolen — the handler then failed extracting a `SessionAuth` that had
        // never been inserted — but the boundary was being enforced by a
        // missing extension rather than by the check that exists for it, and
        // the caller saw `500` where `401` is the truth.
        //
        // Found by a device credential trying to flip a session-tier setting
        // and getting an extension panic instead of a refusal.
        //
        // `Mcp` is in this check for the same reason `Session` is: a device
        // credential is not an MCP credential either, and A14 added a tier
        // rather than widening what the device branch satisfies.
        if matches!(tier, RequiredTier::Session | RequiredTier::Mcp) {
            return tier_error("session_required", StatusCode::UNAUTHORIZED);
        }
        if tier == RequiredTier::Host {
            return tier_error("host_token_required", StatusCode::UNAUTHORIZED);
        }
        // **The lock is released before the handler runs, and that is the whole
        // point of this block's shape.** `auth_db` is a `tokio::sync::Mutex`,
        // which is not reentrant, and every device-tier handler — login,
        // refresh, `users`, `users/first` — takes it. Holding the guard across
        // `next.run(request)` deadlocked the request forever, and because the
        // hung task never released the mutex it took *all* later authentication
        // with it: one login attempt wedged the server until restart.
        //
        // The session branch below has always been written this way (it calls
        // `drop(auth_db)` explicitly). This branch was not, and no test noticed
        // because nothing exercised a device-tier route at all.
        let device = {
            let auth_db = state.auth_db.lock().await;
            match parse_device_credential(rest) {
                Ok((id, secret)) => match auth_db.verify_device_secret(id, secret) {
                    Ok(Some(device)) => device,
                    Ok(None) => {
                        return unauthorized("invalid or missing token");
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "device verification failed");
                        return internal_error();
                    }
                },
                Err(_) => {
                    return unauthorized("invalid or missing token");
                }
            }
        };

        if device.is_revoked() {
            return tier_error("device_revoked", StatusCode::UNAUTHORIZED);
        }
        return next.run(request).await;
    }

    // --- Bearer token: MCP key or session ---
    if let Some(token) = credential.strip_prefix("Bearer ") {
        // **Host tokens, by prefix, and the tier check is both ways round**
        // (decision 77b): a host token on any other tier is refused, and any
        // other bearer on the host tier is refused, before anything is looked
        // up. Either refusal arriving later, as a missing extension, is the
        // `500`-instead-of-`401` bug the device branch once had.
        let is_host_token = token.starts_with(crate::auth::hosts::HOST_TOKEN_PREFIX);
        if is_host_token != (tier == RequiredTier::Host) {
            return tier_error(
                if is_host_token {
                    "host_token_not_accepted_here"
                } else {
                    "host_token_required"
                },
                StatusCode::UNAUTHORIZED,
            );
        }
        if is_host_token {
            // Scoped so the guard drops before the handler, which takes it.
            let host = {
                let mut auth_db = state.auth_db.lock().await;
                let now = crate::index::now_rfc3339();
                match crate::auth::hosts::authenticate_token(&mut auth_db, token, &now) {
                    Ok(host) => host,
                    Err(crate::auth::hosts::HostError::Refused(failure)) => {
                        let _ = auth_db.record_event(
                            crate::auth::hosts::EVENT_HOST_AUTH_REJECTED,
                            None,
                            None,
                            &now,
                            &format!(r#"{{"reason":"{}","via":"token"}}"#, failure.code()),
                        );
                        return unauthorized("invalid or missing token");
                    }
                    Err(crate::auth::hosts::HostError::Internal(e)) => {
                        tracing::error!(error = %e, "host token authentication failed");
                        return internal_error();
                    }
                }
            };
            state.agent.touch(&host.id);
            request.extensions_mut().insert(HostAuth { host });
            return next.run(request).await;
        }

        // **MCP keys, checked first and by prefix** (A14.1). Keys share the
        // `Bearer` scheme because most MCP clients can send nothing else, so
        // the `stk_` prefix is the only thing separating them from session
        // tokens, and it is checked first so a key is never mistaken for one.
        //
        // **A key is accepted on `/mcp` and refused everywhere else** (A14.2),
        // and the refusal happens *here*, in the tier check, rather than
        // downstream in a handler that fails to find an extension. That
        // distinction is not theoretical: it is exactly the `500`-instead-of-
        // `401` bug the device branch above had.
        if token.starts_with(crate::auth::token::KEY_PREFIX) {
            if tier != RequiredTier::Mcp {
                return tier_error("mcp_key_not_accepted_here", StatusCode::UNAUTHORIZED);
            }

            // Scoped so the guard is dropped before `next.run(request)`.
            // `auth_db` is a non-reentrant `tokio::sync::Mutex` and MCP tools
            // take it; holding it across the handler is what wedged the device
            // tier in slice 12 and took every later request down with it.
            let authed = {
                let mut auth_db = state.auth_db.lock().await;
                let now = crate::index::now_rfc3339();
                match crate::auth::keys::authenticate(&mut auth_db, token, &now) {
                    Ok(authed) => authed,
                    Err(crate::auth::keys::KeyError::Refused(failure)) => {
                        // Audited specifically, answered generically: the row
                        // says whether it was unknown, revoked or expired; the
                        // client is told none of that, because distinguishing
                        // "never existed" from "was revoked" is free
                        // reconnaissance for an unauthenticated caller.
                        let _ = auth_db.record_event(
                            crate::auth::keys::EVENT_KEY_REJECTED,
                            None,
                            None,
                            &now,
                            &format!(r#"{{"reason":"{}"}}"#, failure.code()),
                        );
                        return unauthorized("invalid or missing token");
                    }
                    Err(crate::auth::keys::KeyError::Internal(e)) => {
                        tracing::error!(error = %e, "mcp key authentication failed");
                        return internal_error();
                    }
                }
            };

            request.extensions_mut().insert(Actor::Key {
                key_id: authed.key.id.clone(),
                user_id: authed.account.id.clone(),
            });
            return next.run(request).await;
        }

        // Normal session authentication.
        let now = crate::index::now_rfc3339();
        let mut auth_db = state.auth_db.lock().await;
        match crate::auth::sessions::authenticate(&mut auth_db, token, &now) {
            Ok(authenticated) => {
                drop(auth_db);
                request.extensions_mut().insert(Actor::Session {
                    user_id: authenticated.account.id.clone(),
                });
                request
                    .extensions_mut()
                    .insert(SessionAuth { authenticated });
                return next.run(request).await;
            }
            Err(crate::auth::sessions::SessionError::Refused(failure)) => {
                return tier_error(failure.code(), StatusCode::UNAUTHORIZED);
            }
            Err(crate::auth::sessions::SessionError::Internal(e)) => {
                tracing::error!(error = %e, "session authentication failed");
                return internal_error();
            }
        }
    }

    unauthorized("invalid or missing token")
}

/// One percent-decoded query parameter, or `None`.
///
/// Both credential paths go through this so they cannot drift on decoding —
/// the last time only one of them decoded, query authentication was dead for
/// six days (decision 57).
fn query_param(uri: &axum::http::Uri, key: &str) -> Option<String> {
    let prefix = format!("{key}=");
    uri.query()?
        .split('&')
        .find_map(|kv| kv.strip_prefix(&prefix))
        .map(percent_decode)
}

/// Percent-decodes a query-string value, with `+` meaning a space.
///
/// Hand-rolled rather than pulling a crate for it: this decodes exactly one
/// short credential on one code path, and the alternative — routing the
/// request through `Query` — would mean parsing and allocating the whole query
/// map on every authenticated request to save nine lines.
///
/// Invalid escapes are passed through as written rather than rejected. A
/// malformed credential fails the scheme match a moment later anyway, and
/// there is nothing to gain by distinguishing "not a credential" from "not
/// even valid encoding" for an unauthenticated caller.
fn percent_decode(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => match u8::from_str_radix(&raw[i + 1..i + 3], 16) {
                Ok(byte) => {
                    out.push(byte);
                    i += 3;
                }
                Err(_) => {
                    out.push(b'%');
                    i += 1;
                }
            },
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            byte => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Parses `StormDevice <id>:<secret>` → `(id, secret)`.
fn parse_device_credential(rest: &str) -> Result<(&str, &str), ()> {
    let (id, secret) = rest.split_once(':').ok_or(())?;
    if id.is_empty() || secret.is_empty() {
        return Err(());
    }
    Ok((id, secret))
}

fn unauthorized(msg: &str) -> Response {
    ApiError(StatusCode::UNAUTHORIZED, msg.to_string()).into_response()
}

fn tier_error(code: &str, status: StatusCode) -> Response {
    (status, Json(serde_json::json!({ "error": code }))).into_response()
}

fn internal_error() -> Response {
    ApiError(
        StatusCode::INTERNAL_SERVER_ERROR,
        "internal error".to_string(),
    )
    .into_response()
}

/// Refuses `/mcp` while MCP is switched off.
///
/// A 404 rather than a 403: to a client that has not been told otherwise, a
/// disabled endpoint and an absent one are the same thing, and the message says
/// which this is. It runs *after* `require_token`, so an unauthenticated caller
/// gets 401 first and this never confirms the endpoint exists to a stranger.
async fn require_mcp_enabled(
    State(state): State<Shared>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    if state.mcp_enabled.load(std::sync::atomic::Ordering::Relaxed) {
        return next.run(request).await;
    }
    ApiError(
        StatusCode::NOT_FOUND,
        "MCP is switched off on this server. Turn it on in Storm's server settings.".into(),
    )
    .into_response()
}

// ---- handlers ----------------------------------------------------------

async fn health() -> impl IntoResponse {
    Json(serde_json::json!({ "status": "ok", "service": "storm-server" }))
}

// ---- vaults ------------------------------------------------------------

async fn list_vaults(
    State(state): State<Shared>,
    Extension(actor): Extension<Actor>,
) -> ApiResult<Json<serde_json::Value>> {
    let vaults = crate::ops::list_vaults(&state, &actor).await?;
    Ok(Json(serde_json::json!({ "vaults": vaults })))
}

#[derive(Deserialize)]
struct VaultNameBody {
    name: String,
}

async fn create_vault(
    State(state): State<Shared>,
    Json(body): Json<VaultNameBody>,
) -> ApiResult<Json<crate::ops::CreatedVault>> {
    Ok(Json(crate::ops::create_vault(&state, &body.name).await?))
}

async fn rename_vault(
    State(state): State<Shared>,
    Path(vault): Path<String>,
    Json(body): Json<VaultNameBody>,
) -> ApiResult<Json<crate::ops::RenamedVault>> {
    Ok(Json(
        crate::ops::rename_vault(&state, &vault, &body.name).await?,
    ))
}

async fn remove_vault(
    State(state): State<Shared>,
    Path(vault): Path<String>,
) -> ApiResult<Json<crate::ops::RemovedVault>> {
    Ok(Json(crate::ops::remove_vault(&state, &vault).await?))
}

// ---- server configuration ----------------------------------------------

#[derive(Serialize)]
struct ConfigResponse {
    vault_root: String,
    state_dir: String,
    vault_count: usize,
    /// Whether `/mcp` is answering. The app shows this and can change it.
    mcp_enabled: bool,
    /// Whether MCP may create, edit and delete notes.
    mcp_writable: bool,
    /// Whether an agent session may write to the vault chosen at its launch.
    /// Independent of the two MCP switches.
    agent_writes: bool,
    /// The **configured** relay URLs (SRP v1 §4.4) — not the registered set.
    /// `GET /v1/server` reports what is actually live; this is what an
    /// operator or the app set and expects to survive a restart.
    relays: Vec<String>,
    /// This server's release version, for the client's compatibility check.
    version: &'static str,
    /// LAN discovery (decision 118): the setting, whether a flag overrides it,
    /// and whether the server is advertising now.
    discovery: crate::ops::DiscoveryView,
}

async fn get_config(State(state): State<Shared>) -> ApiResult<Json<ConfigResponse>> {
    let vaults = state.vaults.read().await;
    Ok(Json(ConfigResponse {
        vault_root: vaults.registry.root.display().to_string(),
        state_dir: state.state_dir.display().to_string(),
        vault_count: vaults.registry.vaults.len(),
        mcp_enabled: vaults.registry.mcp_enabled,
        mcp_writable: vaults.registry.mcp_writable,
        agent_writes: vaults.registry.agent_writes(),
        relays: vaults.registry.relays.clone(),
        version: env!("CARGO_PKG_VERSION"),
        discovery: crate::ops::discovery_view(&state),
    }))
}

#[derive(Deserialize)]
struct McpBody {
    /// Absent leaves both MCP switches as they are, so a client can change
    /// `agent_writes` alone.
    #[serde(default)]
    enabled: Option<bool>,
    /// Absent means read-only. Callers that do not know about writes therefore
    /// cannot turn them on by omission.
    #[serde(default)]
    writable: bool,
    /// Absent leaves it as it is: an older client toggling MCP must not
    /// change what agents may do.
    #[serde(default)]
    agent_writes: Option<bool>,
}

/// Switches the MCP endpoint and agent writes on or off, now and across
/// restarts.
///
/// Its own route rather than a field on `PUT /v1/config`, because that one
/// re-points the storage root — a heavyweight operation with an orphan check
/// and a watcher respawn. Toggling a read-only endpoint should not have to send
/// a `vault_root` it does not want to change.
///
/// The atomics are set *after* the registry is saved: if the write fails the
/// endpoint keeps its old state, which is the honest outcome. The other order
/// would report success while the setting silently reverted on next boot.
async fn put_mcp(
    State(state): State<Shared>,
    Json(body): Json<McpBody>,
) -> ApiResult<Json<serde_json::Value>> {
    let (enabled, writable, agent_writes) = {
        let mut vaults = state.vaults.write().await;
        if let Some(enabled) = body.enabled {
            vaults.registry.mcp_enabled = enabled;
            // Writes cannot outlive the endpoint being on: leaving them armed
            // while MCP is off would mean switching MCP back on silently
            // restores write access someone thought they had revoked.
            vaults.registry.mcp_writable = enabled && body.writable;
        }
        if let Some(on) = body.agent_writes {
            vaults.registry.set_agent_writes(on);
        }
        vaults.registry.save(&state.state_dir)?;
        (
            vaults.registry.mcp_enabled,
            vaults.registry.mcp_writable,
            vaults.registry.agent_writes(),
        )
    };
    let ordering = std::sync::atomic::Ordering::Relaxed;
    state.mcp_enabled.store(enabled, ordering);
    state.mcp_writable.store(writable, ordering);
    state.agent_writes.store(agent_writes, ordering);

    tracing::info!(enabled, writable, agent_writes, "AI access changed");
    Ok(Json(serde_json::json!({
        "mcp_enabled": enabled,
        "mcp_writable": writable,
        "agent_writes": agent_writes,
    })))
}

#[derive(Deserialize)]
struct DiscoveryBody {
    enabled: bool,
}

/// PUT /v1/config/discovery — Settings › Server › "Discoverable on this
/// network" (decision 118).
async fn put_discovery(
    State(state): State<Shared>,
    Json(body): Json<DiscoveryBody>,
) -> ApiResult<Json<crate::ops::DiscoveryView>> {
    crate::ops::set_discoverable(&state, body.enabled)
        .await
        .map(Json)
}

#[derive(Deserialize)]
struct RelaysBody {
    relays: Vec<String>,
}

/// PUT /v1/config/relays — sets the **configured** relay list (SRP v1 §4.4).
///
/// Its own route for the same reason `/v1/config/mcp` has one: a settings
/// toggle should not have to send a `vault_root` it does not want to touch.
///
/// **Does not register anything itself.** It saves the list and hands it to
/// the tunnel client (`relay::manage`), which connects to added relays and
/// sends `DEREGISTER` to removed ones in the background, without a restart
/// (decision 74). Whether a relay is actually live is reported separately by
/// `GET /v1/server`: this response is a promise to *try*, not proof of a
/// connection, and it returns before any connection is attempted.
///
/// `set_relays` is all-or-nothing and returns `anyhow::Error` on a bad URL —
/// mapped to `400` explicitly here rather than via `ApiError`'s blanket
/// `From<anyhow::Error>`, which would answer `500` for what is really the
/// caller's mistake. The whole point of rejecting the entire list on one bad
/// URL is that the operator finds out, with a message that says which URL and
/// why, rather than losing a relay silently.
async fn put_relays(
    State(state): State<Shared>,
    Json(body): Json<RelaysBody>,
) -> ApiResult<Json<serde_json::Value>> {
    let mut vaults = state.vaults.write().await;
    vaults
        .registry
        .set_relays(&body.relays)
        .map_err(|e| bad_request(e.to_string()))?;
    vaults.registry.save(&state.state_dir)?;
    // After the save, so a list the tunnels act on is one a restart would also
    // read. `send_replace`: it must not fail for want of a receiver.
    state
        .relays_changed
        .send_replace(vaults.registry.relays.clone());

    Ok(Json(
        serde_json::json!({ "relays": vaults.registry.relays }),
    ))
}

#[derive(Deserialize)]
struct ConfigBody {
    vault_root: String,
    /// "I know the vaults listed as orphaned are being left behind."
    #[serde(default)]
    orphan_ok: bool,
}

/// Points the server at a different storage root.
///
/// **This never moves files.** It expects the vault directories to already be
/// where the new root says they are. The failure that guard exists for is
/// quiet: point the root at an empty directory and the server boots perfectly
/// healthy with zero vaults, files safe on disk and invisible to every client —
/// which reads as "my notes are gone".
async fn put_config(
    State(state): State<Shared>,
    Json(body): Json<ConfigBody>,
) -> ApiResult<Json<serde_json::Value>> {
    let candidate = PathBuf::from(&body.vault_root);
    validate_root(&candidate, &state.state_dir).map_err(|e| bad_request(e.to_string()))?;
    let candidate = candidate
        .canonicalize()
        .map_err(|e| bad_request(format!("resolving {}: {e}", candidate.display())))?;

    let mut vaults = state.vaults.write().await;
    let preview = vaults.registry.preview(&candidate, &state.state_dir)?;

    // Nothing registered yet is the first-run case: there is nothing to orphan.
    let would_orphan_everything = preview.found.is_empty() && !vaults.registry.vaults.is_empty();
    if would_orphan_everything && !body.orphan_ok {
        return Err(ApiError(
            StatusCode::CONFLICT,
            format!(
                "none of the {} registered vault(s) were found under {}. \
                 Move the vault directories there first — Storm does not move \
                 files. Repeat with orphan_ok to leave them behind.",
                vaults.registry.vaults.len(),
                candidate.display()
            ),
        ));
    }

    vaults.registry.root = candidate.clone();
    vaults
        .registry
        .scan_root(&state.state_dir, &crate::index::now_rfc3339())?;
    vaults.registry.save(&state.state_dir)?;
    *vaults = open_vaults(&vaults.registry, &state.state_dir)?;

    // The watcher follows the root; without this it keeps watching the old one
    // and external edits in the new location go unnoticed.
    let _ = state.root_changed.send(candidate.clone());

    Ok(Json(serde_json::json!({
        "vault_root": candidate.display().to_string(),
        "found": preview.found,
        "orphaned": preview.orphaned,
        "adopted": preview.adopted,
    })))
}

/// The rules a storage root must satisfy.
///
/// Note what is *not* here: the root is allowed to contain `state_dir`. The
/// `--vault` compatibility shim produces exactly that layout, and
/// `Registry::candidate_dirs` skips `state_dir` so it is never adopted as a
/// vault. Only the other direction — a root *inside* the state directory — is
/// forbidden, because that would put a vault inside Storm's own state.
pub fn validate_root(root: &FsPath, state_dir: &FsPath) -> anyhow::Result<()> {
    if !root.is_absolute() {
        anyhow::bail!("the storage root must be an absolute path");
    }
    if !root.exists() {
        anyhow::bail!("{} does not exist", root.display());
    }
    if !root.is_dir() {
        anyhow::bail!("{} is not a directory", root.display());
    }
    let real_root = root.canonicalize()?;
    if let Ok(real_state) = state_dir.canonicalize()
        && real_root.starts_with(&real_state)
    {
        anyhow::bail!(
            "the storage root cannot be inside the state directory ({})",
            real_state.display()
        );
    }
    // Probe rather than trust permission bits, which say nothing useful under
    // ACLs, containers or a read-only mount.
    let probe = real_root.join(".storm-write-probe");
    std::fs::write(&probe, b"")
        .with_context_msg(|| format!("{} is not writable", real_root.display()))?;
    let _ = std::fs::remove_file(&probe);
    Ok(())
}

/// Small helper so [`validate_root`] can report a friendly reason rather than
/// a raw io error.
trait ContextMsg<T> {
    fn with_context_msg<F: FnOnce() -> String>(self, f: F) -> anyhow::Result<T>;
}

impl<T> ContextMsg<T> for std::io::Result<T> {
    fn with_context_msg<F: FnOnce() -> String>(self, f: F) -> anyhow::Result<T> {
        self.map_err(|e| anyhow::anyhow!("{} ({e})", f()))
    }
}

// ---- recents -----------------------------------------------------------

#[derive(Deserialize)]
struct RecentsQuery {
    #[serde(default = "default_recents_limit")]
    limit: i64,
}

fn default_recents_limit() -> i64 {
    20
}

async fn recents(
    State(state): State<Shared>,
    Extension(actor): Extension<Actor>,
    Query(q): Query<RecentsQuery>,
) -> ApiResult<Json<serde_json::Value>> {
    let recents = crate::ops::recents(&state, &actor, q.limit).await?;
    Ok(Json(serde_json::json!({ "recents": recents })))
}

async fn tree(
    State(state): State<Shared>,
    Extension(actor): Extension<Actor>,
    Path(vault): Path<String>,
) -> ApiResult<Json<crate::ops::Tree>> {
    Ok(Json(crate::ops::tree(&state, &actor, &vault).await?))
}

#[derive(Deserialize)]
struct SyncQuery {
    #[serde(default)]
    since: i64,
    #[serde(default = "default_limit")]
    limit: i64,
}

fn default_limit() -> i64 {
    500
}

async fn sync(
    State(state): State<Shared>,
    Extension(actor): Extension<Actor>,
    Path(vault): Path<String>,
    Query(q): Query<SyncQuery>,
) -> ApiResult<Json<crate::ops::SyncPage>> {
    Ok(Json(
        crate::ops::sync(&state, &actor, &vault, q.since, q.limit).await?,
    ))
}

async fn get_note(
    State(state): State<Shared>,
    Extension(actor): Extension<Actor>,
    Path((vault, id)): Path<(String, String)>,
) -> ApiResult<Json<crate::ops::NoteWithProvenance>> {
    Ok(Json(
        crate::ops::get_note_with_provenance(&state, &actor, &vault, &id).await?,
    ))
}

/// Every stored revision of a note, newest first, without their content.
///
/// Added with MCP but not only for it: no client could see a note's history
/// before this, even though `note_versions` has been populated since M1.
async fn note_versions(
    State(state): State<Shared>,
    Extension(actor): Extension<Actor>,
    Path((vault, id)): Path<(String, String)>,
) -> ApiResult<Json<serde_json::Value>> {
    let versions = crate::ops::note_history(&state, &actor, &vault, &id).await?;
    Ok(Json(serde_json::json!({ "versions": versions })))
}

async fn note_version(
    State(state): State<Shared>,
    Extension(actor): Extension<Actor>,
    Path((vault, id, version)): Path<(String, String, i64)>,
) -> ApiResult<Json<serde_json::Value>> {
    let content = crate::ops::note_version(&state, &actor, &vault, &id, version).await?;
    Ok(Json(
        serde_json::json!({ "version": version, "content": content }),
    ))
}

/// Fire-and-forget from the client.
async fn mark_opened(
    State(state): State<Shared>,
    Extension(actor): Extension<Actor>,
    Path((vault, id)): Path<(String, String)>,
) -> ApiResult<Json<serde_json::Value>> {
    crate::ops::mark_opened(&state, &actor, &vault, &id).await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

#[derive(Deserialize)]
struct CreateBody {
    path: String,
    #[serde(default)]
    content: String,
}

async fn create_note(
    State(state): State<Shared>,
    Extension(actor): Extension<Actor>,
    Path(vault): Path<String>,
    Json(body): Json<CreateBody>,
) -> ApiResult<Json<crate::index::WriteResult>> {
    Ok(Json(
        crate::ops::create_note(&state, &actor, &vault, &body.path, &body.content).await?,
    ))
}

#[derive(Deserialize)]
struct PutBody {
    base_version: i64,
    content: String,
    #[serde(default)]
    device_id: Option<String>,
}

async fn put_note(
    State(state): State<Shared>,
    Extension(actor): Extension<Actor>,
    Path((vault, id)): Path<(String, String)>,
    Json(body): Json<PutBody>,
) -> ApiResult<Json<crate::index::WriteResult>> {
    Ok(Json(
        crate::ops::update_note(
            &state,
            &actor,
            &vault,
            &id,
            body.base_version,
            &body.content,
            body.device_id.as_deref(),
        )
        .await?,
    ))
}

#[derive(Deserialize)]
struct MoveBody {
    new_path: String,
}

async fn move_note(
    State(state): State<Shared>,
    Extension(actor): Extension<Actor>,
    Path((vault, id)): Path<(String, String)>,
    Json(body): Json<MoveBody>,
) -> ApiResult<Json<crate::index::WriteResult>> {
    Ok(Json(
        crate::ops::move_note(&state, &actor, &vault, &id, &body.new_path).await?,
    ))
}

async fn delete_note(
    State(state): State<Shared>,
    Extension(actor): Extension<Actor>,
    Path((vault, id)): Path<(String, String)>,
) -> ApiResult<Json<serde_json::Value>> {
    let seq = crate::ops::delete_note(&state, &actor, &vault, &id).await?;
    Ok(Json(serde_json::json!({ "seq": seq })))
}

// ---- folders -----------------------------------------------------------

#[derive(Deserialize)]
struct FolderBody {
    path: String,
}

async fn create_folder(
    State(state): State<Shared>,
    Extension(actor): Extension<Actor>,
    Path(vault): Path<String>,
    Json(body): Json<FolderBody>,
) -> ApiResult<Json<serde_json::Value>> {
    crate::ops::create_folder(&state, &actor, &vault, &body.path).await?;
    Ok(Json(serde_json::json!({ "path": body.path })))
}

async fn delete_folder(
    State(state): State<Shared>,
    Extension(actor): Extension<Actor>,
    Path((vault, path)): Path<(String, String)>,
) -> ApiResult<Json<serde_json::Value>> {
    crate::ops::delete_folder(&state, &actor, &vault, &path).await?;
    Ok(Json(serde_json::json!({ "deleted": path })))
}

#[derive(Deserialize)]
struct RenameFolderBody {
    from: String,
    to: String,
}

async fn rename_folder(
    State(state): State<Shared>,
    Extension(actor): Extension<Actor>,
    Path(vault): Path<String>,
    Json(body): Json<RenameFolderBody>,
) -> ApiResult<Json<serde_json::Value>> {
    let moved = crate::ops::rename_folder(&state, &actor, &vault, &body.from, &body.to).await?;
    Ok(Json(serde_json::json!({
        "from": body.from,
        "to": body.to,
        "moved": moved,
    })))
}

#[derive(Deserialize)]
struct SearchQuery {
    q: String,
    #[serde(default = "default_search_limit")]
    limit: i64,
}

fn default_search_limit() -> i64 {
    50
}

async fn search(
    State(state): State<Shared>,
    Extension(actor): Extension<Actor>,
    Path(vault): Path<String>,
    Query(q): Query<SearchQuery>,
) -> ApiResult<Json<serde_json::Value>> {
    let hits = crate::ops::search(&state, &actor, &vault, &q.q, q.limit).await?;
    Ok(Json(serde_json::json!({ "hits": hits })))
}

async fn backlinks(
    State(state): State<Shared>,
    Extension(actor): Extension<Actor>,
    Path((vault, id)): Path<(String, String)>,
) -> ApiResult<Json<serde_json::Value>> {
    let links = crate::ops::backlinks(&state, &actor, &vault, &id).await?;
    Ok(Json(
        serde_json::json!({ "title": links.title, "backlinks": links.notes }),
    ))
}

async fn tags(
    State(state): State<Shared>,
    Extension(actor): Extension<Actor>,
    Path(vault): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let tags = crate::ops::list_tags(&state, &actor, &vault).await?;
    Ok(Json(serde_json::json!({ "tags": tags })))
}

async fn notes_by_tag(
    State(state): State<Shared>,
    Extension(actor): Extension<Actor>,
    Path((vault, tag)): Path<(String, String)>,
) -> ApiResult<Json<serde_json::Value>> {
    let notes = crate::ops::notes_by_tag(&state, &actor, &vault, &tag).await?;
    Ok(Json(serde_json::json!({ "notes": notes })))
}

// ---- attachments -------------------------------------------------------

async fn list_attachments(
    State(state): State<Shared>,
    Extension(actor): Extension<Actor>,
    Path(vault): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let attachments = crate::ops::list_attachments(&state, &actor, &vault).await?;
    Ok(Json(serde_json::json!({ "attachments": attachments })))
}

async fn get_attachment(
    State(state): State<Shared>,
    Extension(actor): Extension<Actor>,
    Path((vault, path)): Path<(String, String)>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    let bytes = crate::ops::read_attachment(&state, &actor, &vault, &path).await?;
    // Hashing up to 64 MiB for the ETag stays off the runtime too.
    let content_type = content_type_for(&path);
    tokio::task::spawn_blocking(move || ranged(bytes, content_type, &headers))
        .await
        .map_err(|e| crate::index::IndexError::from(anyhow::Error::from(e)).into())
}

/// A whole attachment, or the single range a resuming client asked for
/// (SRP §5.5 rule 3). A relay ends a download that outruns its reader with
/// `client_backlogged`; the client resumes from what it has rather than
/// starting a 64 MiB file again.
///
/// The `ETag` is the content's hash, and a `Range` whose `If-Range` no longer
/// matches gets the whole new file: an attachment replaced between two
/// attempts must never be spliced onto the old one's prefix.
fn ranged(bytes: Vec<u8>, content_type: &'static str, headers: &HeaderMap) -> Response {
    let etag = format!("\"{}\"", &blake3::hash(&bytes).to_hex()[..32]);
    let len = bytes.len();
    let fresh = headers
        .get(header::IF_RANGE)
        .is_none_or(|v| v.to_str().is_ok_and(|v| v == etag));
    let range = headers
        .get(header::RANGE)
        .filter(|_| fresh)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| parse_byte_range(v, len));
    // An attachment is whatever was uploaded, served from the app's own
    // origin: never sniffed into HTML, and sandboxed if a browser renders it
    // (decision 128g, H1).
    let common = [
        (header::CONTENT_TYPE, content_type.to_string()),
        (header::ACCEPT_RANGES, "bytes".to_string()),
        (header::ETAG, etag),
        (header::X_CONTENT_TYPE_OPTIONS, "nosniff".to_string()),
        (header::CONTENT_SECURITY_POLICY, "sandbox".to_string()),
    ];
    match range {
        None => (common, bytes).into_response(),
        Some(Err(())) => (
            StatusCode::RANGE_NOT_SATISFIABLE,
            common,
            [(header::CONTENT_RANGE, format!("bytes */{len}"))],
        )
            .into_response(),
        Some(Ok((start, end))) => (
            StatusCode::PARTIAL_CONTENT,
            common,
            [(header::CONTENT_RANGE, format!("bytes {start}-{end}/{len}"))],
            bytes[start..=end].to_vec(),
        )
            .into_response(),
    }
}

/// One `bytes=` range over `len` bytes, inclusive. `None` for anything this
/// does not serve as a range (several ranges, another unit, a malformed
/// header), which gets the whole body as RFC 9110 allows; `Err` for a range
/// that starts past the end.
fn parse_byte_range(value: &str, len: usize) -> Option<Result<(usize, usize), ()>> {
    let spec = value.trim().strip_prefix("bytes=")?;
    if spec.contains(',') {
        return None;
    }
    let (first, last) = spec.split_once('-')?;
    let (first, last) = (first.trim(), last.trim());
    if first.is_empty() {
        let suffix: usize = last.parse().ok()?;
        if suffix == 0 || len == 0 {
            return Some(Err(()));
        }
        return Some(Ok((len.saturating_sub(suffix), len - 1)));
    }
    let start: usize = first.parse().ok()?;
    let end = if last.is_empty() {
        len.checked_sub(1)?
    } else {
        let end: usize = last.parse().ok()?;
        if end < start {
            return None;
        }
        end.min(len.checked_sub(1)?)
    };
    if start >= len {
        return Some(Err(()));
    }
    Some(Ok((start, end)))
}

async fn put_attachment(
    State(state): State<Shared>,
    Extension(actor): Extension<Actor>,
    Path((vault, path)): Path<(String, String)>,
    body: axum::body::Bytes,
) -> ApiResult<Json<serde_json::Value>> {
    crate::ops::put_attachment(&state, &actor, &vault, &path, &body).await?;
    Ok(Json(
        serde_json::json!({ "path": path, "size": body.len() }),
    ))
}

async fn delete_attachment(
    State(state): State<Shared>,
    Extension(actor): Extension<Actor>,
    Path((vault, path)): Path<(String, String)>,
) -> ApiResult<Json<serde_json::Value>> {
    crate::ops::delete_attachment(&state, &actor, &vault, &path).await?;
    Ok(Json(serde_json::json!({ "deleted": path })))
}

/// Enough of a MIME table for what a vault actually holds.
///
/// Browsers need this to display an image inline rather than download it, and
/// the web client fetches attachments through the same route.
fn content_type_for(path: &str) -> &'static str {
    let ext = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "pdf" => "application/pdf",
        "txt" => "text/plain; charset=utf-8",
        "json" => "application/json",
        "mp3" => "audio/mpeg",
        "mp4" => "video/mp4",
        _ => "application/octet-stream",
    }
}

// ---- change feed (WebSocket + SSE) --------------------------------------

/// The literal a lagging WebSocket client has always been sent.
///
/// A constant so the "must not change" invariant below has something a test
/// can compare against; these bytes are the wire format, not an implementation
/// detail.
const WS_RESYNC: &str = r#"{"kind":"resync"}"#;

/// The SSE form of the same thing (`docs/srp-v1.md` §5.3). Empty `data:`.
const SSE_RESYNC: &str = "event: resync\ndata:\n\n";

/// An SSE comment, which every reader ignores (SRP §5.3, decision 116b).
const SSE_KEEPALIVE: &str = ":\n\n";
const SSE_KEEPALIVE_EVERY: std::time::Duration = std::time::Duration::from_secs(15);

/// The peer is gone; stop the feed.
struct Gone;

/// Where a change goes once the feed has one.
///
/// Two transports carry the *same* feed. A LAN client upgrades to a WebSocket;
/// a relayed client cannot, because a tunnelled request is dispatched
/// in-process and `WebSocketUpgrade` needs a `hyper::upgrade::OnUpgrade` that
/// only a real hyper connection produces. So `/v1/stream` grew a second
/// response mode rather than a second code path — one loop, two framings.
///
/// **It takes a `Change`, not rendered text.** SSE has to put `vault_id` and
/// `seq` into its `id:` line, so a `send_text(String)` shape could not build
/// its own frame; each implementation decides its own framing instead.
///
/// `impl Future + Send` rather than a bare `async fn` in the trait: the loop is
/// spawned, so its future has to be `Send`, and that is not inferable through
/// an `async fn` declaration.
trait ChangeSink: Send + 'static {
    fn change(&mut self, change: &Change) -> impl Future<Output = Result<(), Gone>> + Send;
    fn resync(&mut self) -> impl Future<Output = Result<(), Gone>> + Send;
}

/// The WebSocket framing, unchanged since the feed existed.
struct WsSink(WebSocket);

impl ChangeSink for WsSink {
    async fn change(&mut self, change: &Change) -> Result<(), Gone> {
        // **Byte-for-byte what this socket has always sent**: a bare
        // `serde_json` `Change` in a text frame, no envelope and no event
        // name. A shipped Flutter client parses exactly this, and
        // `apps/client/test_live/two_client_sync_test.dart` gates releases on
        // it. SSE framing belongs to the other sink and must never leak here.
        let Ok(text) = serde_json::to_string(change) else {
            // Unserializable is skipped, not fatal — as before.
            return Ok(());
        };
        self.0
            .send(Message::Text(text.into()))
            .await
            .map_err(|_| Gone)
    }

    async fn resync(&mut self) -> Result<(), Gone> {
        // A failed resync is ignored rather than ending the feed, which is what
        // this branch has always done: the next change's send is what discovers
        // a dead socket.
        let _ = self.0.send(Message::Text(WS_RESYNC.into())).await;
        Ok(())
    }
}

/// The SSE framing (`docs/srp-v1.md` §5.3), written into the response body's
/// channel.
struct SseSink(tokio::sync::mpsc::Sender<Result<String, std::convert::Infallible>>);

impl ChangeSink for SseSink {
    async fn change(&mut self, change: &Change) -> Result<(), Gone> {
        let Ok(json) = serde_json::to_string(change) else {
            return Ok(());
        };
        // **`id:` is `<vault_id>:<seq>`, never the bare `seq`.** `change_log`
        // lives in each vault's own `index.db` and `seq` is that database's
        // `last_insert_rowid()`, so seq is per vault (M9/M10) while this feed is
        // cross-vault: two vaults both emit 1, 2, 3. A bare id would collide and
        // land a `Last-Event-ID` resume at the wrong position with nothing
        // anywhere reporting an error.
        let frame = format!(
            "event: change\nid: {}:{}\ndata: {json}\n\n",
            change.vault_id, change.seq
        );
        self.0.send(Ok(frame)).await.map_err(|_| Gone)
    }

    async fn resync(&mut self) -> Result<(), Gone> {
        self.0.send(Ok(SSE_RESYNC.into())).await.map_err(|_| Gone)
    }
}

/// The change feed.
///
/// Answers a WebSocket upgrade when one is asked for, and an ordinary
/// `text/event-stream` response when one is not — the shape a relay can carry,
/// since a tunnelled request is dispatched in-process and never has an
/// `OnUpgrade` to hand the upgrade extractor.
///
/// **The branch is chosen here, by hand, and that is not a style choice.**
/// There is no `Option<WebSocketUpgrade>`: `WebSocketUpgrade` implements
/// `FromRequestParts` and *not* `OptionalFromRequestParts`, so writing
/// `Option<WebSocketUpgrade>` in this signature fails as an unsatisfied
/// `Handler` bound that names neither type. The handler therefore takes the
/// whole request, decides, and calls the extractor itself on the upgrade branch.
///
/// Note what did *not* change: this is an ordinary session-tier request behind
/// `require_auth`. Subscribing to `state.events` anywhere else — in a tunnel
/// client, say — would hand every vault's change feed to a caller that
/// presented no credential at all.
async fn stream(State(state): State<Shared>, request: axum::extract::Request) -> Response {
    let (mut parts, _body) = request.into_parts();

    // Subscribed before either branch answers, so nothing is missed between the
    // response going out and the feed task starting.
    let rx = state.events.subscribe();

    if wants_websocket(&parts) {
        return match WebSocketUpgrade::from_request_parts(&mut parts, &state).await {
            Ok(ws) => ws.on_upgrade(move |socket| push_changes(WsSink(socket), rx)),
            Err(rejection) => rejection.into_response(),
        };
    }

    sse_response(&parts.headers, rx)
}

/// Does this request ask to become a WebSocket, as HTTP means it?
///
/// `Connection` is a comma-separated token list — browsers and proxies send
/// `keep-alive, Upgrade` — and both it and `Upgrade: websocket` are
/// case-insensitive, so comparing either header whole would miss real
/// handshakes and answer them with an SSE body they cannot read.
fn wants_websocket(parts: &axum::http::request::Parts) -> bool {
    // Above HTTP/1.1 there is no `Upgrade` header at all: a WebSocket is an
    // extended CONNECT. Mirrors what `WebSocketUpgrade` itself checks, so this
    // branch and the extractor it calls agree on what an upgrade is.
    if parts.version > axum::http::Version::HTTP_11 {
        return parts.method == axum::http::Method::CONNECT;
    }

    header_has_token(&parts.headers, header::CONNECTION, "upgrade")
        && header_has_token(&parts.headers, header::UPGRADE, "websocket")
}

fn header_has_token(headers: &HeaderMap, name: header::HeaderName, token: &str) -> bool {
    headers
        .get_all(name)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .any(|candidate| candidate.trim().eq_ignore_ascii_case(token))
}

/// The non-upgrade response mode: the same feed as `text/event-stream`.
fn sse_response(headers: &HeaderMap, rx: broadcast::Receiver<Change>) -> Response {
    // **`Last-Event-ID` is ignored deliberately.** Whether the origin replays
    // from it is unresolved (`docs/srp-v1.md` §5.3) — replay implies buffering,
    // which the no-relay-storage rule bars — and an EventSource sends the header
    // by itself on every reconnect, so accepting one in silence would read as
    // support for a semantics nobody has chosen. Logged so a resume attempt is
    // at least visible; not honoured.
    if let Some(id) = headers.get("last-event-id") {
        tracing::debug!(
            last_event_id = ?id,
            "ignoring Last-Event-ID: whether the origin replays is undecided (srp-v1 §5.3)"
        );
    }

    // Bounded, so a client that stops reading cannot grow this without limit.
    // Backpressure here stalls the feed task, `broadcast` turns that into
    // `Lagged`, and the client is told to resync — the same recovery a slow
    // WebSocket gets.
    let (tx, body_rx) = tokio::sync::mpsc::channel(64);
    tokio::spawn(feed_with_keepalive(tx, rx, SSE_KEEPALIVE_EVERY));

    (
        [
            (header::CONTENT_TYPE, "text/event-stream"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        // No inactivity timeout: a quiet vault is a normal vault. The comment
        // every 15 s is what lets a reader tell a quiet feed from a dead one.
        axum::body::Body::from_stream(tokio_stream::wrappers::ReceiverStream::new(body_rx)),
    )
        .into_response()
}

/// The feed, with a comment every `period` (SRP §5.3). Over a
/// relay this is the one long-lived stream every client holds, so it is also
/// traffic both trunks can see. Ends with the feed.
async fn feed_with_keepalive(
    tx: tokio::sync::mpsc::Sender<Result<String, std::convert::Infallible>>,
    rx: broadcast::Receiver<Change>,
    period: std::time::Duration,
) {
    let keepalive = {
        let tx = tx.clone();
        async move {
            let mut every = tokio::time::interval(period);
            every.tick().await;
            loop {
                every.tick().await;
                // A full queue is already traffic; this one can be skipped.
                if let Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) =
                    tx.try_send(Ok(SSE_KEEPALIVE.into()))
                {
                    return;
                }
            }
        }
    };
    tokio::select! {
        () = push_changes(SseSink(tx), rx) => {}
        () = keepalive => {}
    }
}

/// Pushes change events so other devices update without polling.
///
/// Events carry only metadata; the client decides what to fetch. That keeps the
/// feed cheap and means a missed message is recoverable by falling back to
/// `GET /v1/sync?since=`.
///
/// Takes the receiver rather than subscribing itself, so the caller can
/// subscribe before it answers — a subscription taken after the response is
/// written would miss every change in between.
async fn push_changes<S: ChangeSink>(mut sink: S, mut rx: broadcast::Receiver<Change>) {
    loop {
        match rx.recv().await {
            Ok(change) => {
                if sink.change(&change).await.is_err() {
                    break;
                }
            }
            // A slow client that fell behind is told to resync rather than being
            // silently left with a gap.
            Err(broadcast::error::RecvError::Lagged(_)) => {
                if sink.resync().await.is_err() {
                    break;
                }
            }
            Err(broadcast::error::RecvError::Closed) => break,
        }
    }
}

// `pub(crate)` so `relay::tests` can build a *real* router and real
// credentials rather than restating forty lines of `AppState`. The relay's
// whole claim is that a tunnelled request reaches the same router a LAN
// request does, and a second, parallel construction of that router is exactly
// the thing that would let the claim quietly stop being true.
#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use tower::ServiceExt;

    /// A router over an empty state directory — no vaults, a real identity.
    ///
    /// Enough for every question about the auth tiers, which are decided
    /// before any vault is touched.
    fn test_router(dir: &FsPath) -> (Router, Arc<crate::auth::ServerIdentity>) {
        let (app, identity, _) = test_router_with_state(dir);
        (app, identity)
    }

    /// A policy that refuses everything, so the refusal path is exercised
    /// for every caller, not only an agent.
    ///
    /// Without it `Decision::Deny` would be code that first runs in production.
    /// The point of the seam is that swapping the policy is all it takes, and
    /// this is that claim under test.
    #[derive(Debug)]
    struct DenyAll;

    impl crate::auth::authz::VaultPolicy for DenyAll {
        fn decide(&self, _: &Actor, _: &str, _: Access) -> Decision {
            Decision::Deny("test policy refuses everything")
        }
    }

    /// As [`test_router`], but hands back the state too — for the tests that
    /// have to look at what a request *persisted*, not only what it answered.
    pub(crate) fn test_router_with_state(
        dir: &FsPath,
    ) -> (Router, Arc<crate::auth::ServerIdentity>, Shared) {
        test_router_with_policy(dir, Arc::new(crate::auth::authz::StormPolicy))
    }

    fn test_router_with_policy(
        dir: &FsPath,
        policy: Arc<dyn crate::auth::authz::VaultPolicy>,
    ) -> (Router, Arc<crate::auth::ServerIdentity>, Shared) {
        test_router_full(
            dir,
            policy,
            crate::auth::ratelimit::LoginLimiter::new(),
            vec![],
        )
    }

    /// As [`test_router_with_state`], but with the login limiter's numbers
    /// chosen by the caller — the rate-limit tests want a burst they can
    /// exhaust in two requests rather than thirty, because every attempt that
    /// reaches the handler pays a real Argon2id verify.
    pub(crate) fn test_router_with_limiter(
        dir: &FsPath,
        limiter: crate::auth::ratelimit::LoginLimiter,
    ) -> (Router, Arc<crate::auth::ServerIdentity>, Shared) {
        test_router_full(
            dir,
            Arc::new(crate::auth::authz::StormPolicy),
            limiter,
            vec![],
        )
    }

    /// As [`test_router_with_state`], but with rmcp's DNS-rebinding list set,
    /// the way a server bound to a concrete address has it.
    pub(crate) fn test_router_with_mcp_hosts(
        dir: &FsPath,
        allowed_hosts: Vec<String>,
    ) -> (Router, Arc<crate::auth::ServerIdentity>, Shared) {
        test_router_full(
            dir,
            Arc::new(crate::auth::authz::StormPolicy),
            crate::auth::ratelimit::LoginLimiter::new(),
            allowed_hosts,
        )
    }

    fn test_router_full(
        dir: &FsPath,
        policy: Arc<dyn crate::auth::authz::VaultPolicy>,
        login_limiter: crate::auth::ratelimit::LoginLimiter,
        allowed_hosts: Vec<String>,
    ) -> (Router, Arc<crate::auth::ServerIdentity>, Shared) {
        let state_dir = dir.join("state");
        std::fs::create_dir_all(&state_dir).unwrap();
        let root = dir.join("vaults");
        std::fs::create_dir_all(&root).unwrap();

        let mut auth_db = crate::auth::AuthDb::open(&state_dir).unwrap();
        let identity = Arc::new(
            crate::auth::identity::load_or_create(&mut auth_db, &state_dir, "2026-08-13T00:00:00Z")
                .unwrap(),
        );
        let (events, _) = broadcast::channel(8);
        let (root_changed, _) = broadcast::channel(2);
        let registry = Registry::load(&state_dir, &root).unwrap();
        let agent = Arc::new(crate::agent::AgentManager::open(&state_dir).unwrap());
        let gateway =
            Arc::new(crate::gateway::Gateway::open(&state_dir, "2026-10-05T00:00:00Z").unwrap());
        let state: Shared = Arc::new(AppState {
            vaults: RwLock::new(VaultSet {
                registry,
                open: HashMap::new(),
            }),
            events,
            state_dir,
            identity: identity.clone(),
            root_changed,
            relays_changed: tokio::sync::watch::channel(Vec::new()).0,
            mcp_enabled: std::sync::atomic::AtomicBool::new(false),
            mcp_writable: std::sync::atomic::AtomicBool::new(false),
            agent_writes: std::sync::atomic::AtomicBool::new(false),
            auth_db: Arc::new(tokio::sync::Mutex::new(auth_db)),
            listen_addr: "http://127.0.0.1:8080".into(),
            vault_policy: policy,
            hasher: crate::auth::Hasher::new(),
            login_limiter,
            host_limiter: crate::auth::ratelimit::LoginLimiter::new(),
            discovery: crate::discovery::Discovery::new(false, true),
            agent,
            gateway,
        });
        (
            router(state.clone(), crate::mcp::McpOptions { allowed_hosts }),
            identity,
            state,
        )
    }

    async fn send(
        app: &Router,
        request: axum::http::Request<axum::body::Body>,
    ) -> (StatusCode, serde_json::Value) {
        let response = app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
            .await
            .unwrap();
        let body = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
        (status, body)
    }

    #[test]
    fn a_byte_range_is_parsed_as_rfc_9110_says() {
        assert_eq!(parse_byte_range("bytes=4-", 10), Some(Ok((4, 9))));
        assert_eq!(parse_byte_range("bytes=2-5", 10), Some(Ok((2, 5))));
        assert_eq!(parse_byte_range("bytes=2-50", 10), Some(Ok((2, 9))));
        assert_eq!(parse_byte_range("bytes=-3", 10), Some(Ok((7, 9))));
        assert_eq!(parse_byte_range("bytes=-30", 10), Some(Ok((0, 9))));
        assert_eq!(parse_byte_range("bytes=10-", 10), Some(Err(())));
        assert_eq!(parse_byte_range("bytes=-0", 10), Some(Err(())));
        for whole in ["bytes=0-1,4-5", "items=0-1", "bytes=5-2", "bytes=x-", "0-1"] {
            assert_eq!(parse_byte_range(whole, 10), None, "{whole}");
        }
    }

    async fn ranged_parts(
        headers: &[(header::HeaderName, &str)],
    ) -> (StatusCode, HeaderMap, Vec<u8>) {
        let mut map = HeaderMap::new();
        for (k, v) in headers {
            map.insert(k.clone(), v.parse().unwrap());
        }
        let response = ranged(b"0123456789".to_vec(), "application/pdf", &map);
        let (parts, body) = response.into_parts();
        let body = axum::body::to_bytes(body, 1 << 20).await.unwrap().to_vec();
        (parts.status, parts.headers, body)
    }

    #[tokio::test]
    async fn an_attachment_resumes_from_a_range_and_only_while_it_is_unchanged() {
        let (status, headers, body) = ranged_parts(&[]).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, b"0123456789");
        assert_eq!(headers[header::ACCEPT_RANGES], "bytes");
        let etag = headers[header::ETAG].to_str().unwrap().to_string();

        let (status, headers, body) =
            ranged_parts(&[(header::RANGE, "bytes=4-"), (header::IF_RANGE, &etag)]).await;
        assert_eq!(status, StatusCode::PARTIAL_CONTENT);
        assert_eq!(headers[header::CONTENT_RANGE], "bytes 4-9/10");
        assert_eq!(body, b"456789");

        // Replaced since the first attempt: the whole new file, never a splice.
        let (status, _, body) =
            ranged_parts(&[(header::RANGE, "bytes=4-"), (header::IF_RANGE, "\"stale\"")]).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, b"0123456789");

        let (status, headers, _) = ranged_parts(&[(header::RANGE, "bytes=10-")]).await;
        assert_eq!(status, StatusCode::RANGE_NOT_SATISFIABLE);
        assert_eq!(headers[header::CONTENT_RANGE], "bytes */10");
    }

    fn get(path: &str) -> axum::http::Request<axum::body::Body> {
        axum::http::Request::builder()
            .uri(path)
            .body(axum::body::Body::empty())
            .unwrap()
    }

    fn post_json(path: &str, body: serde_json::Value) -> axum::http::Request<axum::body::Body> {
        axum::http::Request::builder()
            .method("POST")
            .uri(path)
            .header("content-type", "application/json")
            .body(axum::body::Body::from(body.to_string()))
            .unwrap()
    }

    fn get_with_auth(path: &str, auth: &str) -> axum::http::Request<axum::body::Body> {
        axum::http::Request::builder()
            .uri(path)
            .header("authorization", auth)
            .body(axum::body::Body::empty())
            .unwrap()
    }

    /// A request carrying its credential in the query string and no header —
    /// what a browser WebSocket handshake and an `<img>` tag are limited to.
    fn get_with_query_token(path: &str, token: &str) -> axum::http::Request<axum::body::Body> {
        let sep = if path.contains('?') { '&' } else { '?' };
        axum::http::Request::builder()
            .uri(format!("{path}{sep}token={}", urlencoding_for_test(token)))
            .body(axum::body::Body::empty())
            .unwrap()
    }

    /// Percent-encodes just enough for a credential: the space in `Bearer x`.
    fn urlencoding_for_test(s: &str) -> String {
        s.replace(' ', "%20")
    }

    #[tokio::test]
    async fn a_query_credential_must_name_its_scheme() {
        // The query fallback exists because a browser can set no headers on a
        // WebSocket handshake, and an image widget none on a GET. It takes the
        // *same* credential string the header would carry — scheme and all.
        //
        // Pinning both halves, because neither was covered and the bare form
        // silently stopped working when the shared token was removed: until
        // then `credential` was compared whole against STORM_TOKEN, so a bare
        // value matched. Nothing failed loudly; the change feed just quietly
        // never authenticated again.
        let dir = tempdir::TempDir::new("storm-query-cred").unwrap();
        let (app, _, state) = test_router_with_state(dir.path());
        let owner = seed_owner(&state).await;
        let token = session_token(&state, &owner).await;

        let (bare, _) = send(&app, get_with_query_token("/v1/vaults", &token)).await;
        assert_eq!(
            bare,
            StatusCode::UNAUTHORIZED,
            "a bare token in the query names no scheme and must not authenticate"
        );

        let (scheme, _) = send(
            &app,
            get_with_query_token("/v1/vaults", &format!("Bearer {token}")),
        )
        .await;
        assert_eq!(
            scheme,
            StatusCode::OK,
            "the same credential the header would carry must work in the query"
        );
    }

    fn put_json(
        path: &str,
        body: serde_json::Value,
        auth: Option<&str>,
    ) -> axum::http::Request<axum::body::Body> {
        let mut builder = axum::http::Request::builder()
            .method("PUT")
            .uri(path)
            .header("content-type", "application/json");
        if let Some(credential) = auth {
            builder = builder.header("authorization", credential);
        }
        builder
            .body(axum::body::Body::from(body.to_string()))
            .unwrap()
    }

    /// Sets up the account.
    ///
    /// The stored hash is a fixed PHC string and is never verified: these tests
    /// are about the switch, not about login, and a real Argon2id hash at the
    /// measured parameters would cost 192 MiB and ~170 ms apiece to prove
    /// nothing they assert.
    pub(crate) async fn seed_owner(state: &Shared) -> String {
        let mut auth_db = state.auth_db.lock().await;
        crate::auth::account::create_account(
            &mut auth_db,
            "$argon2id$v=19$m=196608,t=1,p=1$c29tZXNhbHQ$bm90YXJlYWxoYXNo",
            "2026-08-17T00:00:00Z",
        )
        .unwrap()
        .id
    }

    /// A session token for `user_id`, minted directly rather than through
    /// `login`, for the same reason [`seed_owner`] fakes the hash.
    ///
    /// Issued **now**, never at a fixed date. The middleware checks expiry
    /// against the real clock, so a token stamped `2026-08-17` expired 30 days
    /// later, and on 2026-09-16 every test using this helper started failing
    /// with `session_expired` on code nobody had touched.
    pub(crate) async fn session_token(state: &Shared, user_id: &str) -> String {
        let now = crate::index::now_rfc3339();
        let mut auth_db = state.auth_db.lock().await;
        let device = crate::auth::devices::create_synthetic(&mut auth_db, "test", &now).unwrap();
        crate::auth::sessions::create(&mut auth_db, user_id, &device.id, &now)
            .unwrap()
            .access_token
    }

    /// Records every authorization decision, so a test can see which actor
    /// reached which vault.
    ///
    /// Allows everything — the question here is *whose identity arrived*, not
    /// whether it was permitted.
    #[derive(Debug, Default)]
    struct RecordingPolicy {
        /// One entry per decision: the user and vault it was asked about,
        /// and the full actor, for tests asking *what kind* of caller
        /// arrived. **One lock for both**, so entry `i` of `pairs()` and of
        /// `actors()` are always the same call. Two locks let concurrent
        /// decisions interleave between the pushes, and
        /// `concurrent_mcp_calls_do_not_share_an_identity` then reported a
        /// leak that never happened.
        seen: std::sync::Mutex<Vec<((String, String), Actor)>>,
    }

    impl RecordingPolicy {
        fn pairs(&self) -> Vec<(String, String)> {
            self.seen
                .lock()
                .unwrap()
                .iter()
                .map(|(p, _)| p.clone())
                .collect()
        }

        fn actors(&self) -> Vec<Actor> {
            self.seen
                .lock()
                .unwrap()
                .iter()
                .map(|(_, a)| a.clone())
                .collect()
        }
    }

    impl crate::auth::authz::VaultPolicy for RecordingPolicy {
        fn decide(&self, actor: &Actor, vault_id: &str, _: Access) -> Decision {
            // **Via the accessor, not a match on the variant.** A key and a
            // session are the same principal reached two ways; a policy that
            // branched on the variant would need editing every time a new way
            // to hold a credential is added, which is the coupling A14.3
            // exists to avoid.
            let who = actor.user_id().to_string();
            self.seen
                .lock()
                .unwrap()
                .push(((who, vault_id.to_string()), actor.clone()));
            Decision::Allow
        }
    }

    /// One MCP `tools/call` over the real router.
    fn mcp_call(
        tool: &str,
        args: serde_json::Value,
        auth: &str,
    ) -> axum::http::Request<axum::body::Body> {
        let payload = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": { "name": tool, "arguments": args },
        });
        axum::http::Request::builder()
            .method("POST")
            .uri("/mcp")
            .header("content-type", "application/json")
            // Streamable HTTP requires the client to accept both, even when the
            // server answers in plain JSON.
            .header("accept", "application/json, text/event-stream")
            .header("authorization", auth)
            // rmcp rejects a request with no Host header outright (DNS-rebinding
            // defence). A real client always sends one; `oneshot` does not.
            .header("host", "localhost")
            .body(axum::body::Body::from(payload.to_string()))
            .unwrap()
    }

    #[tokio::test]
    async fn an_mcp_call_carries_the_authenticated_user() {
        // The gap this slice closes. An MCP tool used to resolve as a generic
        // `Actor::Mcp` with no user behind it, so the policy could not tell who
        // was asking — harmless while it allows everyone, an authorization
        // bypass the moment it does not.
        let dir = tempdir::TempDir::new("storm-mcp-identity").unwrap();
        let policy = Arc::new(RecordingPolicy::default());
        let (app, _, state) = test_router_with_policy(dir.path(), policy.clone());
        state
            .mcp_enabled
            .store(true, std::sync::atomic::Ordering::Relaxed);
        let owner = seed_owner(&state).await;
        let token = session_token(&state, &owner).await;
        register_vault(&state, "Notes").await;
        let vault = {
            let vaults = state.vaults.read().await;
            vaults.registry.vaults[0].id.clone()
        };

        let (status, body) = send(
            &app,
            mcp_call(
                "get_vault",
                serde_json::json!({ "vault": vault }),
                &format!("Bearer {token}"),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");

        assert_eq!(
            policy.pairs(),
            vec![(owner.clone(), vault)],
            "the tool must reach the policy as the logged-in user, not as an \
             anonymous MCP caller"
        );
    }

    // **`multi_thread` matters.** `#[tokio::test]` defaults to a current-thread
    // runtime, where "concurrent" tasks interleave only at await points on one
    // thread — and a mutation that stored the identity in a shared `Mutex`
    // instead of a per-task scope *passed* under it. Real parallelism is what
    // makes the leak observable.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_mcp_calls_do_not_share_an_identity() {
        let dir = tempdir::TempDir::new("storm-mcp-concurrent").unwrap();
        let policy = Arc::new(RecordingPolicy::default());
        let (app, _, state) = test_router_with_policy(dir.path(), policy.clone());
        state
            .mcp_enabled
            .store(true, std::sync::atomic::Ordering::Relaxed);

        let owner = seed_owner(&state).await;
        let session = format!("Bearer {}", session_token(&state, &owner).await);
        let key = {
            let now = crate::index::now_rfc3339();
            let mut auth_db = state.auth_db.lock().await;
            let (_, secret) =
                crate::auth::keys::create(&mut auth_db, &owner, "laptop", None, None, &now)
                    .unwrap();
            format!("Bearer {secret}")
        };

        register_vault(&state, "Session").await;
        register_vault(&state, "Key").await;
        let (session_vault, key_vault) = {
            let vaults = state.vaults.read().await;
            (
                vaults.registry.vaults[0].id.clone(),
                vaults.registry.vaults[1].id.clone(),
            )
        };

        let mut tasks = Vec::new();
        for i in 0..64 {
            let (vault, auth) = if i % 2 == 0 {
                (session_vault.clone(), session.clone())
            } else {
                (key_vault.clone(), key.clone())
            };
            let app = app.clone();
            tasks.push(tokio::spawn(async move {
                let (status, _) = send(
                    &app,
                    mcp_call("list_tags", serde_json::json!({ "vault": vault }), &auth),
                )
                .await;
                status
            }));
        }
        for task in tasks {
            assert_eq!(task.await.unwrap(), StatusCode::OK);
        }

        let actors = policy.actors();
        let pairs = policy.pairs();
        assert_eq!(pairs.len(), 64, "every call must have reached the policy");
        for (actor, (_, vault)) in actors.iter().zip(pairs) {
            let expected = match actor {
                Actor::Key { .. } => &key_vault,
                _ => &session_vault,
            };
            assert_eq!(
                &vault,
                expected,
                "a {} call was recorded against the other credential's vault — \
                 identity leaked between concurrent MCP requests",
                actor.describe()
            );
        }
    }

    #[tokio::test]
    async fn a_refused_vault_is_403_and_not_404() {
        // The refusal path for a caller `StormPolicy` never refuses. It is also the claim the whole seam rests on: changing
        // the policy is all it takes to change the answer.
        //
        // 403 specifically. Decision 25: "you may not see this" has to be
        // distinguishable from "your notes are gone", so never 404 and never
        // an empty success.
        let dir = tempdir::TempDir::new("storm-authz-deny").unwrap();
        let (app, _, state) = test_router_with_policy(dir.path(), Arc::new(DenyAll));
        let owner = seed_owner(&state).await;
        let token = session_token(&state, &owner).await;

        let (status, body) = send(
            &app,
            get_with_auth("/v1/vaults/any-vault/tree", &format!("Bearer {token}")),
        )
        .await;

        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
        assert_ne!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn a_refusal_does_not_reveal_whether_the_vault_exists() {
        // The policy is consulted before the registry, so a refusal cannot
        // double as a probe for which vault ids are real: a made-up id and a
        // real one have to answer identically.
        let dir = tempdir::TempDir::new("storm-authz-probe").unwrap();
        let (app, _, state) = test_router_with_policy(dir.path(), Arc::new(DenyAll));
        let owner = seed_owner(&state).await;
        let token = session_token(&state, &owner).await;
        let auth = format!("Bearer {token}");

        let (real, _) = send(&app, get_with_auth("/v1/vaults/nope/tree", &auth)).await;
        let (fake, _) = send(
            &app,
            get_with_auth("/v1/vaults/definitely-not-a-vault/tree", &auth),
        )
        .await;

        assert_eq!(real, StatusCode::FORBIDDEN);
        assert_eq!(
            fake,
            StatusCode::FORBIDDEN,
            "the two must be the same answer"
        );
    }

    #[tokio::test]
    async fn a_collection_filters_where_a_named_vault_refuses() {
        // The other half of the rule, and the one that is easy to get wrong by
        // making it consistent: there is no way to 403 half a list, and one
        // unreachable vault must not blank the dashboard.
        let dir = tempdir::TempDir::new("storm-authz-filter").unwrap();
        let (app, _, state) = test_router_with_policy(dir.path(), Arc::new(DenyAll));
        let owner = seed_owner(&state).await;
        let token = session_token(&state, &owner).await;
        let auth = format!("Bearer {token}");
        // A vault has to exist, or "the list is empty" is true whether or not
        // anything filtered — which is how this test first passed while doing
        // nothing at all.
        register_vault(&state, "Notes").await;

        let (status, body) = send(&app, get_with_auth("/v1/vaults", &auth)).await;
        assert_eq!(status, StatusCode::OK, "a list is filtered, never refused");
        assert_eq!(
            body["vaults"].as_array().map(|v| v.len()),
            Some(0),
            "the registered vault must be filtered out, not listed"
        );

        let (status, _) = send(&app, get_with_auth("/v1/recents", &auth)).await;
        assert_eq!(
            status,
            StatusCode::OK,
            "recents spans vaults; it filters too"
        );
    }

    /// Registers a vault so a collection test has something to filter.
    async fn register_vault(state: &Shared, name: &str) {
        let mut vaults = state.vaults.write().await;
        let entry = vaults
            .registry
            .create(name, "2026-08-17T00:00:00Z")
            .unwrap();
        // `create` already appends to the registry; pushing again double-counts.
        std::fs::create_dir_all(vaults.registry.path_of(&entry)).unwrap();
    }

    #[tokio::test]
    async fn a_collection_lists_what_the_shipped_policy_allows() {
        // The pair to the filtering test above. Without this one, "the list is
        // empty" could mean the filter works *or* that listing is broken.
        let dir = tempdir::TempDir::new("storm-authz-list").unwrap();
        let (app, _, state) = test_router_with_state(dir.path());
        let owner = seed_owner(&state).await;
        let token = session_token(&state, &owner).await;
        register_vault(&state, "Notes").await;

        let (status, body) = send(
            &app,
            get_with_auth("/v1/vaults", &format!("Bearer {token}")),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            body["vaults"].as_array().map(|v| v.len()),
            Some(1),
            "StormPolicy must not filter anything out"
        );
    }

    #[tokio::test]
    async fn the_shipped_policy_changes_nothing_for_an_ordinary_caller() {
        // The other direction, and the reason this slice is safe to merge:
        // with `StormPolicy` the boundary is invisible. A vault that is
        // simply absent still answers 404, not 403 — the seam did not turn
        // "no such vault" into "forbidden" for everyone.
        let dir = tempdir::TempDir::new("storm-authz-allow").unwrap();
        let (app, _, state) = test_router_with_state(dir.path());
        let owner = seed_owner(&state).await;
        let token = session_token(&state, &owner).await;

        let (status, _) = send(
            &app,
            get_with_auth("/v1/vaults/nope/tree", &format!("Bearer {token}")),
        )
        .await;

        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn a_rate_limited_login_says_how_long_to_wait() {
        // The protocol table maps rate limiting to 429 + Retry-After, and it is
        // the one login refusal that is not a 401: the remedy is to wait, not
        // to try different credentials. Driving `login_handler` to this state
        // costs five real Argon2 verifies at 192 MiB, so the contract is pinned
        // on the helper instead — the match arm that reaches it is one line.
        let response = rate_limited(240);

        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(
            response
                .headers()
                .get(axum::http::header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok()),
            Some("240"),
            "the header is the correct HTTP answer"
        );

        let bytes = axum::body::to_bytes(response.into_body(), 1 << 16)
            .await
            .unwrap();
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["error"], "rate_limited");
        assert_eq!(
            body["retry_after"], 240,
            "and the body is what the client renders — a message that says \
             'too many attempts' without saying for how long invites the retry \
             it is trying to stop"
        );
    }

    #[tokio::test]
    async fn the_server_endpoints_answer_without_a_token() {
        // The `none` tier. These have to work before the caller owns any
        // credential — a client cannot pair with a server it may not ask who it
        // is. axum applies a layer only to routes registered above it, so this
        // fails the moment they move up past `require_token`.
        let dir = tempdir::TempDir::new("storm-none-tier").unwrap();
        let (app, identity) = test_router(dir.path());

        let (status, body) = send(&app, get("/v1/server")).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["server_id"], identity.server_id);
        assert_eq!(body["key_id"], identity.key_id);
        assert_eq!(body["algorithm"], "ed25519");
        assert_eq!(body["public_key"], identity.public_key_b64());

        let (status, body) = send(
            &app,
            post_json(
                "/v1/server/challenge",
                serde_json::json!({ "nonce": "0123456789abcdef" }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(
            body["signature"],
            identity.sign_challenge("0123456789abcdef")
        );
    }

    #[tokio::test]
    async fn the_server_endpoints_publish_nothing_secret() {
        // The private key is a file precisely so it is never in a payload. The
        // check is on the bytes of the response, not on the field list, so a
        // future field carrying it is caught too.
        let dir = tempdir::TempDir::new("storm-no-secret").unwrap();
        let (app, identity) = test_router(dir.path());
        let key_bytes = std::fs::read(crate::auth::identity::key_path(
            &dir.path().join("state"),
            &identity.key_id,
        ))
        .unwrap();
        let secret_b64 = data_encoding::BASE64URL_NOPAD.encode(&key_bytes);

        let (_, info) = send(&app, get("/v1/server")).await;
        let (_, answer) = send(
            &app,
            post_json(
                "/v1/server/challenge",
                serde_json::json!({ "nonce": "0123456789abcdef" }),
            ),
        )
        .await;
        for body in [info.to_string(), answer.to_string()] {
            assert!(!body.contains(&secret_b64), "the private key is in {body}");
            assert!(
                !body.contains(&data_encoding::HEXLOWER.encode(&key_bytes)),
                "the private key is in {body}"
            );
        }
    }

    #[tokio::test]
    async fn an_ordinary_route_still_needs_a_credential() {
        // The other half of the pair: nothing that used to demand
        // authentication may have stopped demanding it. Without this, moving
        // routes around the auth layer could quietly open the whole surface.
        //
        // Since the cutover the credential is a **session**, not a shared
        // token — so this also pins that an ordinary route is reachable at all
        // once you have signed in, which is the thing the removal could most
        // plausibly have broken.
        let dir = tempdir::TempDir::new("storm-still-authed").unwrap();
        let (app, _, state) = test_router_with_state(dir.path());

        let (status, _) = send(&app, get("/v1/vaults")).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "no credential");

        let owner = seed_owner(&state).await;
        let token = session_token(&state, &owner).await;
        let (status, _) = send(
            &app,
            get_with_auth("/v1/vaults", &format!("Bearer {token}")),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "a real session must still work");
    }

    #[tokio::test]
    async fn a_challenge_nonce_is_validated() {
        let dir = tempdir::TempDir::new("storm-nonce").unwrap();
        let (app, _) = test_router(dir.path());

        let (status, _) = send(
            &app,
            post_json(
                "/v1/server/challenge",
                serde_json::json!({ "nonce": "tiny" }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        let (status, _) = send(
            &app,
            post_json("/v1/server/challenge", serde_json::json!({})),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "a body with no nonce is a malformed request, not a signature"
        );
    }

    #[test]
    fn a_storage_root_must_be_an_existing_writable_directory() {
        let dir = tempdir::TempDir::new("storm-root").unwrap();
        let state = dir.path().join("state");
        std::fs::create_dir_all(&state).unwrap();

        let good = dir.path().join("vaults");
        std::fs::create_dir_all(&good).unwrap();
        assert!(validate_root(&good, &state).is_ok());

        assert!(
            validate_root(FsPath::new("relative/path"), &state).is_err(),
            "relative"
        );
        assert!(
            validate_root(&dir.path().join("nope"), &state).is_err(),
            "missing"
        );

        let file = dir.path().join("notes.md");
        std::fs::write(&file, "x").unwrap();
        assert!(validate_root(&file, &state).is_err(), "a file");
    }

    #[test]
    fn a_root_inside_the_state_directory_is_refused() {
        // That would put a vault inside Storm's own state, inverting the
        // first invariant in CLAUDE.md.
        let dir = tempdir::TempDir::new("storm-inside").unwrap();
        let state = dir.path().join("state");
        let inside = state.join("vaults");
        std::fs::create_dir_all(&inside).unwrap();
        assert!(validate_root(&inside, &state).is_err());
    }

    #[test]
    fn a_root_that_contains_the_state_directory_is_allowed() {
        // The layout the `--vault` shim produces. `Registry::candidate_dirs`
        // skips `state_dir`, so it is never adopted as a vault — which is why
        // this direction does not need forbidding.
        let dir = tempdir::TempDir::new("storm-contains").unwrap();
        let root = dir.path().to_path_buf();
        let state = root.join("state");
        std::fs::create_dir_all(&state).unwrap();
        assert!(validate_root(&root, &state).is_ok());
    }

    // ---- the bootstrap window, and the gate in front of Argon2id ----------

    /// A paired device, which is what the device tier costs.
    ///
    /// Goes through the real pairing code rather than inserting a row, so a
    /// change to how a device credential is minted breaks these tests too.
    pub(crate) async fn pair_a_device(state: &Shared) -> String {
        let now = crate::index::now_rfc3339();
        let mut auth_db = state.auth_db.lock().await;
        let (nonce, _) = crate::auth::pairing::create(
            &mut auth_db,
            crate::auth::pairing::PairingPurpose::FirstUser,
            None,
            None,
            &now,
        )
        .unwrap();
        let paired = crate::auth::pairing::consume(
            &mut auth_db,
            &nonce,
            "test device",
            None,
            None,
            None,
            &now,
        )
        .unwrap();
        format!("StormDevice {}:{}", paired.device_id, paired.device_secret)
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

    #[tokio::test]
    async fn a_device_credential_is_refused_on_a_session_route() {
        // The tiers are a boundary, not a suggestion. A device credential
        // satisfied `require_auth` on session routes and was stopped only by
        // handlers failing to extract a `SessionAuth` nobody had inserted —
        // enforcement by accident, reported as `500`.
        let dir = tempdir::TempDir::new("storm-tier-device").unwrap();
        let (app, _, state) = test_router_with_state(dir.path());
        let device = pair_a_device(&state).await;

        for path in ["/v1/vaults", "/v1/config", "/v1/auth/sessions"] {
            let (status, _) = send(&app, get_with_auth(path, &device)).await;
            assert_eq!(
                status,
                StatusCode::UNAUTHORIZED,
                "{path} must refuse a device credential, not fail on it"
            );
        }

        // Device tier accepts it: login is refused on the credential, not the tier.
        let (status, _) = send(
            &app,
            post_json_with_auth(
                "/v1/auth/login",
                serde_json::json!({"password": "a-long-enough-password"}),
                &device,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    // ---- single user (decision 82) ---------------------------------------

    async fn device_and_owner(state: &Shared) -> (String, String) {
        let device = pair_a_device(state).await;
        let owner = seed_owner(state).await;
        (device, owner)
    }

    #[tokio::test]
    async fn the_multi_user_routes_are_gone() {
        let dir = tempdir::TempDir::new("storm-single-user-routes").unwrap();
        let (app, _, state) = test_router_with_state(dir.path());
        let (device, owner) = device_and_owner(&state).await;
        let token = format!("Bearer {}", session_token(&state, &owner).await);

        for auth in [&device, &token] {
            let (status, _) = send(&app, get_with_auth("/v1/users", auth)).await;
            assert!(
                status == StatusCode::NOT_FOUND || status == StatusCode::UNAUTHORIZED,
                "GET /v1/users answered {status}"
            );
        }
        let (status, _) = send(
            &app,
            post_json_with_auth(
                "/v1/users",
                serde_json::json!({"username": "newcomer", "password": "a-long-enough-password"}),
                &device,
            ),
        )
        .await;
        assert!(!status.is_success(), "POST /v1/users answered {status}");
        let (status, _) = send(&app, get_with_auth("/v1/auth/registration", &device)).await;
        assert!(
            !status.is_success(),
            "GET /v1/auth/registration answered {status}"
        );
        let (status, _) = send(
            &app,
            put_json(
                "/v1/config/registration",
                serde_json::json!({"enabled": true}),
                Some(&token),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "PUT /v1/config/registration");

        let auth_db = state.auth_db.lock().await;
        assert_eq!(
            auth_db.account().unwrap().unwrap().id,
            owner,
            "nothing created a second person"
        );
    }

    #[tokio::test]
    async fn setup_takes_a_password_and_happens_once() {
        let dir = tempdir::TempDir::new("storm-setup-once").unwrap();
        let (app, _, state) = test_router_with_state(dir.path());
        let device = pair_a_device(&state).await;

        let (status, _) = send(
            &app,
            post_json_with_auth(
                "/v1/users/first",
                serde_json::json!({"password": "a-long-enough-password"}),
                &device,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);

        for body in [
            serde_json::json!({"password": "another-long-password"}),
            serde_json::json!({"username": "another", "password": "another-long-password"}),
        ] {
            let (status, _) =
                send(&app, post_json_with_auth("/v1/users/first", body, &device)).await;
            assert_eq!(status, StatusCode::CONFLICT, "still one-shot");
        }
    }

    #[tokio::test]
    async fn a_paired_device_can_ask_whether_setup_happened() {
        let dir = tempdir::TempDir::new("storm-account-state").unwrap();
        let (app, _, state) = test_router_with_state(dir.path());
        let (status, _) = send(&app, get("/v1/account")).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        let device = pair_a_device(&state).await;
        let (status, body) = send(&app, get_with_auth("/v1/account", &device)).await;
        assert_eq!(
            (status, body["exists"].clone()),
            (StatusCode::OK, false.into())
        );
        seed_owner(&state).await;
        let (_, body) = send(&app, get_with_auth("/v1/account", &device)).await;
        assert_eq!(body, serde_json::json!({"exists": true}));
    }

    #[tokio::test]
    async fn an_old_clients_setup_body_still_sets_up() {
        let dir = tempdir::TempDir::new("storm-setup-old-client").unwrap();
        let (app, _, state) = test_router_with_state(dir.path());
        let device = pair_a_device(&state).await;
        let (status, _) = send(
            &app,
            post_json_with_auth(
                "/v1/users/first",
                serde_json::json!({"username": "dewansh", "password": "a-long-enough-password"}),
                &device,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        assert!(state.auth_db.lock().await.has_account().unwrap());
    }

    #[tokio::test]
    async fn a_migrated_database_serves_the_survivor_and_refuses_the_rest() {
        use crate::auth::single_user::tests::{
            access_token, device_secret, fake_hash, key_secret, refresh_token, v5_fixture,
        };
        let fixture = v5_fixture(&fake_hash());
        let dir = tempdir::TempDir::new("storm-migrated-serves").unwrap();
        std::fs::create_dir_all(dir.path().join("state")).unwrap();
        std::fs::copy(&fixture.path, dir.path().join("state/auth.db")).unwrap();
        let (app, _, state) = test_router_with_state(dir.path());
        state
            .mcp_enabled
            .store(true, std::sync::atomic::Ordering::Relaxed);

        let survivor = format!("Bearer {}", access_token("ses_s1"));
        let (status, _) = send(&app, get_with_auth("/v1/vaults", &survivor)).await;
        assert_eq!(
            status,
            StatusCode::OK,
            "the account's old token must keep working"
        );
        let shared = format!("Bearer {}", access_token("ses_s3"));
        let (status, _) = send(&app, get_with_auth("/v1/vaults", &shared)).await;
        assert_eq!(status, StatusCode::OK);
        let revoked = format!("Bearer {}", access_token("ses_s2"));
        let (status, _) = send(&app, get_with_auth("/v1/vaults", &revoked)).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);

        let (status, body) = send(
            &app,
            post_json_with_auth(
                "/v1/auth/refresh",
                serde_json::json!({"refresh_token": refresh_token("ses_s1")}),
                &format!(
                    "StormDevice dev_survivor1:{}",
                    device_secret("dev_survivor1")
                ),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");

        let (status, _) = send(
            &app,
            mcp_request(&format!("Bearer {}", key_secret("key_s1"))),
        )
        .await;
        assert_ne!(status, StatusCode::UNAUTHORIZED);
        let (status, _) = send(
            &app,
            mcp_request(&format!("Bearer {}", key_secret("key_s2"))),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);

        for session in ["ses_a1", "ses_b1", "ses_n1"] {
            let (status, _) = send(
                &app,
                get_with_auth("/v1/vaults", &format!("Bearer {}", access_token(session))),
            )
            .await;
            assert_eq!(status, StatusCode::UNAUTHORIZED, "{session}");
        }
        for key in ["key_a1", "key_n1"] {
            let (status, _) = send(&app, mcp_request(&format!("Bearer {}", key_secret(key)))).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED, "{key}");
        }
        let (status, _) = send(
            &app,
            post_json_with_auth(
                "/v1/auth/login",
                serde_json::json!({"password": "anything at all here"}),
                &format!("StormDevice dev_member_a:{}", device_secret("dev_member_a")),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn an_old_clients_login_body_still_signs_in() {
        let dir = tempdir::TempDir::new("storm-old-login").unwrap();
        let (app, _, state) = test_router_with_state(dir.path());
        let device = pair_a_device(&state).await;
        let (status, _) = send(
            &app,
            post_json_with_auth(
                "/v1/users/first",
                serde_json::json!({"password": "a-long-enough-password"}),
                &device,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);

        for body in [
            serde_json::json!({"username": "whatever-v0.3-had", "password": "a-long-enough-password"}),
            serde_json::json!({"password": "a-long-enough-password"}),
        ] {
            let (status, issued) = send(
                &app,
                post_json_with_auth("/v1/auth/login", body.clone(), &device),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{body} → {issued}");
        }
        let (status, _) = send(
            &app,
            post_json_with_auth(
                "/v1/auth/login",
                serde_json::json!({"username": "whatever", "password": "not-the-password!"}),
                &device,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn the_account_changes_server_config() {
        let dir = tempdir::TempDir::new("storm-config-account").unwrap();
        let (app, _, state) = test_router_with_state(dir.path());
        let (_, owner) = device_and_owner(&state).await;
        let token = format!("Bearer {}", session_token(&state, &owner).await);
        for (path, body) in [
            (
                "/v1/config/mcp",
                serde_json::json!({"enabled": true, "writable": false}),
            ),
            ("/v1/config/relays", serde_json::json!({"relays": []})),
        ] {
            let (status, _) = send(&app, put_json(path, body, Some(&token))).await;
            assert_eq!(status, StatusCode::OK, "{path}");
        }
    }

    // ---- web bootstrap (slice 15) ----------------------------------------

    /// Mints a web-bootstrap nonce for `peer`, the way the index handler does.
    async fn web_nonce(state: &Shared, peer: &str) -> String {
        let now = crate::index::now_rfc3339();
        let mut auth_db = state.auth_db.lock().await;
        let (nonce, _) = crate::auth::pairing::create(
            &mut auth_db,
            crate::auth::pairing::PairingPurpose::WebBootstrap,
            None,
            Some(peer),
            &now,
        )
        .unwrap();
        nonce
    }

    fn pair_from(nonce: &str, peer: &str) -> axum::http::Request<axum::body::Body> {
        let mut req = pair_without_peer(nonce);
        req.extensions_mut().insert(axum::extract::ConnectInfo(
            format!("{peer}:54321")
                .parse::<std::net::SocketAddr>()
                .unwrap(),
        ));
        req
    }

    /// A pairing request with no `ConnectInfo` extension — what the relay's
    /// in-process dispatch produces, since there is no socket behind it.
    fn pair_without_peer(nonce: &str) -> axum::http::Request<axum::body::Body> {
        post_json(
            "/v1/pair",
            serde_json::json!({"n": nonce, "name": "browser", "platform": "web"}),
        )
    }

    #[tokio::test]
    async fn an_unbound_nonce_pairs_with_no_connect_info_at_all() {
        // The extractor trap, on the first route a remote client touches. A
        // bare `ConnectInfo` rejects before the handler runs, so a relayed
        // pairing would fail as a malformed request rather than answering.
        // A QR nonce is unbound precisely so it can be carried elsewhere, so
        // "elsewhere" including a tunnel has to work.
        let dir = tempdir::TempDir::new("storm-pair-nopeer").unwrap();
        let (app, _, state) = test_router_with_state(dir.path());

        // `AddDevice` with no peer is the QR case: unbound on purpose.
        let nonce = {
            let mut auth_db = state.auth_db.lock().await;
            let (nonce, _) = crate::auth::pairing::create(
                &mut auth_db,
                crate::auth::pairing::PairingPurpose::AddDevice,
                None,
                None,
                &crate::index::now_rfc3339(),
            )
            .unwrap();
            nonce
        };

        let (status, body) = send(&app, pair_without_peer(&nonce)).await;
        assert_eq!(
            status,
            StatusCode::OK,
            "an unbound nonce must pair with no socket behind the request: {body:?}"
        );
        assert!(body["device_id"].as_str().unwrap().starts_with("dev_"));
    }

    #[tokio::test]
    async fn a_peer_bound_nonce_is_refused_when_there_is_no_peer() {
        // Not a gap — the correct refusal. A web-bootstrap nonce is bound to
        // the browser that fetched the page directly; there is nothing for it
        // to match against over a tunnel, and inventing a peer would sometimes
        // coincidentally pass.
        let dir = tempdir::TempDir::new("storm-pair-bound-nopeer").unwrap();
        let (app, _, state) = test_router_with_state(dir.path());

        let nonce = web_nonce(&state, "192.168.1.20").await;
        let (status, _) = send(&app, pair_without_peer(&nonce)).await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "a bound nonce with no peer is wrong_peer, not a crash and not a pass"
        );
    }

    #[tokio::test]
    async fn a_web_bootstrap_nonce_pairs_a_real_device() {
        // The whole claim of the design: what comes back is an ordinary device,
        // not a web-shaped special case.
        let dir = tempdir::TempDir::new("storm-wb-device").unwrap();
        let (app, _, state) = test_router_with_state(dir.path());

        let nonce = web_nonce(&state, "192.168.1.20").await;
        let (status, body) = send(&app, pair_from(&nonce, "192.168.1.20")).await;
        assert_eq!(status, StatusCode::OK, "{body:?}");

        let device_id = body["device_id"].as_str().unwrap().to_string();
        let secret = body["device_secret"].as_str().unwrap().to_string();
        assert!(device_id.starts_with("dev_"));

        // A device-tier credential: it can set up this fresh Storm.
        let auth = format!("StormDevice {device_id}:{secret}");
        let (status, _) = send(
            &app,
            post_json_with_auth(
                "/v1/users/first",
                serde_json::json!({"password": "a-long-enough-password"}),
                &auth,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
    }

    #[tokio::test]
    async fn a_web_bootstrap_nonce_is_single_use() {
        let dir = tempdir::TempDir::new("storm-wb-once").unwrap();
        let (app, _, state) = test_router_with_state(dir.path());
        let nonce = web_nonce(&state, "192.168.1.20").await;

        let (first, _) = send(&app, pair_from(&nonce, "192.168.1.20")).await;
        assert_eq!(first, StatusCode::OK);

        let (second, body) = send(&app, pair_from(&nonce, "192.168.1.20")).await;
        assert_eq!(second, StatusCode::CONFLICT, "a replay must not pair again");
        assert_eq!(body["error"], "pairing_consumed");
    }

    #[tokio::test]
    async fn a_web_bootstrap_nonce_is_bound_to_the_peer_it_was_issued_to() {
        // The control that makes "anyone who can fetch the page gets a nonce"
        // survivable: one scraped from a log or a shared screen is not
        // spendable from anywhere else.
        let dir = tempdir::TempDir::new("storm-wb-peer").unwrap();
        let (app, _, state) = test_router_with_state(dir.path());
        let nonce = web_nonce(&state, "192.168.1.20").await;

        let (status, body) = send(&app, pair_from(&nonce, "192.168.1.99")).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body:?}");
        assert_eq!(body["error"], "pairing_wrong_peer");

        // Still spendable by the peer it belongs to — a refusal must not burn
        // the nonce for its rightful owner.
        let (status, _) = send(&app, pair_from(&nonce, "192.168.1.20")).await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn an_ipv4_mapped_peer_matches_its_plain_form() {
        // A dual-stack listener reports an IPv4 client as ::ffff:192.168.1.20
        // on one connection and 192.168.1.20 on another. Textually different,
        // the same machine — and comparing as strings would refuse the
        // legitimate client.
        let dir = tempdir::TempDir::new("storm-wb-v6").unwrap();
        let (app, _, state) = test_router_with_state(dir.path());
        let nonce = web_nonce(&state, "::ffff:192.168.1.20").await;

        let (status, body) = send(&app, pair_from(&nonce, "192.168.1.20")).await;
        assert_eq!(status, StatusCode::OK, "{body:?}");
    }

    #[tokio::test]
    async fn a_qr_nonce_stays_unbound() {
        // Native pairing must be unchanged: a QR is carried across the room to
        // a *different* device, so binding it to the issuing peer would break
        // the only flow it has.
        let dir = tempdir::TempDir::new("storm-wb-qr").unwrap();
        let (app, _, state) = test_router_with_state(dir.path());

        let nonce = {
            let now = crate::index::now_rfc3339();
            let mut auth_db = state.auth_db.lock().await;
            crate::auth::pairing::create(
                &mut auth_db,
                crate::auth::pairing::PairingPurpose::FirstUser,
                None,
                None,
                &now,
            )
            .unwrap()
            .0
        };

        let (status, body) = send(&app, pair_from(&nonce, "10.0.0.7")).await;
        assert_eq!(status, StatusCode::OK, "{body:?}");
    }

    #[tokio::test]
    async fn an_expired_web_bootstrap_nonce_is_refused() {
        let dir = tempdir::TempDir::new("storm-wb-exp").unwrap();
        let (app, _, state) = test_router_with_state(dir.path());

        // Created in the past by more than its TTL.
        let nonce = {
            let past = "2020-01-01T00:00:00Z";
            let mut auth_db = state.auth_db.lock().await;
            crate::auth::pairing::create(
                &mut auth_db,
                crate::auth::pairing::PairingPurpose::WebBootstrap,
                None,
                Some("192.168.1.20"),
                past,
            )
            .unwrap()
            .0
        };

        let (status, body) = send(&app, pair_from(&nonce, "192.168.1.20")).await;
        assert_eq!(status, StatusCode::GONE, "{body:?}");
    }

    #[tokio::test]
    async fn web_bootstrap_does_not_open_the_device_tier_to_everyone() {
        // The line this slice must not cross. Bootstrap hands out a *pairing
        // nonce*, never a session and never an exemption: without a device
        // credential the device tier still refuses.
        let dir = tempdir::TempDir::new("storm-wb-tier").unwrap();
        let (app, _, _) = test_router_with_state(dir.path());

        let (status, _) = send(&app, get("/v1/users")).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);

        let (status, _) = send(
            &app,
            post_json(
                "/v1/auth/login",
                serde_json::json!({"username": "dewansh", "password": "a-long-enough-password"}),
            ),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "login must still demand a StormDevice credential"
        );
    }

    // ---- login rate limiting ----------------------------------------------

    /// A login request, optionally carrying a `ConnectInfo` extension the way
    /// the real service does (`into_make_service_with_connect_info`). Tests
    /// that omit it exercise exactly what the relay's in-process dispatch
    /// produces: no socket.
    fn login_from(
        device: &str,
        username: &str,
        peer: Option<&str>,
    ) -> axum::http::Request<axum::body::Body> {
        let mut req = post_json_with_auth(
            "/v1/auth/login",
            serde_json::json!({"username": username, "password": "a-long-enough-password"}),
            device,
        );
        if let Some(peer) = peer {
            req.extensions_mut().insert(axum::extract::ConnectInfo(
                format!("{peer}:54321")
                    .parse::<std::net::SocketAddr>()
                    .unwrap(),
            ));
        }
        req
    }

    #[tokio::test]
    async fn login_without_a_connect_info_extension_still_reaches_the_handler() {
        // The extractor trap, as a regression test. A bare `ConnectInfo`
        // extractor rejects before the handler runs and the client sees a
        // 500-shaped error; the relay's in-process dispatch sends requests
        // with no socket, so this must keep working — bounded by the
        // `Unattributed` bucket, not refused outright.
        let dir = tempdir::TempDir::new("storm-login-nopeer").unwrap();
        let (app, _, state) = test_router_with_state(dir.path());
        let device = pair_a_device(&state).await;

        let (status, _) = send(&app, login_from(&device, "nobody", None)).await;
        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "the handler must run and answer 401 for unknown credentials, not fail extraction"
        );
    }
    /// A limiter whose per-caller burst is two, so these tests spend two
    /// Argon2id verifies instead of thirty. The global bucket is left roomy —
    /// each test here is about the per-caller half, and a global trip would
    /// mask it.
    pub(crate) fn throttling_limiter() -> crate::auth::ratelimit::LoginLimiter {
        use crate::auth::ratelimit::{Limits, LoginLimiter, tests::TEST_LIMITS};
        LoginLimiter::with_limits(TEST_LIMITS, Limits::per_minute(1_000.0, 1_000.0))
    }

    #[tokio::test]
    async fn a_burst_of_logins_from_one_caller_is_throttled_while_another_is_not() {
        // The per-caller bucket trips first: one address flooding cannot lock
        // anybody else out, which is the whole point of splitting the buckets.
        // Junk usernames deliberately — they never trip the per-user lockout,
        // which is why the limiter exists at all.
        let dir = tempdir::TempDir::new("storm-login-burst").unwrap();
        let (app, _, state) = test_router_with_limiter(dir.path(), throttling_limiter());
        let device = pair_a_device(&state).await;

        for _ in 0..crate::auth::ratelimit::tests::TEST_BURST {
            let (status, _) = send(&app, login_from(&device, "nobody", Some("10.0.0.1"))).await;
            assert_eq!(
                status,
                StatusCode::UNAUTHORIZED,
                "the burst itself fits the budget and reaches the credential check"
            );
        }

        // The next attempt from the flooded address is a 429 with Retry-After,
        // reusing the per-user-lockout shape rather than inventing a second one.
        let response = app
            .clone()
            .oneshot(login_from(&device, "nobody", Some("10.0.0.1")))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        let retry_after = response
            .headers()
            .get(axum::http::header::RETRY_AFTER)
            .expect("Retry-After header")
            .to_str()
            .unwrap()
            .parse::<i64>()
            .unwrap();
        assert!(retry_after >= 1);

        // ...while a different address still gets a credential check (401),
        // not a throttle (429).
        let (status, _) = send(&app, login_from(&device, "nobody", Some("10.0.0.2"))).await;
        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "another caller must not pay for the flood"
        );
    }

    #[tokio::test]
    async fn a_throttled_login_is_recorded_with_its_remote_address() {
        let dir = tempdir::TempDir::new("storm-login-audit").unwrap();
        let (app, _, state) = test_router_with_limiter(dir.path(), throttling_limiter());
        let device = pair_a_device(&state).await;

        for _ in 0..=crate::auth::ratelimit::tests::TEST_BURST {
            send(&app, login_from(&device, "nobody", Some("10.9.9.9"))).await;
        }

        let auth_db = state.auth_db.lock().await;
        let kind = crate::auth::sessions::EVENT_LOGIN_THROTTLED;
        assert!(
            auth_db.event_count(kind).unwrap() >= 1,
            "a throttle refusal leaves an audit trail"
        );
        assert_eq!(
            auth_db.latest_event_remote(kind).unwrap().as_deref(),
            Some("10.9.9.9"),
            "the offending address is auditable"
        );
    }

    #[test]
    fn the_bootstrap_tag_goes_into_the_document() {
        let html = "<html><head><title>Storm</title></head><body></body></html>";
        let out = inject_bootstrap(html, "NONCE", "2026-01-01T00:00:00Z");
        assert!(out.contains(r#"<meta name="storm-bootstrap" content="NONCE""#));
        assert!(out.find("storm-bootstrap").unwrap() < out.find("</head>").unwrap());

        // A document with no head still has to work rather than lose the tag.
        let headless = inject_bootstrap("<body>hi</body>", "N", "E");
        assert!(headless.contains("storm-bootstrap"));
    }

    #[test]
    fn injected_values_cannot_break_out_of_the_attribute() {
        // The nonce is base64url and cannot contain a quote today. That is a
        // property of code somewhere else, and this is the line where trusting
        // it would become an injected attribute.
        let out = inject_bootstrap("<head></head>", r#"" onload="x"#, "E");
        assert!(!out.contains(r#"content="" onload="#));
        assert!(out.contains("&quot;"));
    }

    #[tokio::test]
    async fn a_device_tier_handler_can_take_the_auth_db_lock() {
        // The most expensive kind of bug to find and the cheapest to assert.
        //
        // `require_auth`'s device branch held the `auth_db` guard across
        // `next.run(request)`. `tokio::sync::Mutex` is not reentrant, so every
        // device-tier handler that takes the lock — login, refresh, `users`,
        // `users/first`, i.e. all of them — hung forever. Worse, the wedged
        // task never released the mutex, so every later request needing
        // `auth_db` blocked behind it: **one login attempt took the whole
        // server's authentication down until restart.**
        //
        // The session branch had always dropped the guard first. Nothing caught
        // the device branch because no test used a device credential at all.
        //
        // **Every device-tier route, not just one.** The bug was in the
        // middleware, so it took all four down together; a test that covers one
        // of them would pass over a regression reintroduced for the others (a
        // per-route `drop`, say, instead of a scoped guard).
        //
        // Each request below is chosen to *reach* its handler's lock. That
        // matters for `users/first`, which validates the password before
        // locking: a short password would return 422 without ever touching the
        // mutex, and the test would prove nothing.
        let dir = tempdir::TempDir::new("storm-device-deadlock").unwrap();
        let (app, _, state) = test_router_with_state(dir.path());
        let device = pair_a_device(&state).await;
        seed_owner(&state).await;

        let cases: Vec<(&str, axum::http::Request<axum::body::Body>)> = vec![
            ("GET /v1/users", get_with_auth("/v1/users", &device)),
            (
                // An account exists, so this reaches the lock and is refused by
                // the bootstrap-window check — without paying for a hash.
                "POST /v1/users/first",
                post_json_with_auth(
                    "/v1/users/first",
                    serde_json::json!({"username": "someone", "password": "a-long-enough-password"}),
                    &device,
                ),
            ),
            (
                "POST /v1/auth/login",
                post_json_with_auth(
                    "/v1/auth/login",
                    serde_json::json!({"username": "nobody", "password": "a-long-enough-password"}),
                    &device,
                ),
            ),
            (
                "POST /v1/auth/refresh",
                post_json_with_auth(
                    "/v1/auth/refresh",
                    serde_json::json!({"refresh_token": "not-a-real-token"}),
                    &device,
                ),
            ),
        ];

        for (name, request) in cases {
            let answered =
                tokio::time::timeout(std::time::Duration::from_secs(20), send(&app, request)).await;
            let (status, _) = answered.unwrap_or_else(|_| {
                panic!(
                    "{name} deadlocked: the middleware is holding the auth_db \
                     lock across next.run()"
                )
            });
            // Which status is not the point — that the handler *answered at
            // all* is. A 401 or 409 here still proves it reached the lock and
            // came back.
            assert!(
                status != StatusCode::INTERNAL_SERVER_ERROR,
                "{name} answered {status}"
            );

            // And the lock is free again afterwards. This is the half that
            // turns one hung request into a server-wide outage: the wedged task
            // never released the mutex, so everything behind it queued forever.
            assert!(
                state.auth_db.try_lock().is_ok(),
                "{name} left the auth_db mutex held"
            );
        }
    }

    #[tokio::test]
    async fn the_first_user_endpoint_closes_after_the_first_user() {
        // `POST /v1/users/first` used to refuse only a *duplicate username* —
        // `create_user`'s check — so a paired device could pick an unused name
        // and get another account. This handler hardcodes `Role::Owner`, so
        // every one of those would be an owner: on a server with more than one
        // user that is privilege escalation, a member's device minting an owner
        // and logging into it. A8 calls this a one-shot bootstrap window.
        let dir = tempdir::TempDir::new("storm-first-user").unwrap();
        let (app, _, state) = test_router_with_state(dir.path());
        let device = pair_a_device(&state).await;

        let (status, _) = send(
            &app,
            post_json_with_auth(
                "/v1/users/first",
                serde_json::json!({"username": "dewansh", "password": "a-long-enough-password"}),
                &device,
            ),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::CREATED,
            "the bootstrap window opens once"
        );

        // A *different* username, so a duplicate-name refusal cannot be what
        // makes this pass — which is exactly how the hole stayed open.
        let (status, body) = send(
            &app,
            post_json_with_auth(
                "/v1/users/first",
                serde_json::json!({"username": "someone-else", "password": "a-long-enough-password"}),
                &device,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "setup happens once");
        assert_eq!(body["error"], "an account already exists");

        let auth_db = state.auth_db.lock().await;
        assert!(
            auth_db.has_account().unwrap(),
            "the refusal has to be a refusal, not a 409 after the insert"
        );

        // And the refusal came *before* Argon2id. A caller who can make the
        // server hash on a request it was always going to reject can hold the
        // login path down for everyone: two permits, 192 MiB each.
        assert_eq!(
            state.hasher.jobs_run(),
            1,
            "only the accepted request should have paid for a hash"
        );
    }

    #[tokio::test]
    async fn every_password_hash_goes_through_the_one_shared_hasher() {
        // `Hasher`'s own documentation states the condition: *the bound is only
        // a bound if every caller goes through the same one.* Each handler
        // called `Hasher::new()`, minting a fresh pair of permits per request —
        // so the semaphore bounded one request against itself, and nothing
        // against anything else.
        //
        // Latent rather than live: these handlers hold the `auth_db` mutex
        // across the KDF, so hashes were serialized anyway. This test exists so
        // that when someone narrows that lock scope — and they should — the
        // documented bound is the one still standing.
        //
        // Counting jobs on the state's hasher is what distinguishes the two:
        // with a per-request hasher this count stays at zero however many
        // requests hash.
        let dir = tempdir::TempDir::new("storm-shared-hasher").unwrap();
        let (app, _, state) = test_router_with_state(dir.path());
        let device = pair_a_device(&state).await;

        assert_eq!(state.hasher.jobs_run(), 0);

        // A login for a user who does not exist still pays for a hash — that is
        // deliberate, so response time does not answer "does this account
        // exist". It also means these need no accounts to set up.
        for _ in 0..2 {
            let (status, _) = send(
                &app,
                post_json_with_auth(
                    "/v1/auth/login",
                    serde_json::json!({"username": "nobody", "password": "a-long-enough-password"}),
                    &device,
                ),
            )
            .await;
            assert_eq!(status, StatusCode::UNAUTHORIZED);
        }

        assert_eq!(
            state.hasher.jobs_run(),
            2,
            "every hash must go through the process-wide hasher, or the \
             semaphore bounds nothing across requests"
        );
    }

    // ---- the cutover: there is no shared token ---------------------------

    #[tokio::test]
    async fn no_shared_token_opens_anything() {
        // **The property the cutover exists to create, asserted directly.**
        // `testtoken` was owner-equivalent on every session-tier route until
        // this release. Nothing accepts it now, and nothing should ever accept
        // a bare string again — the only credentials are a paired device, a
        // session, and an MCP key, and all three are minted per-caller and
        // individually revocable.
        //
        // Written as a loop over plausible guesses rather than one value,
        // because what this guards against is someone reintroducing a constant
        // compare, not someone reintroducing this exact string.
        let dir = tempdir::TempDir::new("storm-no-backdoor").unwrap();
        let (app, _, state) = test_router_with_state(dir.path());
        state
            .mcp_enabled
            .store(true, std::sync::atomic::Ordering::Relaxed);
        register_vault(&state, "Notes").await;

        for guess in ["testtoken", "change-me", "storm", "admin"] {
            for (method, uri) in [
                ("GET", "/v1/vaults"),
                ("GET", "/v1/config"),
                ("GET", "/v1/recents"),
                ("GET", "/v1/keys"),
            ] {
                let (status, _) = send(
                    &app,
                    axum::http::Request::builder()
                        .method(method)
                        .uri(uri)
                        .header("Authorization", format!("Bearer {guess}"))
                        .body(axum::body::Body::empty())
                        .unwrap(),
                )
                .await;
                assert_eq!(
                    status,
                    StatusCode::UNAUTHORIZED,
                    "{method} {uri} accepted the bare token {guess:?}"
                );
            }

            // And the MCP surface, which the shared token used to reach.
            let response = app
                .clone()
                .oneshot(mcp_request(&format!("Bearer {guess}")))
                .await
                .unwrap();
            assert_eq!(
                response.status(),
                StatusCode::UNAUTHORIZED,
                "/mcp accepted the bare token {guess:?}"
            );
        }
    }

    #[tokio::test]
    async fn a_server_with_no_users_still_refuses_everything() {
        // The state prod is in the moment it upgrades: a fresh `auth.db`, no
        // users, no devices. Before the cutover the shared token was the way
        // in; now there is **no network route to authentication at all**, and
        // that is deliberate (A8) — bootstrapping is the console pairing nonce
        // or `storm-server user add`, both of which need shell access.
        let dir = tempdir::TempDir::new("storm-empty-server").unwrap();
        let (app, _, _state) = test_router_with_state(dir.path());

        for uri in ["/v1/vaults", "/v1/config", "/v1/keys", "/v1/users"] {
            let (status, _) = send(&app, get_with_auth(uri, "Bearer testtoken")).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED, "{uri}");
        }

        // The two `none`-tier routes still answer, because pinning a server's
        // identity is what a client does *before* it has any credential.
        let (status, _) = send(
            &app,
            axum::http::Request::builder()
                .uri("/v1/server")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
    }

    #[test]
    fn a_registry_written_before_the_cutover_still_loads() {
        // An upgraded server's `vaults.json` still carries
        // `legacy_token_enabled`. serde ignores unknown fields, so the file
        // loads and the setting simply no longer exists — an upgrade must not
        // fail on a key we stopped caring about.
        let dir = tempdir::TempDir::new("storm-old-registry").unwrap();
        std::fs::write(
            dir.path().join("vaults.json"),
            r#"{"root":"/tmp/vaults","vaults":[],"legacy_token_enabled":true,"mcp_enabled":true}"#,
        )
        .unwrap();

        let registry = Registry::load(dir.path(), FsPath::new("/tmp/vaults")).unwrap();
        assert!(registry.mcp_enabled, "the fields we kept must survive");
    }
    // ---- A14: MCP keys ---------------------------------------------------

    async fn seed_key(state: &Shared) -> (String, String) {
        let mut db = state.auth_db.lock().await;
        let account = match db.account().unwrap() {
            Some(account) => account,
            None => crate::auth::account::create_account(&mut db, "hash", "2026-08-19T12:00:00Z")
                .unwrap(),
        };
        let (_, secret) = crate::auth::keys::create(
            &mut db,
            &account.id,
            "a machine",
            None,
            None,
            "2026-08-19T12:00:00Z",
        )
        .unwrap();
        (account.id, secret)
    }

    /// A minimal, valid MCP request — enough to get past the tier check and
    /// see whether the surface answered at all.
    fn mcp_request(bearer: &str) -> axum::http::Request<axum::body::Body> {
        axum::http::Request::builder()
            .method("POST")
            .uri("/mcp")
            .header("Authorization", bearer)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream")
            .body(axum::body::Body::from(
                serde_json::json!({
                    "jsonrpc": "2.0", "id": 1, "method": "tools/list"
                })
                .to_string(),
            ))
            .unwrap()
    }

    #[tokio::test]
    async fn an_mcp_key_reaches_mcp() {
        let dir = tempdir::TempDir::new("storm-key-mcp").unwrap();
        let (app, _, state) = test_router_with_state(dir.path());
        state
            .mcp_enabled
            .store(true, std::sync::atomic::Ordering::Relaxed);
        let (_, secret) = seed_key(&state).await;

        let response = app
            .clone()
            .oneshot(mcp_request(&format!("Bearer {secret}")))
            .await
            .unwrap();
        assert_ne!(
            response.status(),
            StatusCode::UNAUTHORIZED,
            "a live key must authenticate on /mcp"
        );
    }

    #[tokio::test]
    async fn an_mcp_key_is_refused_everywhere_but_mcp() {
        // A14.2. And **the status matters as much as the refusal**: it has to
        // be the tier check saying no, not a handler falling over on a missing
        // extension, which is how the device tier used to answer 500 where 401
        // was the truth.
        let dir = tempdir::TempDir::new("storm-key-scope").unwrap();
        let (app, _, state) = test_router_with_state(dir.path());
        let (_, secret) = seed_key(&state).await;

        for (method, uri) in [
            ("GET", "/v1/vaults"),
            ("GET", "/v1/config"),
            ("GET", "/v1/recents"),
            ("GET", "/v1/keys"),
        ] {
            let (status, _) = send(
                &app,
                axum::http::Request::builder()
                    .method(method)
                    .uri(uri)
                    .header("Authorization", format!("Bearer {secret}"))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await;
            assert_eq!(
                status,
                StatusCode::UNAUTHORIZED,
                "{method} {uri} must refuse an MCP key with 401, not {status}"
            );
        }
    }

    #[tokio::test]
    async fn a_revoked_key_stops_reaching_mcp() {
        let dir = tempdir::TempDir::new("storm-key-revoked").unwrap();
        let (app, _, state) = test_router_with_state(dir.path());
        state
            .mcp_enabled
            .store(true, std::sync::atomic::Ordering::Relaxed);
        let (user_id, secret) = seed_key(&state).await;

        // Works first, so the refusal below cannot be a setup failure.
        let before = app
            .clone()
            .oneshot(mcp_request(&format!("Bearer {secret}")))
            .await
            .unwrap();
        assert_ne!(before.status(), StatusCode::UNAUTHORIZED);

        {
            let mut db = state.auth_db.lock().await;
            let key = db.api_keys_for_user(&user_id).unwrap().pop().unwrap();
            crate::auth::keys::revoke(&mut db, &key.id, None, "test", "2026-08-19T13:00:00Z")
                .unwrap();
        }

        let after = app
            .clone()
            .oneshot(mcp_request(&format!("Bearer {secret}")))
            .await
            .unwrap();
        assert_eq!(
            after.status(),
            StatusCode::UNAUTHORIZED,
            "revocation must take effect on the next request, not the next restart"
        );
    }

    #[tokio::test]
    async fn a_device_credential_is_refused_on_mcp() {
        // MCP is not the device tier, and the refusal must be the tier check.
        let dir = tempdir::TempDir::new("storm-key-device").unwrap();
        let (app, _, state) = test_router_with_state(dir.path());
        state
            .mcp_enabled
            .store(true, std::sync::atomic::Ordering::Relaxed);

        let response = app
            .clone()
            .oneshot(mcp_request("StormDevice dev_1:dvs_whatever"))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn a_minted_key_is_shown_once_and_never_again() {
        // A14.5, end to end over the real routes. The create response is the
        // only place the plaintext exists; the list must not carry it, and
        // neither must the audit trail.
        let dir = tempdir::TempDir::new("storm-key-once").unwrap();
        let (app, _, state) = test_router_with_state(dir.path());
        let owner = seed_owner(&state).await;
        let token = session_token(&state, &owner).await;
        let auth = format!("Bearer {token}");

        let (status, created) = send(
            &app,
            post_json_with_auth(
                "/v1/keys",
                serde_json::json!({ "name": "Claude Code, laptop" }),
                &auth,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{created}");
        let secret = created["secret"].as_str().unwrap().to_string();
        assert!(secret.starts_with("stk_"), "{secret}");

        let (status, listed) = send(&app, get_with_auth("/v1/keys", &auth)).await;
        assert_eq!(status, StatusCode::OK);
        let listed = serde_json::to_string(&listed).unwrap();
        assert!(
            !listed.contains(&secret),
            "listing keys must never return the plaintext again: {listed}"
        );
        assert!(listed.contains("Claude Code, laptop"), "{listed}");
    }

    #[tokio::test]
    async fn a_key_from_a_removed_account_cannot_be_revoked_or_listed() {
        let dir = tempdir::TempDir::new("storm-key-foreign").unwrap();
        let (app, _, state) = test_router_with_state(dir.path());
        let owner = seed_owner(&state).await;
        let auth = format!("Bearer {}", session_token(&state, &owner).await);
        let foreign = {
            let db = state.auth_db.lock().await;
            db.conn_for_tests()
                .execute_batch("PRAGMA foreign_keys = OFF;")
                .unwrap();
            db.conn_for_tests()
                .execute(
                    "INSERT INTO api_keys (id, user_id, name, secret_hash, created)
                     VALUES ('key_foreign', 'usr_gone', 'left behind', x'00', '2026-08-19T12:00:00Z')",
                    [],
                )
                .unwrap();
            db.conn_for_tests()
                .execute_batch("PRAGMA foreign_keys = ON;")
                .unwrap();
            "key_foreign"
        };
        let (status, listed) = send(&app, get_with_auth("/v1/keys", &auth)).await;
        assert_eq!(status, StatusCode::OK);
        assert!(!listed.to_string().contains(foreign), "{listed}");
        let (status, _) = send(
            &app,
            axum::http::Request::builder()
                .method("DELETE")
                .uri(format!("/v1/keys/{foreign}"))
                .header("Authorization", &auth)
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn an_mcp_key_request_carries_its_owners_identity() {
        // **The whole point of A14.3.** The request has to arrive at the
        // authorization boundary as the *user*, with the key alongside for
        // audit — not as some third kind of principal. If this resolved to
        // anything without a `user_id`, a future policy would have one caller
        // whose grants it could not look up, which is the `Actor::Mcp` mistake
        // slice 11 already removed once.
        let dir = tempdir::TempDir::new("storm-key-identity").unwrap();
        let policy = Arc::new(RecordingPolicy::default());
        let (app, _, state) = test_router_with_policy(dir.path(), policy.clone());
        state
            .mcp_enabled
            .store(true, std::sync::atomic::Ordering::Relaxed);
        register_vault(&state, "Notes").await;
        let (user_id, secret) = seed_key(&state).await;

        // A tool that reaches a vault, so the policy is actually consulted.
        let _ = app
            .clone()
            .oneshot(mcp_call(
                "list_vaults",
                serde_json::json!({}),
                &format!("Bearer {secret}"),
            ))
            .await
            .unwrap();

        let seen = policy.actors();
        assert!(
            !seen.is_empty(),
            "the policy was never consulted, so this test proves nothing"
        );
        for actor in &seen {
            assert_eq!(
                actor.user_id(),
                user_id.as_str(),
                "an MCP key must reach the boundary as the account"
            );
            assert!(
                actor.key_id().is_some(),
                "and still say which key acted, for the audit trail"
            );
        }
    }

    // ---- change feed: WebSocket and SSE ---------------------------------

    fn a_change(vault: &str, seq: i64) -> Change {
        Change {
            seq,
            vault_id: vault.into(),
            note_id: "note-1".into(),
            kind: "updated".into(),
            version: 1,
            at: "2026-08-26T00:00:00Z".into(),
        }
    }

    /// A session-tier credential, which is what `/v1/stream` needs.
    async fn stream_credential(state: &Shared) -> String {
        let user = seed_owner(state).await;
        format!("Bearer {}", session_token(state, &user).await)
    }

    /// A receiver that has already fallen behind, so the next `recv` is
    /// `Lagged`.
    ///
    /// Deterministic on purpose: racing a real slow consumer to make it lag is
    /// exactly the kind of test that passes locally and fails in CI.
    fn a_lagged_receiver() -> broadcast::Receiver<Change> {
        let (tx, rx) = broadcast::channel(2);
        for seq in 1..=5 {
            tx.send(a_change("vault-a", seq)).unwrap();
        }
        drop(tx); // so the loop ends rather than waiting forever
        rx
    }

    /// Records what the shared loop asked a sink to deliver, without framing.
    #[derive(Clone, Default)]
    struct RecordingSink(Arc<std::sync::Mutex<Vec<String>>>);

    impl ChangeSink for RecordingSink {
        async fn change(&mut self, change: &Change) -> Result<(), Gone> {
            self.0
                .lock()
                .unwrap()
                .push(format!("change {}:{}", change.vault_id, change.seq));
            Ok(())
        }

        async fn resync(&mut self) -> Result<(), Gone> {
            self.0.lock().unwrap().push("resync".into());
            Ok(())
        }
    }

    /// Reads an endless body until `want` complete SSE events have arrived.
    ///
    /// `axum::body::to_bytes` cannot be used here at all: the change feed has
    /// no end, so reading to completion hangs rather than failing.
    async fn read_events(body: axum::body::Body, want: usize) -> String {
        let mut stream = Box::pin(body.into_data_stream());
        let mut text = String::new();
        while text.matches("\n\n").count() < want {
            let chunk = tokio::time::timeout(
                std::time::Duration::from_secs(5),
                tokio_stream::StreamExt::next(&mut stream),
            )
            .await
            .expect("the feed sent nothing before the timeout")
            .expect("the feed ended, which it must not do")
            .unwrap();
            text.push_str(std::str::from_utf8(&chunk).unwrap());
        }
        text
    }

    #[tokio::test]
    async fn the_feed_loop_sends_changes_and_turns_lagging_into_a_resync() {
        let seen = RecordingSink::default();
        push_changes(seen.clone(), a_lagged_receiver()).await;

        // The two retained changes, and the gap before them reported rather
        // than dropped: a client silently missing changes has stopped syncing
        // with nothing anywhere saying so.
        assert_eq!(
            *seen.0.lock().unwrap(),
            vec!["resync", "change vault-a:4", "change vault-a:5"]
        );
    }

    #[test]
    fn a_lagging_websocket_client_is_sent_the_same_literal_it_always_was() {
        // Not a tautology: these bytes are the wire format a shipped Flutter
        // client parses, so editing `WS_RESYNC` has to fail here rather than in
        // the field.
        assert_eq!(WS_RESYNC, r#"{"kind":"resync"}"#);
    }

    /// The WebSocket branch, over a real socket.
    ///
    /// `oneshot` cannot reach it: `WebSocketUpgrade` needs a
    /// `hyper::upgrade::OnUpgrade` in the request extensions and only a real
    /// hyper connection produces one — which is the whole reason `/v1/stream`
    /// grew a second response mode. So this binds an ephemeral port and speaks
    /// the handshake for real.
    #[tokio::test]
    async fn the_websocket_branch_still_sends_a_bare_change_as_a_text_frame() {
        use tokio_tungstenite::tungstenite::{Message as WsMessage, client::IntoClientRequest};

        let dir = tempdir::TempDir::new("storm").unwrap();
        let (app, _, state) = test_router_with_state(dir.path());
        let credential = stream_credential(&state).await;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
            )
            .await
            .unwrap();
        });

        let mut request = format!("ws://{addr}/v1/stream")
            .into_client_request()
            .unwrap();
        request
            .headers_mut()
            .insert("authorization", credential.parse().unwrap());
        let (mut socket, handshake) = tokio_tungstenite::connect_async(request).await.unwrap();
        assert_eq!(handshake.status(), StatusCode::SWITCHING_PROTOCOLS);

        let change = a_change("vault-a", 3);
        state.events.send(change.clone()).unwrap();

        let frame = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            tokio_stream::StreamExt::next(&mut socket),
        )
        .await
        .expect("the socket sent nothing before the timeout")
        .expect("the socket closed")
        .unwrap();

        // **The invariant, asserted rather than assumed.** A raw `serde_json`
        // `Change` in a text frame: no event name, no envelope, nothing of the
        // SSE framing. `apps/client/test_live/two_client_sync_test.dart` and a
        // shipped client both parse exactly this.
        let WsMessage::Text(text) = frame else {
            panic!("the change feed must send text frames");
        };
        assert_eq!(text.as_str(), serde_json::to_string(&change).unwrap());

        server.abort();
    }

    #[tokio::test]
    async fn a_stream_request_without_an_upgrade_answers_event_stream() {
        let dir = tempdir::TempDir::new("storm").unwrap();
        let (app, _, state) = test_router_with_state(dir.path());
        let credential = stream_credential(&state).await;

        let response = app
            .oneshot(get_with_auth("/v1/stream", &credential))
            .await
            .unwrap();

        // Not a rejection, and not a 426: the relay dispatches this request
        // in-process, where an upgrade is impossible.
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get(header::CONTENT_TYPE).unwrap(),
            "text/event-stream"
        );
    }

    #[tokio::test]
    async fn the_sse_body_frames_a_change_as_one_event() {
        let dir = tempdir::TempDir::new("storm").unwrap();
        let (app, _, state) = test_router_with_state(dir.path());
        let credential = stream_credential(&state).await;

        let response = app
            .oneshot(get_with_auth("/v1/stream", &credential))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        // Safe to send only because the handler subscribes before it answers.
        let change = a_change("vault-a", 7);
        state.events.send(change.clone()).unwrap();

        let json = serde_json::to_string(&change).unwrap();
        assert_eq!(
            read_events(response.into_body(), 1).await,
            format!("event: change\nid: vault-a:7\ndata: {json}\n\n")
        );
    }

    /// The regression test for the M9/M10 invariant.
    ///
    /// `change_log.seq` comes from each vault's own `index.db`, so every vault
    /// counts 1, 2, 3 while this feed is cross-vault. A bare `id: <seq>` would
    /// make these two events indistinguishable and land a `Last-Event-ID`
    /// resume at the wrong position — silently, which is the failure the
    /// composite id exists to prevent.
    #[tokio::test]
    async fn two_vaults_at_the_same_seq_get_distinct_event_ids() {
        let dir = tempdir::TempDir::new("storm").unwrap();
        let (app, _, state) = test_router_with_state(dir.path());
        let credential = stream_credential(&state).await;

        let response = app
            .oneshot(get_with_auth("/v1/stream", &credential))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        state.events.send(a_change("vault-a", 1)).unwrap();
        state.events.send(a_change("vault-b", 1)).unwrap();

        let body = read_events(response.into_body(), 2).await;
        let ids: Vec<&str> = body
            .lines()
            .filter(|line| line.starts_with("id:"))
            .collect();
        assert_eq!(ids, vec!["id: vault-a:1", "id: vault-b:1"]);
        assert_ne!(ids[0], ids[1], "a bare seq would have collided here");
    }

    /// SRP §5.3 (decision 116b): a quiet feed still says something every
    /// period, and the comment ends with the feed rather than holding it open.
    #[tokio::test]
    async fn a_quiet_sse_feed_sends_a_comment_every_period() {
        assert_eq!(SSE_KEEPALIVE_EVERY, std::time::Duration::from_secs(15));
        let period = std::time::Duration::from_millis(200);
        let (events, rx) = broadcast::channel::<Change>(4);
        let (tx, mut frames) = tokio::sync::mpsc::channel(8);
        let feed = tokio::spawn(feed_with_keepalive(tx, rx, period));

        tokio::time::sleep(period / 2).await;
        assert!(frames.try_recv().is_err(), "a comment before the period");
        let comment = tokio::time::timeout(period * 5, frames.recv()).await;
        assert_eq!(comment.unwrap().unwrap().unwrap(), ":\n\n");

        drop(events);
        feed.await.unwrap();
        assert!(frames.recv().await.is_none(), "the feed ended");
    }

    #[tokio::test]
    async fn a_lagging_sse_client_is_sent_a_resync_event() {
        let (tx, mut frames) = tokio::sync::mpsc::channel(8);
        push_changes(SseSink(tx), a_lagged_receiver()).await;

        assert_eq!(
            frames.recv().await.unwrap().unwrap(),
            "event: resync\ndata:\n\n"
        );
    }

    /// The load-bearing one: `/v1/stream` is session tier, and the SSE mode did
    /// not move it out from behind `require_auth`.
    ///
    /// The rejected design had the tunnel client subscribe to `state.events`
    /// in-process instead of calling the handler, which would have handed every
    /// vault's change feed to anyone who could open a trunk. This is what says
    /// that bypass was not reintroduced.
    #[tokio::test]
    async fn the_change_feed_refuses_an_unauthenticated_caller() {
        let dir = tempdir::TempDir::new("storm").unwrap();
        let (app, _, _) = test_router_with_state(dir.path());

        let plain = app.clone().oneshot(get("/v1/stream")).await.unwrap();
        assert_eq!(plain.status(), StatusCode::UNAUTHORIZED);

        // And the upgrade branch too — the credential is checked before either
        // branch is chosen, so neither can be the way in.
        let upgrade = axum::http::Request::builder()
            .uri("/v1/stream")
            .header("connection", "keep-alive, Upgrade")
            .header("upgrade", "WebSocket")
            .header("sec-websocket-version", "13")
            .header("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==")
            .body(axum::body::Body::empty())
            .unwrap();
        let refused = app.oneshot(upgrade).await.unwrap();
        assert_eq!(refused.status(), StatusCode::UNAUTHORIZED);
    }

    /// `Connection` is a token list and both headers are case-insensitive, so
    /// the real-world handshake must not be mistaken for a plain GET.
    #[tokio::test]
    async fn an_upgrade_is_detected_by_token_not_by_whole_header() {
        let dir = tempdir::TempDir::new("storm").unwrap();
        let (app, _, state) = test_router_with_state(dir.path());
        let credential = stream_credential(&state).await;

        let request = axum::http::Request::builder()
            .uri("/v1/stream")
            .header("authorization", &credential)
            .header("connection", "keep-alive, Upgrade")
            .header("upgrade", "WebSocket")
            .header("sec-websocket-version", "13")
            .header("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==")
            .body(axum::body::Body::empty())
            .unwrap();
        let response = app.oneshot(request).await.unwrap();

        // `oneshot` puts no `OnUpgrade` in the extensions, so the handshake
        // cannot complete — but reaching that refusal is the proof the request
        // took the upgrade branch instead of being answered with SSE.
        assert_ne!(response.status(), StatusCode::OK);
        assert_ne!(
            response
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok()),
            Some("text/event-stream"),
            "a real handshake must not be answered with an SSE body"
        );
    }

    // ---- relay configuration (PUT /v1/config/relays) ----------------------

    fn put_json_with_auth(
        path: &str,
        body: serde_json::Value,
        auth: &str,
    ) -> axum::http::Request<axum::body::Body> {
        put_json(path, body, Some(auth))
    }

    #[tokio::test]
    async fn config_reports_an_empty_relay_list_on_a_fresh_server() {
        let dir = tempdir::TempDir::new("storm-relays-config-fresh").unwrap();
        let (app, _, state) = test_router_with_state(dir.path());
        let owner = seed_owner(&state).await;
        let token = session_token(&state, &owner).await;

        let (status, body) = send(
            &app,
            get_with_auth("/v1/config", &format!("Bearer {token}")),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            body["relays"],
            serde_json::json!([]),
            "no relay was ever configured"
        );
    }

    #[tokio::test]
    async fn config_response_is_unchanged_apart_from_the_new_field() {
        // Additive means every key that existed keeps existing, and nothing
        // moved. Asserted explicitly so a future edit cannot reshape this
        // response silently.
        let dir = tempdir::TempDir::new("storm-relays-config-shape").unwrap();
        let (app, _, state) = test_router_with_state(dir.path());
        let owner = seed_owner(&state).await;
        let token = session_token(&state, &owner).await;

        let (status, body) = send(
            &app,
            get_with_auth("/v1/config", &format!("Bearer {token}")),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            body["vault_root"],
            state
                .vaults
                .read()
                .await
                .registry
                .root
                .display()
                .to_string()
        );
        assert_eq!(body["state_dir"], state.state_dir.display().to_string());
        assert_eq!(body["vault_count"], 0);
        assert_eq!(body["mcp_enabled"], false);
        assert_eq!(body["mcp_writable"], false);
        assert!(
            body.get("allow_registration").is_none(),
            "registration went with multi-user Storm (decision 82)"
        );
        assert_eq!(body["relays"], serde_json::json!([]));
        assert_eq!(
            body["agent_writes"], false,
            "fresh installs: agents read only"
        );
        assert_eq!(body["version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(
            body["discovery"],
            serde_json::json!({"discoverable": true, "forced_off": false, "active": true, "untrusted": []}),
            "decision 118: discoverable unless turned off"
        );
        assert_eq!(
            body.as_object().unwrap().len(),
            9,
            "a new key showed up that this test does not know about"
        );
    }

    #[tokio::test]
    async fn setting_relays_persists_across_a_registry_reload() {
        let dir = tempdir::TempDir::new("storm-relays-persist").unwrap();
        let (app, _, state) = test_router_with_state(dir.path());
        let owner = seed_owner(&state).await;
        let token = session_token(&state, &owner).await;

        let (status, body) = send(
            &app,
            put_json_with_auth(
                "/v1/config/relays",
                serde_json::json!({"relays": ["wss://relay.example.com", "wss://relay.two.example"]}),
                &format!("Bearer {token}"),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            body["relays"],
            serde_json::json!(["wss://relay.example.com", "wss://relay.two.example"])
        );

        // Config reflects it immediately, without a restart.
        let (_, config) = send(
            &app,
            get_with_auth("/v1/config", &format!("Bearer {token}")),
        )
        .await;
        assert_eq!(
            config["relays"],
            serde_json::json!(["wss://relay.example.com", "wss://relay.two.example"])
        );

        // And it survives a fresh load from disk, not just the in-memory copy.
        let reloaded =
            Registry::load(&state.state_dir, &state.vaults.read().await.registry.root).unwrap();
        assert_eq!(
            reloaded.relays,
            vec![
                "wss://relay.example.com".to_string(),
                "wss://relay.two.example".to_string(),
            ]
        );
    }

    #[tokio::test]
    async fn a_bad_relay_url_is_a_400_and_changes_nothing() {
        // All-or-nothing is the property under test: both the status and that
        // the previously stored list survived intact.
        let dir = tempdir::TempDir::new("storm-relays-bad-url").unwrap();
        let (app, _, state) = test_router_with_state(dir.path());
        let owner = seed_owner(&state).await;
        let token = session_token(&state, &owner).await;

        let (status, _) = send(
            &app,
            put_json_with_auth(
                "/v1/config/relays",
                serde_json::json!({"relays": ["wss://relay.example.com"]}),
                &format!("Bearer {token}"),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let (status, body) = send(
            &app,
            put_json_with_auth(
                "/v1/config/relays",
                serde_json::json!({"relays": ["wss://relay.example.com", "not-a-relay-url"]}),
                &format!("Bearer {token}"),
            ),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "a bad URL is the caller's mistake, not a server failure"
        );
        assert!(
            body["error"].as_str().unwrap().contains("not-a-relay-url"),
            "the message must say which URL was rejected: {body}"
        );

        let (_, config) = send(
            &app,
            get_with_auth("/v1/config", &format!("Bearer {token}")),
        )
        .await;
        assert_eq!(
            config["relays"],
            serde_json::json!(["wss://relay.example.com"]),
            "the previously stored list must survive a rejected update"
        );
    }

    #[tokio::test]
    async fn saving_relays_hands_the_list_to_the_tunnels() {
        // Decision 74: the tunnels follow a save without a restart. This is
        // the half of that wiring that lives in the handler.
        let dir = tempdir::TempDir::new("storm-relays-handed-over").unwrap();
        let (app, _, state) = test_router_with_state(dir.path());
        let mut changes = state.relays_changed.subscribe();
        let owner = seed_owner(&state).await;
        let token = session_token(&state, &owner).await;

        let (status, _) = send(
            &app,
            put_json_with_auth(
                "/v1/config/relays",
                serde_json::json!({"relays": ["wss://relay.example.com/"]}),
                &format!("Bearer {token}"),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            changes.has_changed().unwrap(),
            "the saved list never reached the tunnels"
        );
        // The normalised form, exactly as a restart would read it back.
        assert_eq!(
            *changes.borrow_and_update(),
            vec!["wss://relay.example.com".to_string()]
        );
    }

    #[tokio::test]
    async fn setting_relays_does_not_register_them() {
        // The distinction the whole design rests on: configuring is not
        // registering. `GET /v1/server` must keep answering an empty set,
        // because nothing here is a tunnel client succeeding at a connection.
        let dir = tempdir::TempDir::new("storm-relays-not-registered").unwrap();
        let (app, _, state) = test_router_with_state(dir.path());
        let owner = seed_owner(&state).await;
        let token = session_token(&state, &owner).await;

        let (status, _) = send(
            &app,
            put_json_with_auth(
                "/v1/config/relays",
                serde_json::json!({"relays": ["wss://relay.example.com"]}),
                &format!("Bearer {token}"),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let (status, server) = send(&app, get("/v1/server")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            server["relays"],
            serde_json::json!([]),
            "a configured relay must not appear on the wire until something registers it"
        );
    }

    #[tokio::test]
    async fn putting_relays_refuses_an_unauthenticated_caller() {
        let dir = tempdir::TempDir::new("storm-relays-unauth").unwrap();
        let (app, _, state) = test_router_with_state(dir.path());
        let _ = seed_owner(&state).await;

        let (status, _) = send(
            &app,
            put_json(
                "/v1/config/relays",
                serde_json::json!({"relays": ["wss://relay.example.com"]}),
                None,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);

        // Nothing was stored.
        let vaults = state.vaults.read().await;
        assert!(vaults.registry.relays.is_empty());
    }

    // ---- Agent Runtime: hosts (decision 77b) ------------------------------

    fn patch_json_with_auth(
        path: &str,
        body: serde_json::Value,
        auth: &str,
    ) -> axum::http::Request<axum::body::Body> {
        axum::http::Request::builder()
            .method("PATCH")
            .uri(path)
            .header("content-type", "application/json")
            .header("authorization", auth)
            .body(axum::body::Body::from(body.to_string()))
            .unwrap()
    }

    fn delete_with_auth(path: &str, auth: &str) -> axum::http::Request<axum::body::Body> {
        axum::http::Request::builder()
            .method("DELETE")
            .uri(path)
            .header("authorization", auth)
            .body(axum::body::Body::empty())
            .unwrap()
    }

    /// Enrolls a host through the real routes and returns `(host_id, token)`.
    async fn enroll_a_host(app: &Router, owner_bearer: &str) -> (String, String) {
        use ed25519_dalek::Signer;
        let (status, body) = send(
            app,
            post_json_with_auth(
                "/v1/agent/hosts/enrollments",
                serde_json::json!({"server_url": "http://127.0.0.1:8484"}),
                owner_bearer,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let enrollment = body["enrollment"].as_str().unwrap().to_string();
        // Parsed from the right, as decision 77b specifies: the URL has colons.
        let mut parts = enrollment.rsplitn(4, ':');
        let token = parts.next().unwrap().to_string();
        let _pubkey = parts.next().unwrap();
        let server_id = parts.next().unwrap().to_string();

        let key = ed25519_dalek::SigningKey::from_bytes(&[42u8; 32]);
        let token_id = crate::auth::hosts::enrollment_token_id(&token).unwrap();
        let sig = data_encoding::BASE64URL_NOPAD.encode(
            &key.sign(&crate::auth::hosts::enroll_message(&server_id, &token_id))
                .to_bytes(),
        );
        let (status, body) = send(
            app,
            post_json(
                "/v1/runtime/enroll",
                serde_json::json!({
                    "token": token,
                    "public_key": data_encoding::BASE64URL_NOPAD.encode(key.verifying_key().as_bytes()),
                    "key_id": "key_test",
                    "name": "build-vm",
                    "signature": sig,
                }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let host_id = body["host_id"].as_str().unwrap().to_string();

        let (status, body) = send(
            app,
            post_json(
                "/v1/runtime/auth/challenge",
                serde_json::json!({"host_id": host_id}),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let nonce = body["nonce"].as_str().unwrap().to_string();
        let sig = data_encoding::BASE64URL_NOPAD.encode(
            &key.sign(&crate::auth::hosts::connect_message(
                &server_id, &host_id, &nonce,
            ))
            .to_bytes(),
        );
        let (status, body) = send(
            app,
            post_json(
                "/v1/runtime/auth",
                serde_json::json!({"host_id": host_id, "nonce": nonce, "signature": sig}),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        (host_id, body["token"].as_str().unwrap().to_string())
    }

    #[tokio::test]
    async fn a_host_enrolls_authenticates_and_stays_in_its_tier() {
        let dir = tempdir::TempDir::new("storm-hosts-flow").unwrap();
        let (app, _, state) = test_router_with_state(dir.path());
        let owner = seed_owner(&state).await;
        let owner_bearer = format!("Bearer {}", session_token(&state, &owner).await);

        let (host_id, token) = enroll_a_host(&app, &owner_bearer).await;
        let host_bearer = format!("Bearer {token}");

        let (status, body) = send(&app, get_with_auth("/v1/runtime/whoami", &host_bearer)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["host_id"], host_id);

        // The tier, both ways round: a host token reaches nothing else, and no
        // user credential reaches the runtime routes.
        for path in ["/v1/vaults", "/v1/config", "/v1/agent/hosts"] {
            let (status, _) = send(&app, get_with_auth(path, &host_bearer)).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED, "{path}");
        }
        let (status, _) = send(&app, get_with_auth("/v1/runtime/whoami", &owner_bearer)).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        let device = pair_a_device(&state).await;
        let (status, _) = send(&app, get_with_auth("/v1/runtime/whoami", &device)).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);

        // The owner sees it, renames it, revokes it — and the token dies.
        let (status, body) = send(&app, get_with_auth("/v1/agent/hosts", &owner_bearer)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body[0]["id"], host_id);
        assert_eq!(body[0]["status"], "offline");
        assert_eq!(body[0]["egress"], "host");
        let path = format!("/v1/agent/hosts/{host_id}");
        let (status, body) = send(
            &app,
            patch_json_with_auth(&path, serde_json::json!({"name": "renamed"}), &owner_bearer),
        )
        .await;
        assert_eq!(
            (status, body["name"].clone()),
            (StatusCode::OK, "renamed".into())
        );
        let (status, _) = send(&app, delete_with_auth(&path, &owner_bearer)).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (status, _) = send(&app, get_with_auth("/v1/runtime/whoami", &host_bearer)).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn an_access_key_cannot_administer_hosts() {
        let dir = tempdir::TempDir::new("storm-hosts-key").unwrap();
        let (app, _, state) = test_router_with_state(dir.path());
        let owner = seed_owner(&state).await;
        let owner_bearer = format!("Bearer {}", session_token(&state, &owner).await);
        let (host_id, _) = enroll_a_host(&app, &owner_bearer).await;
        let (_, secret) = seed_key(&state).await;
        let key = format!("Bearer {secret}");

        let path = format!("/v1/agent/hosts/{host_id}");
        let requests = [
            get_with_auth("/v1/agent/hosts", &key),
            post_json_with_auth(
                "/v1/agent/hosts/enrollments",
                serde_json::json!({"server_url": "http://x"}),
                &key,
            ),
            patch_json_with_auth(&path, serde_json::json!({"name": "mine"}), &key),
            delete_with_auth(&path, &key),
        ];
        for request in requests {
            let uri = request.uri().to_string();
            let (status, _) = send(&app, request).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED, "{uri}");
        }
    }

    // ---- MCP Gateway: integrations (decision 81c) ------------------------

    const CONNECTIONS: &str = "/v1/integrations/connections";

    async fn owner_bearer(state: &Shared) -> (String, String) {
        let owner = seed_owner(state).await;
        let bearer = format!("Bearer {}", session_token(state, &owner).await);
        (owner, bearer)
    }

    async fn create_github(app: &Router, bearer: &str, pat: &str) -> serde_json::Value {
        let (status, body) = send(
            app,
            post_json_with_auth(
                CONNECTIONS,
                serde_json::json!({
                    "display_name": "GitHub (work)",
                    "url": "https://api.example.com/mcp/",
                    "auth_kind": "static",
                    "credential": {"value": format!("Bearer {pat}")},
                }),
                bearer,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        body
    }

    #[tokio::test]
    async fn an_owner_connects_changes_and_disconnects_an_integration() {
        let dir = tempdir::TempDir::new("storm-integrations-flow").unwrap();
        let (app, _, state) = test_router_with_state(dir.path());
        let (_, bearer) = owner_bearer(&state).await;

        let created = create_github(&app, &bearer, "ghp_first").await;
        let id = created["id"].as_str().unwrap().to_string();
        assert!(id.starts_with("mcc_"), "{id}");
        assert_eq!(created["slug"], "github-work");
        assert_eq!(created["status"], "connected");
        assert_eq!(created["has_credential"], true);
        assert_eq!(created["builtin"], false);

        // The built-in connection is always listed first, and cannot go.
        let (status, list) = send(&app, get_with_auth(CONNECTIONS, &bearer)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(list[0]["id"], "storm");
        assert_eq!(list[0]["builtin"], true);
        assert_eq!(list[1]["id"], id);
        assert_eq!(list.as_array().unwrap().len(), 2);
        let (status, _) = send(
            &app,
            delete_with_auth(&format!("{CONNECTIONS}/storm"), &bearer),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        // Disable, re-enable, rotate the token, narrow the allowlist.
        let path = format!("{CONNECTIONS}/{id}");
        let (status, body) = send(
            &app,
            patch_json_with_auth(&path, serde_json::json!({"enabled": false}), &bearer),
        )
        .await;
        assert_eq!(
            (status, body["status"].clone()),
            (StatusCode::OK, "disabled".into())
        );
        let (status, body) = send(
            &app,
            patch_json_with_auth(
                &path,
                serde_json::json!({
                    "enabled": true,
                    "credential": {"value": "Bearer ghp_second"},
                    "tool_allowlist": ["search", "search", "get_issue"],
                    "expose_prompts": false,
                }),
                &bearer,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["status"], "connected");
        assert_eq!(
            body["tool_allowlist"],
            serde_json::json!(["get_issue", "search"])
        );
        assert_eq!(body["expose_prompts"], false);
        // The rotated token is what is sealed now.
        {
            let (sealed, _) = state
                .gateway
                .store
                .lock()
                .unwrap()
                .credential(&id, "static")
                .unwrap()
                .unwrap();
            let opened = state.gateway.keys.open(&id, "static", &sealed).unwrap();
            let credential: crate::gateway::connections::StaticCredential =
                serde_json::from_slice(opened.expose()).unwrap();
            assert_eq!(credential.header, "Authorization");
            assert_eq!(credential.value, "Bearer ghp_second");
        }

        // Re-pointing a connection would hand its token to a new host: the
        // URL is not a field PATCH accepts, so it is unchanged.
        let (_, body) = send(
            &app,
            patch_json_with_auth(
                &path,
                serde_json::json!({"url": "https://evil.example/"}),
                &bearer,
            ),
        )
        .await;
        assert_eq!(body["url"], "https://api.example.com/mcp/");

        // Disconnect: gone from the list, 404 by id, ciphertexts deleted, and
        // the slug is free for a new connection.
        let (status, _) = send(&app, delete_with_auth(&path, &bearer)).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (_, list) = send(&app, get_with_auth(CONNECTIONS, &bearer)).await;
        assert_eq!(list.as_array().unwrap().len(), 1);
        let (status, _) = send(&app, get_with_auth(&path, &bearer)).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert!(
            state
                .gateway
                .store
                .lock()
                .unwrap()
                .credential(&id, "static")
                .unwrap()
                .is_none()
        );
        let again = create_github(&app, &bearer, "ghp_third").await;
        assert_eq!(again["slug"], "github-work");
        assert_ne!(again["id"], id);
    }

    async fn create_against(app: &Router, bearer: &str, url: &str, value: &str) -> String {
        let (status, body) = send(
            app,
            post_json_with_auth(
                CONNECTIONS,
                serde_json::json!({
                    "display_name": "Mock",
                    "url": url,
                    "auth_kind": "static",
                    "credential": {"header": "X-Api-Key", "value": value},
                }),
                bearer,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        body["id"].as_str().unwrap().to_string()
    }

    #[tokio::test]
    async fn the_owners_test_turns_every_tool_on_and_later_tools_stay_off() {
        // G-D16 over the real routes, against a real MCP upstream.
        let dir = tempdir::TempDir::new("storm-integrations-probe").unwrap();
        let (app, _, state) = test_router_with_state(dir.path());
        state.gateway.set_allow_http_upstreams(true);
        let (_, owner) = owner_bearer(&state).await;
        let up = crate::gateway::upstream::tests::mock(&["search", "get_issue"]);
        let url = crate::gateway::upstream::tests::serve_mock(
            up.clone(),
            Some(("x-api-key", "upstream-canary-probe".into())),
        )
        .await;
        let id = create_against(&app, &owner, &url, "upstream-canary-probe").await;

        let (status, test) = send(
            &app,
            post_json_with_auth(
                &format!("{CONNECTIONS}/{id}/test"),
                serde_json::json!({}),
                &owner,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{test}");
        assert_eq!(test["ok"], true);
        assert_eq!(test["server_name"], "mock-upstream");
        assert_eq!(test["tool_count"], 2);
        assert_eq!(test["new_tools"], serde_json::json!([]));
        assert_eq!(
            test["integration"]["tool_allowlist"],
            serde_json::json!(["get_issue", "search"])
        );
        assert!(test["integration"]["last_ok"].is_string());

        // A tool appears upstream: listed, new, and off.
        up.tools.lock().unwrap().push("delete_repo".into());
        let tools_path = format!("{CONNECTIONS}/{id}/tools");
        let (status, tools) = send(&app, get_with_auth(&tools_path, &owner)).await;
        assert_eq!(status, StatusCode::OK, "{tools}");
        let by_name = |name: &str| {
            tools
                .as_array()
                .unwrap()
                .iter()
                .find(|t| t["name"] == name)
                .cloned()
                .unwrap()
        };
        assert_eq!(by_name("delete_repo")["allowed"], false);
        assert_eq!(by_name("delete_repo")["new"], true);
        assert_eq!(by_name("search")["allowed"], true);
        assert_eq!(by_name("search")["new"], false);
        // Spec §9: it stays new — and the integration says so — until the
        // owner reviews it. Listing it again is not a review.
        let delete_repo = |tools: &serde_json::Value| {
            let t = tools
                .as_array()
                .unwrap()
                .iter()
                .find(|t| t["name"] == "delete_repo")
                .unwrap()
                .clone();
            (t["allowed"].clone(), t["new"].clone())
        };
        let (_, tools) = send(&app, get_with_auth(&tools_path, &owner)).await;
        assert_eq!(delete_repo(&tools), (false.into(), true.into()));
        let (_, view) = send(&app, get_with_auth(&format!("{CONNECTIONS}/{id}"), &owner)).await;
        assert_eq!(view["new_tools"], serde_json::json!(["delete_repo"]));
        // Saving the tool list is the review: no longer new, and still off.
        let (status, view) = send(
            &app,
            patch_json_with_auth(
                &format!("{CONNECTIONS}/{id}"),
                serde_json::json!({"tool_allowlist": ["get_issue", "search"]}),
                &owner,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{view}");
        assert_eq!(view["new_tools"], serde_json::json!([]));
        let (_, tools) = send(&app, get_with_auth(&tools_path, &owner)).await;
        assert_eq!(delete_repo(&tools), (false.into(), false.into()));

        // Probes never call a tool, and each one left a metadata-only audit row.
        assert_eq!(up.calls.load(std::sync::atomic::Ordering::SeqCst), 0);
        let calls = state
            .gateway
            .store
            .lock()
            .unwrap()
            .recent_calls(10)
            .unwrap();
        // The test, and three tool listings.
        assert_eq!(calls.len(), 4);
        assert!(
            calls
                .iter()
                .all(|c| c.method == "tools/list" && c.outcome == "ok" && c.connection_id == id)
        );
    }

    #[tokio::test]
    async fn a_refused_credential_marks_the_integration_needs_reauth_until_rotated() {
        let dir = tempdir::TempDir::new("storm-integrations-reauth").unwrap();
        let (app, _, state) = test_router_with_state(dir.path());
        state.gateway.set_allow_http_upstreams(true);
        let (_, owner) = owner_bearer(&state).await;
        let url = crate::gateway::upstream::tests::serve_mock(
            crate::gateway::upstream::tests::mock(&["search"]),
            Some(("x-api-key", "k-right".into())),
        )
        .await;
        let id = create_against(&app, &owner, &url, "k-wrong").await;
        let test_path = format!("{CONNECTIONS}/{id}/test");

        let (status, test) = send(
            &app,
            post_json_with_auth(&test_path, serde_json::json!({}), &owner),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(test["ok"], false);
        assert_eq!(test["error_code"], "upstream_unauthorized");
        assert_eq!(test["integration"]["status"], "needs_reauth");
        assert_eq!(
            test["integration"]["last_error_code"],
            "upstream_unauthorized"
        );
        // The tool listing says the same, as a 502 carrying only the code.
        let (status, body) = send(
            &app,
            get_with_auth(&format!("{CONNECTIONS}/{id}/tools"), &owner),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_GATEWAY);
        assert_eq!(body["error"], "upstream_unauthorized");

        // Rotating the token and testing again reconnects it.
        let path = format!("{CONNECTIONS}/{id}");
        let (_, patched) = send(
            &app,
            patch_json_with_auth(
                &path,
                serde_json::json!({"credential": {"header": "X-Api-Key", "value": "k-right"}}),
                &owner,
            ),
        )
        .await;
        assert_eq!(patched["status"], "connected");
        let (_, test) = send(
            &app,
            post_json_with_auth(&test_path, serde_json::json!({}), &owner),
        )
        .await;
        assert_eq!(test["ok"], true, "{test}");
        assert_eq!(
            test["integration"]["last_error_code"],
            serde_json::Value::Null
        );
    }

    #[tokio::test]
    async fn a_public_http_upstream_needs_the_test_only_flag_and_a_disabled_one_is_not_probed() {
        let dir = tempdir::TempDir::new("storm-integrations-http").unwrap();
        let (app, _, state) = test_router_with_state(dir.path());
        let (_, owner) = owner_bearer(&state).await;
        let (status, _) = send(
            &app,
            post_json_with_auth(
                CONNECTIONS,
                serde_json::json!({"display_name": "Plain", "url": "http://mcp.example.com/mcp", "auth_kind": "none"}),
                &owner,
            ),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "http to a public host must need the flag"
        );

        let id = create_github(&app, &owner, "ghp_x").await["id"]
            .as_str()
            .unwrap()
            .to_string();
        send(
            &app,
            patch_json_with_auth(
                &format!("{CONNECTIONS}/{id}"),
                serde_json::json!({"enabled": false}),
                &owner,
            ),
        )
        .await;
        let (status, _) = send(
            &app,
            post_json_with_auth(
                &format!("{CONNECTIONS}/{id}/test"),
                serde_json::json!({}),
                &owner,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
        let (status, _) = send(
            &app,
            post_json_with_auth(
                &format!("{CONNECTIONS}/storm/test"),
                serde_json::json!({}),
                &owner,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn an_unknown_integration_is_not_found() {
        let dir = tempdir::TempDir::new("storm-integrations-unknown").unwrap();
        let (app, _, state) = test_router_with_state(dir.path());
        let (_, owner) = owner_bearer(&state).await;
        let path = format!("{CONNECTIONS}/mcc_nope");
        for request in [
            get_with_auth(&path, &owner),
            patch_json_with_auth(&path, serde_json::json!({"enabled": false}), &owner),
            delete_with_auth(&path, &owner),
        ] {
            let (status, _) = send(&app, request).await;
            assert_eq!(status, StatusCode::NOT_FOUND);
        }
    }

    #[tokio::test]
    async fn an_mcp_key_cannot_reach_the_integration_routes() {
        // §14: session tier, so a key can never manage an integration.
        let dir = tempdir::TempDir::new("storm-integrations-key").unwrap();
        let (app, _, state) = test_router_with_state(dir.path());
        let (_, owner) = owner_bearer(&state).await;
        let (_, key) = send(
            &app,
            post_json_with_auth("/v1/keys", serde_json::json!({"name": "agent"}), &owner),
        )
        .await;
        let key = format!("Bearer {}", key["secret"].as_str().unwrap());
        for request in [
            get_with_auth(CONNECTIONS, &key),
            post_json_with_auth(
                CONNECTIONS,
                serde_json::json!({"display_name": "X", "url": "https://x.example/", "auth_kind": "none"}),
                &key,
            ),
        ] {
            let (status, _) = send(&app, request).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED);
        }
    }

    #[tokio::test]
    async fn a_new_integration_is_refused_when_its_input_is_wrong() {
        let dir = tempdir::TempDir::new("storm-integrations-input").unwrap();
        let (app, _, state) = test_router_with_state(dir.path());
        let (_, owner) = owner_bearer(&state).await;
        create_github(&app, &owner, "ghp_x").await;
        let cases = [
            (
                serde_json::json!({"display_name": "N", "url": "https://x.example/", "auth_kind": "oauth", "credential": {"value": "x"}}),
                StatusCode::BAD_REQUEST,
            ),
            (
                serde_json::json!({"display_name": "N", "url": "https://x.example/", "auth_kind": "static"}),
                StatusCode::BAD_REQUEST,
            ),
            (
                serde_json::json!({"display_name": "N", "url": "https://x.example/", "auth_kind": "none", "credential": {"value": "x"}}),
                StatusCode::BAD_REQUEST,
            ),
            (
                serde_json::json!({"display_name": "N", "url": "http://x.example/", "auth_kind": "none"}),
                StatusCode::BAD_REQUEST,
            ),
            (
                serde_json::json!({"display_name": "N", "url": "https://u:p@x.example/", "auth_kind": "none"}),
                StatusCode::BAD_REQUEST,
            ),
            (
                serde_json::json!({"display_name": "Storm", "url": "https://x.example/", "auth_kind": "none"}),
                StatusCode::BAD_REQUEST,
            ),
            (
                serde_json::json!({"display_name": "N", "slug": "storm", "url": "https://x.example/", "auth_kind": "none"}),
                StatusCode::BAD_REQUEST,
            ),
            (
                serde_json::json!({"display_name": "N", "url": "https://x.example/", "auth_kind": "static", "credential": {"header": "Host", "value": "x"}}),
                StatusCode::BAD_REQUEST,
            ),
            (
                serde_json::json!({"display_name": "N", "url": "https://x.example/", "auth_kind": "static", "credential": {"value": "a\r\nX-Evil: 1"}}),
                StatusCode::BAD_REQUEST,
            ),
            (
                serde_json::json!({"display_name": "GitHub (work)", "url": "https://x.example/", "auth_kind": "none"}),
                StatusCode::CONFLICT,
            ),
        ];
        for (body, want) in cases {
            let (status, answer) =
                send(&app, post_json_with_auth(CONNECTIONS, body.clone(), &owner)).await;
            assert_eq!(status, want, "{body} -> {answer}");
        }
    }

    #[tokio::test]
    async fn no_integration_secret_reaches_a_response_or_the_security_events() {
        // The invariant every auth area holds, for the gateway: a credential
        // is sealed, never returned, never in `security_events`. A URL can
        // carry a key in its query string, so the audit records its host only.
        let dir = tempdir::TempDir::new("storm-integrations-secrets").unwrap();
        let (app, _, state) = test_router_with_state(dir.path());
        let (_, owner) = owner_bearer(&state).await;
        let mut seen = Vec::new();
        let (status, created) = send(
            &app,
            post_json_with_auth(
                CONNECTIONS,
                serde_json::json!({
                    "display_name": "Linear",
                    "url": "https://mcp.example.com/mcp?api_key=upstream-canary-url",
                    "auth_kind": "static",
                    "credential": {"header": "X-Api-Key", "value": "upstream-canary-one"},
                }),
                &owner,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        seen.push(created.clone());
        let path = format!("{CONNECTIONS}/{}", created["id"].as_str().unwrap());
        let (_, patched) = send(
            &app,
            patch_json_with_auth(
                &path,
                serde_json::json!({"credential": {"header": "X-Api-Key", "value": "upstream-canary-two"}, "enabled": false}),
                &owner,
            ),
        )
        .await;
        seen.push(patched);
        seen.push(send(&app, get_with_auth(&path, &owner)).await.1);
        seen.push(send(&app, get_with_auth(CONNECTIONS, &owner)).await.1);
        let (status, _) = send(&app, delete_with_auth(&path, &owner)).await;
        assert_eq!(status, StatusCode::NO_CONTENT);

        for body in &seen {
            let text = body.to_string();
            assert!(!text.contains("upstream-canary-one"), "{text}");
            assert!(!text.contains("upstream-canary-two"), "{text}");
        }
        let events = state.auth_db.lock().await.all_events().unwrap();
        let kinds: Vec<&str> = events.iter().map(|(k, _)| k.as_str()).collect();
        for kind in [
            "integration_created",
            "integration_reauthorized",
            "integration_disabled",
            "integration_deleted",
        ] {
            assert!(kinds.contains(&kind), "{kind} missing from {kinds:?}");
        }
        for (kind, detail) in &events {
            assert!(!detail.contains("upstream-canary"), "{kind}: {detail}");
        }
        let created_detail = &events
            .iter()
            .find(|(k, _)| k == "integration_created")
            .unwrap()
            .1;
        assert!(
            created_detail.contains("mcp.example.com"),
            "{created_detail}"
        );
    }

    // ---- MCP Gateway: the host-tier route (decision 81e) -----------------

    struct GatewayFixture {
        app: Router,
        state: Shared,
        owner: String,
        owner_bearer: String,
        host_id: String,
        host_bearer: String,
        /// The host's link, so tests can see what was sent down it.
        link: tokio::sync::mpsc::UnboundedReceiver<crate::agent::Envelope>,
        up: crate::gateway::upstream::tests::MockUpstream,
        connection: String,
    }

    fn host_caps(bridge: bool) -> serde_json::Value {
        serde_json::json!({
            "providers": [
                {"id": "claude-code", "kind": "cli", "interactions": ["terminal"], "available": true},
                {"id": "shell", "kind": "cli", "interactions": ["terminal"], "available": true},
            ],
            "workspaces": ["storm"],
            "max_sessions": 8,
            "mcp_bridge": bridge,
        })
    }

    /// An owner, a mock upstream connected as a static integration with every
    /// tool allowed, and an enrolled host that is online and can bridge.
    async fn gateway_fixture(dir: &FsPath) -> GatewayFixture {
        let (app, _, state) = test_router_with_state(dir);
        state.gateway.set_allow_http_upstreams(true);
        let owner = seed_owner(&state).await;
        let owner_bearer = format!("Bearer {}", session_token(&state, &owner).await);
        let up = crate::gateway::upstream::tests::mock(&[
            "echo", "slow", "ask", "ask_url", "caps", "hang", "secret",
        ]);
        let url = crate::gateway::upstream::tests::serve_mock(
            up.clone(),
            Some(("x-api-key", "upstream-canary-route".into())),
        )
        .await;
        let connection = create_against(&app, &owner_bearer, &url, "upstream-canary-route").await;
        // The owner's test turns every tool on; then `secret` is turned off.
        let (_, test) = send(
            &app,
            post_json_with_auth(
                &format!("{CONNECTIONS}/{connection}/test"),
                serde_json::json!({}),
                &owner_bearer,
            ),
        )
        .await;
        assert_eq!(test["ok"], true, "{test}");
        send(
            &app,
            patch_json_with_auth(
                &format!("{CONNECTIONS}/{connection}"),
                serde_json::json!({"tool_allowlist": ["echo", "slow", "ask", "ask_url", "caps", "hang"]}),
                &owner_bearer,
            ),
        )
        .await;
        let (host_id, token) = enroll_a_host(&app, &owner_bearer).await;
        let (_, link) = state.agent.connect_host(&host_id);
        state
            .agent
            .hello(&host_id, serde_json::from_value(host_caps(true)).unwrap())
            .unwrap();
        GatewayFixture {
            app,
            state,
            owner,
            owner_bearer,
            host_id,
            host_bearer: format!("Bearer {token}"),
            link,
            up,
            connection,
        }
    }

    impl GatewayFixture {
        /// Launches a session and has the host report it running.
        async fn launch(&mut self, provider: &str, allow_vault_writes: bool) -> serde_json::Value {
            let (status, body) = send(
                &self.app,
                post_json_with_auth(
                    "/v1/agent/sessions",
                    serde_json::json!({
                        "host_id": self.host_id, "workspace": "storm", "provider": provider,
                        "terminal": {"cols": 80, "rows": 24},
                        "allow_vault_writes": allow_vault_writes,
                    }),
                    &self.owner_bearer,
                ),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{body}");
            let id = body["id"].as_str().unwrap().to_string();
            self.report(&id, "running").await;
            body
        }

        async fn report(&self, session: &str, status: &str) {
            let (code, body) = send(
                &self.app,
                post_json_with_auth(
                    &format!("/v1/runtime/sessions/{session}/status"),
                    serde_json::json!({"status": status}),
                    &self.host_bearer,
                ),
            )
            .await;
            assert!(code.is_success(), "{code} {body}");
        }

        /// One bridge message; every line of the answer.
        async fn mcp(
            &self,
            session: &str,
            connection: &str,
            message: serde_json::Value,
        ) -> Vec<serde_json::Value> {
            mcp_as(&self.app, &self.host_bearer, session, connection, message).await
        }

        async fn initialize(&self, session: &str, connection: &str) -> serde_json::Value {
            let lines = self
                .mcp(
                    session,
                    connection,
                    serde_json::json!({"jsonrpc": "2.0", "id": 0, "method": "initialize", "params": {
                        "protocolVersion": "2025-11-25",
                        "capabilities": {"roots": {"listChanged": true}, "sampling": {}, "elicitation": {"form": {}, "url": {}}},
                        "clientInfo": {"name": "claude-code", "version": "2.1.289"},
                    }}),
                )
                .await;
            assert_eq!(lines.len(), 1, "{lines:?}");
            lines[0]["message"]["result"].clone()
        }

        async fn call(
            &self,
            session: &str,
            connection: &str,
            tool: &str,
        ) -> Vec<serde_json::Value> {
            self.mcp(
                session,
                connection,
                serde_json::json!({"jsonrpc": "2.0", "id": 7, "method": "tools/call",
                    "params": {"name": tool, "arguments": {}}}),
            )
            .await
        }

        fn upstream_calls(&self) -> usize {
            self.up.calls.load(std::sync::atomic::Ordering::SeqCst)
        }
    }

    fn gateway_request(
        bearer: &str,
        session: &str,
        connection: &str,
        message: &serde_json::Value,
    ) -> axum::http::Request<axum::body::Body> {
        post_json_with_auth(
            &format!("/v1/runtime/sessions/{session}/mcp/{connection}"),
            message.clone(),
            bearer,
        )
    }

    async fn mcp_as(
        app: &Router,
        bearer: &str,
        session: &str,
        connection: &str,
        message: serde_json::Value,
    ) -> Vec<serde_json::Value> {
        let response = app
            .clone()
            .oneshot(gateway_request(bearer, session, connection, &message))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), 4 << 20)
            .await
            .unwrap();
        String::from_utf8(bytes.to_vec())
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    fn error_code(lines: &[serde_json::Value]) -> Option<String> {
        lines
            .last()
            .and_then(|l| l.pointer("/message/error/data/storm_error"))
            .and_then(|v| v.as_str())
            .map(str::to_string)
    }

    fn tool_text(lines: &[serde_json::Value]) -> String {
        lines
            .last()
            .and_then(|l| l.pointer("/message/result/content/0/text"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    }

    /// Decision 89 over REST: the host reports the conversation, and a
    /// resume runs where the old session ran, whatever the request says.
    #[tokio::test]
    async fn a_resume_reopens_the_conversation_where_it_ran() {
        let dir = tempdir::TempDir::new("storm-resume-route").unwrap();
        let mut f = gateway_fixture(dir.path()).await;
        let mut caps = host_caps(true);
        caps["resume"] = serde_json::json!(["claude-code"]);
        f.state
            .agent
            .hello(&f.host_id, serde_json::from_value(caps).unwrap())
            .unwrap();
        let old = f.launch("claude-code", false).await["id"]
            .as_str()
            .unwrap()
            .to_string();
        let _start = f.link.try_recv().unwrap();
        let uuid = "80d0cf8c-8176-43b4-8206-7908d74d1387";
        let (code, _) = send(
            &f.app,
            post_json_with_auth(
                &format!("/v1/runtime/sessions/{old}/status"),
                serde_json::json!({"status": "completed", "exit_code": 0, "harness_session": uuid}),
                &f.host_bearer,
            ),
        )
        .await;
        assert!(code.is_success());

        let (status, body) = send(
            &f.app,
            post_json_with_auth(
                "/v1/agent/sessions",
                serde_json::json!({
                    "resume_of": old, "host_id": "hst_ELSEWHERE", "workspace": "other",
                    "provider": "shell", "terminal": {"cols": 80, "rows": 24},
                }),
                &f.owner_bearer,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["host_id"], f.host_id);
        assert_eq!(body["workspace"], "storm");
        assert_eq!(body["provider"], "claude-code");
        assert_eq!(body["resumed_from"], old);
        assert_eq!(body["harness_session"], uuid);
        let start = serde_json::to_value(f.link.try_recv().unwrap()).unwrap();
        assert_eq!(start["resume"], uuid);

        let (status, body) = send(
            &f.app,
            post_json_with_auth(
                "/v1/agent/sessions",
                serde_json::json!({
                    "resume_of": old, "host_id": f.host_id, "workspace": "storm",
                    "terminal": {"cols": 80, "rows": 24},
                }),
                &f.owner_bearer,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
    }

    /// The identity rows of a session's audited calls, newest first.
    fn call_identities(f: &GatewayFixture, session: &str) -> Vec<Option<&'static str>> {
        f.state
            .gateway
            .store
            .lock()
            .unwrap()
            .recent_calls(100)
            .unwrap()
            .into_iter()
            .filter(|c| c.session_id.as_deref() == Some(session))
            .map(|c| c.identity)
            .collect()
    }

    async fn host_identity_seen(f: &GatewayFixture) -> (serde_json::Value, serde_json::Value) {
        let (_, hosts) = send(&f.app, get_with_auth("/v1/agent/hosts", &f.owner_bearer)).await;
        let host = hosts
            .as_array()
            .unwrap()
            .iter()
            .find(|h| h["id"] == f.host_id)
            .unwrap()
            .clone();
        (
            host["identity"].clone(),
            host["capabilities"]["identity"].clone(),
        )
    }

    /// G-135-6 and `old_host` (decision 135d, 135g): a host that does not
    /// say it checks connections keeps every existing tool; its host row,
    /// its sessions and every audit row say unverified; nothing privileged
    /// is listed. A host that says `verified` is audited as such, and a
    /// downgrade is in force from the next call.
    #[tokio::test]
    async fn g_135_6_an_unverified_host_keeps_its_tools_labelled_and_audited() {
        let dir = tempdir::TempDir::new("storm-identity-route").unwrap();
        let mut f = gateway_fixture(dir.path()).await;
        let old_host =
            serde_json::json!({"state": "unavailable", "mechanism": null, "reason": "old_host"});
        assert_eq!(
            host_identity_seen(&f).await,
            (old_host.clone(), old_host.clone())
        );

        let session = f.launch("claude-code", false).await["id"]
            .as_str()
            .unwrap()
            .to_string();
        let _start = f.link.try_recv().unwrap();
        let (_, view) = send(
            &f.app,
            get_with_auth(&format!("/v1/agent/sessions/{session}"), &f.owner_bearer),
        )
        .await;
        assert_eq!(view["identity"], old_host);

        f.initialize(&session, &f.connection).await;
        assert_eq!(
            tool_text(&f.call(&session, &f.connection, "echo").await),
            "called echo"
        );
        f.initialize(&session, "storm").await;
        let listed = f
            .mcp(
                &session,
                "storm",
                serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
            )
            .await;
        let names: Vec<String> = listed[0]["message"]["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_string())
            .collect();
        assert!(names.contains(&"list_vaults".to_string()), "{names:?}");
        // AM-G18's spawn tools need a verified host; none is offered here.
        for privileged in [
            "list_agents",
            "start_agent",
            "agent_status",
            "list_children",
            "wait_agent",
            "send_message",
            "end_agent",
            "report_result",
            "read_messages",
        ] {
            assert!(
                !names.contains(&privileged.to_string()),
                "{privileged} listed"
            );
        }
        let storm = f.call(&session, "storm", "list_vaults").await;
        assert!(
            storm.last().unwrap().pointer("/message/result").is_some(),
            "{storm:?}"
        );
        // A refused call is audited the same way.
        assert_eq!(
            error_code(&f.call(&session, &f.connection, "secret").await).as_deref(),
            Some("tool_not_allowed")
        );
        let rows = call_identities(&f, &session);
        assert!(rows.len() >= 5, "{rows:?}");
        assert!(rows.iter().all(|r| *r == Some("unverified")), "{rows:?}");

        // The host now says it checks every connection.
        let verified =
            serde_json::json!({"state": "verified", "mechanism": "so_peerpidfd", "reason": null});
        let mut caps = host_caps(true);
        caps["identity"] = verified.clone();
        let (code, _) = send(
            &f.app,
            post_json_with_auth("/v1/runtime/inventory", caps.clone(), &f.host_bearer),
        )
        .await;
        assert!(code.is_success());
        assert_eq!(
            host_identity_seen(&f).await,
            (verified.clone(), verified.clone())
        );
        f.call(&session, &f.connection, "echo").await;
        assert_eq!(call_identities(&f, &session)[0], Some("verified"));

        // A runtime downgrade (135d) is in force from the next call.
        let downgraded = serde_json::json!({"state": "unavailable", "mechanism": "so_peerpidfd", "reason": "ptrace_scope"});
        caps["identity"] = downgraded.clone();
        send(
            &f.app,
            post_json_with_auth("/v1/runtime/inventory", caps, &f.host_bearer),
        )
        .await;
        assert_eq!(
            tool_text(&f.call(&session, &f.connection, "echo").await),
            "called echo"
        );
        assert_eq!(call_identities(&f, &session)[0], Some("unverified"));

        // Offline, the host and its sessions keep their last word.
        let generation = f.state.agent.connect_host(&f.host_id).0;
        f.state.agent.disconnect_host(&f.host_id, generation);
        assert_eq!(
            host_identity_seen(&f).await,
            (downgraded.clone(), serde_json::Value::Null)
        );
        let (_, view) = send(
            &f.app,
            get_with_auth(&format!("/v1/agent/sessions/{session}"), &f.owner_bearer),
        )
        .await;
        assert_eq!(view["identity"], downgraded);
    }

    /// 135a: a connection the host refused is a security event on the
    /// session it named, and only the session's own host may say so.
    #[tokio::test]
    async fn a_refused_connection_is_audited_on_its_session() {
        let dir = tempdir::TempDir::new("storm-identity-refused").unwrap();
        let mut f = gateway_fixture(dir.path()).await;
        let session = f.launch("claude-code", false).await["id"]
            .as_str()
            .unwrap()
            .to_string();
        let refuse = |body: serde_json::Value, session: &str, bearer: &str| {
            post_json_with_auth(
                &format!("/v1/runtime/sessions/{session}/identity-refused"),
                body,
                bearer,
            )
        };
        let (code, _) = send(
            &f.app,
            refuse(
                serde_json::json!({"connection": "harness", "reason": "reached_host"}),
                &session,
                &f.host_bearer,
            ),
        )
        .await;
        assert_eq!(code, StatusCode::NO_CONTENT);
        let events = f.state.auth_db.lock().await.all_events().unwrap();
        let (_, detail) = events
            .iter()
            .find(|(kind, _)| kind == crate::ops::EVENT_IDENTITY_MISMATCH)
            .expect("an identity_mismatch event");
        let detail: serde_json::Value = serde_json::from_str(detail).unwrap();
        assert_eq!(
            detail,
            serde_json::json!({"session_id": session, "host_id": f.host_id,
                "connection": "harness", "reason": "reached_host"})
        );

        let (code, _) = send(
            &f.app,
            refuse(
                serde_json::json!({"connection": "bridge", "reason": "not a code\n"}),
                &session,
                &f.host_bearer,
            ),
        )
        .await;
        assert_eq!(code, StatusCode::BAD_REQUEST);
        let (other_host, other_token) = enroll_a_host(&f.app, &f.owner_bearer).await;
        let _other = f.state.agent.connect_host(&other_host);
        let (code, _) = send(
            &f.app,
            refuse(
                serde_json::json!({"connection": "bridge", "reason": "reached_host"}),
                &session,
                &format!("Bearer {other_token}"),
            ),
        )
        .await;
        assert_eq!(code, StatusCode::FORBIDDEN);
        assert_eq!(
            f.state
                .auth_db
                .lock()
                .await
                .event_count(crate::ops::EVENT_IDENTITY_MISMATCH)
                .unwrap(),
            1
        );
    }

    #[tokio::test]
    async fn an_agent_reaches_an_integration_through_the_host_link_and_only_its_allowlist() {
        let dir = tempdir::TempDir::new("storm-gateway-route").unwrap();
        let mut f = gateway_fixture(dir.path()).await;
        let launched = f.launch("claude-code", false).await;
        let session = launched["id"].as_str().unwrap().to_string();
        // The launch granted the owner's connection and `storm`, and told the
        // host ids and slugs only (AM23).
        let granted: Vec<&str> = launched["mcp"]["connections"]
            .as_array()
            .unwrap()
            .iter()
            .map(|g| g["id"].as_str().unwrap())
            .collect();
        assert_eq!(granted, vec!["storm", f.connection.as_str()]);
        let start = serde_json::to_string(&f.link.try_recv().unwrap()).unwrap();
        assert!(start.contains("\"mcp\""), "{start}");
        assert!(
            !start.contains("upstream-canary"),
            "a credential reached the host: {start}"
        );

        // Before `initialize`, the gateway forwards nothing (G-D19).
        let lines = f.call(&session, &f.connection, "echo").await;
        assert_eq!(
            lines,
            vec![serde_json::json!({"storm_error": "session_unknown"})]
        );
        assert_eq!(
            f.upstream_calls(),
            0,
            "a session_unknown request was forwarded"
        );

        let init = f.initialize(&session, &f.connection).await;
        assert_eq!(init["serverInfo"]["name"], "mock-upstream");

        // The agent sees only the allowlist, and calls exactly once.
        let lines = f
            .mcp(
                &session,
                &f.connection,
                serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
            )
            .await;
        let names: Vec<String> = lines[0]["message"]["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_string())
            .collect();
        assert!(!names.contains(&"secret".to_string()), "{names:?}");
        assert!(names.contains(&"echo".to_string()));
        let lines = f.call(&session, &f.connection, "echo").await;
        assert_eq!(tool_text(&lines), "called echo");
        assert_eq!(lines[0]["message"]["id"], 7);
        assert_eq!(f.upstream_calls(), 1);
        let lines = f.call(&session, &f.connection, "secret").await;
        assert_eq!(error_code(&lines).as_deref(), Some("tool_not_allowed"));
        assert_eq!(f.upstream_calls(), 1, "a disallowed tool reached upstream");

        // What went upstream: no sampling, no roots, no URL elicitation (§9).
        let caps: serde_json::Value =
            serde_json::from_str(&tool_text(&f.call(&session, &f.connection, "caps").await))
                .unwrap();
        assert_eq!(caps, serde_json::json!({"elicitation": {"form": {}}}));

        // Every call left a metadata-only audit row with its session.
        let calls = f
            .state
            .gateway
            .store
            .lock()
            .unwrap()
            .recent_calls(20)
            .unwrap();
        assert!(
            calls
                .iter()
                .any(|c| c.session_id.as_deref() == Some(session.as_str())
                    && c.tool.as_deref() == Some("echo")
                    && c.outcome == "ok")
        );
        assert!(
            calls
                .iter()
                .any(|c| c.error_code.as_deref() == Some("tool_not_allowed"))
        );

        // A user credential cannot reach the route; it is the host's tier.
        let response = f
            .app
            .clone()
            .oneshot(gateway_request(
                &f.owner_bearer,
                &session,
                &f.connection,
                &serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "ping"}),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn every_authorization_check_refuses_with_a_stable_code() {
        // Spec §7, one check at a time.
        let dir = tempdir::TempDir::new("storm-gateway-authz").unwrap();
        let mut f = gateway_fixture(dir.path()).await;
        let session = f.launch("claude-code", false).await["id"]
            .as_str()
            .unwrap()
            .to_string();
        f.initialize(&session, &f.connection).await;
        assert_eq!(
            tool_text(&f.call(&session, &f.connection, "echo").await),
            "called echo"
        );

        // A connection added after launch is not granted (§6: fixed grants).
        let later = create_github(&f.app, &f.owner_bearer, "ghp_later").await["id"]
            .as_str()
            .unwrap()
            .to_string();
        assert_eq!(
            error_code(&f.call(&session, &later, "echo").await).as_deref(),
            Some("not_granted")
        );
        // An unknown session, and a session that is not this host's: another
        // enrolled host's valid token reaches the route and is refused.
        assert_eq!(
            error_code(&f.call("ags_NOPE", &f.connection, "echo").await).as_deref(),
            Some("not_your_session")
        );
        let (_, other) = enroll_a_host(&f.app, &f.owner_bearer).await;
        let lines = mcp_as(&f.app, &format!("Bearer {other}"), &session, &f.connection,
            serde_json::json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "echo", "arguments": {}}})).await;
        assert_eq!(error_code(&lines).as_deref(), Some("not_your_session"));
        // Disabled: refused at once on the live session.
        let path = format!("{CONNECTIONS}/{}", f.connection);
        send(
            &f.app,
            patch_json_with_auth(
                &path,
                serde_json::json!({"enabled": false}),
                &f.owner_bearer,
            ),
        )
        .await;
        assert_eq!(
            error_code(&f.call(&session, &f.connection, "echo").await).as_deref(),
            Some("not_granted")
        );
        send(
            &f.app,
            patch_json_with_auth(&path, serde_json::json!({"enabled": true}), &f.owner_bearer),
        )
        .await;
        // Disabling closed the upstream session: the bridge must re-initialize.
        assert_eq!(
            f.call(&session, &f.connection, "echo").await,
            vec![serde_json::json!({"storm_error": "session_unknown"})]
        );
        f.initialize(&session, &f.connection).await;
        assert_eq!(
            tool_text(&f.call(&session, &f.connection, "echo").await),
            "called echo"
        );

        // A session whose owner is not the account (a removed account's).
        {
            let db = f.state.auth_db.lock().await;
            db.conn_for_tests()
                .execute_batch("PRAGMA foreign_keys = OFF; UPDATE users SET id = id || '_moved';")
                .unwrap();
        }
        assert_eq!(
            error_code(&f.call(&session, &f.connection, "echo").await).as_deref(),
            Some("owner_inactive")
        );
        {
            let db = f.state.auth_db.lock().await;
            db.conn_for_tests()
                .execute_batch(
                    "UPDATE users SET id = substr(id, 1, length(id) - 6); PRAGMA foreign_keys = ON;",
                )
                .unwrap();
        }

        // Disconnected mid-session: refused at once, and the grant is revoked.
        let before = f.upstream_calls();
        let (status, _) = send(&f.app, delete_with_auth(&path, &f.owner_bearer)).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        assert_eq!(
            error_code(&f.call(&session, &f.connection, "echo").await).as_deref(),
            Some("not_granted")
        );
        assert_eq!(f.upstream_calls(), before);
        assert_eq!(f.state.agent.grant(&session, &f.connection).unwrap(), None);

        // An ended session: refused, and its upstream sessions are gone.
        f.initialize(&session, "storm").await;
        f.report(&session, "completed").await;
        assert_eq!(
            error_code(&f.call(&session, "storm", "list_vaults").await).as_deref(),
            Some("session_not_live")
        );
        assert!(f.state.gateway.sessions.get(&session, "storm").is_none());
        let _ = &f.owner;
    }

    // ---- MCP Gateway: host scope (decision 121) ---------------------------

    fn granted_ids(launched: &serde_json::Value) -> Vec<String> {
        launched["mcp"]["connections"]
            .as_array()
            .unwrap()
            .iter()
            .map(|g| g["id"].as_str().unwrap().to_string())
            .collect()
    }

    async fn scope(f: &GatewayFixture, id: &str, hosts: serde_json::Value) -> serde_json::Value {
        let (status, body) = send(
            &f.app,
            patch_json_with_auth(
                &format!("{CONNECTIONS}/{id}"),
                serde_json::json!({"hosts": hosts}),
                &f.owner_bearer,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body
    }

    /// `GET /v1/integrations/suggestions`, answering the host's `refresh`
    /// with `caps`: the route asks for a fresh inventory before it lists.
    async fn suggestions(f: &mut GatewayFixture, caps: &serde_json::Value) -> serde_json::Value {
        let GatewayFixture {
            app,
            state,
            owner_bearer,
            host_id,
            link,
            ..
        } = f;
        let get = send(
            app,
            get_with_auth("/v1/integrations/suggestions", owner_bearer),
        );
        let answer = async {
            while let Some(envelope) = link.recv().await {
                if serde_json::to_value(&envelope).unwrap()["type"] == "refresh" {
                    state
                        .agent
                        .inventory(host_id, serde_json::from_value(caps.clone()).unwrap())
                        .unwrap();
                    break;
                }
            }
        };
        let ((status, body), ()) = tokio::join!(get, answer);
        assert_eq!(status, StatusCode::OK, "{body}");
        body
    }

    fn suggested(list: &serde_json::Value) -> Vec<String> {
        list.as_array()
            .unwrap()
            .iter()
            .map(|s| s["plugin"].as_str().unwrap().to_string())
            .collect()
    }

    /// 131g, 131h: a host's remote plugin servers are suggested once per URL,
    /// minus what the owner added or dismissed and anything malformed; one
    /// added from a suggestion keeps its plugin origin and its reporting
    /// host, and its grant carries the origin down to the host.
    #[tokio::test]
    async fn remote_plugin_servers_are_suggested_and_one_added_keeps_its_origin() {
        let dir = tempdir::TempDir::new("storm-plugin-suggestions").unwrap();
        let mut f = gateway_fixture(dir.path()).await;
        let (_, mock) = f.get(&format!("{CONNECTIONS}/{}", f.connection)).await;
        let remote = |agent: &str, plugin: &str, server: &str, url: &str, hint: Option<&str>| {
            serde_json::json!({"agent": agent, "plugin": plugin, "server": server,
                "transport": "http", "url": url, "templated": url.contains("${"), "auth_hint": hint})
        };
        let mut caps = host_caps(true);
        caps["plugin_mcp_remote"] = serde_json::json!([
            remote(
                "claude-code",
                "slack",
                "slack",
                "https://mcp.slack.com/mcp",
                Some("vendor_client")
            ),
            remote(
                "codex",
                "slack",
                "slack",
                "https://MCP.slack.com/mcp/",
                None
            ),
            remote(
                "claude-code",
                "linear",
                "linear",
                "https://mcp.linear.app/mcp",
                Some("header:Bad Name")
            ),
            remote(
                "claude-code",
                "jfrog",
                "jfrog",
                "${JFROG_URL}/mcp",
                Some("header:Authorization")
            ),
            remote(
                "claude-code",
                "mine",
                "mine",
                mock["url"].as_str().unwrap(),
                None
            ),
            remote(
                "claude-code",
                "bad name",
                "x",
                "https://bad.example.com/mcp",
                None
            ),
            remote(
                "claude-code",
                "plain",
                "plain",
                "http://203.0.113.9/mcp",
                None
            ),
        ]);
        f.state.gateway.set_allow_http_upstreams(false);

        let list = suggestions(&mut f, &caps).await;
        assert_eq!(suggested(&list), ["jfrog", "linear", "slack"], "{list}");
        let slack = &list[2];
        assert_eq!(slack["url"], "https://mcp.slack.com/mcp");
        assert_eq!(slack["agents"], serde_json::json!(["claude-code", "codex"]));
        assert_eq!(slack["hosts"], serde_json::json!([f.host_id]));
        assert_eq!(slack["auth_hint"], "vendor_client");
        assert_eq!(list[0]["templated"], true);
        assert_eq!(list[0]["auth_hint"], "header:Authorization");
        assert_eq!(
            list[1]["auth_hint"],
            serde_json::Value::Null,
            "a bad hint is dropped"
        );

        let (status, _) = send(
            &f.app,
            post_json_with_auth(
                "/v1/integrations/suggestions/dismiss",
                serde_json::json!({"plugin": "linear", "server": "linear", "url": "https://mcp.linear.app/mcp/"}),
                &f.owner_bearer,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        assert_eq!(
            suggested(&suggestions(&mut f, &caps).await),
            ["jfrog", "slack"]
        );

        let (status, added) = send(
            &f.app,
            post_json_with_auth(
                CONNECTIONS,
                serde_json::json!({"display_name": "Slack", "url": "https://mcp.slack.com/mcp",
                    "auth_kind": "none", "plugin_origin": {"plugin": "slack", "server": "slack"},
                    "hosts": [f.host_id]}),
                &f.owner_bearer,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{added}");
        assert_eq!(
            added["plugin_origin"],
            serde_json::json!({"plugin": "slack", "server": "slack"})
        );
        assert_eq!(added["hosts"], serde_json::json!([f.host_id]));
        let slack_id = added["id"].as_str().unwrap().to_string();
        let (_, again) = f.get(&format!("{CONNECTIONS}/{slack_id}")).await;
        assert_eq!(again["plugin_origin"]["plugin"], "slack");
        assert_eq!(suggested(&suggestions(&mut f, &caps).await), ["jfrog"]);

        f.launch("claude-code", false).await;
        let start = loop {
            let envelope = serde_json::to_value(f.link.try_recv().unwrap()).unwrap();
            if envelope["type"] == "start" {
                break envelope;
            }
        };
        let grant = start["mcp"]
            .as_array()
            .unwrap()
            .iter()
            .find(|g| g["id"] == slack_id.as_str())
            .unwrap()
            .clone();
        assert_eq!(
            grant["plugin_origin"],
            serde_json::json!({"plugin": "slack", "server": "slack"})
        );
        let mock_grant = start["mcp"]
            .as_array()
            .unwrap()
            .iter()
            .find(|g| g["id"] == f.connection.as_str())
            .unwrap()
            .clone();
        assert!(mock_grant.get("plugin_origin").is_none(), "{mock_grant}");

        for bad in [
            serde_json::json!({"plugin": "a b", "server": "x"}),
            serde_json::json!({"plugin": "", "server": "x"}),
        ] {
            let (status, _) = send(
                &f.app,
                post_json_with_auth(
                    CONNECTIONS,
                    serde_json::json!({"display_name": "Odd", "url": "https://odd.example.com/mcp",
                        "auth_kind": "none", "plugin_origin": bad}),
                    &f.owner_bearer,
                ),
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST);
        }
        let (status, _) = send(
            &f.app,
            post_json_with_auth(
                CONNECTIONS,
                serde_json::json!({"display_name": "Nowhere", "url": "https://nowhere.example.com/mcp",
                    "auth_kind": "none", "hosts": ["rh_missing"]}),
                &f.owner_bearer,
            ),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "a create's hosts are checked"
        );
    }

    /// 131a: the first `running` report's skipped servers stay with the
    /// session, bounded, and a later report never replaces them.
    #[tokio::test]
    async fn a_sessions_skipped_servers_are_kept_from_its_first_report() {
        let dir = tempdir::TempDir::new("storm-mcp-notice").unwrap();
        let mut f = gateway_fixture(dir.path()).await;
        let launched = f.launch("claude-code", false).await;
        let id = launched["id"].as_str().unwrap().to_string();
        let (_, before) = f.get(&format!("/v1/agent/sessions/{id}")).await;
        assert!(before.get("mcp_skipped").is_none(), "{before}");
        let long = "r".repeat(1000);
        for (skipped, local) in [
            (
                serde_json::json!([{"name": "slack", "source": "plugin slack", "reason": long}]),
                serde_json::json!(["plugin_pw_playwright"]),
            ),
            (
                serde_json::json!([{"name": "later", "source": "x", "reason": "y"}]),
                serde_json::json!([]),
            ),
        ] {
            let (code, body) = send(
                &f.app,
                post_json_with_auth(
                    &format!("/v1/runtime/sessions/{id}/status"),
                    serde_json::json!({"status": "running", "mcp_skipped": skipped, "mcp_plugin_local": local}),
                    &f.host_bearer,
                ),
            )
            .await;
            assert!(code.is_success(), "{code} {body}");
        }
        let (_, view) = f.get(&format!("/v1/agent/sessions/{id}")).await;
        assert_eq!(view["mcp_skipped"][0]["name"], "slack", "{view}");
        assert_eq!(view["mcp_skipped"].as_array().unwrap().len(), 1);
        assert_eq!(
            view["mcp_skipped"][0]["reason"].as_str().unwrap().len(),
            256
        );
        assert_eq!(
            view["mcp_plugin_local"],
            serde_json::json!(["plugin_pw_playwright"])
        );
    }

    #[tokio::test]
    async fn a_launch_grants_only_the_integrations_scoped_to_its_host() {
        let dir = tempdir::TempDir::new("storm-host-scope-launch").unwrap();
        let mut f = gateway_fixture(dir.path()).await;
        let (other_host, _) = enroll_a_host(&f.app, &f.owner_bearer).await;
        let elsewhere = create_github(&f.app, &f.owner_bearer, "ghp_elsewhere").await["id"]
            .as_str()
            .unwrap()
            .to_string();
        let (_, list) = f.get(CONNECTIONS).await;
        for view in list.as_array().unwrap() {
            assert_eq!(view["hosts"], "all", "{view}");
        }

        let body = scope(&f, &elsewhere, serde_json::json!([other_host])).await;
        assert_eq!(body["hosts"], serde_json::json!([other_host]));
        let launched = f.launch("claude-code", false).await;
        assert_eq!(
            granted_ids(&launched),
            vec!["storm".to_string(), f.connection.clone()]
        );
        let start = serde_json::to_string(&f.link.try_recv().unwrap()).unwrap();
        assert!(!start.contains(&elsewhere), "{start}");
        let integrations: Vec<&str> = launched["integrations"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["id"].as_str().unwrap())
            .collect();
        assert_eq!(integrations, vec![f.connection.as_str()]);

        // In scope by name, and back to every host: granted.
        scope(&f, &elsewhere, serde_json::json!([other_host, f.host_id])).await;
        let mut granted = granted_ids(&f.launch("claude-code", false).await);
        granted.sort();
        let mut want = vec!["storm".to_string(), f.connection.clone(), elsewhere.clone()];
        want.sort();
        assert_eq!(granted, want);
        let body = scope(&f, &elsewhere, serde_json::json!("all")).await;
        assert_eq!(body["hosts"], "all");
        assert_eq!(granted_ids(&f.launch("claude-code", false).await).len(), 3);
    }

    #[tokio::test]
    async fn narrowing_the_scope_refuses_the_next_call_and_storm_keeps_working() {
        let dir = tempdir::TempDir::new("storm-host-scope-call").unwrap();
        let mut f = gateway_fixture(dir.path()).await;
        let (other_host, _) = enroll_a_host(&f.app, &f.owner_bearer).await;
        let session = f.launch("claude-code", false).await["id"]
            .as_str()
            .unwrap()
            .to_string();
        f.initialize(&session, &f.connection).await;
        f.initialize(&session, "storm").await;
        assert_eq!(
            tool_text(&f.call(&session, &f.connection, "echo").await),
            "called echo"
        );

        let connection = f.connection.clone();
        scope(&f, &connection, serde_json::json!([other_host])).await;
        // The same scope again is no change, and no event.
        scope(&f, &connection, serde_json::json!([other_host, other_host])).await;
        let before = f.upstream_calls();
        assert_eq!(
            error_code(&f.call(&session, &f.connection, "echo").await).as_deref(),
            Some("host_not_in_scope")
        );
        assert_eq!(
            f.upstream_calls(),
            before,
            "an out-of-scope call went upstream"
        );
        assert!(
            f.state
                .gateway
                .store
                .lock()
                .unwrap()
                .recent_calls(20)
                .unwrap()
                .iter()
                .any(|c| c.error_code.as_deref() == Some("host_not_in_scope"))
        );
        let storm = f.call(&session, "storm", "list_vaults").await;
        assert!(
            storm.last().unwrap().pointer("/message/result").is_some(),
            "{storm:?}"
        );

        scope(&f, &connection, serde_json::json!("all")).await;
        let scopes: Vec<serde_json::Value> = f
            .state
            .auth_db
            .lock()
            .await
            .all_events()
            .unwrap()
            .into_iter()
            .filter(|(kind, _)| kind == "integration_hosts_changed")
            .map(|(_, detail)| {
                serde_json::from_str::<serde_json::Value>(&detail).unwrap()["hosts"].clone()
            })
            .collect();
        assert_eq!(
            scopes,
            [serde_json::json!([other_host]), serde_json::json!("all")]
        );
        assert_eq!(
            tool_text(&f.call(&session, &f.connection, "echo").await),
            "called echo"
        );
    }

    #[tokio::test]
    async fn a_host_scope_names_live_hosts_only() {
        let dir = tempdir::TempDir::new("storm-host-scope-patch").unwrap();
        let f = gateway_fixture(dir.path()).await;
        let (revoked, _) = enroll_a_host(&f.app, &f.owner_bearer).await;
        let (status, _) = send(
            &f.app,
            delete_with_auth(&format!("/v1/agent/hosts/{revoked}"), &f.owner_bearer),
        )
        .await;
        assert!(status.is_success(), "{status}");
        let path = format!("{CONNECTIONS}/{}", f.connection);
        for (target, hosts) in [
            (&path, serde_json::json!(["hst_NOPE"])),
            (&path, serde_json::json!([revoked])),
            (&path, serde_json::json!([f.host_id, revoked])),
            (&path, serde_json::json!([])),
            (
                &format!("{CONNECTIONS}/storm"),
                serde_json::json!([f.host_id]),
            ),
        ] {
            let (status, body) = send(
                &f.app,
                patch_json_with_auth(target, serde_json::json!({"hosts": hosts}), &f.owner_bearer),
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{hosts} {body}");
        }
        let (_, view) = f.get(&path).await;
        assert_eq!(view["hosts"], "all", "a refused patch changed the scope");
        let (_, storm) = f.get(&format!("{CONNECTIONS}/storm")).await;
        assert_eq!(storm["hosts"], "all");
    }

    #[tokio::test]
    async fn the_single_user_sweep_retires_what_a_removed_account_left() {
        let dir = tempdir::TempDir::new("storm-single-user-sweep").unwrap();
        let mut f = gateway_fixture(dir.path()).await;
        let live = f.launch("claude-code", false).await["id"]
            .as_str()
            .unwrap()
            .to_string();
        let ended = f.launch("claude-code", false).await["id"]
            .as_str()
            .unwrap()
            .to_string();
        f.report(&ended, "completed").await;

        crate::ops::reconcile_single_user(&f.state).await.unwrap();
        assert_eq!(f.state.agent.get(&live).unwrap().status, "running");

        {
            let db = f.state.auth_db.lock().await;
            db.conn_for_tests()
                .execute_batch(
                    "PRAGMA foreign_keys = OFF; UPDATE users SET id = 'usr_the_survivor'; PRAGMA foreign_keys = ON;",
                )
                .unwrap();
        }
        crate::ops::reconcile_single_user(&f.state).await.unwrap();

        let connection = crate::ops::gateway_store(&f.state)
            .connection(&f.connection)
            .unwrap()
            .unwrap();
        assert_eq!(
            connection.status, "revoked",
            "the removed account's connection"
        );
        assert!(
            !crate::ops::gateway_store(&f.state)
                .has_credential(&f.connection)
                .unwrap(),
            "its sealed credential went with it"
        );
        let retired = f.state.agent.get(&live).unwrap();
        assert_eq!(
            (retired.status.as_str(), retired.end_reason.as_deref()),
            ("failed", Some("owner_removed"))
        );
        assert!(
            f.state.agent.get(&ended).is_err(),
            "an ended one is dismissed"
        );
        assert_eq!(f.state.agent.grant(&live, &f.connection).unwrap(), None);

        let before = f
            .state
            .auth_db
            .lock()
            .await
            .event_count("integration_removed_single_user")
            .unwrap();
        crate::ops::reconcile_single_user(&f.state).await.unwrap();
        let after = f
            .state
            .auth_db
            .lock()
            .await
            .event_count("integration_removed_single_user")
            .unwrap();
        assert_eq!((before, after), (1, 1));
        let _ = &f.owner;
    }

    #[tokio::test]
    async fn shell_gets_nothing_and_an_old_host_is_announced() {
        let dir = tempdir::TempDir::new("storm-gateway-shell").unwrap();
        let mut f = gateway_fixture(dir.path()).await;
        let shell = f.launch("shell", false).await;
        assert_eq!(shell["mcp"]["connections"], serde_json::json!([]));
        assert_eq!(shell["mcp"]["notice"], serde_json::Value::Null);
        let session = shell["id"].as_str().unwrap();
        assert_eq!(
            error_code(&f.call(session, "storm", "list_vaults").await).as_deref(),
            Some("not_granted")
        );

        // The same host, reporting no bridge: no grants, and the launch says why.
        f.state
            .agent
            .hello(
                &f.host_id,
                serde_json::from_value(host_caps(false)).unwrap(),
            )
            .unwrap();
        let old = f.launch("claude-code", false).await;
        assert_eq!(old["mcp"]["connections"], serde_json::json!([]));
        assert_eq!(
            old["mcp"]["notice"],
            "build-vm can't use integrations — update storm-runtime"
        );
    }

    fn tools_of(lines: Vec<serde_json::Value>) -> Vec<String> {
        lines[0]["message"]["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_string())
            .collect()
    }

    fn structured(lines: &[serde_json::Value]) -> serde_json::Value {
        lines
            .last()
            .and_then(|l| l.pointer("/message/result/structuredContent"))
            .cloned()
            .unwrap_or(serde_json::Value::Null)
    }

    fn tool_error(lines: &[serde_json::Value]) -> String {
        let result = &lines.last().unwrap()["message"]["result"];
        assert_eq!(result["isError"], true, "{lines:?}");
        result["structuredContent"]["error"]
            .as_str()
            .unwrap_or("")
            .to_string()
    }

    impl GatewayFixture {
        async fn vault(&self, name: &str) -> String {
            let (status, body) = send(
                &self.app,
                post_json_with_auth(
                    "/v1/vaults",
                    serde_json::json!({"name": name}),
                    &self.owner_bearer,
                ),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{body}");
            body["id"].as_str().unwrap().to_string()
        }

        async fn note(&self, vault: &str, path: &str, content: &str) -> serde_json::Value {
            let (status, body) = send(
                &self.app,
                post_json_with_auth(
                    &format!("/v1/vaults/{vault}/notes"),
                    serde_json::json!({"path": path, "content": content}),
                    &self.owner_bearer,
                ),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{body}");
            body["note"].clone()
        }

        async fn get(&self, path: &str) -> (StatusCode, serde_json::Value) {
            send(&self.app, get_with_auth(path, &self.owner_bearer)).await
        }

        /// Launches with extra fields; the host reports it running.
        async fn launch_body(
            &mut self,
            extra: serde_json::Value,
        ) -> (StatusCode, serde_json::Value) {
            let mut body = serde_json::json!({
                "host_id": self.host_id, "workspace": "storm", "provider": "claude-code",
                "terminal": {"cols": 80, "rows": 24},
            });
            for (k, v) in extra.as_object().unwrap() {
                body[k] = v.clone();
            }
            let (status, body) = send(
                &self.app,
                post_json_with_auth("/v1/agent/sessions", body, &self.owner_bearer),
            )
            .await;
            if status == StatusCode::OK {
                self.report(body["id"].as_str().unwrap(), "running").await;
            }
            (status, body)
        }

        async fn agent_writes(&self, on: bool) {
            let (status, body) = send(
                &self.app,
                put_json_with_auth(
                    "/v1/config/mcp",
                    serde_json::json!({"agent_writes": on}),
                    &self.owner_bearer,
                ),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{body}");
            assert_eq!(body["agent_writes"], on);
        }

        async fn tool(
            &self,
            session: &str,
            tool: &str,
            arguments: serde_json::Value,
        ) -> Vec<serde_json::Value> {
            self.mcp(
                session,
                "storm",
                serde_json::json!({"jsonrpc": "2.0", "id": 9, "method": "tools/call",
                    "params": {"name": tool, "arguments": arguments}}),
            )
            .await
        }
    }

    #[tokio::test]
    async fn an_agent_writes_only_to_its_write_vault_under_agent_writes_and_never_deletes() {
        let dir = tempdir::TempDir::new("storm-gateway-builtin").unwrap();
        let mut f = gateway_fixture(dir.path()).await;
        let work = f.vault("Work").await;
        let personal = f.vault("Personal").await;
        let theirs = f.note(&personal, "Theirs.md", "# Theirs\n").await;
        let list = serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"});

        // No write vault: read only, whatever the switches say.
        f.agent_writes(true).await;
        let (_, s1) = f.launch_body(serde_json::json!({})).await;
        let s1 = s1["id"].as_str().unwrap().to_string();
        assert_eq!(
            f.initialize(&s1, "storm").await["serverInfo"]["name"],
            "storm"
        );
        let tools = tools_of(f.mcp(&s1, "storm", list.clone()).await);
        assert!(tools.contains(&"list_vaults".to_string()), "{tools:?}");
        assert!(tools.contains(&"session_context".to_string()), "{tools:?}");
        assert!(
            !tools
                .iter()
                .any(|t| crate::mcp::WRITE_TOOLS.contains(&t.as_str())),
            "{tools:?}"
        );
        assert_eq!(
            error_code(&f.call(&s1, "storm", "create_note").await).as_deref(),
            Some("tool_not_allowed")
        );

        // An older client's `allow_vault_writes` without a vault: read only,
        // and the launch says so.
        let (_, old) = f
            .launch_body(serde_json::json!({"allow_vault_writes": true}))
            .await;
        assert_eq!(old["write_vault_id"], serde_json::Value::Null);
        assert_eq!(old["mcp"]["allow_vault_writes"], false);
        assert!(
            old["mcp"]["notice"].as_str().unwrap().contains("read only"),
            "{old}"
        );
        let s_old = old["id"].as_str().unwrap().to_string();
        f.initialize(&s_old, "storm").await;
        let tools = tools_of(f.mcp(&s_old, "storm", list.clone()).await);
        assert!(!tools.contains(&"create_note".to_string()), "{tools:?}");

        // A write vault with `agent_writes` on: writes there, refused
        // elsewhere at the vault seam with a stable code, never deletes.
        let (status, launched) = f
            .launch_body(serde_json::json!({"write_vault_id": work}))
            .await;
        assert_eq!(status, StatusCode::OK, "{launched}");
        assert_eq!(launched["write_vault_id"], work.as_str());
        assert_eq!(launched["mcp"]["allow_vault_writes"], true);
        assert_eq!(launched["mcp"]["notice"], serde_json::Value::Null);
        assert_eq!(launched["name"], "storm-3");
        let s3 = launched["id"].as_str().unwrap().to_string();
        f.initialize(&s3, "storm").await;
        let tools = tools_of(f.mcp(&s3, "storm", list.clone()).await);
        assert!(tools.contains(&"create_note".to_string()), "{tools:?}");
        assert!(!tools.contains(&"delete_note".to_string()), "{tools:?}");
        let made = f
            .tool(
                &s3,
                "create_note",
                serde_json::json!({"vault": work, "path": "Plan.md", "content": "# Plan\n"}),
            )
            .await;
        let note = structured(&made)["note"].clone();
        let note_id = note["id"].as_str().unwrap().to_string();
        let lines = f
            .tool(
                &s3,
                "create_note",
                serde_json::json!({"vault": personal, "path": "Stray.md", "content": "x"}),
            )
            .await;
        assert_eq!(
            error_code(&lines).as_deref(),
            Some(crate::auth::authz::AGENT_WRITE_REFUSED),
            "{lines:?}"
        );
        let lines = f
            .tool(
                &s3,
                "update_note",
                serde_json::json!({"vault": personal, "note_id": theirs["id"],
                    "base_version": theirs["version"], "content": "# Theirs\n\nedited\n"}),
            )
            .await;
        assert_eq!(
            error_code(&lines).as_deref(),
            Some(crate::auth::authz::AGENT_WRITE_REFUSED)
        );
        let (_, untouched) = f
            .get(&format!(
                "/v1/vaults/{personal}/notes/{}",
                theirs["id"].as_str().unwrap()
            ))
            .await;
        assert!(!untouched["content"].as_str().unwrap().contains("edited"));
        assert!(untouched.get("agent_write").is_none(), "{untouched}");
        let lines = f
            .tool(
                &s3,
                "get_note",
                serde_json::json!({"vault": personal, "note_id": theirs["id"]}),
            )
            .await;
        assert_eq!(structured(&lines)["title"], "Theirs", "reads anywhere");
        assert_eq!(
            error_code(&f.call(&s3, "storm", "delete_note").await).as_deref(),
            Some("tool_not_allowed")
        );

        // The MCP switches no longer touch agents, and `agent_writes` applies
        // to the live session at once.
        send(
            &f.app,
            put_json_with_auth(
                "/v1/config/mcp",
                serde_json::json!({"enabled": false}),
                &f.owner_bearer,
            ),
        )
        .await;
        let edited = f
            .tool(
                &s3,
                "update_note",
                serde_json::json!({"vault": work, "note_id": note_id,
                    "base_version": note["version"], "content": "# Plan\n\nv2\n"}),
            )
            .await;
        assert_eq!(structured(&edited)["note"]["version"], 2, "{edited:?}");
        f.agent_writes(false).await;
        assert_eq!(
            error_code(&f.call(&s3, "storm", "create_note").await).as_deref(),
            Some("tool_not_allowed")
        );
        let (_, config) = f.get("/v1/config").await;
        assert_eq!(
            (
                config["mcp_enabled"].clone(),
                config["agent_writes"].clone()
            ),
            (serde_json::json!(false), serde_json::json!(false))
        );
    }

    #[tokio::test]
    async fn an_agents_writes_are_recorded_and_its_notes_carry_provenance() {
        let dir = tempdir::TempDir::new("storm-gateway-writes").unwrap();
        let mut f = gateway_fixture(dir.path()).await;
        let work = f.vault("Work").await;
        let human = f.note(&work, "Human.md", "# Human\n").await;
        f.agent_writes(true).await;
        let (_, launched) = f
            .launch_body(serde_json::json!({"write_vault_id": work}))
            .await;
        let session = launched["id"].as_str().unwrap().to_string();
        let name = launched["name"].as_str().unwrap().to_string();
        assert_eq!(launched["wrote_count"], 0);
        let granted = serde_json::json!([
            {"id": f.connection, "slug": launched["mcp"]["connections"][1]["slug"], "display_name": "Mock"}
        ]);
        assert_eq!(launched["integrations"], granted, "{launched}");
        // Launch history: a later rename of the connection changes nothing.
        let (status, _) = send(
            &f.app,
            patch_json_with_auth(
                &format!("{CONNECTIONS}/{}", f.connection),
                serde_json::json!({"display_name": "Renamed"}),
                &f.owner_bearer,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (_, view) = f.get(&format!("/v1/agent/sessions/{session}")).await;
        assert_eq!(view["integrations"], granted, "{view}");
        f.initialize(&session, "storm").await;

        // Each write op through the gateway is one row.
        let made = f
            .tool(
                &session,
                "create_note",
                serde_json::json!({"vault": work, "path": "Agent.md", "content": "# Agent\n"}),
            )
            .await;
        let agent_note = structured(&made)["note"].clone();
        let agent_id = agent_note["id"].as_str().unwrap().to_string();
        let edited = f
            .tool(
                &session,
                "update_note",
                serde_json::json!({"vault": work, "note_id": human["id"],
                    "base_version": human["version"], "content": "# Human\n\nagent was here\n"}),
            )
            .await;
        assert_eq!(structured(&edited)["note"]["version"], 2, "{edited:?}");
        let again = f
            .tool(
                &session,
                "update_note",
                serde_json::json!({"vault": work, "note_id": agent_id,
                    "base_version": 1, "content": "# Agent\n\nmore\n"}),
            )
            .await;
        assert_eq!(structured(&again)["note"]["version"], 2, "{again:?}");

        // A human's own writes are not an agent's.
        f.note(&work, "Mine.md", "# Mine\n").await;

        let (status, writes) = f.get(&format!("/v1/agent/sessions/{session}/writes")).await;
        assert_eq!(status, StatusCode::OK, "{writes}");
        let rows: Vec<(String, String, i64)> = writes
            .as_array()
            .unwrap()
            .iter()
            .map(|w| {
                (
                    w["title"].as_str().unwrap().to_string(),
                    w["kind"].as_str().unwrap().to_string(),
                    w["version"].as_i64().unwrap(),
                )
            })
            .collect();
        assert_eq!(
            rows,
            [
                ("Agent".to_string(), "created".to_string(), 2),
                ("Human".to_string(), "edited".to_string(), 2)
            ],
            "newest write first; created stays created"
        );
        assert_eq!(writes[0]["vault_id"], work.as_str());
        assert_eq!(writes[0]["path"], "Agent.md");
        assert!(writes[0]["at"].as_str().unwrap().ends_with('Z'));
        let (_, view) = f.get(&format!("/v1/agent/sessions/{session}")).await;
        assert_eq!(view["wrote_count"], 2);
        assert_eq!(view["name"], name.as_str());
        assert_eq!(view["write_vault_id"], work.as_str());
        let (_, list) = f.get("/v1/agent/sessions").await;
        let listed = list
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["id"] == session.as_str())
            .unwrap();
        assert_eq!(listed["wrote_count"], 2);

        // Provenance on the note, which a later human edit does not clear.
        let human_id = human["id"].as_str().unwrap();
        let (_, note) = f.get(&format!("/v1/vaults/{work}/notes/{human_id}")).await;
        assert_eq!(note["agent_write"]["session_id"], session.as_str());
        assert_eq!(note["agent_write"]["session_name"], name.as_str());
        assert_eq!(note["agent_write"]["kind"], "edited");
        assert_eq!(note["agent_write"]["version"], 2);
        assert_eq!(note["agent_write"]["session_dismissed"], false);
        send(
            &f.app,
            put_json_with_auth(
                &format!("/v1/vaults/{work}/notes/{human_id}"),
                serde_json::json!({"base_version": 2, "content": "# Human\n\nme again\n"}),
                &f.owner_bearer,
            ),
        )
        .await;
        let (_, note) = f.get(&format!("/v1/vaults/{work}/notes/{human_id}")).await;
        assert_eq!(note["version"], 3);
        assert_eq!(note["agent_write"]["version"], 2, "{note}");

        let (status, map) = f.get(&format!("/v1/vaults/{work}/agent-writes")).await;
        assert_eq!(status, StatusCode::OK);
        let map = map.as_object().unwrap();
        assert_eq!(map.len(), 2, "{map:?}");
        assert_eq!(map[&agent_id]["session_id"], session.as_str());
        assert_eq!(map[&agent_id]["version"], 2);
        assert_eq!(map[human_id]["version"], 2);
        let (status, _) = f.get("/v1/vaults/nope/agent-writes").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (status, _) = f.get("/v1/agent/sessions/ags_NOPE/writes").await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        // A dismissed session's writes still resolve to its name.
        f.report(&session, "completed").await;
        let (status, _) = send(
            &f.app,
            delete_with_auth(&format!("/v1/agent/sessions/{session}"), &f.owner_bearer),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (_, note) = f.get(&format!("/v1/vaults/{work}/notes/{agent_id}")).await;
        assert_eq!(note["agent_write"]["session_name"], name.as_str());
        assert_eq!(note["agent_write"]["session_dismissed"], true);
        assert_eq!(note["agent_write"]["kind"], "created");
    }

    // ---- crews and role labels (decision 138) ----------------------------

    impl GatewayFixture {
        async fn ask(
            &self,
            req: axum::http::Request<axum::body::Body>,
        ) -> (StatusCode, serde_json::Value) {
            send(&self.app, req).await
        }

        async fn crew_of(&self, session: &str) -> serde_json::Value {
            let (_, view) = self.get(&format!("/v1/agent/sessions/{session}")).await;
            view["crew"].clone()
        }

        async fn crew_ids(&self) -> Vec<String> {
            let (status, crews) = self.get("/v1/agent/crews").await;
            assert_eq!(status, StatusCode::OK, "{crews}");
            crews
                .as_array()
                .unwrap()
                .iter()
                .map(|c| c["id"].as_str().unwrap().to_string())
                .collect()
        }
    }

    #[tokio::test]
    async fn crews_group_sessions_one_crew_each_and_change_nothing_about_them() {
        let dir = tempdir::TempDir::new("storm-crews").unwrap();
        let mut f = gateway_fixture(dir.path()).await;
        let mut ids = Vec::new();
        for _ in 0..3 {
            let body = f.launch("claude-code", false).await;
            ids.push(body["id"].as_str().unwrap().to_string());
        }
        let (s1, s2, s3) = (ids[0].clone(), ids[1].clone(), ids[2].clone());
        let (_, before) = f.get(&format!("/v1/agent/sessions/{s1}")).await;
        assert_eq!(before["crew"], serde_json::Value::Null);

        // A crew of two, coloured with the first unused palette word.
        let (status, made) = f
            .ask(post_json_with_auth(
                "/v1/agent/crews",
                serde_json::json!({"name": "  Gateway release ", "session_ids": [s1, s2]}),
                &f.owner_bearer,
            ))
            .await;
        assert_eq!(status, StatusCode::CREATED, "{made}");
        let gw = made["crew"]["id"].as_str().unwrap().to_string();
        assert!(gw.starts_with("crw_"), "{gw}");
        assert_eq!(made["crew"]["name"], "Gateway release");
        assert_eq!(made["crew"]["color"], "coral");
        assert_eq!(made["removed_crews"], serde_json::json!([]));
        assert_eq!(
            f.crew_of(&s1).await,
            serde_json::json!({"id": gw, "order": 0})
        );
        assert_eq!(
            f.crew_of(&s2).await,
            serde_json::json!({"id": gw, "order": 1})
        );
        assert_eq!(f.crew_of(&s3).await, serde_json::Value::Null);

        // A crew changes nothing else about a session (138a).
        let (_, after) = f.get(&format!("/v1/agent/sessions/{s1}")).await;
        let mut stripped = after.clone();
        stripped["crew"] = serde_json::Value::Null;
        assert_eq!(stripped, before);

        // A session is in at most one crew: making a second crew moves it.
        let (status, relay) = f
            .ask(post_json_with_auth(
                "/v1/agent/crews",
                serde_json::json!({"name": "Relay", "color": "sky", "session_ids": [s2]}),
                &f.owner_bearer,
            ))
            .await;
        assert_eq!(status, StatusCode::CREATED, "{relay}");
        let relay = relay["crew"]["id"].as_str().unwrap().to_string();
        assert_eq!(f.crew_of(&s2).await["id"], relay.as_str());
        assert_eq!(f.crew_ids().await, [gw.clone(), relay.clone()]);

        // Moving the last member out deletes the crew and says so (138c).
        let (status, moved) = f
            .ask(put_json_with_auth(
                &format!("/v1/agent/sessions/{s1}/crew"),
                serde_json::json!({"crew_id": relay, "before": s2}),
                &f.owner_bearer,
            ))
            .await;
        assert_eq!(status, StatusCode::OK, "{moved}");
        assert_eq!(moved["removed_crews"][0]["id"], gw.as_str());
        assert_eq!(moved["removed_crews"][0]["name"], "Gateway release");
        assert_eq!(moved["crew"]["id"], relay.as_str());
        assert_eq!(f.crew_ids().await, std::slice::from_ref(&relay));
        assert_eq!(f.crew_of(&s1).await["order"], 0);
        assert_eq!(f.crew_of(&s2).await["order"], 1);

        // Member order: every member exactly once.
        let order = |ids: serde_json::Value| {
            put_json_with_auth(
                &format!("/v1/agent/crews/{relay}/members/order"),
                serde_json::json!({ "ids": ids }),
                &f.owner_bearer,
            )
        };
        let (status, _) = f.ask(order(serde_json::json!([s2, s1]))).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        assert_eq!(f.crew_of(&s2).await["order"], 0);
        for bad in [
            serde_json::json!([s2]),
            serde_json::json!([s2, s1, s1]),
            serde_json::json!([s2, s3]),
        ] {
            let (status, body) = f.ask(order(bad)).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        }

        // Crew order, likewise.
        let (_, third) = f
            .ask(post_json_with_auth(
                "/v1/agent/crews",
                serde_json::json!({"name": "Spikes", "session_ids": [s3]}),
                &f.owner_bearer,
            ))
            .await;
        let spikes = third["crew"]["id"].as_str().unwrap().to_string();
        assert_eq!(
            third["crew"]["color"], "coral",
            "the first colour is free again"
        );
        let (status, _) = f
            .ask(put_json_with_auth(
                "/v1/agent/crews/order",
                serde_json::json!({"ids": [spikes, relay]}),
                &f.owner_bearer,
            ))
            .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        assert_eq!(f.crew_ids().await, [spikes.clone(), relay.clone()]);
        let (status, _) = f
            .ask(put_json_with_auth(
                "/v1/agent/crews/order",
                serde_json::json!({"ids": [spikes]}),
                &f.owner_bearer,
            ))
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        // Rename and recolour, validated.
        let patch = |body: serde_json::Value| {
            patch_json_with_auth(&format!("/v1/agent/crews/{relay}"), body, &f.owner_bearer)
        };
        let (status, renamed) = f
            .ask(patch(
                serde_json::json!({"name": "Relay hardening", "color": "mint"}),
            ))
            .await;
        assert_eq!(status, StatusCode::OK, "{renamed}");
        assert_eq!(renamed["name"], "Relay hardening");
        assert_eq!(renamed["color"], "mint");
        for bad in [
            serde_json::json!({"color": "rose"}),
            serde_json::json!({"name": "   "}),
            serde_json::json!({"name": "x".repeat(81)}),
            serde_json::json!({"name": "two\nlines"}),
        ] {
            let (status, body) = f.ask(patch(bad)).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        }
        let (status, _) = f
            .ask(patch_json_with_auth(
                "/v1/agent/crews/crw_NOPE",
                serde_json::json!({"name": "x"}),
                &f.owner_bearer,
            ))
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        // Moves that cannot be.
        for (session, body, want) in [
            (
                s3.clone(),
                serde_json::json!({"crew_id": "crw_NOPE"}),
                StatusCode::NOT_FOUND,
            ),
            (
                "ags_NOPE".to_string(),
                serde_json::json!({"crew_id": relay}),
                StatusCode::NOT_FOUND,
            ),
            (
                s3.clone(),
                serde_json::json!({"crew_id": relay, "before": s3}),
                StatusCode::BAD_REQUEST,
            ),
            (
                s3.clone(),
                serde_json::json!({"crew_id": null, "before": s1}),
                StatusCode::BAD_REQUEST,
            ),
        ] {
            let (status, out) = f
                .ask(put_json_with_auth(
                    &format!("/v1/agent/sessions/{session}/crew"),
                    body,
                    &f.owner_bearer,
                ))
                .await;
            assert_eq!(status, want, "{out}");
        }
        let (status, _) = f
            .ask(post_json_with_auth(
                "/v1/agent/crews",
                serde_json::json!({"name": "Twice", "session_ids": [s1, s1]}),
                &f.owner_bearer,
            ))
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        // Leaving: the session goes loose, and its emptied crew with it.
        let (status, left) = f
            .ask(put_json_with_auth(
                &format!("/v1/agent/sessions/{s3}/crew"),
                serde_json::json!({"crew_id": null}),
                &f.owner_bearer,
            ))
            .await;
        assert_eq!(status, StatusCode::OK, "{left}");
        assert_eq!(left["removed_crews"][0]["id"], spikes.as_str());
        assert_eq!(f.crew_of(&s3).await, serde_json::Value::Null);

        // Dismissing a member takes it out; dismissing the last one removes
        // the crew and returns it for Undo.
        f.report(&s2, "completed").await;
        let (status, out) = f
            .ask(delete_with_auth(
                &format!("/v1/agent/sessions/{s2}"),
                &f.owner_bearer,
            ))
            .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(out, serde_json::json!({"removed_crew": null}));
        assert_eq!(f.crew_of(&s1).await["order"], 1, "orders are not compacted");
        f.report(&s1, "completed").await;
        let (_, out) = f
            .ask(delete_with_auth(
                &format!("/v1/agent/sessions/{s1}"),
                &f.owner_bearer,
            ))
            .await;
        assert_eq!(out["removed_crew"]["id"], relay.as_str());
        assert_eq!(out["removed_crew"]["name"], "Relay hardening");
        assert!(f.crew_ids().await.is_empty());

        // Deleting a crew leaves its sessions running, loose.
        let (_, made) = f
            .ask(post_json_with_auth(
                "/v1/agent/crews",
                serde_json::json!({"name": "Short", "session_ids": [s3]}),
                &f.owner_bearer,
            ))
            .await;
        let short = made["crew"]["id"].as_str().unwrap().to_string();
        let (status, _) = f
            .ask(delete_with_auth(
                &format!("/v1/agent/crews/{short}"),
                &f.owner_bearer,
            ))
            .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        assert_eq!(f.crew_of(&s3).await, serde_json::Value::Null);
        let (_, view) = f.get(&format!("/v1/agent/sessions/{s3}")).await;
        assert_eq!(view["status"], "running");
        let (status, _) = f
            .ask(delete_with_auth(
                &format!("/v1/agent/crews/{short}"),
                &f.owner_bearer,
            ))
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn crews_and_roles_are_the_signed_in_owners_never_a_keys() {
        let dir = tempdir::TempDir::new("storm-crews-key").unwrap();
        let f = gateway_fixture(dir.path()).await;
        let (_, key) = f
            .ask(post_json_with_auth(
                "/v1/keys",
                serde_json::json!({"name": "a key"}),
                &f.owner_bearer,
            ))
            .await;
        let key = format!("Bearer {}", key["secret"].as_str().unwrap());
        for req in [
            get_with_auth("/v1/agent/crews", &key),
            get_with_auth("/v1/agent/roles", &key),
            post_json_with_auth(
                "/v1/agent/crews",
                serde_json::json!({"name": "x", "session_ids": []}),
                &key,
            ),
            put_json_with_auth(
                "/v1/agent/sessions/ags_X/role",
                serde_json::json!({"vault_id": "v", "note_id": "n"}),
                &key,
            ),
        ] {
            let (status, body) = f.ask(req).await;
            assert!(
                status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN,
                "{status} {body}"
            );
        }
        // And the ops refuse any actor but a signed-in session.
        let agent = Actor::Agent {
            session_id: "ags_X".into(),
            host_id: f.host_id.clone(),
            user_id: f.owner.clone(),
            write_vault: None,
        };
        let refused = crate::ops::list_crews(&f.state, &agent).await.unwrap_err();
        assert_eq!(refused.0, StatusCode::FORBIDDEN);
        let refused = crate::ops::list_roles(&f.state, &agent).await.unwrap_err();
        assert_eq!(refused.0, StatusCode::FORBIDDEN);
    }

    // ---- the spawn switch (decision 134-2) ----

    const SPAWN_134: [&str; 9] = [
        "list_agents",
        "start_agent",
        "agent_status",
        "list_children",
        "wait_agent",
        "send_message",
        "end_agent",
        "report_result",
        "read_messages",
    ];

    impl GatewayFixture {
        async fn spawn(&self, session: &str, enabled: bool) -> (StatusCode, serde_json::Value) {
            self.ask(put_json_with_auth(
                &format!("/v1/agent/sessions/{session}/spawn"),
                serde_json::json!({"enabled": enabled}),
                &self.owner_bearer,
            ))
            .await
        }

        async fn spawn_events(&self) -> Vec<(String, serde_json::Value)> {
            self.state
                .auth_db
                .lock()
                .await
                .all_events()
                .unwrap()
                .into_iter()
                .filter(|(kind, _)| kind.starts_with("spawn_"))
                .map(|(kind, detail)| (kind, serde_json::from_str(&detail).unwrap()))
                .collect()
        }

        async fn storm_tools(&self, session: &str) -> Vec<String> {
            tools_of(
                self.mcp(
                    session,
                    "storm",
                    serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
                )
                .await,
            )
        }

        /// What went down the host's link since the last look.
        fn sent(&mut self) -> Vec<serde_json::Value> {
            std::iter::from_fn(|| self.link.try_recv().ok())
                .map(|e| serde_json::to_value(e).unwrap())
                .collect()
        }

        async fn verified(&self) {
            let mut caps = host_caps(true);
            caps["identity"] =
                serde_json::json!({"state": "verified", "mechanism": "so_peerpidfd"});
            let (code, _) = send(
                &self.app,
                post_json_with_auth("/v1/runtime/inventory", caps, &self.host_bearer),
            )
            .await;
            assert!(code.is_success());
        }
    }

    fn list_changed(sent: &[serde_json::Value]) -> usize {
        sent.iter()
            .filter(|m| {
                m["type"] == "mcp.message"
                    && m["connection"] == "storm"
                    && m["message"]["method"] == "notifications/tools/list_changed"
            })
            .count()
    }

    #[tokio::test]
    async fn the_spawn_switch_is_the_signed_in_owners_never_a_keys_or_an_agents() {
        let dir = tempdir::TempDir::new("storm-spawn-owner").unwrap();
        let mut f = gateway_fixture(dir.path()).await;
        let session = f.launch("claude-code", false).await["id"]
            .as_str()
            .unwrap()
            .to_string();
        let (_, key) = f
            .ask(post_json_with_auth(
                "/v1/keys",
                serde_json::json!({"name": "a key"}),
                &f.owner_bearer,
            ))
            .await;
        let key = format!("Bearer {}", key["secret"].as_str().unwrap());
        for enabled in [true, false] {
            let (status, body) = f
                .ask(put_json_with_auth(
                    &format!("/v1/agent/sessions/{session}/spawn"),
                    serde_json::json!({"enabled": enabled}),
                    &key,
                ))
                .await;
            assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
            // Nor the host's own credential.
            let (status, _) = f
                .ask(put_json_with_auth(
                    &format!("/v1/agent/sessions/{session}/spawn"),
                    serde_json::json!({"enabled": enabled}),
                    &f.host_bearer,
                ))
                .await;
            assert_eq!(status, StatusCode::UNAUTHORIZED);
        }
        // The op refuses any actor but a signed-in session, the session's
        // own agent included, before it looks at anything.
        f.verified().await;
        f.state.agent.verify_isolation_for_tests(&f.host_id);
        for actor in [
            Actor::Agent {
                session_id: session.clone(),
                host_id: f.host_id.clone(),
                user_id: f.owner.clone(),
                write_vault: None,
            },
            Actor::Key {
                key_id: "key_X".into(),
                user_id: f.owner.clone(),
            },
        ] {
            for enabled in [true, false] {
                let refused = crate::ops::set_session_spawn(
                    &f.state,
                    &actor,
                    None,
                    &session,
                    crate::ops::SetSpawn { enabled },
                )
                .await
                .unwrap_err();
                assert_eq!(
                    (refused.0, refused.1.as_str()),
                    (StatusCode::FORBIDDEN, "owner_only")
                );
            }
        }
        // Another account's session is not found.
        let other = Actor::Session {
            user_id: "usr_SOMEONE".into(),
        };
        let refused = crate::ops::set_session_spawn(
            &f.state,
            &other,
            None,
            &session,
            crate::ops::SetSpawn { enabled: true },
        )
        .await
        .unwrap_err();
        assert_eq!(refused.0, StatusCode::NOT_FOUND);
        assert!(f.spawn_events().await.is_empty(), "nothing was changed");
        assert_eq!(list_changed(&f.sent()), 0);
        let (status, body) = f.spawn(&session, true).await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }

    /// C0 on every host today: identity is the fixture's old host, then
    /// verified without isolation. Each refusal is audited and changes nothing.
    #[tokio::test]
    async fn the_spawn_switch_refuses_on_every_host_today() {
        let dir = tempdir::TempDir::new("storm-spawn-c0").unwrap();
        let mut f = gateway_fixture(dir.path()).await;
        let session = f.launch("claude-code", false).await["id"]
            .as_str()
            .unwrap()
            .to_string();
        f.initialize(&session, "storm").await;
        f.sent();
        let (status, body) = f.spawn(&session, true).await;
        assert_eq!(
            (status, body),
            (
                StatusCode::CONFLICT,
                serde_json::json!({"error": "identity_unverified"})
            )
        );
        f.verified().await;
        let (status, body) = f.spawn(&session, true).await;
        assert_eq!(
            (status, body),
            (
                StatusCode::CONFLICT,
                serde_json::json!({"error": "isolation_unverified"})
            )
        );
        let events = f.spawn_events().await;
        assert_eq!(
            events,
            ["identity_unverified", "isolation_unverified"]
                .map(|reason| (
                    "spawn_refused".to_string(),
                    serde_json::json!({"session_id": session, "host_id": f.host_id, "reason": reason})
                ))
                .to_vec()
        );
        assert_eq!(list_changed(&f.sent()), 0);
        let tools = f.storm_tools(&session).await;
        assert!(tools.contains(&"list_vaults".to_string()), "{tools:?}");
        for name in SPAWN_134 {
            assert!(!tools.contains(&name.to_string()), "{name} listed");
            assert_eq!(
                error_code(&f.call(&session, "storm", name).await).as_deref(),
                Some("tool_not_allowed"),
                "{name}"
            );
        }
        // Revoking is always allowed, and audited.
        let (status, body) = f.spawn(&session, false).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(
            body,
            serde_json::json!({"enabled": false, "refused_requests": 0})
        );
        assert_eq!(f.spawn_events().await.last().unwrap().0, "spawn_revoked");
    }

    /// The allowed path, through the test-only isolation hook no host can
    /// reach: a flip tells the agent its tools changed, every change is
    /// audited, and no 134 tool exists to list yet.
    #[tokio::test]
    async fn turning_spawn_on_and_off_tells_the_agent_and_is_audited() {
        let dir = tempdir::TempDir::new("storm-spawn-flip").unwrap();
        let mut f = gateway_fixture(dir.path()).await;
        f.verified().await;
        f.state.agent.verify_isolation_for_tests(&f.host_id);
        let session = f.launch("claude-code", false).await["id"]
            .as_str()
            .unwrap()
            .to_string();
        f.initialize(&session, "storm").await;
        let before = f.storm_tools(&session).await;
        f.sent();

        let (status, body) = f.spawn(&session, true).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(
            body,
            serde_json::json!({"enabled": true, "refused_requests": 0})
        );
        assert_eq!(list_changed(&f.sent()), 1);
        // The list gains exactly the built parent tools (134-5 adds start).
        for name in SPAWN_134 {
            assert!(!before.contains(&name.to_string()), "{name} listed");
        }
        let after = f.storm_tools(&session).await;
        assert_eq!(spawn_134(&after), PARENT_BUILT.to_vec());
        assert_eq!(after.len(), before.len() + PARENT_BUILT.len());

        // Asked again: audited, but nothing flipped, so no notification.
        let (status, _) = f.spawn(&session, true).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(list_changed(&f.sent()), 0);

        // WS-9 from the launch side.
        let (status, body) = f.launch_body(serde_json::json!({})).await;
        assert_eq!(
            (status, body),
            (
                StatusCode::CONFLICT,
                serde_json::json!({"error": "workspace_held"})
            )
        );

        let (status, body) = f.spawn(&session, false).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["enabled"], false);
        assert_eq!(list_changed(&f.sent()), 1);
        assert_eq!(f.storm_tools(&session).await, before);
        let detail = |was: bool| {
            serde_json::json!({"session_id": session, "host_id": f.host_id,
                "was": was, "refused_requests": 0})
        };
        assert_eq!(
            f.spawn_events().await,
            vec![
                ("spawn_enabled".to_string(), detail(false)),
                ("spawn_enabled".to_string(), detail(true)),
                ("spawn_revoked".to_string(), detail(true)),
            ]
        );

        // Another session now shares the workspace, so spawn is held.
        let (status, body) = f.launch_body(serde_json::json!({})).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let (status, body) = f.spawn(&session, true).await;
        assert_eq!(
            (status, body),
            (
                StatusCode::CONFLICT,
                serde_json::json!({"error": "workspace_held"})
            )
        );
        assert_eq!(list_changed(&f.sent()), 0);
    }

    // ---- the approval engine (decision 134-3) ----

    impl GatewayFixture {
        /// A running session with spawn on, on a host whose isolation is
        /// verified by the test-only hook, and a start request from it.
        async fn spawn_request(&mut self) -> (String, String) {
            self.verified().await;
            self.state.agent.verify_isolation_for_tests(&self.host_id);
            let session = self.launch("claude-code", false).await["id"]
                .as_str()
                .unwrap()
                .to_string();
            let (status, body) = self.spawn(&session, true).await;
            assert_eq!(status, StatusCode::OK, "{body}");
            let id = self.request_from(&session).await;
            (session, id)
        }

        async fn request_from(&self, session: &str) -> String {
            crate::ops::create_spawn_request(
                &self.state,
                session,
                crate::agent::store::spawn::NewRequest::Start {
                    task: "write the tests".into(),
                    provider: None,
                },
            )
            .await
            .ok()
            .expect("created")
            .unwrap()
            .id
        }

        async fn decide(
            &self,
            id: &str,
            verb: &str,
            bearer: &str,
        ) -> (StatusCode, serde_json::Value) {
            self.ask(post_json_with_auth(
                &format!("/v1/agent/requests/{id}/{verb}"),
                serde_json::json!({}),
                bearer,
            ))
            .await
        }

        async fn request_events(&self) -> Vec<(String, serde_json::Value)> {
            self.spawn_events()
                .await
                .into_iter()
                .filter(|(kind, _)| kind.starts_with("spawn_request"))
                .collect()
        }
    }

    /// AP-6 and A5: only a signed-in person's device decides. A key, the
    /// host's credential and an agent are refused, and no MCP tool decides.
    #[tokio::test]
    async fn only_a_signed_in_owners_device_approves_or_denies() {
        let dir = tempdir::TempDir::new("storm-approve-owner").unwrap();
        let mut f = gateway_fixture(dir.path()).await;
        let (session, id) = f.spawn_request().await;
        let (_, key) = f
            .ask(post_json_with_auth(
                "/v1/keys",
                serde_json::json!({"name": "a key"}),
                &f.owner_bearer,
            ))
            .await;
        let key = format!("Bearer {}", key["secret"].as_str().unwrap());
        for verb in ["approve", "deny"] {
            for bearer in [&key, &f.host_bearer] {
                let (status, body) = f.decide(&id, verb, bearer).await;
                assert_eq!(status, StatusCode::UNAUTHORIZED, "{verb}: {body}");
            }
        }
        let agent = Actor::Agent {
            session_id: session.clone(),
            host_id: f.host_id.clone(),
            user_id: f.owner.clone(),
            write_vault: None,
        };
        let key_actor = Actor::Key {
            key_id: "key_X".into(),
            user_id: f.owner.clone(),
        };
        for actor in [&agent, &key_actor] {
            let approve = crate::ops::approve_spawn_request(
                &f.state,
                actor,
                "dev_X",
                &id,
                crate::ops::ApproveRequest::default(),
            )
            .await
            .unwrap_err();
            let deny = crate::ops::deny_spawn_request(&f.state, actor, "dev_X", &id)
                .await
                .unwrap_err();
            for refused in [approve, deny] {
                assert_eq!(
                    (refused.0, refused.1.as_str()),
                    (StatusCode::FORBIDDEN, "owner_only")
                );
            }
        }
        let other = Actor::Session {
            user_id: "usr_SOMEONE".into(),
        };
        let refused = crate::ops::deny_spawn_request(&f.state, &other, "dev_X", &id)
            .await
            .unwrap_err();
        assert_eq!(refused.0, StatusCode::NOT_FOUND);
        let (status, _) = f.decide("agr_NOPE", "approve", &f.owner_bearer).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        // No tool an agent or a key reaches decides anything.
        f.initialize(&session, "storm").await;
        let tools = f.storm_tools(&session).await;
        assert!(tools.contains(&"list_vaults".to_string()), "{tools:?}");
        for name in &tools {
            assert!(
                !["approv", "deny", "request"]
                    .iter()
                    .any(|w| name.contains(w)),
                "{name}"
            );
        }
        assert_eq!(f.state.agent.request(&id).unwrap().state, "pending");
        assert!(
            f.request_events()
                .await
                .iter()
                .all(|(kind, _)| kind == "spawn_request"),
            "no refusal of an actor is a decision"
        );
    }

    /// Approving goes through the route, once; a denial and an expiry are
    /// recorded; each decision is audited with what it decided.
    #[tokio::test]
    async fn a_device_decides_a_request_once_through_the_route() {
        let dir = tempdir::TempDir::new("storm-approve-route").unwrap();
        let mut f = gateway_fixture(dir.path()).await;
        let (session, id) = f.spawn_request().await;
        let (status, body) = f.decide(&id, "approve", &f.owner_bearer).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(
            (
                &body["id"],
                &body["state"],
                &body["outcome"],
                &body["requester"]
            ),
            (
                &serde_json::json!(id),
                &serde_json::json!("decided"),
                &serde_json::json!("started"),
                &serde_json::json!(session)
            )
        );
        let child = body["session"].as_str().unwrap().to_string();
        assert!(child.starts_with("ags_"));
        let stored = f.state.agent.request(&id).unwrap();
        assert!(stored.decided_by_device.is_some_and(|d| !d.is_empty()));
        // AP-4 through the route: the second device finds it decided.
        for verb in ["approve", "deny"] {
            let (status, body) = f.decide(&id, verb, &f.owner_bearer).await;
            assert_eq!(
                (status, body),
                (
                    StatusCode::CONFLICT,
                    serde_json::json!({"error": "already_decided"})
                )
            );
        }

        let denied = f.request_from(&session).await;
        let (status, body) = f.decide(&denied, "deny", &f.owner_bearer).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["outcome"], "denied");
        assert!(body.get("session").is_none());

        // AP-5 through the route, and the sweeper's record.
        let expired = |id: &str| crate::agent::store::spawn::Request {
            id: id.into(),
            kind: "start".into(),
            requester: session.clone(),
            target: None,
            payload: r#"{"task":"late"}"#.into(),
            created_at: "2020-01-01T09:00:00Z".into(),
            expires_at: "2020-01-01T09:10:00Z".into(),
            state: "pending".into(),
            decided_by_device: None,
            decided_at: None,
            outcome: None,
            reason: None,
            session: None,
        };
        f.state.agent.insert_request_for_tests(&expired("agr_LATE"));
        let (status, body) = f.decide("agr_LATE", "approve", &f.owner_bearer).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["outcome"], "expired");
        f.state
            .agent
            .insert_request_for_tests(&expired("agr_SWEPT"));
        crate::ops::sweep_spawn_requests(&f.state).await;
        assert_eq!(
            f.state
                .agent
                .request("agr_SWEPT")
                .unwrap()
                .outcome
                .as_deref(),
            Some("expired")
        );

        let events: Vec<(String, Option<String>, Option<String>)> = f
            .request_events()
            .await
            .into_iter()
            .map(|(kind, d)| {
                (
                    kind,
                    d["request_id"].as_str().map(Into::into),
                    d["outcome"].as_str().map(Into::into),
                )
            })
            .collect();
        let row = |kind: &str, id: &str, outcome: Option<&str>| {
            (
                kind.to_string(),
                Some(id.to_string()),
                outcome.map(Into::into),
            )
        };
        assert_eq!(
            events,
            vec![
                row("spawn_request", &id, None),
                row("spawn_request_decided", &id, Some("started")),
                row("spawn_request", &denied, None),
                row("spawn_request_decided", &denied, Some("denied")),
                row("spawn_request_decided", "agr_LATE", Some("expired")),
                row("spawn_request_expired", "agr_SWEPT", Some("expired")),
            ]
        );
    }

    #[tokio::test]
    async fn a_role_label_is_a_kit_agents_note_named_live_and_goes_with_a_dismiss() {
        let dir = tempdir::TempDir::new("storm-crew-roles").unwrap();
        let mut f = gateway_fixture(dir.path()).await;
        let session = f.launch("claude-code", false).await["id"]
            .as_str()
            .unwrap()
            .to_string();

        // No Kit vault: no roles, not an error.
        let (status, roles) = f.get("/v1/agent/roles").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(roles, serde_json::json!([]));

        let kit = f.vault("kit").await;
        let work = f.vault("Work").await;
        let reviewer = f
            .note(
                &kit,
                "agents/Storm Reviewer.md",
                "---\nkind: agent\nsummary: Checks completed work against the task.\n---\n# Storm Reviewer\n",
            )
            .await;
        let release = f
            .note(
                &kit,
                "projects/storm/agents/Release Notes.md",
                "# Release Notes\n",
            )
            .await;
        let other = f.note(&kit, "Notes/Other.md", "# Other\n").await;
        let elsewhere = f.note(&work, "agents/Fake.md", "# Fake\n").await;

        let (status, roles) = f.get("/v1/agent/roles").await;
        assert_eq!(status, StatusCode::OK, "{roles}");
        let listed: Vec<(&str, &str, Option<&str>)> = roles
            .as_array()
            .unwrap()
            .iter()
            .map(|r| {
                (
                    r["title"].as_str().unwrap(),
                    r["folder"].as_str().unwrap(),
                    r["summary"].as_str(),
                )
            })
            .collect();
        assert_eq!(
            listed,
            [
                (
                    "Storm Reviewer",
                    "agents",
                    Some("Checks completed work against the task.")
                ),
                ("Release Notes", "projects/storm/agents", None),
            ]
        );
        assert_eq!(roles[0]["vault_id"], kit.as_str());
        assert_eq!(roles[0]["note_id"], reviewer["id"]);

        // Only a role note can be a label.
        let set = |vault: &str, note: &serde_json::Value| {
            put_json_with_auth(
                &format!("/v1/agent/sessions/{session}/role"),
                serde_json::json!({"vault_id": vault, "note_id": note["id"]}),
                &f.owner_bearer,
            )
        };
        for (vault, note) in [(&kit, &other), (&work, &elsewhere), (&work, &reviewer)] {
            let (status, body) = f.ask(set(vault, note)).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        }
        let (status, _) = f.ask(set(&kit, &release)).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (status, _) = f.ask(set(&kit, &reviewer)).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (_, view) = f.get(&format!("/v1/agent/sessions/{session}")).await;
        assert_eq!(
            view["role"],
            serde_json::json!({"vault_id": kit, "note_id": reviewer["id"], "title": "Storm Reviewer"})
        );
        assert_eq!(
            view["crew"],
            serde_json::Value::Null,
            "a loose session has a role"
        );

        // The label is named live: a renamed note renames it, a deleted one
        // leaves it with no title.
        let reviewer_id = reviewer["id"].as_str().unwrap();
        let (status, body) = f
            .ask(put_json_with_auth(
                &format!("/v1/vaults/{kit}/notes/{reviewer_id}"),
                serde_json::json!({"base_version": reviewer["version"],
                    "content": "---\nkind: agent\n---\n# Code Reviewer\n"}),
                &f.owner_bearer,
            ))
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let (_, list) = f.get("/v1/agent/sessions").await;
        assert_eq!(list[0]["role"]["title"], "Code Reviewer", "{list}");
        let (status, _) = f
            .ask(delete_with_auth(
                &format!("/v1/vaults/{kit}/notes/{reviewer_id}"),
                &f.owner_bearer,
            ))
            .await;
        assert!(status.is_success(), "{status}");
        let (_, view) = f.get(&format!("/v1/agent/sessions/{session}")).await;
        assert_eq!(view["role"]["title"], serde_json::Value::Null, "{view}");
        assert_eq!(view["role"]["note_id"], reviewer_id);

        // Cleared, set again, then gone with the session.
        let (status, _) = f
            .ask(delete_with_auth(
                &format!("/v1/agent/sessions/{session}/role"),
                &f.owner_bearer,
            ))
            .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (_, view) = f.get(&format!("/v1/agent/sessions/{session}")).await;
        assert_eq!(view["role"], serde_json::Value::Null);
        f.ask(set(&kit, &release)).await;
        f.report(&session, "completed").await;
        let (status, _) = f
            .ask(delete_with_auth(
                &format!("/v1/agent/sessions/{session}"),
                &f.owner_bearer,
            ))
            .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(f.state.agent.role_of(&session).unwrap(), None);
        let (status, _) = f.ask(set(&kit, &release)).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn session_context_reads_the_callers_own_note_and_the_launch_validates_it() {
        let dir = tempdir::TempDir::new("storm-gateway-context").unwrap();
        let mut f = gateway_fixture(dir.path()).await;
        let personal = f.vault("Personal").await;
        let a = f
            .note(
                &personal,
                "specs/Gateway spec.md",
                "# Gateway spec\n\nalpha body\n",
            )
            .await;
        let b = f
            .note(&personal, "Other.md", "# Other\n\nbravo body\n")
            .await;
        let context = |n: &serde_json::Value| serde_json::json!({"context": {"vault_id": personal, "note_id": n["id"]}});

        let (status, la) = f.launch_body(context(&a)).await;
        assert_eq!(status, StatusCode::OK, "{la}");
        assert_eq!(la["name"], "gateway-spec");
        assert_eq!(la["context"]["title"], "Gateway spec");
        assert_eq!(la["context"]["note_id"], a["id"]);
        assert_eq!(la["context"]["vault_id"], personal.as_str());
        let start = serde_json::to_value(f.link.try_recv().unwrap()).unwrap();
        assert_eq!(start["context"], true);
        let (_, lb) = f.launch_body(context(&b)).await;
        let _ = f.link.try_recv();
        let (_, dup) = f.launch_body(context(&a)).await;
        assert_eq!(dup["name"], "gateway-spec-2");
        let (_, none) = f.launch_body(serde_json::json!({})).await;
        assert_eq!(none["context"], serde_json::Value::Null);

        let (sa, sb, sn) = (
            la["id"].as_str().unwrap().to_string(),
            lb["id"].as_str().unwrap().to_string(),
            none["id"].as_str().unwrap().to_string(),
        );
        let init = f.initialize(&sa, "storm").await;
        assert!(
            init["instructions"]
                .as_str()
                .unwrap()
                .contains("Call session_context"),
            "{init}"
        );
        let init = f.initialize(&sn, "storm").await;
        assert!(
            !init["instructions"]
                .as_str()
                .unwrap()
                .contains("session_context"),
            "{init}"
        );
        f.initialize(&sb, "storm").await;

        let got = structured(&f.call(&sa, "storm", "session_context").await);
        assert_eq!(got["note_id"], a["id"]);
        assert_eq!(got["title"], "Gateway spec");
        assert_eq!(got["path"], "specs/Gateway spec.md");
        assert!(got["content"].as_str().unwrap().contains("alpha body"));
        let got = structured(&f.call(&sb, "storm", "session_context").await);
        assert!(
            got["content"].as_str().unwrap().contains("bravo body"),
            "{got}"
        );
        assert!(
            tool_error(&f.call(&sn, "storm", "session_context").await)
                .contains("started without a note")
        );

        let a_id = a["id"].as_str().unwrap();
        send(
            &f.app,
            delete_with_auth(
                &format!("/v1/vaults/{personal}/notes/{a_id}"),
                &f.owner_bearer,
            ),
        )
        .await;
        assert!(
            tool_error(&f.call(&sa, "storm", "session_context").await).contains("no longer exists")
        );

        // The launch refuses a context or a write vault that does not resolve.
        let (status, _) = f
            .launch_body(serde_json::json!({"context": {"vault_id": personal, "note_id": a_id}}))
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (status, _) = f
            .launch_body(serde_json::json!({"context": {"vault_id": "nope", "note_id": a_id}}))
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (status, _) = f
            .launch_body(serde_json::json!({"write_vault_id": "nope"}))
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn the_mcp_switches_and_agent_writes_are_set_independently_and_persist() {
        let dir = tempdir::TempDir::new("storm-ai-access").unwrap();
        let f = gateway_fixture(dir.path()).await;
        let put = |body: serde_json::Value| {
            send(
                &f.app,
                put_json_with_auth("/v1/config/mcp", body, &f.owner_bearer),
            )
        };
        let (_, body) = put(serde_json::json!({"agent_writes": true})).await;
        assert_eq!(
            body,
            serde_json::json!({"mcp_enabled": false, "mcp_writable": false, "agent_writes": true})
        );
        // An older client's body leaves agents alone.
        let (_, body) = put(serde_json::json!({"enabled": true, "writable": true})).await;
        assert_eq!(body["agent_writes"], true);
        assert_eq!(body["mcp_writable"], true);
        let (_, body) = put(serde_json::json!({"enabled": false})).await;
        assert_eq!(
            body,
            serde_json::json!({"mcp_enabled": false, "mcp_writable": false, "agent_writes": true})
        );
        let (_, body) = put(serde_json::json!({"agent_writes": false})).await;
        assert_eq!(body["mcp_enabled"], false);
        assert!(
            !f.state
                .agent_writes
                .load(std::sync::atomic::Ordering::Relaxed)
        );
        put(serde_json::json!({"agent_writes": true})).await;
        let reloaded =
            Registry::load(&f.state.state_dir, std::path::Path::new("/nowhere")).unwrap();
        assert!(reloaded.agent_writes() && !reloaded.mcp_enabled);
        let (_, integration) = send(
            &f.app,
            get_with_auth(&format!("{CONNECTIONS}/storm"), &f.owner_bearer),
        )
        .await;
        assert_eq!(integration["vault_writes_available"], true, "{integration}");
    }

    #[tokio::test]
    async fn a_kit_script_is_written_only_by_a_session_whose_write_vault_is_kit_and_is_in_its_wrote_list()
     {
        let dir = tempdir::TempDir::new("storm-gateway-kit").unwrap();
        let mut f = gateway_fixture(dir.path()).await;
        let work = f.vault("Work").await;
        let kit = f.vault("kit").await;
        f.agent_writes(true).await;
        let script = serde_json::json!({"name": "tool.sh", "content": "echo hi\n"});
        for (vault, expect_ok) in [(&work, false), (&kit, true)] {
            let (_, launched) = f
                .launch_body(serde_json::json!({"write_vault_id": vault}))
                .await;
            let session = launched["id"].as_str().unwrap().to_string();
            f.initialize(&session, "storm").await;
            let lines = f.tool(&session, "create_script", script.clone()).await;
            if expect_ok {
                assert_eq!(structured(&lines)["path"], "scripts/tool.sh", "{lines:?}");
                let lines = f
                    .tool(
                        &session,
                        "update_script",
                        serde_json::json!({"name": "tool.sh", "content": "echo bye\n"}),
                    )
                    .await;
                assert_eq!(structured(&lines)["size"], 9, "{lines:?}");
            } else {
                assert_eq!(
                    error_code(&lines).as_deref(),
                    Some(crate::auth::authz::AGENT_WRITE_REFUSED),
                    "{lines:?}"
                );
            }
            let (_, writes) = f.get(&format!("/v1/agent/sessions/{session}/writes")).await;
            let (_, view) = f.get(&format!("/v1/agent/sessions/{session}")).await;
            if expect_ok {
                assert_eq!(
                    writes,
                    serde_json::json!([{
                        "vault_id": kit, "note_id": null, "title": "tool.sh",
                        "path": "scripts/tool.sh", "kind": "script_created", "version": null,
                        "at": writes[0]["at"],
                    }]),
                    "one row per script; created stays created"
                );
                assert_eq!(view["wrote_count"], 1);
                let (_, map) = f.get(&format!("/v1/vaults/{kit}/agent-writes")).await;
                assert_eq!(map, serde_json::json!({}), "a script has no note id");
            } else {
                assert_eq!(
                    writes,
                    serde_json::json!([]),
                    "a refused write is not recorded"
                );
                assert_eq!(view["wrote_count"], 0);
            }
            f.report(&session, "completed").await;
        }
    }

    #[tokio::test]
    async fn the_launch_context_is_a_snapshot_and_session_context_follows_the_note_by_id() {
        let dir = tempdir::TempDir::new("storm-gateway-context-rename").unwrap();
        let mut f = gateway_fixture(dir.path()).await;
        let personal = f.vault("Personal").await;
        let note = f
            .note(
                &personal,
                "specs/Gateway spec.md",
                "# Gateway spec\n\nbody\n",
            )
            .await;
        let note_id = note["id"].as_str().unwrap().to_string();
        let (_, launched) = f
            .launch_body(serde_json::json!({"context": {"vault_id": personal, "note_id": note_id}}))
            .await;
        let session = launched["id"].as_str().unwrap().to_string();
        let launch_context = launched["context"].clone();
        assert_eq!(launch_context["title"], "Gateway spec");

        // Moved and retitled after the launch.
        let (status, moved) = send(
            &f.app,
            post_json_with_auth(
                &format!("/v1/vaults/{personal}/notes/{note_id}/move"),
                serde_json::json!({"new_path": "archive/Renamed plan.md"}),
                &f.owner_bearer,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{moved}");
        let (status, _) = send(
            &f.app,
            put_json_with_auth(
                &format!("/v1/vaults/{personal}/notes/{note_id}"),
                serde_json::json!({"base_version": moved["note"]["version"],
                    "content": "# Renamed plan\n\nbody\n"}),
                &f.owner_bearer,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let (_, view) = f.get(&format!("/v1/agent/sessions/{session}")).await;
        assert_eq!(
            view["context"], launch_context,
            "the launch's context is history"
        );
        assert_eq!(view["name"], "gateway-spec");
        f.initialize(&session, "storm").await;
        let got = structured(&f.call(&session, "storm", "session_context").await);
        assert_eq!(got["note_id"], note_id.as_str());
        assert_eq!(got["path"], "archive/Renamed plan.md");
        assert_eq!(got["title"], "Renamed plan");
    }

    #[tokio::test]
    async fn request_scoped_messages_ride_their_calls_stream() {
        // Progress with the agent's own token, a form elicitation answered by
        // the agent, and a URL elicitation that never reaches it (G-D23).
        let dir = tempdir::TempDir::new("storm-gateway-stream").unwrap();
        let mut f = gateway_fixture(dir.path()).await;
        let session = f.launch("claude-code", false).await["id"]
            .as_str()
            .unwrap()
            .to_string();
        f.initialize(&session, &f.connection).await;

        let lines = f
            .mcp(&session, &f.connection, serde_json::json!({"jsonrpc": "2.0", "id": 9, "method": "tools/call",
                "params": {"name": "slow", "arguments": {}, "_meta": {"progressToken": "agent-tok"}}}))
            .await;
        let progress: Vec<&serde_json::Value> = lines
            .iter()
            .filter(|l| {
                l.pointer("/message/method") == Some(&serde_json::json!("notifications/progress"))
            })
            .collect();
        assert_eq!(progress.len(), 2, "{lines:?}");
        assert!(
            progress
                .iter()
                .all(|p| p.pointer("/message/params/progressToken")
                    == Some(&serde_json::json!("agent-tok")))
        );
        assert_eq!(tool_text(&lines), "slow done");

        // A form elicitation: read the stream until it arrives, answer it on
        // a second POST, then read the final result.
        use futures_util::StreamExt;
        let response = f
            .app
            .clone()
            .oneshot(gateway_request(&f.host_bearer, &session, &f.connection, &serde_json::json!(
                {"jsonrpc": "2.0", "id": 10, "method": "tools/call", "params": {"name": "ask", "arguments": {}}})))
            .await
            .unwrap();
        let mut body = response.into_body().into_data_stream();
        let mut buffer = String::new();
        let elicitation = loop {
            let chunk = body.next().await.unwrap().unwrap();
            buffer.push_str(std::str::from_utf8(&chunk).unwrap());
            if let Some(line) = buffer.lines().next() {
                break serde_json::from_str::<serde_json::Value>(line).unwrap();
            }
        };
        assert_eq!(elicitation["message"]["method"], "elicitation/create");
        let elicit_id = elicitation["message"]["id"].as_str().unwrap().to_string();
        assert!(elicit_id.starts_with("storm-elicit-"));
        let answered = f
            .mcp(
                &session,
                &f.connection,
                serde_json::json!({"jsonrpc": "2.0", "id": elicit_id,
                "result": {"action": "accept", "content": {"answer": "rust"}}}),
            )
            .await;
        assert!(answered.is_empty());
        let mut rest = String::new();
        while let Some(chunk) = body.next().await {
            rest.push_str(std::str::from_utf8(&chunk.unwrap()).unwrap());
        }
        let last: serde_json::Value = serde_json::from_str(rest.lines().last().unwrap()).unwrap();
        let text = last
            .pointer("/message/result/content/0/text")
            .unwrap()
            .as_str()
            .unwrap();
        assert!(text.contains("accept") && text.contains("rust"), "{text}");
        // A late second answer to the same elicitation is dropped.
        assert!(
            f.mcp(
                &session,
                &f.connection,
                serde_json::json!({"jsonrpc": "2.0", "id": elicit_id,
            "result": {"action": "accept"}})
            )
            .await
            .is_empty()
        );

        // URL mode: declined by the gateway, never on the agent's stream.
        let lines = f.call(&session, &f.connection, "ask_url").await;
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(tool_text(&lines).contains("decline"), "{lines:?}");
        let calls = f
            .state
            .gateway
            .store
            .lock()
            .unwrap()
            .recent_calls(50)
            .unwrap();
        assert!(
            calls
                .iter()
                .any(|c| c.error_code.as_deref() == Some("url_elicitation_declined"))
        );
    }

    #[tokio::test]
    async fn an_in_flight_call_fails_once_when_its_upstream_session_is_lost_and_is_not_retried() {
        // R7 at the gateway: a call whose upstream session goes away (as on a
        // server restart) ends with one error, upstream executed it once, and
        // the next request is `session_unknown` — never a silent resend.
        let dir = tempdir::TempDir::new("storm-gateway-inflight").unwrap();
        let mut f = gateway_fixture(dir.path()).await;
        let session = f.launch("claude-code", false).await["id"]
            .as_str()
            .unwrap()
            .to_string();
        f.initialize(&session, &f.connection).await;
        let app = f.app.clone();
        let bearer = f.host_bearer.clone();
        let (s, c) = (session.clone(), f.connection.clone());
        let call = tokio::spawn(async move {
            mcp_as(
                &app,
                &bearer,
                &s,
                &c,
                serde_json::json!({"jsonrpc": "2.0", "id": 11, "method": "tools/call",
                "params": {"name": "hang", "arguments": {}}}),
            )
            .await
        });
        for _ in 0..100 {
            if f.upstream_calls() == 1 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert_eq!(f.upstream_calls(), 1);
        f.state.gateway.sessions.close_session(&session);
        let lines = tokio::time::timeout(std::time::Duration::from_secs(10), call)
            .await
            .unwrap()
            .unwrap();
        assert!(error_code(&lines).is_some(), "{lines:?}");
        assert_eq!(f.upstream_calls(), 1, "the in-flight call was re-sent");
        assert_eq!(
            f.call(&session, &f.connection, "echo").await,
            vec![serde_json::json!({"storm_error": "session_unknown"})]
        );
        assert_eq!(f.upstream_calls(), 1);
    }

    // ---- Token sync: the owner's routes (decision 124n) ---------------------

    const CLAUDE_SUB: &str = "/v1/provider-credentials/claude-subscription";
    const CLAUDE_SUB_HOSTS: &str = "/v1/provider-credentials/claude-subscription/hosts";
    const SUB_CANARY: &str = "sk-ant-oat01-CANARY_124n-route-0123456789abcdef";

    fn put_raw_with_auth(
        path: &str,
        body: &str,
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

    async fn sync_token(app: &Router, bearer: &str, token: &str) -> serde_json::Value {
        let (status, body) = send(
            app,
            put_json_with_auth(CLAUDE_SUB, serde_json::json!({"token": token}), bearer),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body
    }

    async fn security_events(state: &Shared) -> Vec<(String, String)> {
        state.auth_db.lock().await.all_events().unwrap()
    }

    #[tokio::test]
    async fn the_owner_syncs_scopes_replaces_and_removes_the_token_and_never_reads_it_back() {
        let dir = tempdir::TempDir::new("storm-pc-flow").unwrap();
        let (app, _, state) = test_router_with_state(dir.path());
        let (_, owner) = owner_bearer(&state).await;
        let (host, _) = enroll_a_host(&app, &owner).await;
        let mut seen = Vec::new();

        let (status, absent) = send(&app, get_with_auth(CLAUDE_SUB, &owner)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            absent,
            serde_json::json!({"kind": "claude_subscription_token", "present": false, "hosts": []})
        );

        // Pasted from a terminal: bracketed-paste markers and a newline.
        let pasted = format!("\u{1b}[200~{SUB_CANARY}\r\n\u{1b}[201~");
        let first = sync_token(&app, &owner, &pasted).await;
        assert_eq!(first["present"], true);
        assert_eq!(first["status"], "active");
        assert_eq!(first["days_left"], 365);
        assert_eq!(first["warn"], serde_json::Value::Null);
        assert_eq!(first["fingerprint"].as_str().unwrap().len(), 8);
        // Default none (AC-TS3): nothing is scoped by syncing.
        assert_eq!(first["hosts"], serde_json::json!([]));
        seen.push(first.clone());

        let (status, scoped) = send(
            &app,
            patch_json_with_auth(
                CLAUDE_SUB_HOSTS,
                serde_json::json!({"hosts": [host, host]}),
                &owner,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{scoped}");
        assert_eq!(scoped["hosts"], serde_json::json!([host]));
        seen.push(scoped);

        let (status, replaced) = send(
            &app,
            put_json_with_auth(
                CLAUDE_SUB,
                serde_json::json!({"token": format!("{SUB_CANARY}-two"), "minted_at": "2026-01-01"}),
                &owner,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{replaced}");
        assert_ne!(replaced["fingerprint"], first["fingerprint"]);
        assert_eq!(replaced["minted_at"], "2026-01-01T00:00:00Z");
        assert_eq!(replaced["expires_at"], "2027-01-01T00:00:00Z");
        assert_eq!(
            replaced["hosts"],
            serde_json::json!([host]),
            "a replace keeps the scope"
        );
        seen.push(replaced);
        seen.push(send(&app, get_with_auth(CLAUDE_SUB, &owner)).await.1);

        let (status, _) = send(&app, delete_with_auth(CLAUDE_SUB, &owner)).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (_, gone) = send(&app, get_with_auth(CLAUDE_SUB, &owner)).await;
        assert_eq!(gone["present"], false);
        assert_eq!(
            gone["hosts"],
            serde_json::json!([]),
            "a delete takes the scope too"
        );
        seen.push(gone);

        for body in &seen {
            assert!(!body.to_string().contains("CANARY"), "{body}");
        }
        let events = security_events(&state).await;
        for kind in [
            "provider_credential_set",
            "provider_credential_hosts_changed",
            "provider_credential_replaced",
            "provider_credential_removed",
        ] {
            let detail = &events
                .iter()
                .find(|(k, _)| k == kind)
                .unwrap_or_else(|| panic!("{kind} missing"))
                .1;
            let detail: serde_json::Value = serde_json::from_str(detail).unwrap();
            assert_eq!(detail["kind"], "claude_subscription_token", "{kind}");
        }
        for (kind, detail) in &events {
            assert!(!detail.contains("CANARY"), "{kind}: {detail}");
        }
        // The stored row is sealed: no file under state holds the canary.
        for entry in walk(&state.state_dir) {
            let bytes = std::fs::read(&entry).unwrap_or_default();
            assert!(
                !bytes
                    .windows(SUB_CANARY.len())
                    .any(|w| w == SUB_CANARY.as_bytes()),
                "{} holds the token in the clear",
                entry.display()
            );
        }
    }

    fn walk(dir: &FsPath) -> Vec<std::path::PathBuf> {
        let mut out = Vec::new();
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                out.extend(walk(&path));
            } else {
                out.push(path);
            }
        }
        out
    }

    #[tokio::test]
    async fn a_badly_shaped_token_or_body_is_refused_without_echoing_it() {
        let dir = tempdir::TempDir::new("storm-pc-shape").unwrap();
        let (app, _, state) = test_router_with_state(dir.path());
        let (_, owner) = owner_bearer(&state).await;
        for token in [
            "sk-ant-api03-CANARY_not_an_oauth_token_0123456789".to_string(),
            "sk-ant-oat01-CANARY short".to_string(),
            "sk-ant-oat01-CANARY".to_string(),
            format!("sk-ant-oat01-CANARY{}", "a".repeat(400)),
            format!("Bearer {SUB_CANARY}"),
        ] {
            let (status, body) = send(
                &app,
                put_json_with_auth(CLAUDE_SUB, serde_json::json!({"token": token}), &owner),
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{token}");
            assert_eq!(body["error"], "bad_token_shape");
            assert!(!body.to_string().contains("CANARY"), "{body}");
        }
        for raw in [
            format!("{{\"token\": \"{SUB_CANARY}\",}}"),
            format!("{{\"token\": [\"{SUB_CANARY}\"]}}"),
            format!("{{\"token\": \"{SUB_CANARY}\", \"minted_at\": 5}}"),
        ] {
            let (status, body) = send(&app, put_raw_with_auth(CLAUDE_SUB, &raw, &owner)).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{raw}");
            assert_eq!(body["error"], "bad_request_body");
            assert!(!body.to_string().contains("CANARY"), "{body}");
        }
        let (status, body) = send(
            &app,
            put_json_with_auth(
                CLAUDE_SUB,
                serde_json::json!({"token": SUB_CANARY, "minted_at": "2999-01-01"}),
                &owner,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"], "bad_minted_at");
        let huge = format!(
            "{{\"token\": \"{SUB_CANARY}\", \"pad\": \"{}\"}}",
            "x".repeat(5000)
        );
        let (status, body) = send(&app, put_raw_with_auth(CLAUDE_SUB, &huge, &owner)).await;
        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
        assert!(!body.to_string().contains("CANARY"));
        let (_, still) = send(&app, get_with_auth(CLAUDE_SUB, &owner)).await;
        assert_eq!(still["present"], false, "a refused body stored something");
        for (kind, detail) in &security_events(&state).await {
            assert!(!detail.contains("CANARY"), "{kind}: {detail}");
        }
    }

    #[tokio::test]
    async fn the_scope_takes_only_live_hosts_and_a_revoked_host_leaves_it() {
        let dir = tempdir::TempDir::new("storm-pc-scope").unwrap();
        let (app, _, state) = test_router_with_state(dir.path());
        let (_, owner) = owner_bearer(&state).await;
        let (a, _) = enroll_a_host(&app, &owner).await;
        let (b, _) = enroll_a_host(&app, &owner).await;

        let (status, body) = send(
            &app,
            patch_json_with_auth(
                CLAUDE_SUB_HOSTS,
                serde_json::json!({"hosts": ["hst_nope"]}),
                &owner,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"], "unknown_host");

        // Chosen before a token exists, then the token arrives.
        let (status, body) = send(
            &app,
            patch_json_with_auth(
                CLAUDE_SUB_HOSTS,
                serde_json::json!({"hosts": [b, a]}),
                &owner,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        sync_token(&app, &owner, SUB_CANARY).await;
        assert_eq!(
            crate::ops::deliverable_credential(&state, &a),
            Some("claude_subscription_token")
        );
        assert_eq!(
            crate::ops::deliverable_credential(&state, "hst_other"),
            None
        );

        let (status, _) = send(
            &app,
            delete_with_auth(&format!("/v1/agent/hosts/{a}"), &owner),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (_, view) = send(&app, get_with_auth(CLAUDE_SUB, &owner)).await;
        assert_eq!(view["hosts"], serde_json::json!([b]));
        assert_eq!(crate::ops::deliverable_credential(&state, &a), None);
        let (status, body) = send(
            &app,
            patch_json_with_auth(CLAUDE_SUB_HOSTS, serde_json::json!({"hosts": [a]}), &owner),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "a revoked host is not a target"
        );
        assert_eq!(body["error"], "unknown_host");

        // Narrowing to none.
        let (status, view) = send(
            &app,
            patch_json_with_auth(CLAUDE_SUB_HOSTS, serde_json::json!({"hosts": []}), &owner),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(view["hosts"], serde_json::json!([]));
        assert_eq!(crate::ops::deliverable_credential(&state, &b), None);
        let changes = security_events(&state)
            .await
            .into_iter()
            .filter(|(k, _)| k == "provider_credential_hosts_changed")
            .count();
        assert_eq!(changes, 3, "set, revoked, narrowed");
    }

    #[tokio::test]
    async fn only_the_owners_session_reaches_the_token_routes() {
        let dir = tempdir::TempDir::new("storm-pc-tiers").unwrap();
        let (app, _, state) = test_router_with_state(dir.path());
        let (_, owner) = owner_bearer(&state).await;
        let (_, host_token) = enroll_a_host(&app, &owner).await;
        let (_, key) = send(
            &app,
            post_json_with_auth("/v1/keys", serde_json::json!({"name": "agent"}), &owner),
        )
        .await;
        let key = format!("Bearer {}", key["secret"].as_str().unwrap());
        let host = format!("Bearer {host_token}");
        for auth in [
            key.as_str(),
            host.as_str(),
            "Bearer sta_forged",
            "Bearer nonsense",
        ] {
            for request in [
                get_with_auth(CLAUDE_SUB, auth),
                put_json_with_auth(CLAUDE_SUB, serde_json::json!({"token": SUB_CANARY}), auth),
                delete_with_auth(CLAUDE_SUB, auth),
                patch_json_with_auth(CLAUDE_SUB_HOSTS, serde_json::json!({"hosts": []}), auth),
            ] {
                let (status, _) = send(&app, request).await;
                assert_eq!(status, StatusCode::UNAUTHORIZED, "{auth}");
            }
        }
        let (status, _) = send(&app, get(CLAUDE_SUB)).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        let (_, view) = send(&app, get_with_auth(CLAUDE_SUB, &owner)).await;
        assert_eq!(view["present"], false);

        // Behind the route, `ops` refuses a non-session actor too.
        let agent = Actor::Key {
            key_id: "key_x".into(),
            user_id: "usr_x".into(),
        };
        let token = crate::gateway::provider::Secret::new(SUB_CANARY.into());
        let refused = crate::ops::put_provider_credential(&state, &agent, token, None)
            .await
            .unwrap_err();
        assert_eq!(refused.status, StatusCode::FORBIDDEN);
        assert!(
            crate::ops::delete_provider_credential(&state, &agent)
                .await
                .is_err()
        );
        assert!(
            crate::ops::set_provider_credential_hosts(&state, &agent, Vec::new())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn a_lost_key_shows_needs_resync_is_audited_once_and_is_never_delivered() {
        let dir = tempdir::TempDir::new("storm-pc-lost").unwrap();
        let (app, _, state) = test_router_with_state(dir.path());
        let (_, owner) = owner_bearer(&state).await;
        let (host, _) = enroll_a_host(&app, &owner).await;
        send(
            &app,
            patch_json_with_auth(
                CLAUDE_SUB_HOSTS,
                serde_json::json!({"hosts": [host]}),
                &owner,
            ),
        )
        .await;
        let synced = sync_token(&app, &owner, SUB_CANARY).await;
        // What `Gateway::open` does on finding the key gone (tested there).
        let stamp = crate::index::now_rfc3339();
        assert!(
            crate::ops::gateway_store(&state)
                .mark_provider_needs_resync(crate::gateway::provider::CREDENTIAL_ID, &stamp)
                .unwrap()
        );
        state
            .gateway
            .provider_unreadable
            .lock()
            .unwrap()
            .push(synced["fingerprint"].as_str().unwrap().into());
        crate::ops::reconcile_single_user(&state).await.unwrap();
        crate::ops::reconcile_single_user(&state).await.unwrap();

        let (_, view) = send(&app, get_with_auth(CLAUDE_SUB, &owner)).await;
        assert_eq!(view["status"], "needs_resync");
        assert_eq!(crate::ops::deliverable_credential(&state, &host), None);
        let unreadable: Vec<_> = security_events(&state)
            .await
            .into_iter()
            .filter(|(k, _)| k == "provider_credential_unreadable")
            .collect();
        assert_eq!(unreadable.len(), 1);
        assert!(
            unreadable[0]
                .1
                .contains(synced["fingerprint"].as_str().unwrap())
        );

        // A re-sync brings it back.
        let again = sync_token(&app, &owner, SUB_CANARY).await;
        assert_eq!(again["status"], "active");
        assert!(crate::ops::deliverable_credential(&state, &host).is_some());
    }

    #[tokio::test]
    async fn the_single_user_sweep_drops_a_token_another_account_left() {
        let dir = tempdir::TempDir::new("storm-pc-sweep").unwrap();
        let (app, _, state) = test_router_with_state(dir.path());
        let (_, owner) = owner_bearer(&state).await;
        sync_token(&app, &owner, SUB_CANARY).await;
        crate::ops::gateway_store(&state)
            .conn()
            .execute(
                "UPDATE provider_credentials SET owner_user_id = 'usr_gone'",
                [],
            )
            .unwrap();
        crate::ops::reconcile_single_user(&state).await.unwrap();
        let (_, view) = send(&app, get_with_auth(CLAUDE_SUB, &owner)).await;
        assert_eq!(view["present"], false);
        assert!(
            security_events(&state)
                .await
                .iter()
                .any(|(k, d)| k == "provider_credential_removed" && d.contains("single_user"))
        );
    }

    #[tokio::test]
    async fn an_expired_token_warns_is_audited_once_and_is_not_delivered() {
        let dir = tempdir::TempDir::new("storm-pc-expired").unwrap();
        let (app, _, state) = test_router_with_state(dir.path());
        let (_, owner) = owner_bearer(&state).await;
        let (host, _) = enroll_a_host(&app, &owner).await;
        send(
            &app,
            patch_json_with_auth(
                CLAUDE_SUB_HOSTS,
                serde_json::json!({"hosts": [host]}),
                &owner,
            ),
        )
        .await;
        // Minted 360 days ago: inside the 7-day window.
        let minted = time::OffsetDateTime::now_utc() - time::Duration::days(360);
        let (_, view) = send(
            &app,
            put_json_with_auth(
                CLAUDE_SUB,
                serde_json::json!({"token": SUB_CANARY,
                                   "minted_at": crate::gateway::provider::format_time(minted)}),
                &owner,
            ),
        )
        .await;
        assert_eq!(view["warn"], "7d");
        assert_eq!(view["days_left"], 5);
        assert!(crate::ops::deliverable_credential(&state, &host).is_some());

        crate::ops::gateway_store(&state)
            .conn()
            .execute(
                "UPDATE provider_credentials SET expires_at = '2026-01-01T00:00:00Z'",
                [],
            )
            .unwrap();
        for _ in 0..2 {
            let (_, view) = send(&app, get_with_auth(CLAUDE_SUB, &owner)).await;
            assert_eq!(view["warn"], "expired");
            assert_eq!(view["days_left"], 0);
        }
        assert_eq!(crate::ops::deliverable_credential(&state, &host), None);
        assert_eq!(
            crate::ops::provider_credential_availability(&state, &host),
            crate::gateway::provider::Availability::Expired
        );
        let expired = security_events(&state)
            .await
            .into_iter()
            .filter(|(k, _)| k == "provider_credential_expired")
            .count();
        assert_eq!(expired, 1, "provider_credential_expired is written once");
    }

    #[tokio::test]
    async fn the_pull_is_the_starting_sessions_own_host_in_scope_and_nothing_else() {
        let dir = tempdir::TempDir::new("storm-pc-pull").unwrap();
        let (app, _, state) = test_router_with_state(dir.path());
        let (_, owner) = owner_bearer(&state).await;
        let (a, a_token) = enroll_a_host(&app, &owner).await;
        let (b, _) = enroll_a_host(&app, &owner).await;
        let mut links = Vec::new();
        for host in [&a, &b] {
            links.push(state.agent.connect_host(host).1);
            state
                .agent
                .hello(host, serde_json::from_value(sync_caps(true)).unwrap())
                .unwrap();
        }
        let launch = |host: String, provider: &'static str| {
            let app = app.clone();
            let owner = owner.clone();
            async move {
                let (status, body) = send(
                    &app,
                    post_json_with_auth(
                        "/v1/agent/sessions",
                        serde_json::json!({"host_id": host, "workspace": "storm",
                                           "provider": provider,
                                           "terminal": {"cols": 80, "rows": 24}}),
                        &owner,
                    ),
                )
                .await;
                assert_eq!(status, StatusCode::OK, "{body}");
                body["id"].as_str().unwrap().to_string()
            }
        };
        let host_a = state.auth_db.lock().await.host_by_id(&a).unwrap().unwrap();
        let host_b = state.auth_db.lock().await.host_by_id(&b).unwrap().unwrap();
        let code = |r: Result<crate::gateway::provider::Secret, crate::gateway::provider::Code>| {
            r.map(|_| "ok").unwrap_or_else(|c| c.code)
        };

        let unticked = launch(a.clone(), "claude-code").await;
        assert_eq!(
            code(crate::ops::pull_provider_credential(&state, &host_a, &unticked).await),
            "host_not_in_scope"
        );
        send(
            &app,
            patch_json_with_auth(CLAUDE_SUB_HOSTS, serde_json::json!({"hosts": [a]}), &owner),
        )
        .await;
        sync_token(&app, &owner, SUB_CANARY).await;
        // Widening reaches only launches told to pull.
        assert_eq!(
            code(crate::ops::pull_provider_credential(&state, &host_a, &unticked).await),
            "host_not_in_scope"
        );
        let on_a = launch(a.clone(), "claude-code").await;
        let shell_on_a = launch(a.clone(), "shell").await;
        let on_b = launch(b.clone(), "claude-code").await;
        let pulled = crate::ops::pull_provider_credential(&state, &host_a, &on_a)
            .await
            .unwrap();
        assert_eq!(pulled.expose(), SUB_CANARY);
        assert!(!format!("{pulled:?}").contains("CANARY"));
        // Retries while starting are allowed.
        assert!(
            crate::ops::pull_provider_credential(&state, &host_a, &on_a)
                .await
                .is_ok()
        );

        for (host, session, want) in [
            (&host_b, on_a.as_str(), "not_your_session"),
            (&host_a, on_b.as_str(), "not_your_session"),
            (&host_a, "ags_nope", "not_your_session"),
            (&host_a, shell_on_a.as_str(), "wrong_provider"),
            (&host_b, on_b.as_str(), "host_not_in_scope"),
        ] {
            assert_eq!(
                code(crate::ops::pull_provider_credential(&state, host, session).await),
                want,
                "{session}"
            );
        }
        let (status, _) = send(
            &app,
            post_json_with_auth(
                &format!("/v1/runtime/sessions/{on_a}/status"),
                serde_json::json!({"status": "running"}),
                &format!("Bearer {a_token}"),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        assert_eq!(
            code(crate::ops::pull_provider_credential(&state, &host_a, &on_a).await),
            "session_not_starting"
        );
        let pending = launch(a.clone(), "claude-code").await;
        let (status, _) = send(&app, delete_with_auth(CLAUDE_SUB, &owner)).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        send(
            &app,
            patch_json_with_auth(CLAUDE_SUB_HOSTS, serde_json::json!({"hosts": [a]}), &owner),
        )
        .await;
        assert_eq!(
            code(crate::ops::pull_provider_credential(&state, &host_a, &pending).await),
            "no_credential"
        );

        let events = security_events(&state).await;
        let delivered: Vec<_> = events
            .iter()
            .filter(|(k, _)| k == "provider_credential_delivered")
            .collect();
        assert_eq!(delivered.len(), 2);
        assert!(delivered[0].1.contains(&on_a) && delivered[0].1.contains(&a));
        for (kind, detail) in &events {
            assert!(!detail.contains("CANARY"), "{kind}: {detail}");
        }
    }

    // ---- MCP Gateway: OAuth (decision 81g) --------------------------------

    /// A mock authorization server and protected MCP resource on one port.
    #[derive(Default)]
    struct MockAs {
        access: std::sync::Mutex<String>,
        refresh: std::sync::Mutex<String>,
        n: std::sync::atomic::AtomicUsize,
        expires_in: std::sync::atomic::AtomicU64,
        reject_refresh: std::sync::atomic::AtomicBool,
        registrations: std::sync::Mutex<Vec<serde_json::Value>>,
        token_requests: std::sync::Mutex<Vec<HashMap<String, String>>>,
        revoked: std::sync::Mutex<Vec<String>>,
        hits: std::sync::atomic::AtomicUsize,
        /// `token_endpoint_auth_methods_supported`, when it says.
        auth_methods: Option<Vec<&'static str>>,
        /// No `registration_endpoint` (Slack's shape).
        no_registration: bool,
        /// Registration hands out a client secret.
        confidential: bool,
        /// The protected resource's `scopes_supported`.
        resource_scopes: Option<Vec<&'static str>>,
    }

    impl MockAs {
        fn refreshes(&self) -> usize {
            self.token_requests
                .lock()
                .unwrap()
                .iter()
                .filter(|r| r.get("grant_type").map(String::as_str) == Some("refresh_token"))
                .count()
        }

        fn issue(&self) -> serde_json::Value {
            let n = self.n.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
            *self.access.lock().unwrap() = format!("upstream-canary-at-{n}");
            *self.refresh.lock().unwrap() = format!("upstream-canary-rt-{n}");
            serde_json::json!({
                "access_token": format!("upstream-canary-at-{n}"),
                "refresh_token": format!("upstream-canary-rt-{n}"),
                "token_type": "Bearer",
                "expires_in": self.expires_in.load(std::sync::atomic::Ordering::SeqCst),
            })
        }
    }

    async fn serve_oauth_upstream(tools: &[&str]) -> (String, Arc<MockAs>) {
        serve_oauth_as(tools, MockAs::default()).await
    }

    async fn serve_oauth_as(tools: &[&str], mock: MockAs) -> (String, Arc<MockAs>) {
        use axum::response::IntoResponse;
        let mock = Arc::new(mock);
        mock.expires_in
            .store(3600, std::sync::atomic::Ordering::SeqCst);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let up = crate::gateway::upstream::tests::mock(tools);
        let mcp = rmcp::transport::streamable_http_server::StreamableHttpService::new(
            move || Ok(up.clone()),
            Arc::new(rmcp::transport::streamable_http_server::session::local::LocalSessionManager::default()),
            rmcp::transport::streamable_http_server::StreamableHttpServerConfig::default()
                .disable_allowed_hosts(),
        );
        let gate_mock = mock.clone();
        let gate_base = base.clone();
        let gate = move |request: axum::extract::Request, next: axum::middleware::Next| {
            let mock = gate_mock.clone();
            let base = gate_base.clone();
            async move {
                let want = format!("Bearer {}", mock.access.lock().unwrap());
                let ok = request
                    .headers()
                    .get("authorization")
                    .and_then(|v| v.to_str().ok())
                    .is_some_and(|v| v == want && !mock.access.lock().unwrap().is_empty());
                if !ok {
                    return (
                        StatusCode::UNAUTHORIZED,
                        [(
                            "www-authenticate",
                            format!(
                                "Bearer resource_metadata=\"{base}/.well-known/oauth-protected-resource/mcp\""
                            ),
                        )],
                    )
                        .into_response();
                }
                next.run(request).await
            }
        };
        let m = mock.clone();
        let b = base.clone();
        let prm = move || {
            let b = b.clone();
            m.hits.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let scopes = m.resource_scopes.clone();
            async move {
                let mut prm = serde_json::json!({"resource": format!("{b}/mcp"), "authorization_servers": [b]});
                if let Some(scopes) = scopes {
                    prm["scopes_supported"] = serde_json::json!(scopes);
                }
                Json(prm)
            }
        };
        let b = base.clone();
        let m = mock.clone();
        let asm = move || {
            let b = b.clone();
            let m = m.clone();
            async move {
                let mut meta = serde_json::json!({
                    "issuer": b,
                    "authorization_endpoint": format!("{b}/authorize"),
                    "token_endpoint": format!("{b}/token"),
                    "registration_endpoint": format!("{b}/register"),
                    "revocation_endpoint": format!("{b}/revoke"),
                    "response_types_supported": ["code"],
                    "code_challenge_methods_supported": ["S256"],
                    "grant_types_supported": ["authorization_code", "refresh_token"],
                });
                if m.no_registration {
                    meta.as_object_mut()
                        .unwrap()
                        .remove("registration_endpoint");
                }
                if let Some(methods) = &m.auth_methods {
                    meta["token_endpoint_auth_methods_supported"] = serde_json::json!(methods);
                }
                Json(meta)
            }
        };
        let m = mock.clone();
        let register = move |Json(body): Json<serde_json::Value>| {
            let m = m.clone();
            async move {
                m.registrations.lock().unwrap().push(body.clone());
                let mut reply = serde_json::json!({"client_id": "dcr-client", "redirect_uris": body["redirect_uris"]});
                if m.confidential {
                    reply["client_secret"] = serde_json::json!("upstream-canary-dcr-secret");
                }
                (StatusCode::CREATED, Json(reply))
            }
        };
        let m = mock.clone();
        let token = move |headers: axum::http::HeaderMap,
                          axum::extract::Form(mut form): axum::extract::Form<
            HashMap<String, String>,
        >| {
            let m = m.clone();
            async move {
                // How the client authenticated, for the tests to read.
                if let Some(basic) = headers
                    .get("authorization")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.strip_prefix("Basic "))
                {
                    let decoded = data_encoding::BASE64
                        .decode(basic.as_bytes())
                        .unwrap_or_default();
                    form.insert("_basic".into(), String::from_utf8_lossy(&decoded).into());
                }
                m.token_requests.lock().unwrap().push(form.clone());
                let bad = || {
                    (
                        StatusCode::BAD_REQUEST,
                        Json(serde_json::json!({"error": "invalid_grant"})),
                    )
                        .into_response()
                };
                match form.get("grant_type").map(String::as_str) {
                    Some("authorization_code")
                        if form.get("code").map(String::as_str) == Some("good-code")
                            && form.get("code_verifier").is_some_and(|v| v.len() >= 43)
                            && form.contains_key("resource") =>
                    {
                        Json(m.issue()).into_response()
                    }
                    Some("refresh_token")
                        if !m.reject_refresh.load(std::sync::atomic::Ordering::SeqCst)
                            && form.get("refresh_token")
                                == Some(&m.refresh.lock().unwrap().clone()) =>
                    {
                        Json(m.issue()).into_response()
                    }
                    _ => bad(),
                }
            }
        };
        let m = mock.clone();
        let revoke =
            move |axum::extract::Form(form): axum::extract::Form<HashMap<String, String>>| {
                let m = m.clone();
                async move {
                    m.revoked
                        .lock()
                        .unwrap()
                        .push(form.get("token").cloned().unwrap_or_default());
                    StatusCode::OK
                }
            };
        let app = axum::Router::new()
            .route(
                "/.well-known/oauth-protected-resource/mcp",
                axum::routing::get(prm.clone()),
            )
            .route(
                "/.well-known/oauth-protected-resource",
                axum::routing::get(prm),
            )
            .route(
                "/.well-known/oauth-authorization-server",
                axum::routing::get(asm),
            )
            .route("/register", axum::routing::post(register))
            .route("/token", axum::routing::post(token))
            .route("/revoke", axum::routing::post(revoke))
            .nest_service(
                "/mcp",
                axum::Router::new()
                    .fallback_service(mcp)
                    .layer(axum::middleware::from_fn(gate)),
            );
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (base, mock)
    }

    fn query_param(url: &str, name: &str) -> Option<String> {
        let parsed = url::Url::parse(url).unwrap();
        parsed
            .query_pairs()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.to_string())
    }

    async fn authorize(app: &Router, bearer: &str, id: &str) -> (StatusCode, serde_json::Value) {
        send(
            app,
            post_json_with_auth(
                &format!("{CONNECTIONS}/{id}/authorize"),
                serde_json::json!({"redirect_uri": "http://127.0.0.1:53682/callback"}),
                bearer,
            ),
        )
        .await
    }

    async fn callback(
        app: &Router,
        bearer: &str,
        state: &str,
        code: &str,
    ) -> (StatusCode, serde_json::Value) {
        send(
            app,
            post_json_with_auth(
                "/v1/integrations/oauth/callback",
                serde_json::json!({"state": state, "code": code}),
                bearer,
            ),
        )
        .await
    }

    #[tokio::test]
    async fn an_oauth_integration_is_authorized_through_the_client_and_its_tokens_sealed() {
        let dir = tempdir::TempDir::new("storm-oauth-flow").unwrap();
        let (app, _, state) = test_router_with_state(dir.path());
        state.gateway.set_allow_http_upstreams(true);
        let (_, owner) = owner_bearer(&state).await;
        let (base, mock) = serve_oauth_upstream(&["search"]).await;
        let (status, created) = send(
            &app,
            post_json_with_auth(
                CONNECTIONS,
                serde_json::json!({"display_name": "Notion", "url": format!("{base}/mcp"), "auth_kind": "oauth"}),
                &owner,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{created}");
        assert_eq!(created["status"], "pending_auth");
        let id = created["id"].as_str().unwrap().to_string();

        // Only a loopback or storm:// redirect (G-D13).
        let (status, _) = send(
            &app,
            post_json_with_auth(
                &format!("{CONNECTIONS}/{id}/authorize"),
                serde_json::json!({"redirect_uri": "http://192.168.1.51:8484/oauth"}),
                &owner,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        let (status, started) = authorize(&app, &owner, &id).await;
        assert_eq!(status, StatusCode::OK, "{started}");
        let url = started["authorization_url"].as_str().unwrap().to_string();
        assert!(url.starts_with(&format!("{base}/authorize")), "{url}");
        assert_eq!(
            query_param(&url, "code_challenge_method").as_deref(),
            Some("S256")
        );
        assert_eq!(
            query_param(&url, "client_id").as_deref(),
            Some("dcr-client")
        );
        assert!(query_param(&url, "resource").is_some(), "{url}");
        let flow_state = query_param(&url, "state").unwrap();
        // Registered dynamically, as a public client.
        let reg = mock.registrations.lock().unwrap().clone();
        assert_eq!(reg.len(), 1);
        assert_eq!(reg[0]["token_endpoint_auth_method"], "none");
        // A second authorization reuses that client.
        let (_, again) = authorize(&app, &owner, &id).await;
        assert_eq!(mock.registrations.lock().unwrap().len(), 1);
        let spare_state =
            query_param(again["authorization_url"].as_str().unwrap(), "state").unwrap();

        // The state is stored only as its hash.
        let flows: i64 = state
            .gateway
            .store
            .lock()
            .unwrap()
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM oauth_flows WHERE state_hash = ?1",
                rusqlite::params![flow_state],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(flows, 0, "a raw state was stored");

        // The browser comes back to the client; the client relays it.
        let (status, test) = callback(&app, &owner, &flow_state, "good-code").await;
        assert_eq!(status, StatusCode::OK, "{test}");
        assert_eq!(test["ok"], true, "{test}");
        assert_eq!(test["integration"]["status"], "connected");
        assert_eq!(test["integration"]["has_credential"], true);
        assert_eq!(
            test["integration"]["tool_allowlist"],
            serde_json::json!(["search"])
        );
        let exchange = mock.token_requests.lock().unwrap()[0].clone();
        assert!(exchange.contains_key("resource") && exchange.contains_key("code_verifier"));

        // Single use: the same state again exchanges nothing.
        let (status, body) = callback(&app, &owner, &flow_state, "good-code").await;
        assert_eq!(
            (status, body["error"].clone()),
            (StatusCode::BAD_REQUEST, "oauth_flow_expired_or_used".into())
        );
        assert_eq!(mock.token_requests.lock().unwrap().len(), 1);
        // An expired flow is refused too.
        state
            .gateway
            .store
            .lock()
            .unwrap()
            .conn()
            .execute(
                "UPDATE oauth_flows SET expires_at = '2000-01-01T00:00:00Z'",
                [],
            )
            .unwrap();
        let (status, _) = callback(&app, &owner, &spare_state, "good-code").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        // The tokens are sealed: not in the database, not in any event.
        let bytes = std::fs::read(crate::gateway::db_path(&state.state_dir)).unwrap();
        let wal = std::fs::read(format!(
            "{}-wal",
            crate::gateway::db_path(&state.state_dir).display()
        ))
        .unwrap_or_default();
        for b in [&bytes, &wal] {
            assert!(
                !b.windows(16).any(|w| w == b"upstream-canary-"),
                "a token is in gateway.db"
            );
        }
        let events = state.auth_db.lock().await.all_events().unwrap();
        assert!(events.iter().any(|(k, _)| k == "integration_authorized"));
        assert!(events.iter().all(|(_, d)| !d.contains("upstream-canary")));
    }

    #[tokio::test]
    async fn tokens_refresh_once_under_concurrency_persist_rotated_and_a_rejected_refresh_needs_reauth()
     {
        let dir = tempdir::TempDir::new("storm-oauth-refresh").unwrap();
        let (app, _, state) = test_router_with_state(dir.path());
        state.gateway.set_allow_http_upstreams(true);
        let (_, owner) = owner_bearer(&state).await;
        let (base, mock) = serve_oauth_upstream(&["search"]).await;
        let (_, created) = send(
            &app,
            post_json_with_auth(
                CONNECTIONS,
                serde_json::json!({"display_name": "Linear", "url": format!("{base}/mcp"), "auth_kind": "oauth"}),
                &owner,
            ),
        )
        .await;
        let id = created["id"].as_str().unwrap().to_string();
        // Tokens that are always within a minute of expiry.
        mock.expires_in
            .store(30, std::sync::atomic::Ordering::SeqCst);
        let (_, started) = authorize(&app, &owner, &id).await;
        let flow_state =
            query_param(started["authorization_url"].as_str().unwrap(), "state").unwrap();
        let (_, test) = callback(&app, &owner, &flow_state, "good-code").await;
        assert_eq!(test["ok"], true, "{test}");

        // Two calls at once find the stored token stale: one refresh between
        // them, and the fresh token (an hour long) serves the second.
        mock.expires_in
            .store(3600, std::sync::atomic::Ordering::SeqCst);
        let before = mock.refreshes();
        let c = state
            .gateway
            .store
            .lock()
            .unwrap()
            .connection(&id)
            .unwrap()
            .unwrap();
        let (a, b) = tokio::join!(
            state.gateway.target(&c, false),
            state.gateway.target(&c, false)
        );
        assert!(a.is_ok() && b.is_ok());
        assert_eq!(
            mock.refreshes(),
            before + 1,
            "the refresh was not single-flight"
        );
        // The rotated pair is what is stored, before anything used it.
        let stored = crate::gateway::oauth::TokenStore {
            gateway: state.gateway.clone(),
            connection: id.clone(),
        };
        let saved = rmcp::transport::auth::CredentialStore::load(&stored)
            .await
            .unwrap()
            .unwrap();
        {
            use oauth2::TokenResponse;
            let t = saved.token_response.unwrap();
            assert_eq!(
                t.refresh_token().unwrap().secret(),
                &*mock.refresh.lock().unwrap()
            );
        }

        // A rejected refresh: the integration needs reconnecting, audited.
        // Its stored token is made stale again, so the test must refresh.
        mock.expires_in
            .store(30, std::sync::atomic::Ordering::SeqCst);
        state.gateway.target(&c, true).await.unwrap();
        mock.reject_refresh
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let (_, test) = send(
            &app,
            post_json_with_auth(
                &format!("{CONNECTIONS}/{id}/test"),
                serde_json::json!({}),
                &owner,
            ),
        )
        .await;
        assert_eq!(test["ok"], false);
        assert_eq!(test["integration"]["status"], "needs_reauth", "{test}");
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let events = state.auth_db.lock().await.all_events().unwrap();
        assert!(
            events
                .iter()
                .any(|(k, _)| k == "integration_refresh_failed"),
            "{events:?}"
        );

        // Reconnecting through a new authorization restores it.
        mock.reject_refresh
            .store(false, std::sync::atomic::Ordering::SeqCst);
        mock.expires_in
            .store(3600, std::sync::atomic::Ordering::SeqCst);
        let (_, started) = authorize(&app, &owner, &id).await;
        let flow_state =
            query_param(started["authorization_url"].as_str().unwrap(), "state").unwrap();
        let (_, test) = callback(&app, &owner, &flow_state, "good-code").await;
        assert_eq!(test["integration"]["status"], "connected", "{test}");
        let events = state.auth_db.lock().await.all_events().unwrap();
        assert!(events.iter().any(|(k, _)| k == "integration_reauthorized"));

        // Disconnecting revokes upstream, best effort (RFC 7009).
        let latest = mock.refresh.lock().unwrap().clone();
        let (status, _) = send(
            &app,
            delete_with_auth(&format!("{CONNECTIONS}/{id}"), &owner),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        for _ in 0..50 {
            if !mock.revoked.lock().unwrap().is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert_eq!(*mock.revoked.lock().unwrap(), vec![latest]);
    }

    #[tokio::test]
    async fn a_flow_is_spent_once_even_by_racing_callbacks() {
        let dir = tempdir::TempDir::new("storm-oauth-race").unwrap();
        let (app, _, state) = test_router_with_state(dir.path());
        state.gateway.set_allow_http_upstreams(true);
        let (_, owner) = owner_bearer(&state).await;
        let (base, mock) = serve_oauth_upstream(&["search"]).await;
        let (_, created) = send(
            &app,
            post_json_with_auth(
                CONNECTIONS,
                serde_json::json!({"display_name": "Race", "url": format!("{base}/mcp"), "auth_kind": "oauth"}),
                &owner,
            ),
        )
        .await;
        let id = created["id"].as_str().unwrap().to_string();
        let (_, started) = authorize(&app, &owner, &id).await;
        let flow_state =
            query_param(started["authorization_url"].as_str().unwrap(), "state").unwrap();

        let (status, body) = callback(&app, &owner, "not-the-state", "good-code").await;
        assert_eq!(
            (status, body["error"].clone()),
            (StatusCode::BAD_REQUEST, "oauth_flow_expired_or_used".into())
        );
        assert!(mock.token_requests.lock().unwrap().is_empty());

        // Two callbacks at once: exactly one exchanges the code.
        let (a, b) = tokio::join!(
            callback(&app, &owner, &flow_state, "good-code"),
            callback(&app, &owner, &flow_state, "good-code")
        );
        let oks = [a.0, b.0].iter().filter(|s| **s == StatusCode::OK).count();
        assert_eq!(oks, 1, "{a:?} {b:?}");
        let exchanges = mock
            .token_requests
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.get("grant_type").map(String::as_str) == Some("authorization_code"))
            .count();
        assert_eq!(exchanges, 1, "a flow was spent twice");
    }

    /// Every byte of the gateway's database files under `dir`, WAL included.
    fn gateway_db_bytes(dir: &FsPath) -> Vec<u8> {
        let mut out = Vec::new();
        let mut stack = vec![dir.to_path_buf()];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).unwrap().flatten() {
                let path = e.path();
                if path.is_dir() {
                    stack.push(path);
                } else if path
                    .file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with("gateway.db"))
                {
                    out.extend(std::fs::read(&path).unwrap_or_default());
                }
            }
        }
        out
    }

    async fn oauth_connection(app: &Router, owner: &str, base: &str) -> String {
        let (status, created) = send(
            app,
            post_json_with_auth(
                CONNECTIONS,
                serde_json::json!({"display_name": "Slack", "url": format!("{base}/mcp"), "auth_kind": "oauth"}),
                owner,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{created}");
        created["id"].as_str().unwrap().to_string()
    }

    /// 131e: a server with no registration and `client_secret_post` only (the
    /// Slack shape) is signed in with a pasted client: its secret goes in the
    /// token request's body, never in the clear at rest, and with no scopes
    /// given the resource's own are asked for.
    #[tokio::test]
    async fn a_slack_shaped_server_signs_in_with_a_pasted_client() {
        let dir = tempdir::TempDir::new("storm-oauth-pasted").unwrap();
        let (app, _, state) = test_router_with_state(dir.path());
        state.gateway.set_allow_http_upstreams(true);
        let (_, owner) = owner_bearer(&state).await;
        let (base, mock) = serve_oauth_as(
            &["search"],
            MockAs {
                no_registration: true,
                auth_methods: Some(vec!["client_secret_post"]),
                resource_scopes: Some(vec!["chat:write", "search:read"]),
                ..Default::default()
            },
        )
        .await;
        let id = oauth_connection(&app, &owner, &base).await;

        let (status, refused) = authorize(&app, &owner, &id).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(refused["error"], "oauth_client_required", "{refused}");

        let secret = "upstream-canary-pasted-secret";
        let (status, started) = send(
            &app,
            post_json_with_auth(
                &format!("{CONNECTIONS}/{id}/authorize"),
                serde_json::json!({"redirect_uri": "http://127.0.0.1:53682/callback",
                    "client_id": "pasted-id", "client_secret": secret}),
                &owner,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{started}");
        let url = started["authorization_url"].as_str().unwrap();
        assert_eq!(query_param(url, "client_id").as_deref(), Some("pasted-id"));
        assert_eq!(
            query_param(url, "scope").as_deref(),
            Some("chat:write search:read"),
            "MCP's scope rule, with none given"
        );
        let flow = query_param(url, "state").unwrap();
        let (status, test) = callback(&app, &owner, &flow, "good-code").await;
        assert_eq!(status, StatusCode::OK, "{test}");
        assert_eq!(test["integration"]["status"], "connected", "{test}");
        let exchange = mock.token_requests.lock().unwrap()[0].clone();
        assert_eq!(
            exchange.get("client_secret").map(String::as_str),
            Some(secret)
        );
        assert!(!exchange.contains_key("_basic"), "{exchange:?}");
        assert!(mock.registrations.lock().unwrap().is_empty());
        let bytes = gateway_db_bytes(dir.path());
        assert!(!bytes.is_empty());
        assert!(
            !bytes.windows(secret.len()).any(|w| w == secret.as_bytes()),
            "a pasted secret is sealed"
        );

        for bad in [
            serde_json::json!({"client_id": "x", "client_secret": ""}),
            serde_json::json!({"client_id": "x", "scopes": ["two words"]}),
        ] {
            let mut body = bad.clone();
            body["redirect_uri"] = serde_json::json!("http://127.0.0.1:53682/callback");
            let (status, _) = send(
                &app,
                post_json_with_auth(&format!("{CONNECTIONS}/{id}/authorize"), body, &owner),
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");
        }
    }

    /// 131f: a server that offers no public registration registers Storm as a
    /// confidential client with the method its token client will use, and the
    /// secret it hands out is sealed and used.
    #[tokio::test]
    async fn a_server_without_public_registration_gets_a_confidential_client() {
        for (methods, want) in [
            (
                vec!["client_secret_basic", "client_secret_post"],
                "client_secret_basic",
            ),
            (vec!["client_secret_post"], "client_secret_post"),
        ] {
            let dir = tempdir::TempDir::new("storm-oauth-confidential").unwrap();
            let (app, _, state) = test_router_with_state(dir.path());
            state.gateway.set_allow_http_upstreams(true);
            let (_, owner) = owner_bearer(&state).await;
            let (base, mock) = serve_oauth_as(
                &["search"],
                MockAs {
                    auth_methods: Some(methods.clone()),
                    confidential: true,
                    ..Default::default()
                },
            )
            .await;
            let id = oauth_connection(&app, &owner, &base).await;
            let (status, started) = authorize(&app, &owner, &id).await;
            assert_eq!(status, StatusCode::OK, "{started}");
            let reg = mock.registrations.lock().unwrap().clone();
            assert_eq!(reg.len(), 1);
            assert_eq!(reg[0]["token_endpoint_auth_method"], want, "{methods:?}");
            let url = started["authorization_url"].as_str().unwrap();
            assert_eq!(query_param(url, "client_id").as_deref(), Some("dcr-client"));
            let flow = query_param(url, "state").unwrap();
            let (status, test) = callback(&app, &owner, &flow, "good-code").await;
            assert_eq!(status, StatusCode::OK, "{test}");
            assert_eq!(test["integration"]["status"], "connected", "{test}");
            let exchange = mock.token_requests.lock().unwrap()[0].clone();
            let secret = "upstream-canary-dcr-secret";
            match want {
                "client_secret_basic" => assert_eq!(
                    exchange.get("_basic").map(String::as_str),
                    Some(format!("dcr-client:{secret}").as_str())
                ),
                _ => assert_eq!(
                    exchange.get("client_secret").map(String::as_str),
                    Some(secret)
                ),
            }
            let bytes = gateway_db_bytes(dir.path());
            assert!(!bytes.windows(secret.len()).any(|w| w == secret.as_bytes()));
        }
    }

    /// 131f: a registration that comes back without a secret is a failure,
    /// never a public client by accident.
    #[tokio::test]
    async fn a_confidential_registration_without_a_secret_fails() {
        let dir = tempdir::TempDir::new("storm-oauth-no-secret").unwrap();
        let (app, _, state) = test_router_with_state(dir.path());
        state.gateway.set_allow_http_upstreams(true);
        let (_, owner) = owner_bearer(&state).await;
        let (base, _) = serve_oauth_as(
            &["search"],
            MockAs {
                auth_methods: Some(vec!["client_secret_post"]),
                ..Default::default()
            },
        )
        .await;
        let id = oauth_connection(&app, &owner, &base).await;
        let (status, refused) = authorize(&app, &owner, &id).await;
        assert_eq!(status, StatusCode::BAD_GATEWAY);
        assert_eq!(refused["error"], "oauth_registration_failed", "{refused}");
    }

    #[tokio::test]
    async fn oauth_discovery_never_reaches_a_private_address() {
        let dir = tempdir::TempDir::new("storm-oauth-boundary").unwrap();
        let (app, _, state) = test_router_with_state(dir.path());
        let (_, owner) = owner_bearer(&state).await;
        let (base, mock) = serve_oauth_upstream(&["search"]).await;
        // https on a loopback address: a URL the owner may save, and one
        // discovery must refuse to touch (no test switch here).
        let loopback = base.replace("http://", "https://");
        let (_, created) = send(
            &app,
            post_json_with_auth(
                CONNECTIONS,
                serde_json::json!({"display_name": "Sneaky", "url": format!("{loopback}/mcp"), "auth_kind": "oauth"}),
                &owner,
            ),
        )
        .await;
        let id = created["id"].as_str().unwrap().to_string();
        let (status, body) = authorize(&app, &owner, &id).await;
        assert_eq!(status, StatusCode::BAD_GATEWAY, "{body}");
        assert_eq!(body["error"], "oauth_discovery_failed");
        assert_eq!(mock.hits.load(std::sync::atomic::Ordering::SeqCst), 0);

        // A static integration is not authorized this way.
        let static_id = create_github(&app, &owner, "ghp_x").await["id"]
            .as_str()
            .unwrap()
            .to_string();
        let (status, _) = authorize(&app, &owner, &static_id).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn a_refused_host_learns_nothing_about_why() {
        let dir = tempdir::TempDir::new("storm-hosts-refusal").unwrap();
        let (app, _, _) = test_router_with_state(dir.path());
        let (status, body) = send(
            &app,
            post_json(
                "/v1/runtime/auth/challenge",
                serde_json::json!({"host_id": "hst_01HB6V3Z7Q2M4N8P0R5S9T1W3X"}),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(body["error"], "refused");
    }

    // ---- LAN discovery pairing (decision 118) ------------------------------

    const LAN_PASSWORD: &str = "correct horse battery staple";

    pub(crate) async fn seed_owner_with_password(state: &Shared, password: &str) {
        let hash = state.hasher.hash(password.to_string()).await.unwrap();
        let mut auth_db = state.auth_db.lock().await;
        crate::auth::account::create_account(&mut auth_db, &hash, "2026-10-14T00:00:00Z").unwrap();
    }

    pub(crate) fn pair_password_from(
        password: &str,
        peer: Option<&str>,
    ) -> axum::http::Request<axum::body::Body> {
        let mut req = post_json(
            "/v1/pairings/password",
            serde_json::json!({
                "password": password,
                "device_name": "Pixel 10",
                "platform": "android",
                "version": "0.9.0",
            }),
        );
        if let Some(peer) = peer {
            req.extensions_mut()
                .insert(axum::extract::ConnectInfo(std::net::SocketAddr::new(
                    peer.parse().unwrap(),
                    54321,
                )));
        }
        req
    }

    #[tokio::test]
    async fn password_pairing_on_the_lan_returns_device_credentials() {
        let dir = tempdir::TempDir::new("storm-lan-pair").unwrap();
        let (app, identity, state) = test_router_with_state(dir.path());
        seed_owner_with_password(&state, LAN_PASSWORD).await;

        let (status, body) =
            send(&app, pair_password_from(LAN_PASSWORD, Some("192.168.1.40"))).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let device_id = body["device_id"].as_str().unwrap();
        let secret = body["device_secret"].as_str().unwrap();
        assert!(device_id.starts_with("dev_") && secret.starts_with("dvs_"));
        assert_eq!(body["server_id"], identity.server_id);
        assert_eq!(body["public_key"], identity.public_key_b64());

        // The credential is a real device credential: it signs in.
        let (status, _) = send(
            &app,
            post_json_with_auth(
                "/v1/auth/login",
                serde_json::json!({"password": LAN_PASSWORD}),
                &format!("StormDevice {device_id}:{secret}"),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let devices = state.auth_db.lock().await.list_devices().unwrap();
        let device = devices.iter().find(|d| d.id == device_id).unwrap();
        assert_eq!(device.paired_by.as_deref(), Some("lan_password"));
        assert_eq!(device.name, "Pixel 10");
    }

    #[tokio::test]
    async fn every_lan_scoped_peer_class_may_pair() {
        let dir = tempdir::TempDir::new("storm-lan-classes").unwrap();
        let (app, _, state) = test_router_with_state(dir.path());
        seed_owner_with_password(&state, LAN_PASSWORD).await;
        for peer in [
            "127.0.0.1",
            "::1",
            "10.1.2.3",
            "172.16.0.5",
            "192.168.1.40",
            "169.254.1.1",
            "fe80::1",
            "fd00::1",
            "::ffff:192.168.1.41",
        ] {
            let (status, body) = send(&app, pair_password_from(LAN_PASSWORD, Some(peer))).await;
            assert_eq!(status, StatusCode::OK, "{peer}: {body}");
        }
    }

    #[tokio::test]
    async fn password_pairing_through_a_relay_is_lan_only() {
        let dir = tempdir::TempDir::new("storm-lan-relayed").unwrap();
        let (app, _, state) = test_router_with_state(dir.path());
        seed_owner_with_password(&state, LAN_PASSWORD).await;

        // A relay attests a LAN address when its client is on the relay's
        // LAN; the marker, not the address, is what refuses it.
        let mut req = pair_password_from(LAN_PASSWORD, Some("192.168.1.40"));
        req.extensions_mut().insert(crate::relay::Relayed);
        let (status, body) = send(&app, req).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body["error"], "lan_only");
        assert!(
            state
                .auth_db
                .lock()
                .await
                .list_devices()
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn password_pairing_from_off_the_lan_is_refused() {
        let dir = tempdir::TempDir::new("storm-lan-public").unwrap();
        let (app, _, state) = test_router_with_state(dir.path());
        seed_owner_with_password(&state, LAN_PASSWORD).await;

        for peer in [
            Some("203.0.113.9"),
            Some("2606:4700::1111"),
            Some("100.64.0.1"),
            Some("100.101.102.103"),
            None,
        ] {
            let (status, body) = send(&app, pair_password_from(LAN_PASSWORD, peer)).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{peer:?}");
            assert_eq!(body["error"], "lan_only", "{peer:?}");
        }
    }

    #[tokio::test]
    async fn password_pairing_from_an_untrusted_network_is_refused() {
        let dir = tempdir::TempDir::new("storm-lan-untrusted").unwrap();
        let (app, _, state) = test_router_with_state(dir.path());
        seed_owner_with_password(&state, LAN_PASSWORD).await;
        let os = crate::nettrust::fake::FakeOs::default()
            .with("firewall-cmd --state", "running\n")
            .with(
                "firewall-cmd --get-active-zones",
                "public\n  interfaces: wlan0\nhome\n  interfaces: enp3s0\n",
            );
        let ifaces = [
            crate::nettrust::Iface {
                name: "wlan0".into(),
                ip: "192.168.1.13".parse().unwrap(),
                prefix: 24,
            },
            crate::nettrust::Iface {
                name: "enp3s0".into(),
                ip: "10.0.0.5".parse().unwrap(),
                prefix: 24,
            },
        ];
        state
            .discovery
            .trust
            .refresh(crate::nettrust::Platform::Linux, &os, &ifaces);

        let (status, body) =
            send(&app, pair_password_from(LAN_PASSWORD, Some("192.168.1.40"))).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
        assert_eq!(body["error"], "lan_only");
        assert!(
            state
                .auth_db
                .lock()
                .await
                .list_devices()
                .unwrap()
                .is_empty()
        );

        let (status, body) = send(&app, pair_password_from(LAN_PASSWORD, Some("10.0.0.9"))).await;
        assert_eq!(
            status,
            StatusCode::OK,
            "a trusted network still pairs: {body}"
        );
    }

    #[tokio::test]
    async fn password_pairing_is_refused_while_discovery_is_off() {
        let dir = tempdir::TempDir::new("storm-lan-off").unwrap();
        let (app, _, state) = test_router_with_state(dir.path());
        seed_owner_with_password(&state, LAN_PASSWORD).await;
        let user = state.auth_db.lock().await.account().unwrap().unwrap().id;
        let token = session_token(&state, &user).await;

        let (status, body) = send(
            &app,
            put_json(
                "/v1/config/discovery",
                serde_json::json!({"enabled": false}),
                Some(&format!("Bearer {token}")),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["active"], false);
        assert!(
            !state.vaults.read().await.registry.discoverable(),
            "persisted"
        );

        let (status, body) = send(&app, pair_password_from(LAN_PASSWORD, Some("10.0.0.7"))).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body["error"], "lan_only");

        let (_, config) = send(
            &app,
            get_with_auth("/v1/config", &format!("Bearer {token}")),
        )
        .await;
        assert_eq!(config["discovery"]["discoverable"], false);
        assert_eq!(config["discovery"]["forced_off"], false);
    }

    #[tokio::test]
    async fn password_pairing_never_claims_a_fresh_server() {
        let dir = tempdir::TempDir::new("storm-lan-fresh").unwrap();
        let (app, _, state) = test_router_with_state(dir.path());
        let (status, body) =
            send(&app, pair_password_from(LAN_PASSWORD, Some("192.168.1.40"))).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(body["error"], "setup_required");
        assert!(
            state
                .auth_db
                .lock()
                .await
                .list_devices()
                .unwrap()
                .is_empty()
        );
        assert!(!state.auth_db.lock().await.has_account().unwrap());
    }

    #[tokio::test]
    async fn a_wrong_pairing_password_answers_as_login_does() {
        let dir = tempdir::TempDir::new("storm-lan-wrong").unwrap();
        let (app, _, state) = test_router_with_state(dir.path());
        seed_owner_with_password(&state, LAN_PASSWORD).await;
        let device = pair_a_device(&state).await;

        let (pair_status, pair_body) = send(
            &app,
            pair_password_from("not the password", Some("192.168.1.40")),
        )
        .await;
        let (login_status, login_body) = send(
            &app,
            post_json_with_auth(
                "/v1/auth/login",
                serde_json::json!({"password": "not the password"}),
                &device,
            ),
        )
        .await;
        assert_eq!(pair_status, StatusCode::UNAUTHORIZED);
        assert_eq!((pair_status, pair_body), (login_status, login_body));
    }

    #[tokio::test]
    async fn password_pairing_is_charged_to_the_login_limiter_and_refunded() {
        let dir = tempdir::TempDir::new("storm-lan-limiter").unwrap();
        let (app, _, state) = test_router_with_limiter(dir.path(), throttling_limiter());
        seed_owner_with_password(&state, LAN_PASSWORD).await;
        let peer = Some("192.168.1.40");

        // More successes than the burst: each is refunded.
        for _ in 0..crate::auth::ratelimit::tests::TEST_BURST + 1 {
            let (status, body) = send(&app, pair_password_from(LAN_PASSWORD, peer)).await;
            assert_eq!(status, StatusCode::OK, "{body}");
        }
        // Failures are not.
        for _ in 0..crate::auth::ratelimit::tests::TEST_BURST {
            let (status, _) = send(&app, pair_password_from("wrong", peer)).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED);
        }
        let (status, body) = send(&app, pair_password_from(LAN_PASSWORD, peer)).await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(body["error"], "rate_limited");
        assert!(body["retry_after"].as_i64().unwrap() > 0);

        // The same bucket as sign-in: a login from that peer is throttled too.
        let device = pair_a_device(&state).await;
        let (status, _) = send(&app, login_from(&device, "", Some("192.168.1.40"))).await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    }

    #[tokio::test]
    async fn password_pairing_records_a_security_event_without_the_secret() {
        let dir = tempdir::TempDir::new("storm-lan-event").unwrap();
        let (app, _, state) = test_router_with_state(dir.path());
        seed_owner_with_password(&state, LAN_PASSWORD).await;
        let (status, body) =
            send(&app, pair_password_from(LAN_PASSWORD, Some("192.168.1.40"))).await;
        assert_eq!(status, StatusCode::OK);
        let secret = body["device_secret"].as_str().unwrap().to_string();
        let device_id = body["device_id"].as_str().unwrap().to_string();

        let db = state.auth_db.lock().await;
        let kind = crate::ops::EVENT_DEVICE_PAIRED_PASSWORD;
        assert_eq!(db.event_count(kind).unwrap(), 1);
        assert_eq!(
            db.latest_event_remote(kind).unwrap().as_deref(),
            Some("192.168.1.40")
        );
        let rows = db.all_events().unwrap();
        let detail = &rows.iter().find(|(k, _)| k == kind).unwrap().1;
        assert!(detail.contains("android"), "{detail}");
        let device = db.find_device(&device_id).unwrap().unwrap();
        assert_eq!(device.paired_by.as_deref(), Some("lan_password"));
        for (kind, detail) in &rows {
            assert!(!detail.contains(LAN_PASSWORD), "{kind}: {detail}");
            assert!(!detail.contains(&secret), "{kind}: {detail}");
            assert!(!detail.contains("argon2"), "{kind}: {detail}");
        }
    }

    // ---- token sync: the launch wire and the pull route (decision 124) ---

    const CANARY: &str = "sk-ant-oat01-CANARY-124o-0123456789abcdefghij";
    const KIND: &str = crate::gateway::provider::KIND;

    struct SyncFixture {
        app: Router,
        state: Shared,
        owner: String,
        owner_bearer: String,
        host: String,
        host_bearer: String,
        link: tokio::sync::mpsc::UnboundedReceiver<crate::agent::Envelope>,
    }

    fn sync_caps(can_sync: bool) -> serde_json::Value {
        let mut caps = host_caps(true);
        caps["providers"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({"id": "opencode", "kind": "cli", "interactions": ["terminal"], "available": true}));
        if can_sync {
            caps["provider_credentials"] = serde_json::json!([KIND]);
        }
        caps
    }

    async fn sync_fixture(dir: &FsPath, can_sync: bool) -> SyncFixture {
        let (app, _, state) = test_router_with_state(dir);
        let owner = seed_owner(&state).await;
        let owner_bearer = format!("Bearer {}", session_token(&state, &owner).await);
        let (host, token) = enroll_a_host(&app, &owner_bearer).await;
        let link = online_host(&state, &host, can_sync);
        SyncFixture {
            app,
            state,
            owner,
            owner_bearer,
            host,
            host_bearer: format!("Bearer {token}"),
            link,
        }
    }

    fn online_host(
        state: &Shared,
        host: &str,
        can_sync: bool,
    ) -> tokio::sync::mpsc::UnboundedReceiver<crate::agent::Envelope> {
        let (_, link) = state.agent.connect_host(host);
        state
            .agent
            .hello(host, serde_json::from_value(sync_caps(can_sync)).unwrap())
            .unwrap();
        link
    }

    impl SyncFixture {
        fn actor(&self) -> Actor {
            Actor::Session {
                user_id: self.owner.clone(),
            }
        }

        async fn sync(&self, minted_at: Option<&str>) {
            crate::ops::put_provider_credential(
                &self.state,
                &self.actor(),
                crate::gateway::provider::Secret::new(CANARY.into()),
                minted_at.map(Into::into),
            )
            .await
            .unwrap();
        }

        async fn tick(&self, hosts: &[&str]) {
            crate::ops::set_provider_credential_hosts(
                &self.state,
                &self.actor(),
                hosts.iter().map(|h| h.to_string()).collect(),
            )
            .await
            .unwrap();
        }

        /// Launches on `host` without reporting it running: the session
        /// stays `starting`, the only state a pull is allowed in.
        async fn launch_on(&self, host: &str, provider: &str) -> serde_json::Value {
            let (status, body) = send(
                &self.app,
                post_json_with_auth(
                    "/v1/agent/sessions",
                    serde_json::json!({
                        "host_id": host, "workspace": "storm", "provider": provider,
                        "terminal": {"cols": 80, "rows": 24},
                    }),
                    &self.owner_bearer,
                ),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{body}");
            body
        }

        async fn launch(&self, provider: &str) -> serde_json::Value {
            self.launch_on(&self.host.clone(), provider).await
        }

        fn start(&mut self) -> serde_json::Value {
            let cmd = self.link.try_recv().unwrap();
            let wire = serde_json::to_value(&cmd).unwrap();
            assert_eq!(wire["type"], "start", "{wire}");
            wire
        }

        async fn pull_as(&self, bearer: &str, session: &str) -> (StatusCode, HeaderMap, String) {
            let response = self
                .app
                .clone()
                .oneshot(get_with_auth(
                    &format!("/v1/runtime/sessions/{session}/provider-credential"),
                    bearer,
                ))
                .await
                .unwrap();
            let status = response.status();
            let headers = response.headers().clone();
            let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
                .await
                .unwrap();
            (status, headers, String::from_utf8(bytes.to_vec()).unwrap())
        }

        async fn pull(&self, session: &str) -> (StatusCode, HeaderMap, String) {
            self.pull_as(&self.host_bearer.clone(), session).await
        }

        async fn report(&self, session: &str, status: &str) {
            let (code, body) = send(
                &self.app,
                post_json_with_auth(
                    &format!("/v1/runtime/sessions/{session}/status"),
                    serde_json::json!({"status": status}),
                    &self.host_bearer,
                ),
            )
            .await;
            assert!(code.is_success(), "{code} {body}");
        }
    }

    fn refusal_code(body: &str) -> String {
        serde_json::from_str::<serde_json::Value>(body).unwrap()["error"]
            .as_str()
            .unwrap()
            .to_string()
    }

    fn id_of(launched: &serde_json::Value) -> String {
        launched["id"].as_str().unwrap().to_string()
    }

    #[tokio::test]
    async fn a_native_launch_sends_the_start_bytes_it_always_did() {
        let dir = tempdir::TempDir::new("storm-ts-native").unwrap();
        let mut f = sync_fixture(dir.path(), true).await;
        // A token synced but no host ticked (default none, AC-TS3).
        f.sync(None).await;
        let launched = f.launch("claude-code").await;
        assert_eq!(launched["credential"], "native", "{launched}");
        assert!(launched["credential_notice"].is_null(), "{launched}");
        let wire = serde_json::to_string(&f.link.try_recv().unwrap()).unwrap();
        let session = id_of(&launched);
        let expected = format!(
            r#"{{"cmd_seq":1,"type":"start","session":"{session}","workspace":"storm","provider":"claude-code","interaction":"terminal","terminal":{{"cols":80,"rows":24}},"mcp":[{{"id":"storm","slug":"storm"}}]}}"#
        );
        assert_eq!(wire, expected);
        let (status, _, body) = f.pull(&session).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(refusal_code(&body), "host_not_in_scope");
    }

    #[tokio::test]
    async fn a_ticked_host_is_told_the_kind_and_pulls_the_token_while_starting() {
        let dir = tempdir::TempDir::new("storm-ts-synced").unwrap();
        let mut f = sync_fixture(dir.path(), true).await;
        f.sync(None).await;
        f.tick(&[&f.host.clone()]).await;
        let launched = f.launch("claude-code").await;
        assert_eq!(launched["credential"], "synced", "{launched}");
        assert!(!launched.to_string().contains(CANARY));
        let wire = f.start();
        assert_eq!(wire["provider_credential"], KIND, "{wire}");
        assert!(!wire.to_string().contains(CANARY), "{wire}");

        let session = id_of(&launched);
        let (status, headers, body) = f.pull(&session).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(headers["cache-control"], "no-store");
        let pulled: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(pulled["kind"], KIND);
        assert_eq!(pulled["value"], CANARY);
        assert!(pulled["expires_at"].is_string(), "{pulled}");
        // A retry during `starting` is allowed: a blip must not fail a launch.
        assert_eq!(f.pull(&session).await.0, StatusCode::OK);

        // Once running, never again.
        f.report(&session, "running").await;
        let (status, headers, body) = f.pull(&session).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(refusal_code(&body), "session_not_starting");
        assert!(!body.contains(CANARY));
        assert!(headers.get("cache-control").is_none());

        let events = f.state.auth_db.lock().await.all_events().unwrap();
        let delivered: Vec<_> = events
            .iter()
            .filter(|(k, _)| k == "provider_credential_delivered")
            .collect();
        assert_eq!(delivered.len(), 2, "{events:?}");
        assert!(delivered[0].1.contains(&session), "{delivered:?}");
        assert!(delivered[0].1.contains(KIND), "{delivered:?}");
        assert!(delivered[0].1.contains("fingerprint"), "{delivered:?}");
        assert!(
            events
                .iter()
                .any(|(k, d)| k == "provider_credential_refused"
                    && d.contains("session_not_starting")),
            "{events:?}"
        );
        for (kind, detail) in &events {
            assert!(!detail.contains(CANARY), "{kind}: {detail}");
            assert!(!detail.contains("CANARY-124o"), "{kind}: {detail}");
        }
    }

    #[tokio::test]
    async fn the_pull_is_refused_for_another_hosts_session_and_unknown_ones() {
        let dir = tempdir::TempDir::new("storm-ts-other").unwrap();
        let mut f = sync_fixture(dir.path(), true).await;
        let (other, other_token) = enroll_a_host(&f.app, &f.owner_bearer).await;
        let mut other_link = online_host(&f.state, &other, true);
        f.sync(None).await;
        // Both ticked, so only ownership can refuse it.
        f.tick(&[&f.host.clone(), &other]).await;
        let session = id_of(&f.launch("claude-code").await);
        f.start();
        let (status, _, body) = f.pull_as(&format!("Bearer {other_token}"), &session).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(refusal_code(&body), "not_your_session");
        let (status, _, body) = f.pull("ags_NOPE").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(refusal_code(&body), "not_your_session");

        // The other host's own session, launched while it was ticked.
        let theirs = id_of(&f.launch_on(&other, "claude-code").await);
        let wire = serde_json::to_value(other_link.try_recv().unwrap()).unwrap();
        assert_eq!(wire["provider_credential"], KIND);
        assert_eq!(f.pull(&theirs).await.0, StatusCode::NOT_FOUND);
        assert_eq!(
            f.pull_as(&format!("Bearer {other_token}"), &theirs).await.0,
            StatusCode::OK
        );
    }

    #[tokio::test]
    async fn narrowing_and_removal_stop_the_pull_and_widening_waits_for_a_new_launch() {
        let dir = tempdir::TempDir::new("storm-ts-scope").unwrap();
        let mut f = sync_fixture(dir.path(), true).await;
        let host = f.host.clone();
        f.sync(None).await;
        f.tick(&[&host]).await;
        let session = id_of(&f.launch("claude-code").await);
        assert_eq!(f.start()["provider_credential"], KIND);
        // Unticked while starting: the in-flight pull is refused (AC-TS5).
        f.tick(&[]).await;
        let (status, _, body) = f.pull(&session).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(refusal_code(&body), "host_not_in_scope");
        // Widening does not reach a launch that was not told to pull.
        let unsynced = f.launch("claude-code").await;
        assert_eq!(unsynced["credential"], "native");
        assert!(f.start().get("provider_credential").is_none());
        f.tick(&[&host]).await;
        let (status, _, body) = f.pull(&id_of(&unsynced)).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(refusal_code(&body), "host_not_in_scope");
        // Removed: no field on a new launch, and a pending pull finds nothing
        // (AC-TS4).
        let pending = id_of(&f.launch("claude-code").await);
        assert_eq!(f.start()["provider_credential"], KIND);
        crate::ops::delete_provider_credential(&f.state, &f.actor())
            .await
            .unwrap();
        f.tick(&[&host]).await;
        let (status, _, body) = f.pull(&pending).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(refusal_code(&body), "no_credential");
        let after = f.launch("claude-code").await;
        assert_eq!(after["credential"], "native");
        assert!(f.start().get("provider_credential").is_none());
    }

    #[tokio::test]
    async fn other_providers_never_get_the_field_or_the_token() {
        let dir = tempdir::TempDir::new("storm-ts-providers").unwrap();
        let mut f = sync_fixture(dir.path(), true).await;
        f.sync(None).await;
        f.tick(&[&f.host.clone()]).await;
        for provider in ["opencode", "shell"] {
            let launched = f.launch(provider).await;
            assert_eq!(launched["credential"], "native", "{launched}");
            assert!(f.start().get("provider_credential").is_none(), "{provider}");
            let (status, _, body) = f.pull(&id_of(&launched)).await;
            assert_eq!(status, StatusCode::CONFLICT, "{provider}");
            assert_eq!(refusal_code(&body), "wrong_provider");
        }
    }

    #[tokio::test]
    async fn an_old_host_launches_on_its_own_login_with_a_notice() {
        let dir = tempdir::TempDir::new("storm-ts-old").unwrap();
        let mut f = sync_fixture(dir.path(), false).await;
        f.sync(None).await;
        f.tick(&[&f.host.clone()]).await;
        let launched = f.launch("claude-code").await;
        assert_eq!(launched["status"], "starting", "{launched}");
        assert_eq!(launched["credential"], "old_host");
        let notice = launched["credential_notice"].as_str().unwrap();
        assert!(notice.contains("build-vm"), "{notice}");
        assert!(notice.contains("update storm-runtime"), "{notice}");
        assert!(f.start().get("provider_credential").is_none());
        let (status, _, _) = f.pull(&id_of(&launched)).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn an_expired_token_is_not_delivered_and_the_launch_says_so() {
        let dir = tempdir::TempDir::new("storm-ts-expired").unwrap();
        let mut f = sync_fixture(dir.path(), true).await;
        f.sync(Some("2020-01-01T00:00:00Z")).await;
        f.tick(&[&f.host.clone()]).await;
        let launched = f.launch("claude-code").await;
        assert_eq!(launched["credential"], "expired", "{launched}");
        assert!(
            launched["credential_notice"]
                .as_str()
                .unwrap()
                .contains("re-sync"),
            "{launched}"
        );
        assert!(f.start().get("provider_credential").is_none());
    }

    #[tokio::test]
    async fn only_a_host_token_reaches_the_pull_route() {
        let dir = tempdir::TempDir::new("storm-ts-tiers").unwrap();
        let mut f = sync_fixture(dir.path(), true).await;
        f.sync(None).await;
        f.tick(&[&f.host.clone()]).await;
        let session = id_of(&f.launch("claude-code").await);
        f.start();
        let (status, created) = send(
            &f.app,
            post_json_with_auth(
                "/v1/keys",
                serde_json::json!({"name": "a key"}),
                &f.owner_bearer,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{created}");
        let key = format!("Bearer {}", created["secret"].as_str().unwrap());
        for bearer in [f.owner_bearer.clone(), key, "Bearer nonsense".into()] {
            let (status, _, body) = f.pull_as(&bearer, &session).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED, "{bearer}: {body}");
            assert!(!body.contains(CANARY));
        }
        assert_eq!(f.pull(&session).await.0, StatusCode::OK);
    }

    #[test]
    fn the_pulled_credential_never_debug_prints() {
        let pulled = crate::ops::provider_wire::PulledCredential {
            kind: KIND,
            value: crate::gateway::provider::Secret::new(CANARY.into()),
            expires_at: None,
        };
        let debug = format!("{pulled:?}");
        assert!(!debug.contains("CANARY"), "{debug}");
        assert!(serde_json::to_string(&pulled).unwrap().contains(CANARY));
    }

    // ---- agent installs (decision 133) ------------------------------------

    fn install_caps(agent_install: bool) -> serde_json::Value {
        let provider = |id: &str, extra: serde_json::Value| {
            let mut p = serde_json::json!({
                "id": id, "kind": "cli", "interactions": ["terminal"], "available": true,
            });
            p.as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            p
        };
        serde_json::json!({
            "version": "0.14.0",
            "providers": [
                provider("claude-code", serde_json::json!({"version": "2.1.0", "source": "storm", "installable": true})),
                provider("opencode", serde_json::json!({"version": "1.0.0", "source": "system", "installable": true, "latest": "1.1.0"})),
                provider("codex", serde_json::json!({"available": false, "installable": false, "install_blocked": "no_node"})),
                provider("cursor", serde_json::json!({"available": false, "installable": true, "checksum_verified": false})),
                provider("command-code", serde_json::json!({"version": "0.3.0", "source": "storm", "installable": true})),
                provider("shell", serde_json::json!({})),
            ],
            "workspaces": ["storm"],
            "max_sessions": 8,
            "mcp_bridge": true,
            "agent_install": agent_install,
        })
    }

    struct InstallFixture {
        app: Router,
        state: Shared,
        owner_bearer: String,
        host: String,
        host_bearer: String,
        link: tokio::sync::mpsc::UnboundedReceiver<crate::agent::Envelope>,
    }

    async fn install_fixture(dir: &FsPath) -> InstallFixture {
        let (app, _, state) = test_router_with_state(dir);
        let owner = seed_owner(&state).await;
        let owner_bearer = format!("Bearer {}", session_token(&state, &owner).await);
        let (host, token) = enroll_a_host(&app, &owner_bearer).await;
        let link = install_host_online(&state, &host, true);
        InstallFixture {
            app,
            state,
            owner_bearer,
            host,
            host_bearer: format!("Bearer {token}"),
            link,
        }
    }

    fn install_host_online(
        state: &Shared,
        host: &str,
        agent_install: bool,
    ) -> tokio::sync::mpsc::UnboundedReceiver<crate::agent::Envelope> {
        let (_, link) = state.agent.connect_host(host);
        state
            .agent
            .hello(
                host,
                serde_json::from_value(install_caps(agent_install)).unwrap(),
            )
            .unwrap();
        link
    }

    impl InstallFixture {
        async fn start_on(
            &self,
            host: &str,
            agent: &str,
            op: &str,
        ) -> (StatusCode, serde_json::Value) {
            send(
                &self.app,
                post_json_with_auth(
                    &format!("/v1/agent/hosts/{host}/agents/{agent}"),
                    serde_json::json!({ "op": op }),
                    &self.owner_bearer,
                ),
            )
            .await
        }

        async fn start(&self, agent: &str, op: &str) -> (StatusCode, serde_json::Value) {
            self.start_on(&self.host.clone(), agent, op).await
        }

        async fn job(&self, job: &str) -> (StatusCode, serde_json::Value) {
            send(
                &self.app,
                get_with_auth(
                    &format!("/v1/agent/hosts/{}/agents/jobs/{job}", self.host),
                    &self.owner_bearer,
                ),
            )
            .await
        }

        async fn report_as(
            &self,
            bearer: &str,
            job: &str,
            body: serde_json::Value,
        ) -> (StatusCode, serde_json::Value) {
            send(
                &self.app,
                post_json_with_auth(&format!("/v1/runtime/agents/jobs/{job}"), body, bearer),
            )
            .await
        }

        async fn report(&self, job: &str, body: serde_json::Value) -> StatusCode {
            self.report_as(&self.host_bearer.clone(), job, body).await.0
        }

        async fn install_events(&self) -> Vec<serde_json::Value> {
            security_events(&self.state)
                .await
                .into_iter()
                .filter(|(kind, _)| kind == crate::ops::EVENT_AGENT_INSTALL)
                .map(|(_, detail)| serde_json::from_str(&detail).unwrap())
                .collect()
        }
    }

    #[tokio::test]
    async fn the_owner_installs_an_agent_and_the_host_reports_it_through() {
        let dir = tempdir::TempDir::new("storm-install-flow").unwrap();
        let mut f = install_fixture(dir.path()).await;

        // The host list carries what the host reported, additively.
        let (status, hosts) = send(&f.app, get_with_auth("/v1/agent/hosts", &f.owner_bearer)).await;
        assert_eq!(status, StatusCode::OK);
        let caps = &hosts[0]["capabilities"];
        assert_eq!(caps["agent_install"], true);
        assert_eq!(caps["providers"][1]["latest"], "1.1.0");
        assert_eq!(caps["providers"][2]["install_blocked"], "no_node");
        assert_eq!(caps["providers"][3]["checksum_verified"], false);
        assert!(caps["providers"][5].get("source").is_none(), "{caps}");

        let (status, job) = f.start("cursor", "install").await;
        assert_eq!(status, StatusCode::ACCEPTED, "{job}");
        let id = job["id"].as_str().unwrap().to_string();
        assert!(id.starts_with("agj_") && id.len() == 28, "{id}");
        assert_eq!(job["host_id"], f.host);
        assert_eq!(job["agent"], "cursor");
        assert_eq!(job["op"], "install");
        assert_eq!(job["state"], "queued");
        assert_eq!(job["auto"], false);
        assert!(
            job.get("step").is_none() && job.get("error").is_none(),
            "{job}"
        );
        assert!(job["created"].as_str().is_some() && job["updated"].as_str().is_some());

        let sent = serde_json::to_value(f.link.try_recv().unwrap()).unwrap();
        assert_eq!(
            sent,
            serde_json::json!({
                "cmd_seq": sent["cmd_seq"], "type": "agent.install",
                "job": id, "agent": "cursor", "op": "install",
            })
        );
        assert!(sent["cmd_seq"].as_u64().unwrap() >= 1);

        assert_eq!(
            f.report(&id, serde_json::json!({"step": "Downloading"}))
                .await,
            StatusCode::NO_CONTENT
        );
        let (_, job) = f.job(&id).await;
        assert_eq!(
            (job["state"].clone(), job["step"].clone()),
            ("running".into(), "Downloading".into())
        );
        assert!(f.install_events().await.is_empty());

        assert_eq!(
            f.report(
                &id,
                serde_json::json!({"done": {"version": "2026.10.24-1"}})
            )
            .await,
            StatusCode::NO_CONTENT
        );
        let (status, job) = f.job(&id).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(job["state"], "done");
        assert_eq!(job["version"], "2026.10.24-1");

        // A finished job keeps its result.
        assert_eq!(
            f.report(&id, serde_json::json!({"failed": "disk_full"}))
                .await,
            StatusCode::NO_CONTENT
        );
        let (_, again) = f.job(&id).await;
        assert_eq!(again, job);

        let events = f.install_events().await;
        assert_eq!(events.len(), 1, "{events:?}");
        assert_eq!(
            events[0],
            serde_json::json!({
                "job_id": id, "op": "install", "agent": "cursor", "host_id": f.host,
                "result": "done", "version": "2026.10.24-1", "auto": false,
            })
        );
        let host_token = f.host_bearer.trim_start_matches("Bearer ");
        let owner_token = f.owner_bearer.trim_start_matches("Bearer ");
        for (kind, detail) in security_events(&f.state).await {
            assert!(!detail.contains(host_token), "{kind}: {detail}");
            assert!(!detail.contains(owner_token), "{kind}: {detail}");
        }
    }

    #[tokio::test]
    async fn a_failed_job_is_audited_with_its_code_and_never_the_hosts_text() {
        let dir = tempdir::TempDir::new("storm-install-failed").unwrap();
        let f = install_fixture(dir.path()).await;
        let (_, job) = f.start("opencode", "update").await;
        let id = job["id"].as_str().unwrap();
        let (status, body) = f
            .report_as(&f.host_bearer, id, serde_json::json!({"step": "Compiling"}))
            .await;
        assert_eq!(
            (status, body["error"].clone()),
            (StatusCode::BAD_REQUEST, "unknown_step".into())
        );
        let (status, _) = f
            .report_as(
                &f.host_bearer,
                id,
                serde_json::json!({"done": {"version": "1 https://x"}}),
            )
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let (status, _) = f.report_as(&f.host_bearer, id, serde_json::json!({})).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(f.job(id).await.1["state"], "queued");

        let leak = "npm ERR! https://registry.example/token=SECRET";
        assert_eq!(
            f.report(id, serde_json::json!({ "failed": leak })).await,
            StatusCode::NO_CONTENT
        );
        let (_, job) = f.job(id).await;
        assert_eq!(
            (job["state"].clone(), job["error"].clone()),
            ("failed".into(), "unknown".into())
        );
        let events = f.install_events().await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["result"], "unknown");
        assert_eq!(events[0]["version"], serde_json::Value::Null);
        for (_, detail) in security_events(&f.state).await {
            assert!(
                !detail.contains("SECRET") && !detail.contains("https://"),
                "{detail}"
            );
        }
    }

    #[tokio::test]
    async fn an_install_is_refused_with_a_stable_code() {
        let dir = tempdir::TempDir::new("storm-install-refusals").unwrap();
        let f = install_fixture(dir.path()).await;
        let code = |r: (StatusCode, serde_json::Value)| {
            (r.0, r.1["error"].as_str().unwrap_or("").to_string())
        };

        assert_eq!(
            f.start_on("hst_nope", "codex", "install").await.0,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            code(f.start("vim", "install").await),
            (StatusCode::BAD_REQUEST, "unknown_agent".into())
        );
        assert_eq!(
            code(f.start("shell", "install").await),
            (StatusCode::BAD_REQUEST, "unknown_agent".into())
        );
        assert_eq!(
            code(f.start("cursor", "reinstall").await),
            (StatusCode::BAD_REQUEST, "unknown_op".into())
        );

        let (old, _) = enroll_a_host(&f.app, &f.owner_bearer).await;
        let _old_link = install_host_online(&f.state, &old, false);
        assert_eq!(
            code(f.start_on(&old, "cursor", "install").await),
            (StatusCode::CONFLICT, "host_too_old".into())
        );
        let (offline, _) = enroll_a_host(&f.app, &f.owner_bearer).await;
        assert_eq!(
            f.start_on(&offline, "cursor", "install").await.0,
            StatusCode::SERVICE_UNAVAILABLE
        );

        assert_eq!(
            code(f.start("codex", "install").await),
            (StatusCode::CONFLICT, "no_node".into())
        );
        assert_eq!(
            code(f.start("codex", "update").await),
            (StatusCode::CONFLICT, "no_node".into())
        );
        assert_eq!(
            code(f.start("opencode", "remove").await),
            (StatusCode::CONFLICT, "not_managed".into())
        );
        assert_eq!(
            code(f.start("cursor", "remove").await),
            (StatusCode::CONFLICT, "not_managed".into())
        );

        let (status, body) = send(
            &f.app,
            post_json_with_auth(
                "/v1/agent/sessions",
                serde_json::json!({
                    "host_id": f.host, "workspace": "storm", "provider": "claude-code",
                    "terminal": {"cols": 80, "rows": 24},
                }),
                &f.owner_bearer,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(
            code(f.start("claude-code", "remove").await),
            (StatusCode::CONFLICT, "agent_in_use".into())
        );
        // Only a removal waits for the sessions.
        assert_eq!(
            f.start("claude-code", "update").await.0,
            StatusCode::ACCEPTED
        );
        assert_eq!(
            code(f.start("claude-code", "update").await),
            (StatusCode::CONFLICT, "busy".into())
        );
        assert_eq!(
            f.start("command-code", "remove").await.0,
            StatusCode::ACCEPTED
        );
        assert_eq!(
            code(f.start("cursor", "install").await),
            (StatusCode::CONFLICT, "busy".into())
        );
        assert!(f.install_events().await.is_empty());
    }

    #[tokio::test]
    async fn a_host_reports_only_its_own_jobs() {
        let dir = tempdir::TempDir::new("storm-install-other-host").unwrap();
        let f = install_fixture(dir.path()).await;
        let (other, other_token) = enroll_a_host(&f.app, &f.owner_bearer).await;
        let _other_link = install_host_online(&f.state, &other, true);
        let other_bearer = format!("Bearer {other_token}");

        let (_, job) = f.start("cursor", "install").await;
        let id = job["id"].as_str().unwrap();
        let (status, _) = f
            .report_as(
                &other_bearer,
                id,
                serde_json::json!({"done": {"version": "9"}}),
            )
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (status, _) = f
            .report_as(
                &f.host_bearer,
                "agj_000000000000000000000000",
                serde_json::json!({"step": "Verifying"}),
            )
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        // Nor does the owner find it under another host.
        let (status, _) = send(
            &f.app,
            get_with_auth(
                &format!("/v1/agent/hosts/{other}/agents/jobs/{id}"),
                &f.owner_bearer,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(f.job(id).await.1["state"], "queued");
    }

    #[tokio::test]
    async fn agent_installs_stay_in_their_tiers() {
        let dir = tempdir::TempDir::new("storm-install-tiers").unwrap();
        let f = install_fixture(dir.path()).await;
        let (_, job) = f.start("cursor", "install").await;
        let id = job["id"].as_str().unwrap();
        let (_, secret) = seed_key(&f.state).await;
        let key = format!("Bearer {secret}");

        let start = format!("/v1/agent/hosts/{}/agents/opencode", f.host);
        let get = format!("/v1/agent/hosts/{}/agents/jobs/{id}", f.host);
        for bearer in [&key, &f.host_bearer] {
            let (status, _) = send(
                &f.app,
                post_json_with_auth(&start, serde_json::json!({"op": "update"}), bearer),
            )
            .await;
            assert_eq!(status, StatusCode::UNAUTHORIZED, "{start}");
            let (status, _) = send(&f.app, get_with_auth(&get, bearer)).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED, "{get}");
        }
        for bearer in [&key, &f.owner_bearer] {
            let (status, _) = f
                .report_as(bearer, id, serde_json::json!({"done": {"version": "1"}}))
                .await;
            assert_eq!(status, StatusCode::UNAUTHORIZED);
            let (status, _) = send(
                &f.app,
                post_json_with_auth(
                    "/v1/runtime/agents/jobs",
                    serde_json::json!({"agent": "opencode", "op": "update", "auto": true}),
                    bearer,
                ),
            )
            .await;
            assert_eq!(status, StatusCode::UNAUTHORIZED);
        }
        assert_eq!(f.job(id).await.1["state"], "queued");
        // No MCP tool starts one.
        assert!(
            !include_str!("mcp.rs").contains("start_agent_job"),
            "agent installs are never an MCP tool"
        );
    }

    #[tokio::test]
    async fn a_hosts_own_update_is_a_job_it_reports_and_is_audited_as_automatic() {
        let dir = tempdir::TempDir::new("storm-install-auto").unwrap();
        let f = install_fixture(dir.path()).await;
        let auto = |agent: &str, op: &str, bearer: &str| {
            let request = post_json_with_auth(
                "/v1/runtime/agents/jobs",
                serde_json::json!({"agent": agent, "op": op, "auto": true}),
                bearer,
            );
            let app = f.app.clone();
            async move { send(&app, request).await }
        };

        let (status, body) = auto("opencode", "install", &f.host_bearer).await;
        assert_eq!(
            (status, body["error"].clone()),
            (StatusCode::BAD_REQUEST, "auto_update_only".into())
        );
        let (status, body) = auto("vim", "update", &f.host_bearer).await;
        assert_eq!(
            (status, body["error"].clone()),
            (StatusCode::BAD_REQUEST, "unknown_agent".into())
        );

        let (status, body) = auto("opencode", "update", &f.host_bearer).await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        let id = body["job"].as_str().unwrap().to_string();
        assert!(id.starts_with("agj_"));
        assert_eq!(body.as_object().unwrap().len(), 1, "{body}");

        let (status, job) = f.job(&id).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            (job["state"].clone(), job["auto"].clone(), job["op"].clone()),
            ("running".into(), true.into(), "update".into())
        );

        // It holds its agent, and the host's second slot, like any job.
        let (status, body) = auto("opencode", "update", &f.host_bearer).await;
        assert_eq!(
            (status, body["error"].clone()),
            (StatusCode::CONFLICT, "busy".into())
        );
        assert_eq!(f.start("opencode", "update").await.1["error"], "busy");
        assert_eq!(f.start("cursor", "install").await.0, StatusCode::ACCEPTED);
        let (status, body) = auto("command-code", "update", &f.host_bearer).await;
        assert_eq!(
            (status, body["error"].clone()),
            (StatusCode::CONFLICT, "busy".into())
        );

        let (other, other_token) = enroll_a_host(&f.app, &f.owner_bearer).await;
        let _other_link = install_host_online(&f.state, &other, true);
        let (status, _) = f
            .report_as(
                &format!("Bearer {other_token}"),
                &id,
                serde_json::json!({"step": "Installing"}),
            )
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        assert_eq!(
            f.report(&id, serde_json::json!({"step": "Installing"}))
                .await,
            StatusCode::NO_CONTENT
        );
        assert_eq!(f.job(&id).await.1["step"], "Installing");
        assert_eq!(
            f.report(&id, serde_json::json!({"done": {"version": "1.1.0"}}))
                .await,
            StatusCode::NO_CONTENT
        );
        assert_eq!(f.job(&id).await.1["state"], "done");
        let events = f.install_events().await;
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0],
            serde_json::json!({
                "job_id": id, "op": "update", "agent": "opencode", "host_id": f.host,
                "result": "done", "version": "1.1.0", "auto": true,
            })
        );
    }

    /// Decision 128a: a 404 must mean only "no such note". The client's
    /// outbox drops a queued edit on 404, so any other failure reported as one
    /// loses the user's work.
    mod index_errors {
        use super::*;

        struct Fixture {
            _dir: tempdir::TempDir,
            app: Router,
            bearer: String,
            vault: String,
            vault_dir: std::path::PathBuf,
        }

        async fn fixture() -> Fixture {
            let dir = tempdir::TempDir::new("storm-index-errors").unwrap();
            let (app, _, state) = test_router_with_state(dir.path());
            let (_, bearer) = owner_bearer(&state).await;
            let (status, body) = send(
                &app,
                post_json_with_auth("/v1/vaults", serde_json::json!({"name": "notes"}), &bearer),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{body}");
            let vault = body["id"].as_str().unwrap().to_string();
            let vault_dir = dir.path().join("vaults").join("notes");
            Fixture {
                _dir: dir,
                app,
                bearer,
                vault,
                vault_dir,
            }
        }

        impl Fixture {
            async fn create(&self, path: &str) -> (StatusCode, serde_json::Value) {
                send(
                    &self.app,
                    post_json_with_auth(
                        &format!("/v1/vaults/{}/notes", self.vault),
                        serde_json::json!({"path": path, "content": "# Body\n"}),
                        &self.bearer,
                    ),
                )
                .await
            }

            async fn note_id(&self, path: &str) -> String {
                let (status, body) = self.create(path).await;
                assert_eq!(status, StatusCode::OK, "{body}");
                body["note"]["id"].as_str().unwrap().to_string()
            }

            async fn put(&self, id: &str, base_version: i64) -> StatusCode {
                send(
                    &self.app,
                    put_json_with_auth(
                        &format!("/v1/vaults/{}/notes/{id}", self.vault),
                        serde_json::json!({"base_version": base_version, "content": "edited\n"}),
                        &self.bearer,
                    ),
                )
                .await
                .0
            }

            async fn attachment(&self, method: &str, path: &str) -> StatusCode {
                let request = axum::http::Request::builder()
                    .method(method)
                    .uri(format!("/v1/vaults/{}/attachments/{path}", self.vault))
                    .header("authorization", &self.bearer)
                    .body(axum::body::Body::from("clobbered"))
                    .unwrap();
                send(&self.app, request).await.0
            }

            /// A read-only vault directory is how these tests make a write fail
            /// at the disk. Unix only: on Windows the directory attribute does
            /// not stop a file being created in it.
            #[cfg(unix)]
            fn read_only(&self, on: bool) {
                use std::os::unix::fs::PermissionsExt;
                let mode = if on { 0o555 } else { 0o755 };
                std::fs::set_permissions(&self.vault_dir, std::fs::Permissions::from_mode(mode))
                    .unwrap();
            }
        }

        #[cfg(unix)]
        #[tokio::test]
        async fn a_save_that_cannot_be_written_is_a_500_not_a_404() {
            let f = fixture().await;
            let id = f.note_id("A.md").await;
            f.read_only(true);
            let status = f.put(&id, 1).await;
            f.read_only(false);
            assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        }

        #[cfg(unix)]
        #[tokio::test]
        async fn a_delete_that_cannot_be_written_is_a_500_not_a_404() {
            let f = fixture().await;
            let id = f.note_id("A.md").await;
            f.read_only(true);
            let (status, _) = send(
                &f.app,
                delete_with_auth(&format!("/v1/vaults/{}/notes/{id}", f.vault), &f.bearer),
            )
            .await;
            f.read_only(false);
            assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        }

        #[tokio::test]
        async fn only_an_unknown_note_is_a_404() {
            let f = fixture().await;
            assert_eq!(f.put("no-such-note", 1).await, StatusCode::NOT_FOUND);
        }

        #[tokio::test]
        async fn a_note_path_must_end_in_md() {
            let f = fixture().await;
            assert_eq!(f.create("Ideas").await.0, StatusCode::BAD_REQUEST);
            assert_eq!(f.create("Ideas.txt").await.0, StatusCode::BAD_REQUEST);
            let id = f.note_id("A.md").await;
            let (status, _) = send(
                &f.app,
                post_json_with_auth(
                    &format!("/v1/vaults/{}/notes/{id}/move", f.vault),
                    serde_json::json!({"new_path": "A.txt"}),
                    &f.bearer,
                ),
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST);
            assert!(
                f.vault_dir.join("A.md").is_file(),
                "the refused move left the note alone"
            );
        }

        #[tokio::test]
        async fn creating_onto_an_existing_path_is_a_conflict() {
            let f = fixture().await;
            f.note_id("A.md").await;
            assert_eq!(f.create("A.md").await.0, StatusCode::CONFLICT);
        }

        #[tokio::test]
        async fn moving_onto_an_existing_note_is_a_conflict() {
            let f = fixture().await;
            let id = f.note_id("A.md").await;
            f.note_id("B.md").await;
            let (status, _) = send(
                &f.app,
                post_json_with_auth(
                    &format!("/v1/vaults/{}/notes/{id}/move", f.vault),
                    serde_json::json!({"new_path": "B.md"}),
                    &f.bearer,
                ),
            )
            .await;
            assert_eq!(status, StatusCode::CONFLICT);
        }

        #[tokio::test]
        async fn the_attachment_routes_cannot_reach_a_note() {
            let f = fixture().await;
            f.note_id("N.md").await;
            let before = std::fs::read_to_string(f.vault_dir.join("N.md")).unwrap();
            assert_eq!(f.attachment("PUT", "N.md").await, StatusCode::BAD_REQUEST);
            assert_eq!(
                f.attachment("DELETE", "N.md").await,
                StatusCode::BAD_REQUEST
            );
            assert_eq!(
                std::fs::read_to_string(f.vault_dir.join("N.md")).unwrap(),
                before
            );
        }

        #[tokio::test]
        async fn a_missing_attachment_is_a_404() {
            let f = fixture().await;
            assert_eq!(f.attachment("GET", "nope.png").await, StatusCode::NOT_FOUND);
        }

        #[tokio::test]
        async fn deleting_a_folder_that_does_not_exist_is_a_404() {
            let f = fixture().await;
            let (status, _) = send(
                &f.app,
                delete_with_auth(&format!("/v1/vaults/{}/folders/Nope", f.vault), &f.bearer),
            )
            .await;
            assert_eq!(status, StatusCode::NOT_FOUND);
        }
    }

    /// Decision 128g: what the HTTP surface accepts and how it serves files.
    mod http_surface {
        use super::*;

        const MIB: usize = 1024 * 1024;

        struct Fixture {
            _dir: tempdir::TempDir,
            app: Router,
            bearer: String,
            vault: String,
            vault_dir: std::path::PathBuf,
        }

        async fn fixture() -> Fixture {
            let dir = tempdir::TempDir::new("storm-http-surface").unwrap();
            let (app, _, state) = test_router_with_state(dir.path());
            let (_, bearer) = owner_bearer(&state).await;
            let (status, body) = send(
                &app,
                post_json_with_auth("/v1/vaults", serde_json::json!({"name": "notes"}), &bearer),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{body}");
            let vault = body["id"].as_str().unwrap().to_string();
            let vault_dir = dir.path().join("vaults").join("notes");
            Fixture {
                _dir: dir,
                app,
                bearer,
                vault,
                vault_dir,
            }
        }

        impl Fixture {
            fn notes(&self) -> String {
                format!("/v1/vaults/{}/notes", self.vault)
            }

            async fn raw(&self, method: &str, uri: &str, body: Vec<u8>) -> Response {
                let request = axum::http::Request::builder()
                    .method(method)
                    .uri(uri)
                    .header("authorization", &self.bearer)
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(body))
                    .unwrap();
                self.app.clone().oneshot(request).await.unwrap()
            }

            async fn create(&self, path: &str, content: &str) -> StatusCode {
                let body = serde_json::json!({"path": path, "content": content});
                self.raw("POST", &self.notes(), body.to_string().into_bytes())
                    .await
                    .status()
            }

            async fn note_id(&self, path: &str) -> String {
                let (status, body) = send(
                    &self.app,
                    post_json_with_auth(
                        &self.notes(),
                        serde_json::json!({"path": path, "content": "# A\n"}),
                        &self.bearer,
                    ),
                )
                .await;
                assert_eq!(status, StatusCode::OK, "{body}");
                body["note"]["id"].as_str().unwrap().to_string()
            }

            async fn put_note(&self, id: &str, content: &str) -> StatusCode {
                let body = serde_json::json!({"base_version": 1, "content": content});
                self.raw(
                    "PUT",
                    &format!("{}/{id}", self.notes()),
                    body.to_string().into_bytes(),
                )
                .await
                .status()
            }

            fn attachment_uri(&self, path: &str) -> String {
                format!("/v1/vaults/{}/attachments/{path}", self.vault)
            }
        }

        #[tokio::test]
        async fn a_body_that_is_not_an_attachment_or_a_note_keeps_the_2_mib_default() {
            let f = fixture().await;
            let pad = "x".repeat(3 * MIB);
            let body = format!("{{\"vault_root\": \"/nowhere\", \"pad\": \"{pad}\"}}");
            let response = f.raw("PUT", "/v1/config", body.into_bytes()).await;
            assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
        }

        #[tokio::test]
        async fn a_note_takes_up_to_16_mib() {
            let f = fixture().await;
            assert_eq!(
                f.create("Big.md", &"x".repeat(3 * MIB)).await,
                StatusCode::OK
            );
            assert_eq!(
                f.create("Huge.md", &"x".repeat(17 * MIB)).await,
                StatusCode::PAYLOAD_TOO_LARGE
            );
            let id = f.note_id("A.md").await;
            assert_eq!(f.put_note(&id, &"y".repeat(3 * MIB)).await, StatusCode::OK);
            assert_eq!(
                f.put_note(&id, &"y".repeat(17 * MIB)).await,
                StatusCode::PAYLOAD_TOO_LARGE
            );
        }

        #[tokio::test]
        async fn an_attachment_still_takes_up_to_64_mib() {
            let f = fixture().await;
            let response = f
                .raw("PUT", &f.attachment_uri("scan.pdf"), vec![7u8; 60 * MIB])
                .await;
            assert_eq!(response.status(), StatusCode::OK);
            let response = f
                .raw("PUT", &f.attachment_uri("bigger.pdf"), vec![7u8; 65 * MIB])
                .await;
            assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
        }

        /// H1: an attachment is whatever someone uploaded. Served from the
        /// server's own origin, an HTML or SVG file would run as the web app.
        #[tokio::test]
        async fn an_attachment_is_served_unsniffed_and_sandboxed() {
            let f = fixture().await;
            let uri = f.attachment_uri("page.html");
            let put = f
                .raw("PUT", &uri, b"<script>alert(1)</script>".to_vec())
                .await;
            assert_eq!(put.status(), StatusCode::OK);
            for range in [None, Some("bytes=0-3")] {
                let mut request = axum::http::Request::builder()
                    .uri(&uri)
                    .header("authorization", &f.bearer);
                if let Some(range) = range {
                    request = request.header(header::RANGE, range);
                }
                let response = f
                    .app
                    .clone()
                    .oneshot(request.body(axum::body::Body::empty()).unwrap())
                    .await
                    .unwrap();
                assert!(response.status().is_success(), "{range:?}");
                let headers = response.headers();
                assert_eq!(headers["x-content-type-options"], "nosniff", "{range:?}");
                assert_eq!(headers["content-security-policy"], "sandbox", "{range:?}");
            }
        }

        /// E3: reading an attachment is file I/O of up to 64 MiB, and it must
        /// not hold the vault's indexer lock while it happens.
        #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
        async fn a_note_op_does_not_wait_for_an_attachment_read() {
            let f = fixture().await;
            let uri = f.attachment_uri("held-read.bin");
            let put = f.raw("PUT", &uri, b"done".to_vec()).await;
            assert_eq!(put.status(), StatusCode::OK);

            let release = crate::vault::read_gate::hold("held-read.bin");
            let request = get_with_auth(&uri, &f.bearer);
            let app = f.app.clone();
            let download = tokio::spawn(async move { app.oneshot(request).await.unwrap() });
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;

            let created =
                tokio::time::timeout(std::time::Duration::from_secs(5), f.create("B.md", "b\n"))
                    .await;
            // Let the read finish whatever happened, so a failure is a failure
            // and not a hung test.
            release.send(()).unwrap();
            let response = download.await.unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(
                created.expect("a note op waited for an attachment read"),
                StatusCode::OK
            );
        }

        /// B5: on a case-insensitive disk (macOS, Windows) `a.md` *is*
        /// `A.md`, so "the destination exists" refused every case-only
        /// rename. Linux runs it too; CI's macOS and Windows jobs are where it
        /// fails without the fix.
        #[tokio::test]
        async fn a_note_can_be_renamed_by_case_alone() {
            let f = fixture().await;
            let id = f.note_id("Ideas.md").await;
            let (status, body) = send(
                &f.app,
                post_json_with_auth(
                    &format!("{}/{id}/move", f.notes()),
                    serde_json::json!({"new_path": "ideas.md"}),
                    &f.bearer,
                ),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{body}");
            let names: Vec<String> = std::fs::read_dir(&f.vault_dir)
                .unwrap()
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .filter(|n| n.ends_with(".md"))
                .collect();
            assert_eq!(names, vec!["ideas.md".to_string()]);
        }

        #[tokio::test]
        async fn a_folder_can_be_renamed_by_case_alone() {
            let f = fixture().await;
            f.note_id("Projects/A.md").await;
            let (status, body) = send(
                &f.app,
                post_json_with_auth(
                    &format!("/v1/vaults/{}/folders/rename", f.vault),
                    serde_json::json!({"from": "Projects", "to": "projects"}),
                    &f.bearer,
                ),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{body}");
            assert!(f.vault_dir.join("projects/A.md").exists());
        }
    }

    // ---- the read and report tools (decision 134-4) ----

    /// What a parent is offered once 134-6 is built: 134-4's reads and
    /// `send_message`, in `SPAWN_134`'s order.
    const PARENT_BUILT: [&str; 5] = [
        "list_agents",
        "agent_status",
        "list_children",
        "wait_agent",
        "send_message",
    ];
    const CHILD_BUILT: [&str; 2] = ["report_result", "read_messages"];

    impl GatewayFixture {
        /// A running parent with spawn on, its host C0 by the test-only
        /// hook, and a running child recorded as an approval records one.
        async fn tree(&mut self) -> (String, String) {
            self.verified().await;
            self.state.agent.verify_isolation_for_tests(&self.host_id);
            let parent = self.launch("claude-code", false).await["id"]
                .as_str()
                .unwrap()
                .to_string();
            let (status, body) = self.spawn(&parent, true).await;
            assert_eq!(status, StatusCode::OK, "{body}");
            let child = self
                .state
                .agent
                .spawn_child_for_tests(&parent, "count the tests")
                .id;
            self.initialize(&parent, "storm").await;
            self.initialize(&child, "storm").await;
            (parent, child)
        }

        fn agent(&self, session: &str) -> Actor {
            Actor::Agent {
                session_id: session.into(),
                host_id: self.host_id.clone(),
                user_id: self.owner.clone(),
                write_vault: None,
            }
        }
    }

    fn spawn_134(names: &[String]) -> Vec<&'static str> {
        SPAWN_134
            .iter()
            .copied()
            .filter(|t| names.iter().any(|n| n == t))
            .collect()
    }

    /// Production on every host in this stack: no host reports isolation,
    /// so nothing of 134-4 is listed or callable, by the gateway or by ops,
    /// even with the switch's row written past the route.
    #[tokio::test]
    async fn no_read_or_report_tool_without_c0_and_the_switch() {
        let dir = tempdir::TempDir::new("storm-134-4-dark").unwrap();
        let mut f = gateway_fixture(dir.path()).await;
        f.verified().await;
        let parent = f.launch("claude-code", false).await["id"]
            .as_str()
            .unwrap()
            .to_string();
        let (status, _) = f.spawn(&parent, true).await;
        assert_eq!(status, StatusCode::CONFLICT, "C0 refuses the switch");
        f.state.agent.force_spawn_for_tests(&parent);
        let child = f
            .state
            .agent
            .spawn_child_for_tests(&parent, "count the tests")
            .id;
        for session in [&parent, &child] {
            f.initialize(session, "storm").await;
            let names = f.storm_tools(session).await;
            assert!(names.contains(&"list_vaults".to_string()), "{names:?}");
            assert!(spawn_134(&names).is_empty(), "{names:?}");
            for tool in PARENT_BUILT.iter().chain(&CHILD_BUILT) {
                let args = match *tool {
                    "agent_status" => serde_json::json!({"id": child}),
                    "wait_agent" => serde_json::json!({"ids": [child]}),
                    "send_message" => serde_json::json!({"id": child, "text": "hi"}),
                    "report_result" => serde_json::json!({"outcome": "succeeded", "summary": "s"}),
                    _ => serde_json::json!({}),
                };
                assert_eq!(
                    error_code(&f.tool(session, tool, args).await).as_deref(),
                    Some("tool_not_allowed"),
                    "{tool} from {session}"
                );
            }
        }
        // Around the gateway, ops refuses as well.
        let (p, c) = (f.agent(&parent), f.agent(&child));
        assert_eq!(
            crate::ops::list_agents(&f.state, &p).await.unwrap_err().0,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            crate::ops::agent_status(&f.state, &p, &child)
                .await
                .unwrap_err()
                .0,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            crate::ops::list_children(&f.state, &p).await.unwrap_err().0,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            crate::ops::wait_agent(&f.state, &p, std::slice::from_ref(&child), Some(1))
                .await
                .unwrap_err()
                .0,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            crate::ops::report_result(&f.state, &c, "succeeded", "s", None)
                .await
                .unwrap_err()
                .0,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            crate::ops::send_message(&f.state, &p, &child, "hi")
                .await
                .unwrap_err()
                .0,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            crate::ops::read_messages(&f.state, &c, None, None)
                .await
                .unwrap_err()
                .0,
            StatusCode::FORBIDDEN
        );
        assert!(f.state.agent.inbox_of(&child).unwrap().is_empty());
        assert!(f.state.agent.requests_of_for_tests(&parent).is_empty());
        // The task is not given out either.
        assert_eq!(
            crate::ops::session_context(&f.state, &c)
                .await
                .unwrap_err()
                .0,
            StatusCode::NOT_FOUND
        );
        assert!(f.state.agent.result_of(&child).unwrap().is_none());

        // With isolation but the switch off: still nothing for the parent.
        f.state.agent.verify_isolation_for_tests(&f.host_id);
        f.spawn(&parent, false).await;
        assert!(spawn_134(&f.storm_tools(&parent).await).is_empty());
    }

    #[tokio::test]
    async fn a_parent_with_the_switch_and_c0_gets_the_reads_and_nothing_else() {
        let dir = tempdir::TempDir::new("storm-134-4-parent").unwrap();
        let mut f = gateway_fixture(dir.path()).await;
        let (parent, child) = f.tree().await;
        assert_eq!(
            spawn_134(&f.storm_tools(&parent).await),
            PARENT_BUILT.to_vec()
        );
        assert_eq!(
            spawn_134(&f.storm_tools(&child).await),
            CHILD_BUILT.to_vec()
        );

        let listed = structured(&f.tool(&parent, "list_agents", serde_json::json!({})).await);
        assert_eq!(
            listed["agents"],
            serde_json::json!([{"provider": "claude-code"}])
        );
        assert_eq!(
            listed["limits"],
            serde_json::json!({"depth": 2, "children": 3, "tree": 6, "pending": 2})
        );
        assert_eq!(listed["start_refused"], serde_json::Value::Null);
        assert_eq!(listed["wait_cap_secs"], 30);

        let children = structured(
            &f.tool(&parent, "list_children", serde_json::json!({}))
                .await,
        );
        let only = &children["agents"][0];
        assert_eq!(children["agents"].as_array().unwrap().len(), 1);
        assert_eq!(only["id"], child);
        assert_eq!(only["parent_session"], parent);
        assert_eq!(only["root_session"], parent);
        assert_eq!(only["depth"], 1);
        assert_eq!(only["settled"], false);
        assert!(only.get("task").is_none(), "{only}");

        let status = structured(
            &f.tool(&parent, "agent_status", serde_json::json!({"id": child}))
                .await,
        );
        assert_eq!(status["task"], "count the tests");
        assert_eq!(status["result"], serde_json::Value::Null);

        // Its own session, and an unknown one, are out of scope.
        for id in [parent.as_str(), "ags_NOPE"] {
            let err = tool_error(
                &f.tool(&parent, "agent_status", serde_json::json!({"id": id}))
                    .await,
            );
            assert!(err.contains("no such agent"), "{err}");
        }

        let started = std::time::Instant::now();
        let waited = structured(
            &f.tool(
                &parent,
                "wait_agent",
                serde_json::json!({"ids": [child], "timeout": 1}),
            )
            .await,
        );
        assert_eq!(waited["timed_out"], true);
        assert!(started.elapsed() >= std::time::Duration::from_secs(1));

        // A child's own report reaches the parent's next wait at once.
        let reported = structured(
            &f.tool(
                &child,
                "report_result",
                serde_json::json!({"outcome": "succeeded", "summary": "41 tests", "details": "all green"}),
            )
            .await,
        );
        assert_eq!(reported["outcome"], "succeeded");
        let waited = structured(
            &f.tool(
                &parent,
                "wait_agent",
                serde_json::json!({"ids": [child], "timeout": 20}),
            )
            .await,
        );
        assert_eq!(waited["timed_out"], false);
        assert_eq!(waited["agents"][0]["settled"], true);
        assert_eq!(waited["agents"][0]["result"]["summary"], "41 tests");
        assert_eq!(waited["agents"][0]["result"]["details"], "all green");

        // A downgrade takes them away on the next call.
        f.state.agent.lose_isolation_for_tests(&f.host_id);
        assert!(spawn_134(&f.storm_tools(&parent).await).is_empty());
        assert!(spawn_134(&f.storm_tools(&child).await).is_empty());
        assert_eq!(
            error_code(
                &f.tool(&parent, "list_children", serde_json::json!({}))
                    .await
            )
            .as_deref(),
            Some("tool_not_allowed")
        );
    }

    #[tokio::test]
    async fn another_trees_sessions_are_never_seen() {
        let dir = tempdir::TempDir::new("storm-134-4-trees").unwrap();
        let mut f = gateway_fixture(dir.path()).await;
        // Two trees on one host. They share its one workspace, which WS-9
        // refuses at the route, so both switches are written past it.
        f.verified().await;
        f.state.agent.verify_isolation_for_tests(&f.host_id);
        let mut roots = Vec::new();
        for _ in 0..2 {
            let id = f.launch("claude-code", false).await["id"]
                .as_str()
                .unwrap()
                .to_string();
            roots.push(id);
        }
        for id in &roots {
            f.state.agent.force_spawn_for_tests(id);
            f.initialize(id, "storm").await;
        }
        let (parent, other) = (roots[0].clone(), roots[1].clone());
        let child = f.state.agent.spawn_child_for_tests(&parent, "here").id;
        f.initialize(&child, "storm").await;
        let stranger = f.state.agent.spawn_child_for_tests(&other, "elsewhere").id;
        f.initialize(&stranger, "storm").await;

        for (caller, target) in [(&parent, &stranger), (&parent, &other), (&other, &child)] {
            let err = tool_error(
                &f.tool(caller, "agent_status", serde_json::json!({"id": target}))
                    .await,
            );
            assert!(
                err.contains("no such agent"),
                "{caller} saw {target}: {err}"
            );
            let err = tool_error(
                &f.tool(caller, "wait_agent", serde_json::json!({"ids": [target]}))
                    .await,
            );
            assert!(
                err.contains("no such agent"),
                "{caller} waited on {target}: {err}"
            );
        }
        let children = structured(&f.tool(&other, "list_children", serde_json::json!({})).await);
        assert_eq!(children["agents"].as_array().unwrap().len(), 1);
        assert_eq!(children["agents"][0]["id"], stranger);

        // A child sees no other session at all: it has no parent tools.
        assert_eq!(
            error_code(
                &f.tool(&child, "agent_status", serde_json::json!({"id": parent}))
                    .await
            )
            .as_deref(),
            Some("tool_not_allowed")
        );
        // Its report is its own: the stranger's record is untouched.
        f.tool(
            &child,
            "report_result",
            serde_json::json!({"outcome": "partial", "summary": "half"}),
        )
        .await;
        assert!(f.state.agent.result_of(&stranger).unwrap().is_none());
        assert!(f.state.agent.result_of(&parent).unwrap().is_none());
    }

    #[tokio::test]
    async fn a_spawned_session_reads_its_task_and_reports_with_a_fixed_outcome() {
        let dir = tempdir::TempDir::new("storm-134-4-child").unwrap();
        let mut f = gateway_fixture(dir.path()).await;
        let (parent, child) = f.tree().await;

        let context = structured(
            &f.tool(&child, "session_context", serde_json::json!({}))
                .await,
        );
        assert_eq!(context, serde_json::json!({"task": "count the tests"}));
        // A top-level session's answer has no task and is as before.
        assert_eq!(
            crate::ops::session_context(&f.state, &f.agent(&parent))
                .await
                .unwrap_err()
                .0,
            StatusCode::NOT_FOUND
        );

        let err = tool_error(
            &f.tool(
                &child,
                "report_result",
                serde_json::json!({"outcome": "done", "summary": "s"}),
            )
            .await,
        );
        assert!(err.contains("outcome is one of"), "{err}");
        assert!(f.state.agent.result_of(&child).unwrap().is_none());
        // The top-level session cannot report.
        assert_eq!(
            error_code(
                &f.tool(
                    &parent,
                    "report_result",
                    serde_json::json!({"outcome": "failed", "summary": "s"})
                )
                .await
            )
            .as_deref(),
            Some("tool_not_allowed")
        );

        for outcome in ["blocked", "failed"] {
            let r = structured(
                &f.tool(
                    &child,
                    "report_result",
                    serde_json::json!({"outcome": outcome, "summary": "s"}),
                )
                .await,
            );
            assert_eq!(r["outcome"], outcome);
        }
        assert_eq!(
            f.state.agent.result_of(&child).unwrap().unwrap().outcome,
            "failed",
            "a later report replaces an earlier one"
        );
        let events: Vec<serde_json::Value> = f
            .spawn_events()
            .await
            .into_iter()
            .filter(|(kind, _)| kind == crate::ops::EVENT_SPAWN_RESULT)
            .map(|(_, d)| d)
            .collect();
        assert_eq!(events.len(), 2);
        assert_eq!(
            events[1],
            serde_json::json!({"session_id": child, "host_id": f.host_id, "outcome": "failed"})
        );
        assert!(!events[1].to_string().contains("\"s\""), "metadata only");
    }

    #[tokio::test]
    async fn session_views_carry_lineage() {
        let dir = tempdir::TempDir::new("storm-134-4-views").unwrap();
        let mut f = gateway_fixture(dir.path()).await;
        // A launch's answer says top-level.
        let launched = f.launch("claude-code", false).await;
        assert_eq!(launched["depth"], 0);
        assert_eq!(launched["root_session"], launched["id"]);
        assert_eq!(launched["parent_session"], serde_json::Value::Null);
        f.report(launched["id"].as_str().unwrap(), "completed")
            .await;
        let (parent, child) = f.tree().await;
        let view = |id: String| {
            let f = &f;
            async move {
                let (status, view) = send(
                    &f.app,
                    get_with_auth(&format!("/v1/agent/sessions/{id}"), &f.owner_bearer),
                )
                .await;
                assert_eq!(status, StatusCode::OK);
                view
            }
        };
        let p = view(parent.clone()).await;
        assert_eq!(p["parent_session"], serde_json::Value::Null);
        assert_eq!(p["root_session"], parent);
        assert_eq!(p["depth"], 0);
        let c = view(child.clone()).await;
        assert_eq!(c["parent_session"], parent);
        assert_eq!(c["root_session"], parent);
        assert_eq!(c["depth"], 1);
        let (_, all) = send(&f.app, get_with_auth("/v1/agent/sessions", &f.owner_bearer)).await;
        let listed = all
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["id"] == child)
            .unwrap()
            .clone();
        assert_eq!(listed["depth"], 1);
    }

    // ---- the inbox (decision 134-6) ----

    /// No command meant for a terminal went down the host's link (C9,
    /// G-134-3).
    fn no_terminal_input(sent: &[serde_json::Value]) {
        for m in sent {
            assert!(
                !["terminal.input", "terminal.resize", "terminal.image"]
                    .contains(&m["type"].as_str().unwrap_or("")),
                "{m}"
            );
        }
    }

    /// 134j's states through the real tools and routes: pending until a
    /// device approves, then delivered once to its child, read at least once
    /// and acknowledged by the cursor; a denial never arrives.
    #[tokio::test]
    async fn a_message_is_approved_through_the_route_and_read_by_its_child() {
        let dir = tempdir::TempDir::new("storm-134-6-message").unwrap();
        let mut f = gateway_fixture(dir.path()).await;
        let (parent, child) = f.tree().await;
        f.sent();
        let sent = structured(
            &f.tool(
                &parent,
                "send_message",
                serde_json::json!({"id": child, "text": "check the cap"}),
            )
            .await,
        );
        assert_eq!(sent["state"], "pending", "{sent}");
        let id = sent["request_id"].as_str().unwrap().to_string();

        let read = structured(&f.tool(&child, "read_messages", serde_json::json!({})).await);
        assert_eq!(read, serde_json::json!({"messages": [], "timed_out": true}));
        let status = structured(
            &f.tool(&parent, "agent_status", serde_json::json!({"id": child}))
                .await,
        );
        assert_eq!(status["messages"][0]["request_id"], id);
        assert_eq!(status["messages"][0]["state"], "pending_approval");

        let (code, body) = f.decide(&id, "approve", &f.owner_bearer).await;
        assert_eq!(code, StatusCode::OK, "{body}");
        assert_eq!(
            (&body["outcome"], &body["target"]),
            (&serde_json::json!("delivered"), &serde_json::json!(child))
        );
        let (code, body) = f.decide(&id, "approve", &f.owner_bearer).await;
        assert_eq!(
            (code, body),
            (
                StatusCode::CONFLICT,
                serde_json::json!({"error": "already_decided"})
            )
        );

        let read = structured(
            &f.tool(&child, "read_messages", serde_json::json!({"wait": 5}))
                .await,
        );
        assert_eq!(read["timed_out"], false);
        let message = &read["messages"][0];
        assert_eq!(
            (&message["sender"], &message["text"]),
            (
                &serde_json::json!(parent),
                &serde_json::json!("check the cap")
            )
        );
        let seq = message["seq"].as_i64().unwrap();
        let again = structured(&f.tool(&child, "read_messages", serde_json::json!({})).await);
        assert_eq!(again["messages"][0]["seq"], seq, "at least once");
        let acked = structured(
            &f.tool(&child, "read_messages", serde_json::json!({"after": seq}))
                .await,
        );
        assert_eq!(acked["messages"], serde_json::json!([]));

        let denied = structured(
            &f.tool(
                &parent,
                "send_message",
                serde_json::json!({"id": child, "text": "never mind"}),
            )
            .await,
        );
        let denied = denied["request_id"].as_str().unwrap().to_string();
        let (code, body) = f.decide(&denied, "deny", &f.owner_bearer).await;
        assert_eq!(
            (code, &body["outcome"]),
            (StatusCode::OK, &serde_json::json!("denied"))
        );
        let read = structured(&f.tool(&child, "read_messages", serde_json::json!({})).await);
        assert_eq!(read["messages"], serde_json::json!([]));
        let status = structured(
            &f.tool(&parent, "agent_status", serde_json::json!({"id": child}))
                .await,
        );
        assert_eq!(
            status["messages"]
                .as_array()
                .unwrap()
                .iter()
                .map(|m| m["state"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["acknowledged", "denied"]
        );

        no_terminal_input(&f.sent());
        let events: Vec<(String, Option<String>)> = f
            .request_events()
            .await
            .into_iter()
            .map(|(kind, d)| (kind, d["outcome"].as_str().map(Into::into)))
            .collect();
        assert_eq!(
            events,
            [
                ("spawn_request".to_string(), None),
                (
                    "spawn_request_decided".to_string(),
                    Some("delivered".into())
                ),
                ("spawn_request".to_string(), None),
                ("spawn_request_decided".to_string(), Some("denied".into())),
            ]
        );
        assert!(
            !f.spawn_events()
                .await
                .iter()
                .any(|(_, d)| d.to_string().contains("check the cap")),
            "metadata only"
        );
    }

    /// A2 over the gateway: only the parent's own descendants, never itself,
    /// an unknown id or anyone from a child; a cap refuses with its name.
    #[tokio::test]
    async fn a_message_reaches_only_the_callers_descendants() {
        let dir = tempdir::TempDir::new("storm-134-6-scope").unwrap();
        let mut f = gateway_fixture(dir.path()).await;
        let (parent, child) = f.tree().await;
        for id in [parent.as_str(), "ags_NOPE"] {
            let err = tool_error(
                &f.tool(
                    &parent,
                    "send_message",
                    serde_json::json!({"id": id, "text": "hi"}),
                )
                .await,
            );
            assert!(err.contains("no such agent"), "{err}");
        }
        assert_eq!(
            error_code(
                &f.tool(
                    &child,
                    "send_message",
                    serde_json::json!({"id": parent, "text": "hi"})
                )
                .await
            )
            .as_deref(),
            Some("tool_not_allowed")
        );
        assert_eq!(
            error_code(
                &f.tool(&parent, "read_messages", serde_json::json!({}))
                    .await
            )
            .as_deref(),
            Some("tool_not_allowed"),
            "a top-level session has no inbox tool"
        );
        let err = tool_error(
            &f.tool(
                &parent,
                "send_message",
                serde_json::json!({"id": child, "text": "  "}),
            )
            .await,
        );
        assert!(err.contains("needs text"), "{err}");
        for _ in 0..2 {
            let ok = structured(
                &f.tool(
                    &parent,
                    "send_message",
                    serde_json::json!({"id": child, "text": "hi"}),
                )
                .await,
            );
            assert_eq!(ok["state"], "pending");
        }
        let refused = structured(
            &f.tool(
                &parent,
                "send_message",
                serde_json::json!({"id": child, "text": "hi"}),
            )
            .await,
        );
        assert_eq!(
            refused,
            serde_json::json!({"request_id": null, "state": "refused", "reason": "limit", "cap": "pending"})
        );
        assert!(
            f.state
                .agent
                .messages_from(&parent, &parent)
                .unwrap()
                .is_empty()
        );
    }

    /// LT-13 over the real routes: the parent's own end puts Storm's notice
    /// in its child's inbox; 30 s later the child is sent `end`, recorded
    /// `parent_ended` and audited; no terminal hears any of it.
    #[tokio::test]
    async fn a_parent_ending_by_itself_gives_its_child_the_notice_then_stops_it() {
        use crate::agent::store::spawn::{NOTICE_GRACE_SECS, PARENT_ENDED_NOTICE};
        let dir = tempdir::TempDir::new("storm-134-6-notice").unwrap();
        let mut f = gateway_fixture(dir.path()).await;
        let (parent, child) = f.tree().await;
        f.sent();
        f.report(&parent, "completed").await;
        let read = structured(&f.tool(&child, "read_messages", serde_json::json!({})).await);
        assert_eq!(read["messages"][0]["sender"], "storm");
        assert_eq!(read["messages"][0]["text"], PARENT_ENDED_NOTICE);
        // Its report still goes on its record.
        f.tool(
            &child,
            "report_result",
            serde_json::json!({"outcome": "partial", "summary": "half done"}),
        )
        .await;

        let at = |secs: i64| crate::index::rfc3339_minus_secs(&crate::index::now_rfc3339(), -secs);
        crate::ops::stop_notified_children(&f.state, &at(NOTICE_GRACE_SECS - 1)).await;
        assert!(f.sent().is_empty(), "nothing before the grace is over");
        crate::ops::stop_notified_children(&f.state, &at(NOTICE_GRACE_SECS + 1)).await;
        let sent = f.sent();
        no_terminal_input(&sent);
        assert_eq!(
            sent.iter()
                .map(|m| (m["type"].as_str().unwrap(), m["session"].as_str().unwrap()))
                .collect::<Vec<_>>(),
            [("end", child.as_str())]
        );
        f.report(&child, "stopped").await;
        let (status, view) = send(
            &f.app,
            get_with_auth(&format!("/v1/agent/sessions/{child}"), &f.owner_bearer),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            (&view["status"], &view["end_reason"]),
            (
                &serde_json::json!("stopped"),
                &serde_json::json!("parent_ended")
            )
        );
        assert_eq!(
            f.state.agent.result_of(&child).unwrap().unwrap().summary,
            "half done"
        );
        let stops: Vec<serde_json::Value> = f
            .spawn_events()
            .await
            .into_iter()
            .filter(|(kind, _)| kind == crate::ops::EVENT_SPAWN_PARENT_ENDED)
            .map(|(_, d)| d)
            .collect();
        assert_eq!(
            stops,
            [
                serde_json::json!({"session_id": child, "host_id": f.host_id,
                "parent_session": parent, "end": "sent"})
            ]
        );
    }
}
