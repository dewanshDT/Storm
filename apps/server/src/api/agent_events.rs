//! `GET /v1/agent/events` (decision 154, AM109; freeze §11.1): session state
//! pushed, so a card not on screen follows its session within a second.
//!
//! `snapshot {epoch, sessions}` on connect, the same set as
//! `GET /v1/agent/sessions`; then `session` (one whole view, replacing by
//! id), `removed {id}`, and `resync` (discard; a snapshot follows). No cursor
//! and no `id:`: a reconnect is a new snapshot (SRP §5.3).
//!
//! SSE, or JSON `{event, data}` text frames on a direct WebSocket upgrade,
//! the framing `/v1/stream` has (Q5), so web-direct stays off the browser's
//! six-socket pool.

use std::collections::HashMap;
use std::time::Duration;

use axum::{
    extract::{
        FromRequestParts, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::Serialize;
use tokio::sync::{mpsc, watch};
use tokio::time::Instant;

use super::{ApiError, Shared, wants_websocket};
use crate::auth::authz::Actor;
use crate::ops::SessionView;

/// How the stream paces itself (Q6).
#[derive(Clone, Copy)]
pub(super) struct Pace {
    /// A wake waits this long for the rest of a burst.
    coalesce: Duration,
    /// A session's state changes go at once, then at most this often.
    state: Duration,
    /// A change only to `output_end` / `last_activity` waits this long
    /// after the session's last event.
    output: Duration,
    keepalive: Duration,
}

const PACE: Pace = Pace {
    coalesce: Duration::from_millis(100),
    state: Duration::from_secs(1),
    output: Duration::from_secs(5),
    keepalive: Duration::from_secs(15),
};

/// Bounded like `/v1/stream`'s body: a reader that stops reading gets
/// `resync`, never a growing queue.
const QUEUE: usize = 64;

/// The two fields that move with output alone, and go on the slow lane.
const OUTPUT_FIELDS: [&str; 2] = ["output_end", "last_activity"];

pub(super) enum Frame {
    Event { name: &'static str, data: String },
    Keepalive,
}

impl Frame {
    fn event(name: &'static str, data: String) -> Self {
        Self::Event { name, data }
    }

    fn resync() -> Self {
        Self::event("resync", String::new())
    }

    fn sse(&self) -> String {
        match self {
            Self::Event { name, data } if data.is_empty() => format!("event: {name}\ndata:\n\n"),
            Self::Event { name, data } => format!("event: {name}\ndata: {data}\n\n"),
            Self::Keepalive => ":\n\n".into(),
        }
    }

    fn ws(&self) -> Message {
        match self {
            Self::Event { name, data } => {
                let data = if data.is_empty() { "null" } else { data };
                Message::Text(format!(r#"{{"event":"{name}","data":{data}}}"#).into())
            }
            Self::Keepalive => Message::Ping(Default::default()),
        }
    }
}

pub(super) async fn agent_events(
    State(state): State<Shared>,
    request: axum::extract::Request,
) -> Response {
    let (mut parts, _body) = request.into_parts();
    if !matches!(parts.extensions.get::<Actor>(), Some(Actor::Session { .. })) {
        return ApiError(StatusCode::FORBIDDEN, "owner_only".into()).into_response();
    }
    // Subscribed before the snapshot is built, so no change falls between.
    let changes = state.agent.subscribe_changes();
    let (tx, rx) = mpsc::channel(QUEUE);

    if wants_websocket(&parts) {
        return match WebSocketUpgrade::from_request_parts(&mut parts, &state).await {
            Ok(ws) => ws.on_upgrade(move |socket| async move {
                tokio::select! {
                    () = feed(state, changes, tx, PACE) => {}
                    () = pump_ws(socket, rx) => {}
                }
            }),
            Err(rejection) => rejection.into_response(),
        };
    }

    if let Some(id) = parts.headers.get("last-event-id") {
        tracing::debug!(last_event_id = ?id, "ignoring Last-Event-ID: a reconnect is a new snapshot");
    }
    tokio::spawn(feed(state, changes, tx, PACE));
    let body = tokio_stream::StreamExt::map(tokio_stream::wrappers::ReceiverStream::new(rx), |f| {
        Ok::<_, std::convert::Infallible>(f.sse())
    });
    (
        [
            (header::CONTENT_TYPE, "text/event-stream"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        axum::body::Body::from_stream(body),
    )
        .into_response()
}

async fn pump_ws(mut socket: WebSocket, mut rx: mpsc::Receiver<Frame>) {
    loop {
        tokio::select! {
            frame = rx.recv() => {
                let Some(frame) = frame else { return };
                if socket.send(frame.ws()).await.is_err() {
                    return;
                }
            }
            incoming = socket.recv() => match incoming {
                Some(Ok(Message::Close(_)) | Err(_)) | None => return,
                Some(Ok(_)) => {}
            },
        }
    }
}

enum Stop {
    Overflowed,
    Gone,
}

/// Snapshot, then follow; a reader that fell behind is sent `resync` and a
/// fresh snapshot.
pub(super) async fn feed(
    state: Shared,
    mut changes: watch::Receiver<u64>,
    tx: mpsc::Sender<Frame>,
    pace: Pace,
) {
    let mut sub = Subscriber::default();
    loop {
        let snapshot = match sub.snapshot(&state, &mut changes).await {
            Ok(frame) => frame,
            Err(e) => {
                tracing::warn!(error = %e.1, "agent events: the snapshot failed");
                return;
            }
        };
        if tx.send(snapshot).await.is_err() {
            return;
        }
        match sub.follow(&state, &mut changes, &tx, pace).await {
            Stop::Overflowed => {
                if tx.send(Frame::resync()).await.is_err() {
                    return;
                }
            }
            Stop::Gone => return,
        }
    }
}

struct Sent {
    view: serde_json::Value,
    state: serde_json::Value,
    /// When a state change was last sent; `None` until one was.
    state_at: Option<Instant>,
    /// When this view was last sent, in any event.
    any_at: Instant,
}

#[derive(Default)]
struct Subscriber {
    /// The manager's all-views generation at the last full build.
    all: u64,
    /// Each session's version when its view was last built.
    versions: HashMap<String, u64>,
    sent: HashMap<String, Sent>,
    /// Sessions held back by their lane, and when each may go.
    due: HashMap<String, Instant>,
}

#[derive(Serialize)]
struct Snapshot<'a> {
    epoch: &'a str,
    sessions: &'a [SessionView],
}

fn state_of(view: &serde_json::Value) -> serde_json::Value {
    let mut view = view.clone();
    if let Some(fields) = view.as_object_mut() {
        for field in OUTPUT_FIELDS {
            fields.remove(field);
        }
    }
    view
}

fn removed(id: &str) -> Frame {
    Frame::event("removed", serde_json::json!({ "id": id }).to_string())
}

impl Subscriber {
    async fn snapshot(
        &mut self,
        state: &Shared,
        changes: &mut watch::Receiver<u64>,
    ) -> super::ApiResult<Frame> {
        changes.borrow_and_update();
        self.all = state.agent.views_generation();
        self.versions = state.agent.session_versions();
        let views = crate::ops::list_sessions(state).await?;
        let now = Instant::now();
        self.sent.clear();
        self.due.clear();
        for view in &views {
            let value = serde_json::to_value(view).unwrap_or_default();
            self.sent.insert(
                view.session.id.clone(),
                Sent {
                    state: state_of(&value),
                    view: value,
                    state_at: None,
                    any_at: now,
                },
            );
        }
        let data = serde_json::to_string(&Snapshot {
            epoch: state.agent.epoch(),
            sessions: &views,
        })
        .unwrap_or_default();
        Ok(Frame::event("snapshot", data))
    }

    async fn follow(
        &mut self,
        state: &Shared,
        changes: &mut watch::Receiver<u64>,
        tx: &mpsc::Sender<Frame>,
        pace: Pace,
    ) -> Stop {
        let mut keepalive =
            tokio::time::interval_at(Instant::now() + pace.keepalive, pace.keepalive);
        loop {
            let due = self.due.values().min().copied();
            tokio::select! {
                changed = changes.changed() => {
                    if changed.is_err() {
                        return Stop::Gone;
                    }
                    tokio::time::sleep(pace.coalesce).await;
                }
                () = tokio::time::sleep_until(due.unwrap_or_else(Instant::now)), if due.is_some() => {}
                _ = keepalive.tick() => {
                    // A full queue is already traffic.
                    if let Err(mpsc::error::TrySendError::Closed(_)) = tx.try_send(Frame::Keepalive) {
                        return Stop::Gone;
                    }
                    continue;
                }
                () = tx.closed() => return Stop::Gone,
            }
            for frame in self.scan(state, changes, pace).await {
                match tx.try_send(frame) {
                    Ok(()) => {}
                    Err(mpsc::error::TrySendError::Full(_)) => return Stop::Overflowed,
                    Err(mpsc::error::TrySendError::Closed(_)) => return Stop::Gone,
                }
            }
        }
    }

    /// Rebuilds the views whose session moved, or every view when the
    /// all-views generation did, and says what is now due.
    async fn scan(
        &mut self,
        state: &Shared,
        changes: &mut watch::Receiver<u64>,
        pace: Pace,
    ) -> Vec<Frame> {
        changes.borrow_and_update();
        let all = state.agent.views_generation();
        let versions = state.agent.session_versions();
        let now = Instant::now();
        let mut frames = Vec::new();
        if all != self.all {
            let views = match crate::ops::list_sessions(state).await {
                Ok(views) => views,
                Err(e) => {
                    tracing::warn!(error = %e.1, "agent events: listing sessions failed");
                    return frames;
                }
            };
            self.all = all;
            let listed: std::collections::HashSet<&str> =
                views.iter().map(|v| v.session.id.as_str()).collect();
            let gone: Vec<String> = self
                .sent
                .keys()
                .filter(|id| !listed.contains(id.as_str()))
                .cloned()
                .collect();
            for id in gone {
                self.forget(&id);
                frames.push(removed(&id));
            }
            for view in &views {
                frames.extend(self.offer(view, now, pace));
            }
        } else {
            let moved: Vec<String> = versions
                .iter()
                .filter(|(id, v)| self.versions.get(*id) != Some(v))
                .map(|(id, _)| id.clone())
                .chain(
                    self.due
                        .iter()
                        .filter(|(_, at)| **at <= now)
                        .map(|(id, _)| id.clone()),
                )
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect();
            for id in moved {
                match crate::ops::get_session(state, &id).await {
                    Ok(view) => frames.extend(self.offer(&view, now, pace)),
                    Err(ApiError(StatusCode::NOT_FOUND, _)) => {
                        if self.forget(&id) {
                            frames.push(removed(&id));
                        }
                    }
                    Err(e) => {
                        tracing::warn!(session = %id, error = %e.1, "agent events: a view failed")
                    }
                }
            }
        }
        self.versions = versions;
        frames
    }

    fn forget(&mut self, id: &str) -> bool {
        self.due.remove(id);
        self.versions.remove(id);
        self.sent.remove(id).is_some()
    }

    /// The view as a `session` event if it differs from what was sent and
    /// its lane lets it go now; otherwise held until it may.
    fn offer(&mut self, view: &SessionView, now: Instant, pace: Pace) -> Option<Frame> {
        let id = &view.session.id;
        let value = serde_json::to_value(view).unwrap_or_default();
        let fields = state_of(&value);
        let frame = || Frame::event("session", serde_json::to_string(view).unwrap_or_default());
        let Some(sent) = self.sent.get_mut(id) else {
            self.due.remove(id);
            self.sent.insert(
                id.clone(),
                Sent {
                    view: value,
                    state: fields,
                    state_at: Some(now),
                    any_at: now,
                },
            );
            return Some(frame());
        };
        if sent.view == value {
            self.due.remove(id);
            return None;
        }
        let state_moved = sent.state != fields;
        let at = if state_moved {
            sent.state_at.map_or(now, |t| t + pace.state)
        } else {
            sent.any_at + pace.output
        };
        if at > now {
            self.due.insert(id.clone(), at);
            return None;
        }
        self.due.remove(id);
        sent.view = value;
        sent.state = fields;
        sent.any_at = now;
        if state_moved {
            sent.state_at = Some(now);
        }
        Some(frame())
    }
}

#[cfg(test)]
mod tests;
