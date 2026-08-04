//! HTTP + WebSocket surface.
//!
//! Deliberately small. The interesting logic lives in [`crate::index`]; this
//! module only translates it to and from JSON, and enforces the bearer token.

use std::sync::Arc;

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Path, Query, State,
    },
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{delete, get, post, put},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, Mutex};

use crate::db::Change;
use crate::index::Indexer;

pub struct AppState {
    pub indexer: Mutex<Indexer>,
    pub events: broadcast::Sender<Change>,
    pub token: String,
}

pub type Shared = Arc<AppState>;

/// Errors rendered as JSON rather than bare status codes, so the Flutter
/// client can show something useful.
pub struct ApiError(StatusCode, String);

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

fn bad_request(msg: impl Into<String>) -> ApiError {
    ApiError(StatusCode::BAD_REQUEST, msg.into())
}

fn not_found(msg: impl Into<String>) -> ApiError {
    ApiError(StatusCode::NOT_FOUND, msg.into())
}

type ApiResult<T> = Result<T, ApiError>;

pub fn router(state: Shared) -> Router {
    Router::new()
        .route("/v1/health", get(health))
        .route("/v1/tree", get(tree))
        .route("/v1/sync", get(sync))
        .route("/v1/notes", post(create_note))
        .route("/v1/notes/{id}", get(get_note))
        .route("/v1/notes/{id}", put(put_note))
        .route("/v1/notes/{id}", delete(delete_note))
        .route("/v1/notes/{id}/move", post(move_note))
        .route("/v1/notes/{id}/backlinks", get(backlinks))
        .route("/v1/search", get(search))
        .route("/v1/tags", get(tags))
        .route("/v1/tags/{tag}", get(notes_by_tag))
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
        _ => ApiError(StatusCode::UNAUTHORIZED, "invalid or missing token".into())
            .into_response(),
    }
}

/// Compares without leaking length or position through timing.
fn constant_time_eq(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

// ---- handlers ----------------------------------------------------------

async fn health() -> impl IntoResponse {
    Json(serde_json::json!({ "status": "ok", "service": "storm-server" }))
}

#[derive(Serialize)]
struct TreeResponse {
    notes: Vec<crate::db::NoteRow>,
    folders: Vec<String>,
    seq: i64,
}

async fn tree(State(state): State<Shared>) -> ApiResult<Json<TreeResponse>> {
    let ix = state.indexer.lock().await;
    let notes = ix.db.list_notes()?;

    // Derive folders from note paths — the vault tree has no separate record.
    let mut folders: Vec<String> = notes
        .iter()
        .flat_map(|n| {
            let mut acc = Vec::new();
            let parts: Vec<&str> = n.path.split('/').collect();
            for i in 1..parts.len() {
                acc.push(parts[..i].join("/"));
            }
            acc
        })
        .collect();
    folders.sort();
    folders.dedup();

    let seq = ix.db.latest_seq()?;
    Ok(Json(TreeResponse { notes, folders, seq }))
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
    Query(q): Query<SyncQuery>,
) -> ApiResult<Json<SyncResponse>> {
    let ix = state.indexer.lock().await;
    let changes = ix.db.changes_since(q.since, q.limit.clamp(1, 5000))?;
    let seq = ix.db.latest_seq()?;
    Ok(Json(SyncResponse { changes, seq }))
}

#[derive(Serialize)]
struct NoteResponse {
    #[serde(flatten)]
    note: crate::db::NoteRow,
    content: String,
}

async fn get_note(
    State(state): State<Shared>,
    Path(id): Path<String>,
) -> ApiResult<Json<NoteResponse>> {
    let ix = state.indexer.lock().await;
    let note = ix.db.get_note(&id)?.ok_or_else(|| not_found("no such note"))?;
    let content = ix.vault.read(&note.path)?;
    Ok(Json(NoteResponse { note, content }))
}

#[derive(Deserialize)]
struct CreateBody {
    path: String,
    #[serde(default)]
    content: String,
}

async fn create_note(
    State(state): State<Shared>,
    Json(body): Json<CreateBody>,
) -> ApiResult<Json<crate::index::WriteResult>> {
    let mut ix = state.indexer.lock().await;
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
    Path(id): Path<String>,
    Json(body): Json<PutBody>,
) -> ApiResult<Json<crate::index::WriteResult>> {
    let mut ix = state.indexer.lock().await;
    let result = ix
        .put_note(&id, body.base_version, &body.content, body.device_id.as_deref())
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
    Path(id): Path<String>,
    Json(body): Json<MoveBody>,
) -> ApiResult<Json<crate::index::WriteResult>> {
    let mut ix = state.indexer.lock().await;
    let result = ix
        .move_note(&id, &body.new_path)
        .map_err(|e| bad_request(e.to_string()))?;
    broadcast_latest(&state, &ix, result.seq);
    Ok(Json(result))
}

async fn delete_note(
    State(state): State<Shared>,
    Path(id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let mut ix = state.indexer.lock().await;
    let seq = ix.delete_note(&id).map_err(|e| not_found(e.to_string()))?;
    broadcast_latest(&state, &ix, seq);
    Ok(Json(serde_json::json!({ "seq": seq })))
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
    Query(q): Query<SearchQuery>,
) -> ApiResult<Json<serde_json::Value>> {
    let ix = state.indexer.lock().await;
    // FTS5 treats bare punctuation as syntax; a user typing `foo-bar` should
    // get a search, not a parse error.
    let sanitized = sanitize_fts_query(&q.q);
    if sanitized.is_empty() {
        return Ok(Json(serde_json::json!({ "hits": [] })));
    }
    let hits = ix
        .db
        .search(&sanitized, q.limit.clamp(1, 500))
        .map_err(|e| bad_request(e.to_string()))?;
    Ok(Json(serde_json::json!({ "hits": hits })))
}

/// Quotes each term so FTS5 special characters can't produce a syntax error.
fn sanitize_fts_query(raw: &str) -> String {
    raw.split_whitespace()
        .map(|term| term.replace('"', ""))
        .filter(|t| !t.is_empty())
        .map(|t| format!("\"{t}\""))
        .collect::<Vec<_>>()
        .join(" ")
}

async fn backlinks(
    State(state): State<Shared>,
    Path(id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let ix = state.indexer.lock().await;
    let note = ix.db.get_note(&id)?.ok_or_else(|| not_found("no such note"))?;
    let links = ix.db.backlinks(&note.title)?;
    Ok(Json(serde_json::json!({ "title": note.title, "backlinks": links })))
}

async fn tags(State(state): State<Shared>) -> ApiResult<Json<serde_json::Value>> {
    let ix = state.indexer.lock().await;
    let tags: Vec<_> = ix
        .db
        .all_tags()?
        .into_iter()
        .map(|(tag, count)| serde_json::json!({ "tag": tag, "count": count }))
        .collect();
    Ok(Json(serde_json::json!({ "tags": tags })))
}

async fn notes_by_tag(
    State(state): State<Shared>,
    Path(tag): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let ix = state.indexer.lock().await;
    Ok(Json(serde_json::json!({ "notes": ix.db.notes_with_tag(&tag)? })))
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
    if let Ok(Some(change)) = ix.db.changes_since(seq - 1, 1).map(|c| c.into_iter().next()) {
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
    fn fts_queries_are_sanitized() {
        assert_eq!(sanitize_fts_query("hello world"), "\"hello\" \"world\"");
        // Characters that would otherwise be FTS5 syntax errors.
        assert_eq!(sanitize_fts_query("foo-bar"), "\"foo-bar\"");
        assert_eq!(sanitize_fts_query("NEAR(a b)"), "\"NEAR(a\" \"b)\"");
        assert_eq!(sanitize_fts_query("  "), "");
        assert_eq!(sanitize_fts_query("say \"hi\""), "\"say\" \"hi\"");
    }
}
