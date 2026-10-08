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
    /// Bootstrap pairing nonce (plaintext), if one was created at boot when
    /// the user table was empty. Needed to reconstruct the QR payload for
    /// the console log. Not read after boot — the field exists so a future
    /// CLI or settings surface can regenerate the QR without restarting.
    #[allow(dead_code)]
    pub bootstrap_nonce: Option<String>,
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
        .route("/v1/runtime/inventory", post(runtime_inventory))
        .route(
            "/v1/runtime/sessions/{id}/terminal/output",
            post(runtime_output),
        )
        .route("/v1/runtime/sessions/{id}/status", post(runtime_status))
        // The MCP Gateway (decision 81e): a session's bridge traffic, carried
        // by the host's existing token — no new credential (G-D3).
        .route(
            "/v1/runtime/sessions/{id}/mcp/{connection}",
            post(runtime_mcp),
        )
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
        // MCP keys (A14). **Session tier**: minting a key costs a real
        // sign-in on a paired device, which is also what makes "a key cannot
        // mint a key" true without a special case — a key never reaches here.
        .route("/v1/keys", get(list_keys).post(create_key))
        .route("/v1/keys/{id}", delete(revoke_key))
        .route("/v1/recents", get(recents))
        .route("/v1/vaults/{vault}/tree", get(tree))
        .route("/v1/vaults/{vault}/sync", get(sync))
        .route("/v1/vaults/{vault}/notes", post(create_note))
        .route("/v1/vaults/{vault}/notes/{id}", get(get_note))
        .route("/v1/vaults/{vault}/notes/{id}", put(put_note))
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
                .delete(delete_attachment),
        )
        .layer(axum::extract::DefaultBodyLimit::max(MAX_ATTACHMENT_BYTES))
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
        .route(
            "/v1/agent/sessions",
            get(agent_list_sessions).post(agent_launch),
        )
        .route(
            "/v1/agent/sessions/{id}",
            get(agent_get_session).delete(agent_dismiss_session),
        )
        .route("/v1/agent/sessions/{id}/end", post(agent_end_session))
        .route("/v1/agent/sessions/{id}/writes", get(agent_session_writes))
        .route(
            "/v1/agent/sessions/{id}/terminal/stream",
            get(agent_terminal_stream),
        )
        .route("/v1/agent/sessions/{id}/terminal/input", post(agent_input))
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
        .map(|_| StatusCode::CREATED)
        .map_err(|e| ApiError(StatusCode::CONFLICT, e.to_string()))
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

async fn agent_dismiss_session(
    State(state): State<Shared>,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    crate::ops::dismiss_session(&state, &id).await?;
    Ok(StatusCode::NO_CONTENT)
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

#[derive(Deserialize)]
struct ResizeRequest {
    cols: u16,
    rows: u16,
    /// Whether this client just took focus. The PTY follows the most recently
    /// active client either way (freeze §11.4); the flag is the client saying
    /// "that is me now" without typing.
    #[serde(default)]
    #[allow(dead_code)]
    focus: bool,
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
}

/// GET …/terminal/stream?offset=N — the terminal as SSE (freeze §11.2).
///
/// Every stream starts with a `status` event. Output is `event: output` with
/// `id:` the end offset and base64 `data:`; a range no longer retained is an
/// explicit `event: gap`. Resume is by offset, never `Last-Event-ID`, so it
/// is exact and crosses the relay (SRP §5.3). The stream ends once the session
/// has ended and everything retained has been sent.
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

/// A device that has proven it is paired with this server.
///
/// Set as a request extension by `require_auth` on device-tier and session-tier
/// routes when the credential is a `StormDevice` header.
#[derive(Clone)]
pub struct DeviceAuth {
    #[allow(dead_code)]
    pub device: crate::auth::devices::Device,
}

/// A caller holding a valid MCP key (A14).
///
/// Set on `Mcp`-tier routes when the credential is `Bearer stk_…`. The
/// authorization-facing identity is the `Actor::Key` inserted beside it; this
/// carries the record itself, for handlers and audit rows that want the key's
/// name or owner without a second lookup.
#[derive(Clone)]
pub struct KeyAuth {
    #[allow(dead_code)] // Read by audit/logging work, not by the boundary.
    pub authed: crate::auth::keys::AuthenticatedKey,
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
        request.extensions_mut().insert(DeviceAuth { device });
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
            request.extensions_mut().insert(KeyAuth { authed });
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
) -> ApiResult<Json<serde_json::Value>> {
    let mut vaults = state.vaults.write().await;
    let entry = vaults
        .registry
        .create(&body.name, &crate::index::now_rfc3339())
        .map_err(|e| bad_request(e.to_string()))?;
    vaults.registry.save(&state.state_dir)?;

    let vault = Vault::new(vaults.registry.path_of(&entry))?;
    let db = Db::open(&state.state_dir.join(&entry.id).join("index.db"), &entry.id)?;
    vaults.open.insert(
        entry.id.clone(),
        Arc::new(VaultHandle {
            indexer: Mutex::new(Indexer::new(vault, db)),
        }),
    );

    Ok(Json(serde_json::json!({
        "id": entry.id, "name": entry.name, "dir": entry.dir,
    })))
}

async fn rename_vault(
    State(state): State<Shared>,
    Path(vault): Path<String>,
    Json(body): Json<VaultNameBody>,
) -> ApiResult<Json<serde_json::Value>> {
    let mut vaults = state.vaults.write().await;
    let entry = vaults
        .registry
        .rename(&vault, &body.name)
        .map_err(|e| bad_request(e.to_string()))?;
    // Nothing to do to the open handle: it holds only the index, and the name
    // lives in the registry, which is what every reader consults.
    vaults.registry.save(&state.state_dir)?;

    Ok(Json(
        serde_json::json!({ "id": entry.id, "name": entry.name }),
    ))
}

/// Unregisters a vault. **Never deletes files.**
async fn remove_vault(
    State(state): State<Shared>,
    Path(vault): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let mut vaults = state.vaults.write().await;
    let entry = vaults
        .registry
        .remove(&vault)
        .map_err(|e| not_found(e.to_string()))?;
    vaults.registry.save(&state.state_dir)?;
    vaults.open.remove(&vault);

    Ok(Json(serde_json::json!({
        "removed": entry.id,
        "name": entry.name,
        // Said explicitly so no caller has to guess.
        "files_kept_at": vaults.registry.root.join(&entry.dir).display().to_string(),
    })))
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

#[derive(Serialize)]
struct TreeResponse {
    notes: Vec<crate::db::NoteRow>,
    folders: Vec<String>,
    seq: i64,
}

async fn tree(
    State(state): State<Shared>,
    Extension(actor): Extension<Actor>,
    Path(vault): Path<String>,
) -> ApiResult<Json<TreeResponse>> {
    let handle = vault_of(&state, &actor, Access::Read, &vault).await?;
    let ix = handle.indexer.lock().await;
    let notes = ix.db.list_notes()?;
    // Derived from note paths, union the ones created explicitly — only the
    // union makes an empty folder visible.
    let folders = ix.all_folders()?;
    let seq = ix.db.latest_seq()?;
    Ok(Json(TreeResponse {
        notes,
        folders,
        seq,
    }))
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

#[derive(Serialize)]
struct SyncResponse {
    changes: Vec<Change>,
    seq: i64,
}

async fn sync(
    State(state): State<Shared>,
    Extension(actor): Extension<Actor>,
    Path(vault): Path<String>,
    Query(q): Query<SyncQuery>,
) -> ApiResult<Json<SyncResponse>> {
    let handle = vault_of(&state, &actor, Access::Read, &vault).await?;
    let ix = handle.indexer.lock().await;
    let changes = ix.db.changes_since(q.since, q.limit.clamp(1, 5000))?;
    let seq = ix.db.latest_seq()?;
    Ok(Json(SyncResponse { changes, seq }))
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

/// Records that a note was opened, feeding the cross-vault recents list.
///
/// Fire-and-forget from the client and deliberately outside the change log:
/// opening a note is not an edit, must not bump `version`, and must not wake
/// every other device.
async fn mark_opened(
    State(state): State<Shared>,
    Extension(actor): Extension<Actor>,
    Path((vault, id)): Path<(String, String)>,
) -> ApiResult<Json<serde_json::Value>> {
    let handle = vault_of(&state, &actor, Access::Write, &vault).await?;
    let ix = handle.indexer.lock().await;
    if ix.db.get_note(&id)?.is_none() {
        return Err(not_found("no such note"));
    }
    ix.db.touch_note_access(&id, &crate::index::now_rfc3339())?;
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
    let handle = vault_of(&state, &actor, Access::Write, &vault).await?;
    let mut ix = handle.indexer.lock().await;
    let result = ix
        .move_note(&id, &body.new_path)
        .map_err(|e| bad_request(e.to_string()))?;
    crate::ops::broadcast_latest(&state, &ix, result.seq);
    Ok(Json(result))
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
    let handle = vault_of(&state, &actor, Access::Write, &vault).await?;
    let mut ix = handle.indexer.lock().await;
    ix.create_folder(&body.path)
        .map_err(|e| bad_request(e.to_string()))?;
    Ok(Json(serde_json::json!({ "path": body.path })))
}

async fn delete_folder(
    State(state): State<Shared>,
    Extension(actor): Extension<Actor>,
    Path((vault, path)): Path<(String, String)>,
) -> ApiResult<Json<serde_json::Value>> {
    let handle = vault_of(&state, &actor, Access::Write, &vault).await?;
    let mut ix = handle.indexer.lock().await;
    // Refused rather than recursive: deleting someone's notes because they
    // tapped "delete folder" is not something to do quietly.
    ix.delete_folder(&path)
        .map_err(|e| conflict(e.to_string()))?;
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
    let handle = vault_of(&state, &actor, Access::Write, &vault).await?;
    let mut ix = handle.indexer.lock().await;
    let seqs = ix
        .rename_folder(&body.from, &body.to)
        .map_err(|e| bad_request(e.to_string()))?;
    // One `moved` per contained note, so every client follows the rename
    // rather than discovering it at the next full tree fetch.
    for seq in &seqs {
        crate::ops::broadcast_latest(&state, &ix, *seq);
    }
    Ok(Json(serde_json::json!({
        "from": body.from,
        "to": body.to,
        "moved": seqs.len(),
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
    let handle = vault_of(&state, &actor, Access::Read, &vault).await?;
    let ix = handle.indexer.lock().await;
    Ok(Json(
        serde_json::json!({ "notes": ix.db.notes_with_tag(&tag)? }),
    ))
}

// ---- attachments -------------------------------------------------------

async fn list_attachments(
    State(state): State<Shared>,
    Extension(actor): Extension<Actor>,
    Path(vault): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let handle = vault_of(&state, &actor, Access::Read, &vault).await?;
    let ix = handle.indexer.lock().await;
    Ok(Json(
        serde_json::json!({ "attachments": ix.db.list_attachments()? }),
    ))
}

async fn get_attachment(
    State(state): State<Shared>,
    Extension(actor): Extension<Actor>,
    Path((vault, path)): Path<(String, String)>,
) -> ApiResult<Response> {
    let handle = vault_of(&state, &actor, Access::Read, &vault).await?;
    let ix = handle.indexer.lock().await;
    let bytes = ix.attachment(&path).map_err(|e| not_found(e.to_string()))?;

    Ok(([(header::CONTENT_TYPE, content_type_for(&path))], bytes).into_response())
}

async fn put_attachment(
    State(state): State<Shared>,
    Extension(actor): Extension<Actor>,
    Path((vault, path)): Path<(String, String)>,
    body: axum::body::Bytes,
) -> ApiResult<Json<serde_json::Value>> {
    if body.is_empty() {
        return Err(bad_request("empty upload"));
    }
    let handle = vault_of(&state, &actor, Access::Write, &vault).await?;
    let mut ix = handle.indexer.lock().await;
    ix.put_attachment(&path, &body)
        .map_err(|e| bad_request(e.to_string()))?;
    Ok(Json(
        serde_json::json!({ "path": path, "size": body.len() }),
    ))
}

async fn delete_attachment(
    State(state): State<Shared>,
    Extension(actor): Extension<Actor>,
    Path((vault, path)): Path<(String, String)>,
) -> ApiResult<Json<serde_json::Value>> {
    let handle = vault_of(&state, &actor, Access::Write, &vault).await?;
    let mut ix = handle.indexer.lock().await;
    ix.delete_attachment(&path)
        .map_err(|e| not_found(e.to_string()))?;
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
    tokio::spawn(push_changes(SseSink(tx), rx));

    (
        [
            (header::CONTENT_TYPE, "text/event-stream"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        // No keep-alive comments and no inactivity timeout: a quiet vault is a
        // normal vault, and a feed that hung up on silence would be
        // indistinguishable from a broken one.
        axum::body::Body::from_stream(tokio_stream::wrappers::ReceiverStream::new(body_rx)),
    )
        .into_response()
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
        test_router_full(dir, policy, crate::auth::ratelimit::LoginLimiter::new())
    }

    /// As [`test_router_with_state`], but with the login limiter's numbers
    /// chosen by the caller — the rate-limit tests want a burst they can
    /// exhaust in two requests rather than thirty, because every attempt that
    /// reaches the handler pays a real Argon2id verify.
    pub(crate) fn test_router_with_limiter(
        dir: &FsPath,
        limiter: crate::auth::ratelimit::LoginLimiter,
    ) -> (Router, Arc<crate::auth::ServerIdentity>, Shared) {
        test_router_full(dir, Arc::new(crate::auth::authz::StormPolicy), limiter)
    }

    fn test_router_full(
        dir: &FsPath,
        policy: Arc<dyn crate::auth::authz::VaultPolicy>,
        login_limiter: crate::auth::ratelimit::LoginLimiter,
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
            bootstrap_nonce: None,
            listen_addr: "http://127.0.0.1:8080".into(),
            vault_policy: policy,
            hasher: crate::auth::Hasher::new(),
            login_limiter,
            host_limiter: crate::auth::ratelimit::LoginLimiter::new(),
            agent,
            gateway,
        });
        (
            router(
                state.clone(),
                crate::mcp::McpOptions {
                    allowed_hosts: vec![],
                },
            ),
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
            body.as_object().unwrap().len(),
            8,
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
    async fn an_http_upstream_needs_the_test_only_flag_and_a_disabled_one_is_not_probed() {
        let dir = tempdir::TempDir::new("storm-integrations-http").unwrap();
        let (app, _, state) = test_router_with_state(dir.path());
        let (_, owner) = owner_bearer(&state).await;
        let (status, _) = send(
            &app,
            post_json_with_auth(
                CONNECTIONS,
                serde_json::json!({"display_name": "Plain", "url": "http://127.0.0.1:9/mcp", "auth_kind": "none"}),
                &owner,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "http must need the flag");

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
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (_, note) = f.get(&format!("/v1/vaults/{work}/notes/{agent_id}")).await;
        assert_eq!(note["agent_write"]["session_name"], name.as_str());
        assert_eq!(note["agent_write"]["session_dismissed"], true);
        assert_eq!(note["agent_write"]["kind"], "created");
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
        use axum::response::IntoResponse;
        let mock = Arc::new(MockAs::default());
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
            async move {
                Json(
                    serde_json::json!({"resource": format!("{b}/mcp"), "authorization_servers": [b]}),
                )
            }
        };
        let b = base.clone();
        let asm = move || {
            let b = b.clone();
            async move {
                Json(serde_json::json!({
                    "issuer": b,
                    "authorization_endpoint": format!("{b}/authorize"),
                    "token_endpoint": format!("{b}/token"),
                    "registration_endpoint": format!("{b}/register"),
                    "revocation_endpoint": format!("{b}/revoke"),
                    "response_types_supported": ["code"],
                    "code_challenge_methods_supported": ["S256"],
                    "grant_types_supported": ["authorization_code", "refresh_token"],
                }))
            }
        };
        let m = mock.clone();
        let register = move |Json(body): Json<serde_json::Value>| {
            let m = m.clone();
            async move {
                m.registrations.lock().unwrap().push(body.clone());
                (
                    StatusCode::CREATED,
                    Json(
                        serde_json::json!({"client_id": "dcr-client", "redirect_uris": body["redirect_uris"]}),
                    ),
                )
            }
        };
        let m = mock.clone();
        let token =
            move |axum::extract::Form(form): axum::extract::Form<HashMap<String, String>>| {
                let m = m.clone();
                async move {
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
}
