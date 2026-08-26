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
    /// Whether `/mcp` answers. Mirrors `Registry::mcp_enabled`, which is the
    /// persisted copy; this one exists so the gate can read it without taking
    /// the vault lock on every request.
    pub mcp_enabled: std::sync::atomic::AtomicBool,
    /// Whether MCP may change the vault. Read per request, so the switch takes
    /// effect on the next call rather than the next restart.
    pub mcp_writable: std::sync::atomic::AtomicBool,
    /// The auth database, held open for request-time use (device lookup,
    /// session authenticate, login, refresh, ws-ticket).
    pub auth_db: Arc<tokio::sync::Mutex<crate::auth::AuthDb>>,
    ///
    /// Whether registration is open (A13), mirrored out of the registry.
    ///
    /// Atomic and read per request for the same reason as the switch above: an
    /// operator who turns registration off and is still handing out accounts
    /// until they restart has not turned it off.
    pub allow_registration: std::sync::atomic::AtomicBool,
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
    /// `AllowAuthenticated`, which is the behaviour the server already had.
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
/// doubles as a probe for which vault ids are real. `AllowAuthenticated`
/// never refuses today; the ordering is here so the answer does not change
/// when a policy that does arrives.
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
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            "you do not have access to this vault".into(),
        ));
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
        .layer(Extension(RequiredTier::None));

    // ---- device tier: `StormDevice <id>:<secret>` -------------------------
    let device_router = Router::new()
        .route("/v1/users", get(list_users))
        .route("/v1/users", post(register_user))
        .route("/v1/auth/registration", get(registration_state))
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
        .route("/v1/config/registration", put(put_registration))
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

/// GET /v1/users — list all users (device tier).
async fn list_users(State(state): State<Shared>) -> ApiResult<Json<Vec<crate::auth::users::User>>> {
    let auth_db = state.auth_db.lock().await;
    let users = auth_db
        .list_users()
        .map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(users))
}

/// GET /v1/auth/registration — is registration open?
///
/// Device tier, because the login screen already holds a device credential by
/// the time it needs to ask, and whether accounts can be created is policy
/// rather than identity — which is why it is not a field on `GET /v1/server`,
/// whose field set is asserted exactly and whose job is the identity
/// handshake.
///
/// **This is UX, not a gate.** `POST /v1/users` enforces the switch itself; a
/// client that lies to itself about this learns the truth at `403`.
async fn registration_state(State(state): State<Shared>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "enabled": state
            .allow_registration
            .load(std::sync::atomic::Ordering::Relaxed)
    }))
}

/// POST /v1/users — create an ordinary account, when registration is open.
///
/// Device tier: the caller has a paired device, which after web bootstrap
/// means anyone who can reach the web app. That is the deliberate meaning of
/// the switch (A13), and the reason it is off until someone turns it on.
///
/// **Always a member.** Owner belongs to the bootstrap account; registration
/// must not be able to mint one, or an open server would hand out the role
/// that can disable every other account.
async fn register_user(
    State(state): State<Shared>,
    Json(body): Json<FirstUserRequest>,
) -> ApiResult<Json<serde_json::Value>> {
    if !state
        .allow_registration
        .load(std::sync::atomic::Ordering::Relaxed)
    {
        // Explicit rather than a bare 404: a client that raced the setting —
        // drew the button, then submitted after it was turned off — can say
        // what happened instead of guessing.
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            "registration_disabled".into(),
        ));
    }

    if let Err(msg) = crate::auth::password::validate_password(&body.password) {
        return Err(ApiError(StatusCode::UNPROCESSABLE_ENTITY, msg));
    }
    if let Err(msg) = crate::auth::users::validate_username(&body.username) {
        return Err(ApiError(StatusCode::UNPROCESSABLE_ENTITY, msg));
    }

    let now = crate::index::now_rfc3339();
    let mut auth_db = state.auth_db.lock().await;

    // Registration cannot be the first account. The bootstrap window is
    // `/v1/users/first`'s alone, and `create_user` would otherwise force this
    // one to be an owner — exactly the thing this endpoint must never mint.
    if auth_db
        .count_users()
        .map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        == 0
    {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "this server has no owner yet; create the first account instead".into(),
        ));
    }

    let hasher = &state.hasher;
    let hash = hasher
        .hash(body.password.clone())
        .await
        .map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    match crate::auth::users::create_user(
        &mut auth_db,
        crate::auth::users::NewUser {
            username: &body.username,
            display_name: None,
            password_hash: &hash,
            role: crate::auth::users::Role::Member,
        },
        &now,
    ) {
        Ok(user) => Ok(Json(serde_json::json!({
            "user_id": user.id,
            "role": user.role.as_str(),
        }))),
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("UNIQUE") || msg.contains("already") {
                Err(ApiError(StatusCode::CONFLICT, "username_taken".into()))
            } else {
                Err(ApiError(StatusCode::INTERNAL_SERVER_ERROR, msg))
            }
        }
    }
}

/// PUT /v1/config/registration — open or close registration (A13).
///
/// Owner only. Roles are enforced against each other elsewhere but consulted
/// for access nowhere yet, so this is one explicit check rather than the start
/// of a policy system: opening a server to the world is not a thing a member
/// should be able to do to an owner.
async fn put_registration(
    State(state): State<Shared>,
    Extension(auth): Extension<SessionAuth>,
    Json(body): Json<RegistrationBody>,
) -> ApiResult<Json<serde_json::Value>> {
    if auth.authenticated.user.role != crate::auth::users::Role::Owner {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            "only an owner can change registration".into(),
        ));
    }

    {
        let mut vaults = state.vaults.write().await;
        vaults.registry.allow_registration = body.enabled;
        vaults.registry.save(&state.state_dir)?;
    }
    // Atomic, so it applies to the next request rather than the next restart.
    state
        .allow_registration
        .store(body.enabled, std::sync::atomic::Ordering::Relaxed);

    let auth_db = state.auth_db.lock().await;
    let _ = auth_db.record_event(
        "registration_changed",
        Some(&auth.authenticated.user.id),
        None,
        &crate::index::now_rfc3339(),
        &format!(r#"{{"enabled":{}}}"#, body.enabled),
    );

    Ok(Json(serde_json::json!({ "enabled": body.enabled })))
}

#[derive(Deserialize)]
struct RegistrationBody {
    enabled: bool,
}

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

/// GET /v1/keys — the caller's keys, or another user's if the caller is an owner.
async fn list_keys(
    State(state): State<Shared>,
    Extension(actor): Extension<Actor>,
    Query(q): Query<ListKeysQuery>,
) -> ApiResult<Json<Vec<crate::auth::keys::ApiKey>>> {
    Ok(Json(
        crate::ops::list_api_keys(&state, &actor, q.user.as_deref()).await?,
    ))
}

#[derive(Deserialize)]
struct ListKeysQuery {
    #[serde(default)]
    user: Option<String>,
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

/// POST /v1/users/first — create the owner account, once.
///
/// **Registered in the *device* tier, not the `none` tier** (A8): creating a
/// user over the network costs a paired device. The doc comment here used to
/// say "genuinely unauthenticated", which described neither the routing nor
/// the intent.
///
/// The password is hashed here and stored immediately. The caller must supply a
/// valid password that passes `validate_password`. The operator `user add` CLI
/// is the other path and is more ergonomic for interactive use.
async fn create_first_user(
    State(state): State<Shared>,
    Json(body): Json<FirstUserRequest>,
) -> ApiResult<StatusCode> {
    if let Err(msg) = crate::auth::password::validate_password(&body.password) {
        return Err(ApiError(StatusCode::UNPROCESSABLE_ENTITY, msg));
    }
    if let Err(msg) = crate::auth::users::validate_username(&body.username) {
        return Err(ApiError(StatusCode::UNPROCESSABLE_ENTITY, msg));
    }

    let now = crate::index::now_rfc3339();
    let mut auth_db = state.auth_db.lock().await;

    // **The bootstrap window closes after the first account, and stays closed.**
    // Without this, the only refusal was `create_user`'s duplicate-username
    // check, so any paired device could pick an unused name and get another
    // account — and this handler hardcodes `Role::Owner`, so every one of them
    // would be an owner. That is privilege escalation the moment a server has a
    // second user: a member's device mints an owner and logs into it.
    //
    // Checked before the hash on purpose. A refusal must not consume one of the
    // two `Hasher` permits, or an attacker can hold the login path down by
    // POSTing here.
    if auth_db
        .count_users()
        .map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        > 0
    {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "an account already exists".into(),
        ));
    }

    let hasher = &state.hasher;
    let hash = hasher
        .hash(body.password.clone())
        .await
        .map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    match crate::auth::users::create_user(
        &mut auth_db,
        crate::auth::users::NewUser {
            username: &body.username,
            display_name: None,
            password_hash: &hash,
            role: crate::auth::users::Role::Owner,
        },
        &now,
    ) {
        Ok(_) => Ok(StatusCode::CREATED),
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("UNIQUE") || msg.contains("already") {
                Err(ApiError(StatusCode::CONFLICT, "username taken".into()))
            } else {
                Err(ApiError(StatusCode::INTERNAL_SERVER_ERROR, msg))
            }
        }
    }
}

#[derive(Deserialize)]
pub struct FirstUserRequest {
    username: String,
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

/// POST /v1/auth/login — exchange username/password for a token pair.
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
        // Never a secret: the submitted username is quoted verbatim because a
        // flood of junk names is exactly what the operator needs to see.
        if let Err(e) = auth_db.record_event_from(
            crate::auth::sessions::EVENT_LOGIN_THROTTLED,
            None,
            Some(&device_id),
            remote.as_deref(),
            &now,
            &format!(
                r#"{{"username":{:?},"retry_after_secs":{}}}"#,
                body.username, retry_after_secs
            ),
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
        &body.username,
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

#[derive(Deserialize)]
pub struct LoginRequest {
    username: String,
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
        .list_sessions(Some(&auth.authenticated.user.id))
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

    if session.user_id != auth.authenticated.user.id {
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
        .password_hash_of(&auth.authenticated.user.id)
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

    crate::auth::users::set_password(
        &mut auth_db,
        &auth.authenticated.user.username,
        &new_hash,
        &now,
    )
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
const FORWARDING_HEADERS: [&str; 4] = [
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
        crate::ops::issue_pairing_qr(&state, purpose, Some(&auth.authenticated.user.id), None)
            .await?,
    ))
}

#[derive(Deserialize)]
pub struct IssuePairingRequest {
    /// The pairing purpose: "first_user" or "add_device" (default).
    purpose: Option<String>,
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
            request
                .uri()
                .query()
                .and_then(|q| {
                    q.split('&')
                        .find_map(|kv| kv.strip_prefix("token=").map(str::to_string))
                })
                .map(|raw| percent_decode(&raw))
        });

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
                user_id: authed.user.id.clone(),
                role: authed.user.role,
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
                    user_id: authenticated.user.id.clone(),
                    role: authenticated.user.role,
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
    /// Whether anyone with a device credential may create an account (A13).
    allow_registration: bool,
}

async fn get_config(State(state): State<Shared>) -> ApiResult<Json<ConfigResponse>> {
    let vaults = state.vaults.read().await;
    Ok(Json(ConfigResponse {
        vault_root: vaults.registry.root.display().to_string(),
        state_dir: state.state_dir.display().to_string(),
        vault_count: vaults.registry.vaults.len(),
        mcp_enabled: vaults.registry.mcp_enabled,
        mcp_writable: vaults.registry.mcp_writable,
        allow_registration: vaults.registry.allow_registration,
    }))
}

#[derive(Deserialize)]
struct McpBody {
    enabled: bool,
    /// Absent means read-only. Callers that do not know about writes therefore
    /// cannot turn them on by omission.
    #[serde(default)]
    writable: bool,
}

/// Switches the MCP endpoint on or off, now and across restarts.
///
/// Its own route rather than a field on `PUT /v1/config`, because that one
/// re-points the storage root — a heavyweight operation with an orphan check
/// and a watcher respawn. Toggling a read-only endpoint should not have to send
/// a `vault_root` it does not want to change.
///
/// The atomic is set *after* the registry is saved: if the write fails the
/// endpoint keeps its old state, which is the honest outcome. The other order
/// would report success while the setting silently reverted on next boot.
async fn put_mcp(
    State(state): State<Shared>,
    Json(body): Json<McpBody>,
) -> ApiResult<Json<serde_json::Value>> {
    {
        let mut vaults = state.vaults.write().await;
        vaults.registry.mcp_enabled = body.enabled;
        // Writes cannot outlive the endpoint being on: leaving them armed while
        // MCP is off would mean switching MCP back on silently restores write
        // access someone thought they had revoked.
        vaults.registry.mcp_writable = body.enabled && body.writable;
        vaults.registry.save(&state.state_dir)?;
    }
    let ordering = std::sync::atomic::Ordering::Relaxed;
    state.mcp_enabled.store(body.enabled, ordering);
    state
        .mcp_writable
        .store(body.enabled && body.writable, ordering);

    tracing::info!(
        enabled = body.enabled,
        writable = body.enabled && body.writable,
        "MCP endpoint toggled"
    );
    Ok(Json(serde_json::json!({
        "mcp_enabled": body.enabled,
        "mcp_writable": body.enabled && body.writable,
    })))
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
) -> ApiResult<Json<crate::ops::NoteDetail>> {
    Ok(Json(
        crate::ops::get_note(&state, &actor, &vault, &id).await?,
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

#[cfg(test)]
mod tests {
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

    /// A policy that refuses everything — the only way to exercise the refusal
    /// path while the shipped policy is `AllowAuthenticated`.
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
    fn test_router_with_state(dir: &FsPath) -> (Router, Arc<crate::auth::ServerIdentity>, Shared) {
        test_router_with_policy(dir, Arc::new(crate::auth::authz::AllowAuthenticated))
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
    fn test_router_with_limiter(
        dir: &FsPath,
        limiter: crate::auth::ratelimit::LoginLimiter,
    ) -> (Router, Arc<crate::auth::ServerIdentity>, Shared) {
        test_router_full(
            dir,
            Arc::new(crate::auth::authz::AllowAuthenticated),
            limiter,
        )
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
        let state: Shared = Arc::new(AppState {
            vaults: RwLock::new(VaultSet {
                registry,
                open: HashMap::new(),
            }),
            events,
            state_dir,
            identity: identity.clone(),
            root_changed,
            mcp_enabled: std::sync::atomic::AtomicBool::new(false),
            mcp_writable: std::sync::atomic::AtomicBool::new(false),
            auth_db: Arc::new(tokio::sync::Mutex::new(auth_db)),
            allow_registration: std::sync::atomic::AtomicBool::new(false),
            bootstrap_nonce: None,
            listen_addr: "http://127.0.0.1:8080".into(),
            vault_policy: policy,
            hasher: crate::auth::Hasher::new(),
            login_limiter,
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

    /// Gives the server one active owner, so the "nobody could administer this"
    /// guard is satisfied and the *other* guard is what a test is measuring.
    ///
    /// The stored hash is a fixed PHC string and is never verified: these tests
    /// are about the switch, not about login, and a real Argon2id hash at the
    /// measured parameters would cost 192 MiB and ~170 ms apiece to prove
    /// nothing they assert.
    async fn seed_owner(state: &Shared) -> String {
        let mut auth_db = state.auth_db.lock().await;
        crate::auth::users::create_user(
            &mut auth_db,
            crate::auth::users::NewUser {
                username: "dewansh",
                display_name: None,
                password_hash: "$argon2id$v=19$m=196608,t=1,p=1$c29tZXNhbHQ$bm90YXJlYWxoYXNo",
                role: crate::auth::users::Role::Owner,
            },
            "2026-08-17T00:00:00Z",
        )
        .unwrap()
        .id
    }

    /// A session token for `user_id`, minted directly rather than through
    /// `login`, for the same reason [`seed_owner`] fakes the hash.
    async fn session_token(state: &Shared, user_id: &str) -> String {
        let mut auth_db = state.auth_db.lock().await;
        let device =
            crate::auth::devices::create_synthetic(&mut auth_db, "test", "2026-08-17T00:00:00Z")
                .unwrap();
        crate::auth::sessions::create(&mut auth_db, user_id, &device.id, "2026-08-17T00:00:00Z")
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
        seen: std::sync::Mutex<Vec<(String, String)>>,
        /// The full actors, for tests asking *what kind* of caller arrived
        /// rather than only which user.
        actors: std::sync::Mutex<Vec<Actor>>,
    }

    impl RecordingPolicy {
        fn pairs(&self) -> Vec<(String, String)> {
            self.seen.lock().unwrap().clone()
        }

        fn actors(&self) -> Vec<Actor> {
            self.actors.lock().unwrap().clone()
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
            self.seen.lock().unwrap().push((who, vault_id.to_string()));
            self.actors.lock().unwrap().push(actor.clone());
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

    /// A second account, so two MCP requests can carry different identities.
    async fn seed_member(state: &Shared, username: &str) -> String {
        let mut auth_db = state.auth_db.lock().await;
        crate::auth::users::create_user(
            &mut auth_db,
            crate::auth::users::NewUser {
                username,
                display_name: None,
                password_hash: "$argon2id$v=19$m=196608,t=1,p=1$c29tZXNhbHQ$bm90YXJlYWxoYXNo",
                role: crate::auth::users::Role::Member,
            },
            "2026-08-17T00:00:00Z",
        )
        .unwrap()
        .id
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
        // The property the whole mechanism rests on. rmcp builds the handler on
        // the request task and then runs it inside a `tokio::spawn`, so the
        // identity crosses that boundary by **ownership** — captured into the
        // handler by the factory — rather than as ambient state a second
        // request could overwrite.
        //
        // Two users, two vaults, many interleaved requests. If the identity
        // were shared, some request would be recorded against the other user's
        // vault.
        let dir = tempdir::TempDir::new("storm-mcp-concurrent").unwrap();
        let policy = Arc::new(RecordingPolicy::default());
        let (app, _, state) = test_router_with_policy(dir.path(), policy.clone());
        state
            .mcp_enabled
            .store(true, std::sync::atomic::Ordering::Relaxed);

        let alice = seed_owner(&state).await;
        let bob = seed_member(&state, "bob").await;
        let alice_token = session_token(&state, &alice).await;
        let bob_token = session_token(&state, &bob).await;

        register_vault(&state, "Alice").await;
        register_vault(&state, "Bob").await;
        let (alice_vault, bob_vault) = {
            let vaults = state.vaults.read().await;
            (
                vaults.registry.vaults[0].id.clone(),
                vaults.registry.vaults[1].id.clone(),
            )
        };

        // Interleaved on purpose: alternating users maximises the chance that
        // one request's scope is live while another's factory runs.
        let mut tasks = Vec::new();
        for i in 0..64 {
            let (user_vault, auth) = if i % 2 == 0 {
                (alice_vault.clone(), format!("Bearer {alice_token}"))
            } else {
                (bob_vault.clone(), format!("Bearer {bob_token}"))
            };
            let app = app.clone();
            tasks.push(tokio::spawn(async move {
                // `list_tags`, because it makes **exactly one** authorization
                // decision. `get_vault` also enumerates vaults to build its
                // listing, and those filter checks legitimately ask about other
                // users' vaults — which would make a crossed pair ambiguous
                // rather than proof of a leak.
                let (status, _) = send(
                    &app,
                    mcp_call(
                        "list_tags",
                        serde_json::json!({ "vault": user_vault }),
                        &auth,
                    ),
                )
                .await;
                status
            }));
        }
        for task in tasks {
            assert_eq!(task.await.unwrap(), StatusCode::OK);
        }

        let pairs = policy.pairs();
        assert_eq!(pairs.len(), 64, "every call must have reached the policy");
        for (who, vault) in pairs {
            let expected = if who == alice {
                &alice_vault
            } else {
                &bob_vault
            };
            assert_eq!(
                &vault, expected,
                "{who} was recorded against another user's vault — identity leaked \
                 between concurrent MCP requests"
            );
        }
    }

    #[tokio::test]
    async fn a_refused_vault_is_403_and_not_404() {
        // The refusal path, which `AllowAuthenticated` never takes — so
        // without a policy swap this would be code that first runs in
        // production. It is also the claim the whole seam rests on: changing
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
            "AllowAuthenticated must not filter anything out"
        );
    }

    #[tokio::test]
    async fn the_shipped_policy_changes_nothing_for_an_ordinary_caller() {
        // The other direction, and the reason this slice is safe to merge:
        // with `AllowAuthenticated` the boundary is invisible. A vault that is
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
    async fn pair_a_device(state: &Shared) -> String {
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

        // And the device tier still accepts it, which is the half that matters.
        let (status, _) = send(&app, get_with_auth("/v1/users", &device)).await;
        assert_eq!(status, StatusCode::OK);
    }

    // ---- registration (slice 16, A13) ------------------------------------

    /// A paired device plus an owner, which is what registration needs around
    /// it: it is device tier, and it refuses to be the first account.
    async fn device_and_owner(state: &Shared) -> (String, String) {
        let device = pair_a_device(state).await;
        let owner = seed_owner(state).await;
        (device, owner)
    }

    fn register(auth: &str, username: &str) -> axum::http::Request<axum::body::Body> {
        post_json_with_auth(
            "/v1/users",
            serde_json::json!({"username": username, "password": "a-long-enough-password"}),
            auth,
        )
    }

    #[tokio::test]
    async fn registration_is_off_until_someone_turns_it_on() {
        // The default is the decision. Turning it on composes with web
        // bootstrap into "anyone who can reach this server can make an
        // account", so it is never the shipped state.
        let dir = tempdir::TempDir::new("storm-reg-default").unwrap();
        let (app, _, state) = test_router_with_state(dir.path());
        let (device, _) = device_and_owner(&state).await;

        let (status, body) = send(&app, get_with_auth("/v1/auth/registration", &device)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["enabled"], false, "registration must ship closed");

        let (status, body) = send(&app, register(&device, "stranger")).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(
            body["error"], "registration_disabled",
            "an explicit code, so a client that raced the switch can say so"
        );

        let auth_db = state.auth_db.lock().await;
        assert_eq!(auth_db.count_users().unwrap(), 1, "no account was created");
    }

    #[tokio::test]
    async fn an_owner_opens_registration_and_it_applies_at_once() {
        // No restart. A switch you cannot verify by flipping it is not a
        // switch — the same property the legacy-token switch has.
        let dir = tempdir::TempDir::new("storm-reg-on").unwrap();
        let (app, _, state) = test_router_with_state(dir.path());
        let (device, owner) = device_and_owner(&state).await;
        let token = session_token(&state, &owner).await;

        let (status, body) = send(
            &app,
            put_json(
                "/v1/config/registration",
                serde_json::json!({"enabled": true}),
                Some(&format!("Bearer {token}")),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body:?}");

        // Visible immediately, on this same running server.
        let (_, body) = send(&app, get_with_auth("/v1/auth/registration", &device)).await;
        assert_eq!(body["enabled"], true);

        let (status, body) = send(&app, register(&device, "newcomer")).await;
        assert_eq!(status, StatusCode::CREATED.min(StatusCode::OK), "{body:?}");
        assert_eq!(
            body["role"], "member",
            "registration mints members and nothing else"
        );

        // And it survives a reload of the registry, because it is persisted.
        let vaults = state.vaults.read().await;
        assert!(vaults.registry.allow_registration);
    }

    #[tokio::test]
    async fn registration_can_never_mint_an_owner() {
        // The guarantee that makes an open switch survivable: owner is the
        // bootstrap account, and an open endpoint must not reach the role that
        // can disable every other account.
        let dir = tempdir::TempDir::new("storm-reg-role").unwrap();
        let (app, _, state) = test_router_with_state(dir.path());
        let (device, _) = device_and_owner(&state).await;
        state
            .allow_registration
            .store(true, std::sync::atomic::Ordering::Relaxed);

        send(&app, register(&device, "newcomer")).await;

        let auth_db = state.auth_db.lock().await;
        let users = auth_db.list_users().unwrap();
        let owners: Vec<_> = users
            .iter()
            .filter(|u| u.role == crate::auth::users::Role::Owner)
            .collect();
        assert_eq!(owners.len(), 1, "still exactly one owner");
        assert_eq!(owners[0].username, "dewansh", "and it is the bootstrap one");
    }

    #[tokio::test]
    async fn registration_still_needs_a_device_credential() {
        // Open does not mean unauthenticated. The device tier is what the
        // whole design rests on, and this endpoint sits behind it like the
        // rest.
        let dir = tempdir::TempDir::new("storm-reg-tier").unwrap();
        let (app, _, state) = test_router_with_state(dir.path());
        device_and_owner(&state).await;
        state
            .allow_registration
            .store(true, std::sync::atomic::Ordering::Relaxed);

        let (status, _) = send(
            &app,
            post_json(
                "/v1/users",
                serde_json::json!({"username": "nobody", "password": "a-long-enough-password"}),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);

        let (status, _) = send(&app, get("/v1/auth/registration")).await;
        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "even the question is device tier"
        );
    }

    #[tokio::test]
    async fn only_an_owner_may_change_registration() {
        let dir = tempdir::TempDir::new("storm-reg-owner").unwrap();
        let (app, _, state) = test_router_with_state(dir.path());
        let (_, _owner) = device_and_owner(&state).await;

        // A member's session must not be able to open the server to the world.
        let member = {
            let mut auth_db = state.auth_db.lock().await;
            crate::auth::users::create_user(
                &mut auth_db,
                crate::auth::users::NewUser {
                    username: "member",
                    display_name: None,
                    password_hash: "$argon2id$v=19$m=196608,t=1,p=1$c29tZXNhbHQ$bm90YXJlYWxoYXNo",
                    role: crate::auth::users::Role::Member,
                },
                "2026-08-19T00:00:00Z",
            )
            .unwrap()
            .id
        };
        let token = session_token(&state, &member).await;

        let (status, _) = send(
            &app,
            put_json(
                "/v1/config/registration",
                serde_json::json!({"enabled": true}),
                Some(&format!("Bearer {token}")),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert!(
            !state
                .allow_registration
                .load(std::sync::atomic::Ordering::Relaxed)
        );
    }

    #[tokio::test]
    async fn the_bootstrap_flow_is_untouched_by_registration() {
        // `/v1/users/first` is the one-shot bootstrap and must behave exactly
        // as before: registration being open does not reopen it, and
        // registration cannot stand in for it on an empty server.
        let dir = tempdir::TempDir::new("storm-reg-bootstrap").unwrap();
        let (app, _, state) = test_router_with_state(dir.path());
        let device = pair_a_device(&state).await;
        state
            .allow_registration
            .store(true, std::sync::atomic::Ordering::Relaxed);

        // Registration refuses to be the first account.
        let (status, _) = send(&app, register(&device, "first")).await;
        assert_eq!(
            status,
            StatusCode::CONFLICT,
            "an empty server has no owner; that is the bootstrap's job"
        );

        // The bootstrap still works, and still closes afterwards.
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

        let (status, _) = send(
            &app,
            post_json_with_auth(
                "/v1/users/first",
                serde_json::json!({"username": "another", "password": "a-long-enough-password"}),
                &device,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "still one-shot");
    }

    #[tokio::test]
    async fn closing_registration_leaves_existing_accounts_alone() {
        let dir = tempdir::TempDir::new("storm-reg-close").unwrap();
        let (app, _, state) = test_router_with_state(dir.path());
        let (device, owner) = device_and_owner(&state).await;
        state
            .allow_registration
            .store(true, std::sync::atomic::Ordering::Relaxed);
        send(&app, register(&device, "newcomer")).await;

        let token = session_token(&state, &owner).await;
        let (status, _) = send(
            &app,
            put_json(
                "/v1/config/registration",
                serde_json::json!({"enabled": false}),
                Some(&format!("Bearer {token}")),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        // The account made while it was open is still there and still active.
        let auth_db = state.auth_db.lock().await;
        let users = auth_db.list_users().unwrap();
        assert!(
            users.iter().any(|u| u.username == "newcomer"
                && u.status == crate::auth::users::Status::Active),
            "closing the door must not evict the people already through it"
        );
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

        // And it is a *device tier* credential, which is the point.
        let auth = format!("StormDevice {device_id}:{secret}");
        let (status, _) = send(&app, get_with_auth("/v1/users", &auth)).await;
        assert_eq!(status, StatusCode::OK);
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
    fn throttling_limiter() -> crate::auth::ratelimit::LoginLimiter {
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
        assert_eq!(
            status,
            StatusCode::CONFLICT,
            "a second account through the bootstrap endpoint is a second owner"
        );
        assert_eq!(body["error"], "an account already exists");

        let auth_db = state.auth_db.lock().await;
        assert_eq!(
            auth_db.count_users().unwrap(),
            1,
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

    /// Mints a user and a key on `state`, returning `(user_id, plaintext)`.
    async fn seed_key(
        state: &Shared,
        username: &str,
        role: crate::auth::users::Role,
    ) -> (String, String) {
        let mut db = state.auth_db.lock().await;
        // The first account has to be an owner — a server whose only user is a
        // member has nobody who can create the next one. So a member needs one
        // ahead of it.
        if role != crate::auth::users::Role::Owner {
            crate::auth::users::create_user(
                &mut db,
                crate::auth::users::NewUser {
                    username: "bootstrap-owner",
                    display_name: None,
                    password_hash: "hash",
                    role: crate::auth::users::Role::Owner,
                },
                "2026-08-19T11:00:00Z",
            )
            .unwrap();
        }
        let user = crate::auth::users::create_user(
            &mut db,
            crate::auth::users::NewUser {
                username,
                display_name: None,
                password_hash: "hash",
                role,
            },
            "2026-08-19T12:00:00Z",
        )
        .unwrap();
        let (_, secret) = crate::auth::keys::create(
            &mut db,
            &user.id,
            "a machine",
            None,
            None,
            "2026-08-19T12:00:00Z",
        )
        .unwrap();
        (user.id, secret)
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
        let (_, secret) = seed_key(&state, "dewansh", crate::auth::users::Role::Owner).await;

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
        let (_, secret) = seed_key(&state, "dewansh", crate::auth::users::Role::Owner).await;

        for (method, uri) in [
            ("GET", "/v1/vaults"),
            ("GET", "/v1/config"),
            ("GET", "/v1/recents"),
            ("GET", "/v1/users"),
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
        let (user_id, secret) = seed_key(&state, "dewansh", crate::auth::users::Role::Owner).await;

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
    async fn a_member_reaches_only_their_own_keys() {
        let dir = tempdir::TempDir::new("storm-key-scope-user").unwrap();
        let (app, _, state) = test_router_with_state(dir.path());
        let owner = seed_owner(&state).await;
        let member = {
            let mut db = state.auth_db.lock().await;
            crate::auth::users::create_user(
                &mut db,
                crate::auth::users::NewUser {
                    username: "member",
                    display_name: None,
                    password_hash: "hash",
                    role: crate::auth::users::Role::Member,
                },
                "2026-08-19T12:00:00Z",
            )
            .unwrap()
            .id
        };
        let member_auth = format!("Bearer {}", session_token(&state, &member).await);
        let owner_auth = format!("Bearer {}", session_token(&state, &owner).await);

        // The owner mints one for themselves.
        let (status, owners_key) = send(
            &app,
            post_json_with_auth(
                "/v1/keys",
                serde_json::json!({ "name": "owner key" }),
                &owner_auth,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let owners_key_id = owners_key["id"].as_str().unwrap().to_string();

        // A member may not read the owner's keys...
        let (status, _) = send(
            &app,
            get_with_auth(&format!("/v1/keys?user={owner}"), &member_auth),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        // ...nor revoke one. **404, not 403** — a member probing key ids must
        // not learn which exist.
        let (status, _) = send(
            &app,
            axum::http::Request::builder()
                .method("DELETE")
                .uri(format!("/v1/keys/{owners_key_id}"))
                .header("Authorization", &member_auth)
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        // The owner may reach the member's keys, which is the one asymmetry
        // A14 grants and the whole of it.
        let (status, _) = send(
            &app,
            get_with_auth(&format!("/v1/keys?user={member}"), &owner_auth),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
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
        let (user_id, secret) = seed_key(&state, "dewansh", crate::auth::users::Role::Member).await;

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
                "an MCP key must reach the boundary as its owner"
            );
            assert_eq!(
                actor.role(),
                crate::auth::users::Role::Member,
                "and with its owner's role, unnarrowed"
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
}
