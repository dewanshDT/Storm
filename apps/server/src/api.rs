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
        Path, Query, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{delete, get, patch, post, put},
};
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, RwLock, broadcast};

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
    /// Notifies the watcher that the root moved, so it can be respawned
    /// against the new one. Sends the new root.
    pub root_changed: broadcast::Sender<PathBuf>,
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

/// Resolves a vault id to its open handle.
///
/// One place, so no handler repeats the distinction: unknown is a 404, while a
/// registered vault whose directory has gone is a 409 — the difference between
/// "never existed" and "is not where it should be" is the whole point of
/// keeping missing vaults in the registry.
pub async fn vault_of(state: &Shared, id: &str) -> ApiResult<Arc<VaultHandle>> {
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

/// Builds the HTTP surface.
///
/// `mcp` mounts the Model Context Protocol endpoint. It is a parameter rather
/// than something bolted on afterwards for a load-bearing reason: axum applies
/// a layer only to the routes registered *above* it, so `/mcp` has to be nested
/// before `require_token` or it would be the one unauthenticated route on the
/// server. `mcp_requires_the_bearer_token` is the test that holds this.
pub fn router(state: Shared, mcp: Option<crate::mcp::McpOptions>) -> Router {
    let mut router = Router::new().route("/v1/health", get(health));

    if let Some(options) = mcp {
        router = router.nest_service("/mcp", crate::mcp::service(state.clone(), options));
    }

    router
        // Server-level: which vaults exist and where they live.
        .route("/v1/vaults", get(list_vaults).post(create_vault))
        .route("/v1/vaults/{vault}", patch(rename_vault))
        .route("/v1/vaults/{vault}", delete(remove_vault))
        .route("/v1/config", get(get_config).put(put_config))
        .route("/v1/recents", get(recents))
        // Vault-scoped: everything that touches notes.
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
        // Attachments are images and PDFs, well past axum's 2 MB default.
        // Capped so one upload can't exhaust a small homelab box.
        .layer(axum::extract::DefaultBodyLimit::max(MAX_ATTACHMENT_BYTES))
        // One socket for every vault, with `vault_id` on each frame. A socket
        // per vault would hold connections open for vaults nobody is looking
        // at.
        .route("/v1/stream", get(stream))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            require_token,
        ))
        .with_state(state)
}

/// Single shared bearer token, per the v1 auth decision.
///
/// This is only defensible because v1 binds to the LAN. Exposing the server
/// beyond it requires TLS and per-device token rotation *first*.
async fn require_token(
    State(state): State<Shared>,
    headers: HeaderMap,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    if request.uri().path() == "/v1/health" {
        return next.run(request).await;
    }

    let presented = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        // Browsers can't set headers on a WebSocket handshake, so the token may
        // also arrive as a query parameter.
        .map(str::to_string)
        .or_else(|| {
            request.uri().query().and_then(|q| {
                q.split('&')
                    .find_map(|kv| kv.strip_prefix("token=").map(str::to_string))
            })
        });

    match presented {
        Some(t) if constant_time_eq(&t, &state.token) => next.run(request).await,
        _ => ApiError(StatusCode::UNAUTHORIZED, "invalid or missing token".into()).into_response(),
    }
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

async fn list_vaults(State(state): State<Shared>) -> ApiResult<Json<serde_json::Value>> {
    let vaults = crate::ops::list_vaults(&state).await?;
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
}

async fn get_config(State(state): State<Shared>) -> ApiResult<Json<ConfigResponse>> {
    let vaults = state.vaults.read().await;
    Ok(Json(ConfigResponse {
        vault_root: vaults.registry.root.display().to_string(),
        state_dir: state.state_dir.display().to_string(),
        vault_count: vaults.registry.vaults.len(),
    }))
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
    Query(q): Query<RecentsQuery>,
) -> ApiResult<Json<serde_json::Value>> {
    let recents = crate::ops::recents(&state, q.limit).await?;
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
    Path(vault): Path<String>,
) -> ApiResult<Json<TreeResponse>> {
    let handle = vault_of(&state, &vault).await?;
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
    Path(vault): Path<String>,
    Query(q): Query<SyncQuery>,
) -> ApiResult<Json<SyncResponse>> {
    let handle = vault_of(&state, &vault).await?;
    let ix = handle.indexer.lock().await;
    let changes = ix.db.changes_since(q.since, q.limit.clamp(1, 5000))?;
    let seq = ix.db.latest_seq()?;
    Ok(Json(SyncResponse { changes, seq }))
}

async fn get_note(
    State(state): State<Shared>,
    Path((vault, id)): Path<(String, String)>,
) -> ApiResult<Json<crate::ops::NoteDetail>> {
    Ok(Json(crate::ops::get_note(&state, &vault, &id).await?))
}

/// Every stored revision of a note, newest first, without their content.
///
/// Added with MCP but not only for it: no client could see a note's history
/// before this, even though `note_versions` has been populated since M1.
async fn note_versions(
    State(state): State<Shared>,
    Path((vault, id)): Path<(String, String)>,
) -> ApiResult<Json<serde_json::Value>> {
    let versions = crate::ops::note_history(&state, &vault, &id).await?;
    Ok(Json(serde_json::json!({ "versions": versions })))
}

async fn note_version(
    State(state): State<Shared>,
    Path((vault, id, version)): Path<(String, String, i64)>,
) -> ApiResult<Json<serde_json::Value>> {
    let content = crate::ops::note_version(&state, &vault, &id, version).await?;
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
    Path((vault, id)): Path<(String, String)>,
) -> ApiResult<Json<serde_json::Value>> {
    let handle = vault_of(&state, &vault).await?;
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
    Path(vault): Path<String>,
    Json(body): Json<CreateBody>,
) -> ApiResult<Json<crate::index::WriteResult>> {
    let handle = vault_of(&state, &vault).await?;
    let mut ix = handle.indexer.lock().await;
    let result = ix
        .create_note(&body.path, &body.content)
        .map_err(|e| bad_request(e.to_string()))?;
    broadcast_latest(&state, &ix, result.seq);
    Ok(Json(result))
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
    Path((vault, id)): Path<(String, String)>,
    Json(body): Json<PutBody>,
) -> ApiResult<Json<crate::index::WriteResult>> {
    let handle = vault_of(&state, &vault).await?;
    let mut ix = handle.indexer.lock().await;
    let result = ix
        .put_note(
            &id,
            body.base_version,
            &body.content,
            body.device_id.as_deref(),
        )
        .map_err(|e| not_found(e.to_string()))?;
    broadcast_latest(&state, &ix, result.seq);
    Ok(Json(result))
}

#[derive(Deserialize)]
struct MoveBody {
    new_path: String,
}

async fn move_note(
    State(state): State<Shared>,
    Path((vault, id)): Path<(String, String)>,
    Json(body): Json<MoveBody>,
) -> ApiResult<Json<crate::index::WriteResult>> {
    let handle = vault_of(&state, &vault).await?;
    let mut ix = handle.indexer.lock().await;
    let result = ix
        .move_note(&id, &body.new_path)
        .map_err(|e| bad_request(e.to_string()))?;
    broadcast_latest(&state, &ix, result.seq);
    Ok(Json(result))
}

async fn delete_note(
    State(state): State<Shared>,
    Path((vault, id)): Path<(String, String)>,
) -> ApiResult<Json<serde_json::Value>> {
    let handle = vault_of(&state, &vault).await?;
    let mut ix = handle.indexer.lock().await;
    let seq = ix.delete_note(&id).map_err(|e| not_found(e.to_string()))?;
    broadcast_latest(&state, &ix, seq);
    Ok(Json(serde_json::json!({ "seq": seq })))
}

// ---- folders -----------------------------------------------------------

#[derive(Deserialize)]
struct FolderBody {
    path: String,
}

async fn create_folder(
    State(state): State<Shared>,
    Path(vault): Path<String>,
    Json(body): Json<FolderBody>,
) -> ApiResult<Json<serde_json::Value>> {
    let handle = vault_of(&state, &vault).await?;
    let mut ix = handle.indexer.lock().await;
    ix.create_folder(&body.path)
        .map_err(|e| bad_request(e.to_string()))?;
    Ok(Json(serde_json::json!({ "path": body.path })))
}

async fn delete_folder(
    State(state): State<Shared>,
    Path((vault, path)): Path<(String, String)>,
) -> ApiResult<Json<serde_json::Value>> {
    let handle = vault_of(&state, &vault).await?;
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
    Path(vault): Path<String>,
    Json(body): Json<RenameFolderBody>,
) -> ApiResult<Json<serde_json::Value>> {
    let handle = vault_of(&state, &vault).await?;
    let mut ix = handle.indexer.lock().await;
    let seqs = ix
        .rename_folder(&body.from, &body.to)
        .map_err(|e| bad_request(e.to_string()))?;
    // One `moved` per contained note, so every client follows the rename
    // rather than discovering it at the next full tree fetch.
    for seq in &seqs {
        broadcast_latest(&state, &ix, *seq);
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
    Path(vault): Path<String>,
    Query(q): Query<SearchQuery>,
) -> ApiResult<Json<serde_json::Value>> {
    let hits = crate::ops::search(&state, &vault, &q.q, q.limit).await?;
    Ok(Json(serde_json::json!({ "hits": hits })))
}

async fn backlinks(
    State(state): State<Shared>,
    Path((vault, id)): Path<(String, String)>,
) -> ApiResult<Json<serde_json::Value>> {
    let links = crate::ops::backlinks(&state, &vault, &id).await?;
    Ok(Json(
        serde_json::json!({ "title": links.title, "backlinks": links.notes }),
    ))
}

async fn tags(
    State(state): State<Shared>,
    Path(vault): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let tags = crate::ops::list_tags(&state, &vault).await?;
    Ok(Json(serde_json::json!({ "tags": tags })))
}

async fn notes_by_tag(
    State(state): State<Shared>,
    Path((vault, tag)): Path<(String, String)>,
) -> ApiResult<Json<serde_json::Value>> {
    let handle = vault_of(&state, &vault).await?;
    let ix = handle.indexer.lock().await;
    Ok(Json(
        serde_json::json!({ "notes": ix.db.notes_with_tag(&tag)? }),
    ))
}

// ---- attachments -------------------------------------------------------

async fn list_attachments(
    State(state): State<Shared>,
    Path(vault): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let handle = vault_of(&state, &vault).await?;
    let ix = handle.indexer.lock().await;
    Ok(Json(
        serde_json::json!({ "attachments": ix.db.list_attachments()? }),
    ))
}

async fn get_attachment(
    State(state): State<Shared>,
    Path((vault, path)): Path<(String, String)>,
) -> ApiResult<Response> {
    let handle = vault_of(&state, &vault).await?;
    let ix = handle.indexer.lock().await;
    let bytes = ix.attachment(&path).map_err(|e| not_found(e.to_string()))?;

    Ok(([(header::CONTENT_TYPE, content_type_for(&path))], bytes).into_response())
}

async fn put_attachment(
    State(state): State<Shared>,
    Path((vault, path)): Path<(String, String)>,
    body: axum::body::Bytes,
) -> ApiResult<Json<serde_json::Value>> {
    if body.is_empty() {
        return Err(bad_request("empty upload"));
    }
    let handle = vault_of(&state, &vault).await?;
    let mut ix = handle.indexer.lock().await;
    ix.put_attachment(&path, &body)
        .map_err(|e| bad_request(e.to_string()))?;
    Ok(Json(
        serde_json::json!({ "path": path, "size": body.len() }),
    ))
}

async fn delete_attachment(
    State(state): State<Shared>,
    Path((vault, path)): Path<(String, String)>,
) -> ApiResult<Json<serde_json::Value>> {
    let handle = vault_of(&state, &vault).await?;
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

/// Publishes the change identified by `seq` to every connected client.
fn broadcast_latest(state: &Shared, ix: &Indexer, seq: i64) {
    if let Ok(Some(change)) = ix
        .db
        .changes_since(seq - 1, 1)
        .map(|c| c.into_iter().next())
    {
        let _ = state.events.send(change);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
