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
        Extension, Path, Query, State,
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
    pub token: String,
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
    /// Whether the legacy `STORM_TOKEN` is accepted on session-tier routes.
    ///
    /// Mirrors `Registry::legacy_token_enabled`, which is the persisted copy;
    /// this one exists so the middleware can read it without taking the vault
    /// lock on every request. Atomic rather than a plain `bool` because the
    /// switch has to take effect on the *next request* — an operator turning
    /// the legacy token off and still being let in until they restart would
    /// make the confirmation step of the A10 cutover meaningless.
    pub legacy_token_enabled: std::sync::atomic::AtomicBool,
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
        .route("/v1/config/legacy-token", put(put_legacy_token))
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

    // MCP needs session auth + the mcp_enabled gate. It is nested under /mcp
    // rather than merged, so its paths don't collide with REST routes.
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
        .layer(Extension(RequiredTier::Session));

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

/// POST /v1/users/first — create the owner account (unauthenticated).
///
/// Fails if any user already exists. This is the one endpoint that is
/// genuinely unauthenticated; it is registered in the `none` tier.
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
    let hasher = crate::auth::Hasher::new();
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

/// POST /v1/auth/login — exchange username/password for a token pair.
///
/// The caller must present a `StormDevice` header (device tier). The device
/// must be paired; if not the login fails with 401. After authentication the
/// caller receives access + refresh tokens bound to this device.
async fn login_handler(
    State(state): State<Shared>,
    headers: HeaderMap,
    Json(body): Json<LoginRequest>,
) -> Result<Json<crate::auth::sessions::IssuedSession>, Response> {
    let now = crate::index::now_rfc3339();

    // The device must be present and paired — require_auth already checked
    // this, but we need the device_id for session binding.
    let device_id = extract_device_id(&headers).ok_or_else(|| {
        ApiError(StatusCode::UNAUTHORIZED, "device header required".into()).into_response()
    })?;

    let hasher = crate::auth::Hasher::new();
    let mut auth_db = state.auth_db.lock().await;
    match crate::auth::sessions::login(
        &mut auth_db,
        &hasher,
        &body.username,
        body.password.clone(),
        &device_id,
        &now,
    )
    .await
    {
        Ok(issued) => Ok(Json(issued)),
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

    let hasher = crate::auth::Hasher::new();
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

/// POST /v1/pair — consume a pairing nonce and receive device credentials.
///
/// This is a `none`-tier endpoint: the nonce itself is the trust anchor. The
/// client has already verified the server's identity via the challenge step
/// before reaching here.
async fn pair_handler(
    State(state): State<Shared>,
    Json(body): Json<PairRequest>,
) -> ApiResult<Json<crate::auth::pairing::ConsumeResult>> {
    let now = crate::index::now_rfc3339();
    let mut auth_db = state.auth_db.lock().await;
    crate::auth::pairing::consume(
        &mut auth_db,
        &body.n,
        &body.name,
        body.platform.as_deref(),
        body.version.as_deref(),
        &now,
    )
    .map_err(|e| {
        let msg = e.to_string();
        if msg.contains("already used") {
            ApiError(StatusCode::CONFLICT, "pairing_consumed".into())
        } else if msg.contains("expired") {
            ApiError(StatusCode::GONE, "pairing_expired".into())
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
        crate::ops::issue_pairing_qr(&state, purpose, Some(&auth.authenticated.user.id)).await?,
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

/// A caller that has proven who it is (user + device + session).
///
/// Set as a request extension by `require_auth` on session-tier routes when the
/// credential is a `Bearer` token.
#[derive(Clone)]
pub struct SessionAuth {
    pub authenticated: crate::auth::sessions::Authenticated,
}

/// The caller authenticated with the legacy shared `STORM_TOKEN`.
///
/// Set instead of [`SessionAuth`] on the legacy path, which has no user,
/// device or session behind it. Only `put_legacy_token` reads it, to refuse a
/// request that would switch off the very credential it arrived on.
#[derive(Clone)]
pub struct LegacyAuth;

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
/// On session tier only, the legacy `STORM_TOKEN` is accepted as
/// owner-equivalent when `legacy_token_enabled` is on (A10). It is never
/// accepted on device tier — login and refresh require a real device.
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
            // Browsers can't set headers on a WebSocket handshake, so the token
            // may also arrive as a query parameter.
            request.uri().query().and_then(|q| {
                q.split('&')
                    .find_map(|kv| kv.strip_prefix("token=").map(str::to_string))
            })
        });

    let Some(credential) = presented else {
        return unauthorized("invalid or missing token");
    };

    // --- StormDevice: `StormDevice <id>:<secret>` ---
    if let Some(rest) = credential.strip_prefix("StormDevice ") {
        let auth_db = state.auth_db.lock().await;
        match parse_device_credential(rest) {
            Ok((id, secret)) => match auth_db.verify_device_secret(id, secret) {
                Ok(Some(device)) => {
                    if device.is_revoked() {
                        return tier_error("device_revoked", StatusCode::UNAUTHORIZED);
                    }
                    request.extensions_mut().insert(DeviceAuth { device });
                    return next.run(request).await;
                }
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
    }

    // --- Bearer token: session or legacy ---
    if let Some(token) = credential.strip_prefix("Bearer ") {
        // Session-tier only: legacy token fallback.
        if tier == RequiredTier::Session
            && state
                .legacy_token_enabled
                .load(std::sync::atomic::Ordering::Relaxed)
            && constant_time_eq(token, &state.token)
        {
            // Legacy token: owner-equivalent. No device, no session extension —
            // handlers that need them will have to handle the absence. This is
            // the deliberate contract: the legacy path exists for backward
            // compatibility and cannot create the first user (A10).
            //
            // The marker lets a handler notice it is talking to the legacy
            // credential. Only the legacy-token switch uses it, to refuse to
            // saw off the branch the caller is sitting on.
            request.extensions_mut().insert(LegacyAuth);
            request.extensions_mut().insert(Actor::Legacy);
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

/// Compares without leaking length or position through timing.
fn constant_time_eq(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.bytes()
        .zip(b.bytes())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
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
    /// Whether the legacy shared token is still accepted (A10). The app shows
    /// this and can change it, which is the whole point — the migration is a
    /// reversible switch, not a release boundary.
    legacy_token_enabled: bool,
}

async fn get_config(State(state): State<Shared>) -> ApiResult<Json<ConfigResponse>> {
    let vaults = state.vaults.read().await;
    Ok(Json(ConfigResponse {
        vault_root: vaults.registry.root.display().to_string(),
        state_dir: state.state_dir.display().to_string(),
        vault_count: vaults.registry.vaults.len(),
        mcp_enabled: vaults.registry.mcp_enabled,
        mcp_writable: vaults.registry.mcp_writable,
        legacy_token_enabled: vaults.registry.legacy_token_enabled,
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
struct LegacyTokenBody {
    enabled: bool,
}

/// Turns the legacy shared `STORM_TOKEN` on or off, now and across restarts.
///
/// This is the switch A10 specifies, and the last step of the migration off
/// the shared token. It is deliberately a *switch* rather than a release
/// boundary: the operator turns it off, checks that their paired devices still
/// work, and turns it back on if anything broke. A migration you cannot undo,
/// on the machine holding the only copy of a vault, is how someone locks
/// themselves out.
///
/// **One refusal guards exactly that: you may not switch it off over the
/// legacy token itself.** A caller authenticated by `STORM_TOKEN` disabling
/// `STORM_TOKEN` loses access with the response and cannot turn it back on, so
/// the reversibility above would be a fiction. A10 orders it plainly: pair,
/// create the user, log in, *then* switch.
///
/// That one check also covers "nobody could administer this server afterwards",
/// which looks like it wants a guard of its own and does not. The last-active-
/// owner invariant means any existing user implies an active owner, so zero
/// owners is only reachable when there are *no users at all* — and then no
/// session can exist, so the only credential that can reach this route is the
/// legacy one, which is already refused. A separate owner count here would be
/// unreachable code. *(It was written, and a mutation campaign showed nothing
/// could make it fire.)*
///
/// Re-enabling is unguarded. Getting back in is always allowed.
///
/// The atomic is set *after* the registry is saved, matching `put_mcp`: if the
/// write fails the credential keeps working, which is the honest outcome. The
/// other order would report success while the setting reverted on next boot.
async fn put_legacy_token(
    State(state): State<Shared>,
    legacy: Option<Extension<LegacyAuth>>,
    Json(body): Json<LegacyTokenBody>,
) -> ApiResult<Json<serde_json::Value>> {
    if !body.enabled && legacy.is_some() {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "this request is authenticated with the legacy token, and turning it \
             off would revoke your own access with no way to turn it back on. \
             Pair a device, create an account and log in, then disable it."
                .into(),
        ));
    }

    {
        let mut vaults = state.vaults.write().await;
        vaults.registry.legacy_token_enabled = body.enabled;
        vaults.registry.save(&state.state_dir)?;
    }
    state
        .legacy_token_enabled
        .store(body.enabled, std::sync::atomic::Ordering::Relaxed);

    tracing::info!(
        enabled = body.enabled,
        "legacy shared token toggled (A10 migration switch)"
    );
    Ok(Json(serde_json::json!({
        "legacy_token_enabled": body.enabled,
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

// ---- websocket ---------------------------------------------------------

async fn stream(State(state): State<Shared>, ws: WebSocketUpgrade) -> Response {
    ws.on_upgrade(move |socket| push_changes(socket, state))
}

/// Pushes change events so other devices update without polling.
///
/// Events carry only metadata; the client decides what to fetch. That keeps
/// the socket cheap and means a missed message is recoverable by falling back
/// to `GET /v1/sync?since=`.
async fn push_changes(mut socket: WebSocket, state: Shared) {
    let mut rx = state.events.subscribe();
    loop {
        match rx.recv().await {
            Ok(change) => {
                let Ok(text) = serde_json::to_string(&change) else {
                    continue;
                };
                if socket.send(Message::Text(text.into())).await.is_err() {
                    break;
                }
            }
            // A slow client that fell behind is told to resync rather than
            // being silently left with a gap.
            Err(broadcast::error::RecvError::Lagged(_)) => {
                let _ = socket
                    .send(Message::Text(r#"{"kind":"resync"}"#.into()))
                    .await;
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
            token: "testtoken".into(),
            state_dir,
            identity: identity.clone(),
            root_changed,
            mcp_enabled: std::sync::atomic::AtomicBool::new(false),
            mcp_writable: std::sync::atomic::AtomicBool::new(false),
            auth_db: Arc::new(tokio::sync::Mutex::new(auth_db)),
            legacy_token_enabled: std::sync::atomic::AtomicBool::new(true),
            bootstrap_nonce: None,
            listen_addr: "http://127.0.0.1:8080".into(),
            vault_policy: policy,
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

    #[tokio::test]
    async fn the_legacy_token_switch_persists_and_takes_effect_at_once() {
        // The A10 migration switch. It has to survive a restart (the registry)
        // *and* apply to the next request (the atomic) — a switch that only
        // takes effect on reboot makes "turn it off and check" meaningless.
        let dir = tempdir::TempDir::new("storm-legacy-switch").unwrap();
        let (app, _, state) = test_router_with_state(dir.path());
        let owner = seed_owner(&state).await;
        let token = session_token(&state, &owner).await;

        let (status, body) = send(
            &app,
            put_json(
                "/v1/config/legacy-token",
                serde_json::json!({ "enabled": false }),
                Some(&format!("Bearer {token}")),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["legacy_token_enabled"], false);

        // Applied now, without a restart.
        assert!(
            !state
                .legacy_token_enabled
                .load(std::sync::atomic::Ordering::Relaxed)
        );
        // And the legacy credential really stops working.
        let (status, _) = send(&app, get_with_auth("/v1/config", "Bearer testtoken")).await;
        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "the shared token must be refused once the switch is off"
        );

        // Persisted, so a restart does not silently re-enable it.
        let reloaded = Registry::load(&state.state_dir, FsPath::new("/nonexistent")).unwrap();
        assert!(!reloaded.legacy_token_enabled);
    }

    #[tokio::test]
    async fn the_legacy_token_cannot_switch_itself_off() {
        // Reversibility is the whole point of A10's switch, and a caller who
        // disables the credential they are holding cannot turn it back on.
        let dir = tempdir::TempDir::new("storm-legacy-selfoff").unwrap();
        let (app, _, state) = test_router_with_state(dir.path());
        seed_owner(&state).await;

        let (status, body) = send(
            &app,
            put_json(
                "/v1/config/legacy-token",
                serde_json::json!({ "enabled": false }),
                Some("Bearer testtoken"),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        assert!(
            state
                .legacy_token_enabled
                .load(std::sync::atomic::Ordering::Relaxed),
            "a refused request must not have changed the setting"
        );
    }

    #[tokio::test]
    async fn the_legacy_token_stays_on_while_nobody_can_log_in() {
        // With no account, turning the shared token off leaves no way into the
        // server except a pairing QR in the journal.
        //
        // The same guard as the test above catches it, and that is the point:
        // with no users there is no session, so the legacy credential is the
        // only thing that can reach this route. Kept as its own test because it
        // is a distinct *state* an operator can be in — a fresh server they
        // have not paired yet — not a distinct code path.
        let dir = tempdir::TempDir::new("storm-legacy-noowner").unwrap();
        let (app, _, state) = test_router_with_state(dir.path());

        let (status, body) = send(
            &app,
            put_json(
                "/v1/config/legacy-token",
                serde_json::json!({ "enabled": false }),
                Some("Bearer testtoken"),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        assert!(
            state
                .legacy_token_enabled
                .load(std::sync::atomic::Ordering::Relaxed)
        );
    }

    #[tokio::test]
    async fn re_enabling_the_legacy_token_is_always_allowed() {
        // Getting back in is never guarded. The switch is only dangerous in one
        // direction, and treating both the same would strand an operator who
        // turned it off and found their devices broken.
        let dir = tempdir::TempDir::new("storm-legacy-reenable").unwrap();
        let (app, _, state) = test_router_with_state(dir.path());
        let owner = seed_owner(&state).await;
        let token = session_token(&state, &owner).await;

        let (status, _) = send(
            &app,
            put_json(
                "/v1/config/legacy-token",
                serde_json::json!({ "enabled": false }),
                Some(&format!("Bearer {token}")),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let (status, body) = send(
            &app,
            put_json(
                "/v1/config/legacy-token",
                serde_json::json!({ "enabled": true }),
                Some(&format!("Bearer {token}")),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["legacy_token_enabled"], true);
        assert!(
            state
                .legacy_token_enabled
                .load(std::sync::atomic::Ordering::Relaxed)
        );
    }

    /// Records every authorization decision, so a test can see which actor
    /// reached which vault.
    ///
    /// Allows everything — the question here is *whose identity arrived*, not
    /// whether it was permitted.
    #[derive(Debug, Default)]
    struct RecordingPolicy {
        seen: std::sync::Mutex<Vec<(String, String)>>,
    }

    impl RecordingPolicy {
        fn pairs(&self) -> Vec<(String, String)> {
            self.seen.lock().unwrap().clone()
        }
    }

    impl crate::auth::authz::VaultPolicy for RecordingPolicy {
        fn decide(&self, actor: &Actor, vault_id: &str, _: Access) -> Decision {
            let who = match actor {
                Actor::Session { user_id, .. } => user_id.clone(),
                Actor::Legacy => "legacy".to_string(),
            };
            self.seen.lock().unwrap().push((who, vault_id.to_string()));
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

    #[test]
    fn a_registry_predating_the_switch_keeps_the_legacy_token() {
        // The direction that matters. An upgrade must not refuse every client
        // that has been working all along, so an absent field reads as `true` —
        // the opposite of the MCP flags beside it.
        let registry: Registry =
            serde_json::from_str(r#"{"root":"/srv/storm/vaults","vaults":[],"mcp_enabled":true}"#)
                .unwrap();
        assert!(
            registry.legacy_token_enabled,
            "an older registry must load with the shared token still accepted"
        );
        // And the type's own default agrees, so a `Registry::default()` cannot
        // lock a server out either.
        assert!(Registry::default().legacy_token_enabled);
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
    async fn an_ordinary_route_still_needs_the_token() {
        // The other half of the pair: this slice is additive, so nothing that
        // used to demand a token may have stopped. Without it, moving the two
        // new routes below the layer could take everything else with them.
        let dir = tempdir::TempDir::new("storm-still-authed").unwrap();
        let (app, _) = test_router(dir.path());

        let (status, _) = send(&app, get("/v1/vaults")).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);

        let authed = axum::http::Request::builder()
            .uri("/v1/vaults")
            .header("authorization", "Bearer testtoken")
            .body(axum::body::Body::empty())
            .unwrap();
        let (status, _) = send(&app, authed).await;
        assert_eq!(status, StatusCode::OK);
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
    fn constant_time_eq_behaves_like_eq() {
        assert!(constant_time_eq("abc", "abc"));
        assert!(!constant_time_eq("abc", "abd"));
        assert!(!constant_time_eq("abc", "ab"));
        assert!(constant_time_eq("", ""));
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
}
