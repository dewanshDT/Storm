//! An agent session's upstream MCP sessions (spec §9, decision 81e).
//!
//! **One upstream MCP session per (agent session, connection)**, opened by the
//! agent's `initialize` and held in memory. A server restart loses them all on
//! purpose: the next request finds none and is answered `session_unknown`
//! without being forwarded, which is what lets the bridge replay `initialize`
//! safely (G-D19, bridge rule 2).
//!
//! **Request-scoped messages ride their call's own stream.** Every agent
//! request the gateway forwards gets an `mpsc` channel that becomes the POST's
//! response body. Progress and elicitation for that call are found by the
//! upstream request id rmcp tags them with (`InboundStreamOrigin`, SEP-2260)
//! and written to that channel. When the call ends, its route is removed, so
//! a late progress or elicitation has nowhere to go: "no stale messages after
//! a failure" holds by construction, which is what R8 showed.
//!
//! **Requests go upstream with `send_cancellable_request`**: one send, exactly
//! like the `*_once` helpers, which are thin wrappers over the same call. The
//! gateway needs the request's id and rmcp-assigned progress token at send
//! time, to route its stream; the helpers do not expose them. The re-sending
//! helpers (`call_tool`, …) stay forbidden by
//! `upstream::tests::only_the_once_methods_send_a_request_upstream`.
//!
//! No lock here is held across an `.await`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rmcp::model::{
    ClientCapabilities, ClientInfo, ClientRequest, ElicitRequestParams, ElicitResult,
    ElicitationAction, ProgressNotificationParam, ProgressToken, RequestId,
};
use rmcp::service::{
    InboundStreamOrigin, NotificationContext, Peer, PeerRequestOptions, RequestContext, RoleClient,
    RunningService,
};
use rmcp::{ClientHandler, ErrorData};
use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot};

/// The spec's operational defaults (§9).
pub const CALL_TIMEOUT: Duration = Duration::from_secs(60);
pub const RESPONSE_CAP: usize = 1024 * 1024;

/// The length of `value` serialized, without building the bytes: a result
/// can be up to [`RESPONSE_CAP`], and it is serialized for real on the way out.
pub fn serialized_len(value: &Value) -> usize {
    struct Count(usize);
    impl std::io::Write for Count {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0 += bytes.len();
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut count = Count(0);
    let _ = serde_json::to_writer(&mut count, value);
    count.0
}
pub const PER_SESSION_CONCURRENT: usize = 4;
pub const PER_SESSION_PER_SECOND: usize = 10;
pub const PER_CONNECTION_CONCURRENT: usize = 8;

/// Whether a forwarded method counts against the per-session budget
/// ([`PER_SESSION_CONCURRENT`], [`PER_SESSION_PER_SECOND`]).
///
/// **Listings do not** (AM-G11, approved 2026-10-08): an agent CLI lists
/// every server's tools, prompts and resources in parallel as it starts, so
/// with the built-in connection and two integrations Claude Code's own
/// startup was refused `gateway_rate_limited` and an integration had no tools
/// for the whole session (found in acceptance). A listing executes nothing;
/// it is still bounded per connection ([`PER_CONNECTION_CONCURRENT`]). The
/// budget stays on what it exists for: the work an agent asks an upstream to
/// do. `initialize` and `ping` never reach the limiter at all.
pub fn counts_against_session(method: &str) -> bool {
    !matches!(
        method,
        "tools/list" | "resources/list" | "resources/templates/list" | "prompts/list"
    )
}

/// A line of a POST's response body: a JSON-RPC message for the agent, or a
/// `storm_error` the bridge acts on.
pub type Line = Value;

pub fn message_line(message: Value) -> Line {
    json!({ "message": message })
}

/// A refusal or failure as the agent sees it: one JSON-RPC error with a stable
/// code in `message` and `data.storm_error` (spec §7, §12). Never a 500.
pub fn error_line(id: &Value, code: &str) -> Line {
    message_line(json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": -32001, "message": code, "data": { "storm_error": code } },
    }))
}

/// Strips what the gateway never forwards upstream (§9): `sampling`, `roots`
/// and `elicitation.url`. Never adds anything the agent did not declare.
pub fn upstream_capabilities(agent: &Value) -> Value {
    let mut caps = agent.as_object().cloned().unwrap_or_default();
    caps.remove("sampling");
    caps.remove("roots");
    if let Some(e) = caps.get("elicitation").cloned() {
        let mut e = e.as_object().cloned().unwrap_or_default();
        let url_only = e.contains_key("url") && !e.contains_key("form");
        e.remove("url");
        if url_only {
            // The agent can do URL mode only, which the gateway never offers
            // (G-D23): it gets no elicitation at all.
            caps.remove("elicitation");
        } else {
            caps.insert("elicitation".into(), Value::Object(e));
        }
    }
    Value::Object(caps)
}

fn agent_does_form_elicitation(upstream_caps: &Value) -> bool {
    upstream_caps.get("elicitation").is_some()
}

/// Where a forwarded call's request-scoped messages go.
struct CallRoute {
    tx: mpsc::Sender<Line>,
    /// rmcp's own token on the upstream request, and the agent's, if it asked
    /// for progress at all.
    upstream_progress: ProgressToken,
    agent_progress: Option<Value>,
}

struct PendingElicitation {
    call: RequestId,
    answer: oneshot::Sender<ElicitResult>,
}

/// Something the relay reports outward: unsolicited messages for the link,
/// and audit facts.
pub enum RelayEvent {
    Unsolicited(Value),
    UrlElicitationDeclined,
}

struct RelayState {
    capabilities: ClientCapabilities,
    form_elicitation: bool,
    calls: Mutex<HashMap<RequestId, CallRoute>>,
    elicitations: Mutex<HashMap<String, PendingElicitation>>,
    next_elicitation: AtomicU64,
    /// Random per upstream session, so an elicitation id is never reused —
    /// not after a restart, not after a re-`initialize`. A late answer to an
    /// old elicitation must never be taken for an answer to a new one.
    elicitation_prefix: String,
    events: Box<dyn Fn(RelayEvent) + Send + Sync>,
}

/// The gateway's `ClientHandler` for one upstream session.
#[derive(Clone)]
pub struct Relay(Arc<RelayState>);

impl Relay {
    pub fn new(
        agent_capabilities: &Value,
        events: impl Fn(RelayEvent) + Send + Sync + 'static,
    ) -> Self {
        let forwarded = upstream_capabilities(agent_capabilities);
        let capabilities: ClientCapabilities =
            serde_json::from_value(forwarded.clone()).unwrap_or_default();
        Relay(Arc::new(RelayState {
            form_elicitation: agent_does_form_elicitation(&forwarded),
            capabilities,
            calls: Mutex::new(HashMap::new()),
            elicitations: Mutex::new(HashMap::new()),
            next_elicitation: AtomicU64::new(1),
            elicitation_prefix: {
                use rand::Rng;
                let mut b = [0u8; 6];
                rand::rng().fill_bytes(&mut b);
                data_encoding::HEXLOWER.encode(&b)
            },
            events: Box::new(events),
        }))
    }

    fn route_of(&self, id: &RequestId) -> Option<mpsc::Sender<Line>> {
        self.0.calls.lock().unwrap().get(id).map(|r| r.tx.clone())
    }

    /// Ends a call's route and cancels every elicitation it opened: their
    /// answer senders are dropped, so upstream is told `cancel`.
    fn end_call(&self, id: &RequestId) {
        self.0.calls.lock().unwrap().remove(id);
        self.0
            .elicitations
            .lock()
            .unwrap()
            .retain(|_, p| &p.call != id);
    }

    /// The agent's answer to an elicitation. Unknown or late ids are dropped
    /// and never sent upstream (bridge rule 5 holds here too).
    pub fn answer(&self, agent_id: &str, message: &Value) -> bool {
        let Some(pending) = self.0.elicitations.lock().unwrap().remove(agent_id) else {
            return false;
        };
        let result = message
            .get("result")
            .and_then(|r| serde_json::from_value::<ElicitResult>(r.clone()).ok())
            .unwrap_or_else(cancelled);
        pending.answer.send(result).is_ok()
    }
}

fn cancelled() -> ElicitResult {
    ElicitResult::new(ElicitationAction::Cancel)
}

fn declined() -> ElicitResult {
    ElicitResult::new(ElicitationAction::Decline)
}

impl ClientHandler for Relay {
    fn get_info(&self) -> ClientInfo {
        super::upstream::client_info(self.0.capabilities.clone())
    }

    async fn create_elicitation(
        &self,
        request: ElicitRequestParams,
        context: RequestContext<RoleClient>,
    ) -> Result<ElicitResult, ErrorData> {
        // URL mode never reaches the agent (G-D23): declined and audited.
        if matches!(request, ElicitRequestParams::UrlElicitationParams { .. }) {
            (self.0.events)(RelayEvent::UrlElicitationDeclined);
            return Ok(declined());
        }
        if !self.0.form_elicitation {
            return Ok(declined());
        }
        // Only a request-scoped elicitation has a stream to ride; one on the
        // standalone stream has no call to belong to.
        let Some(InboundStreamOrigin::OutboundRequest(call)) =
            context.extensions.get::<InboundStreamOrigin>().cloned()
        else {
            return Ok(declined());
        };
        // The route is registered right after the request is queued; an
        // elicitation cannot outrun the HTTP round trip, but a short wait
        // makes that a guarantee rather than an assumption.
        let mut tx = self.route_of(&call);
        for _ in 0..50 {
            if tx.is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
            tx = self.route_of(&call);
        }
        let Some(tx) = tx else {
            return Ok(cancelled());
        };
        let n = self.0.next_elicitation.fetch_add(1, Ordering::Relaxed);
        let agent_id = format!("storm-elicit-{}-{n}", self.0.elicitation_prefix);
        let (answer, rx) = oneshot::channel();
        self.0.elicitations.lock().unwrap().insert(
            agent_id.clone(),
            PendingElicitation {
                call: call.clone(),
                answer,
            },
        );
        let line = message_line(json!({
            "jsonrpc": "2.0",
            "id": agent_id,
            "method": "elicitation/create",
            "params": request,
        }));
        if tx.send(line).await.is_err() {
            self.0.elicitations.lock().unwrap().remove(&agent_id);
            return Ok(cancelled());
        }
        tokio::select! {
            r = rx => Ok(r.unwrap_or_else(|_| cancelled())),
            _ = context.ct.cancelled() => {
                self.0.elicitations.lock().unwrap().remove(&agent_id);
                Ok(cancelled())
            }
        }
    }

    async fn on_progress(
        &self,
        params: ProgressNotificationParam,
        _context: NotificationContext<RoleClient>,
    ) {
        let target = {
            let calls = self.0.calls.lock().unwrap();
            calls
                .values()
                .find(|r| r.upstream_progress == params.progress_token)
                .and_then(|r| r.agent_progress.clone().map(|t| (r.tx.clone(), t)))
        };
        // An agent that did not ask for progress gets none.
        if let Some((tx, token)) = target {
            let mut p = serde_json::to_value(&params).unwrap_or_default();
            p["progressToken"] = token;
            let _ = tx
                .send(message_line(json!({
                    "jsonrpc": "2.0",
                    "method": "notifications/progress",
                    "params": p,
                })))
                .await;
        }
    }

    async fn on_tool_list_changed(&self, _context: NotificationContext<RoleClient>) {
        (self.0.events)(RelayEvent::Unsolicited(
            json!({"jsonrpc": "2.0", "method": "notifications/tools/list_changed"}),
        ));
    }

    async fn on_resource_list_changed(&self, _context: NotificationContext<RoleClient>) {
        (self.0.events)(RelayEvent::Unsolicited(
            json!({"jsonrpc": "2.0", "method": "notifications/resources/list_changed"}),
        ));
    }

    async fn on_prompt_list_changed(&self, _context: NotificationContext<RoleClient>) {
        (self.0.events)(RelayEvent::Unsolicited(
            json!({"jsonrpc": "2.0", "method": "notifications/prompts/list_changed"}),
        ));
    }

    async fn on_resource_updated(
        &self,
        params: rmcp::model::ResourceUpdatedNotificationParam,
        _context: NotificationContext<RoleClient>,
    ) {
        (self.0.events)(RelayEvent::Unsolicited(json!({
            "jsonrpc": "2.0",
            "method": "notifications/resources/updated",
            "params": params,
        })));
    }
}

/// One live upstream MCP session.
pub struct Upstream {
    pub relay: Relay,
    peer: Peer<RoleClient>,
    /// Ends the upstream session now. A closed entry may still be held by an
    /// in-flight call; cancelling (rather than waiting for the last `Arc`)
    /// is what makes that call fail at once instead of finishing against a
    /// connection that was disconnected or disabled.
    stop: Mutex<Option<rmcp::service::RunningServiceCancellationToken>>,
    /// Held so the session lives; dropping it cancels the upstream session.
    _service: RunningService<RoleClient, Relay>,
}

impl Upstream {
    pub fn new(service: RunningService<RoleClient, Relay>) -> Self {
        Upstream {
            relay: service.service().clone(),
            peer: service.peer().clone(),
            stop: Mutex::new(Some(service.cancellation_token())),
            _service: service,
        }
    }

    pub fn close(&self) {
        if let Some(token) = self.stop.lock().unwrap().take() {
            token.cancel();
        }
    }

    /// Forwards one agent request upstream, at most once, writing every
    /// request-scoped message and then the final answer to `tx`. Returns the
    /// result (or the error code) for the caller's audit and filtering.
    pub async fn forward(
        &self,
        request: ClientRequest,
        agent_progress: Option<Value>,
        tx: mpsc::Sender<Line>,
    ) -> Result<Value, ForwardError> {
        let options = PeerRequestOptions::with_timeout(CALL_TIMEOUT);
        let handle = self
            .peer
            .send_cancellable_request(request, options)
            .await
            .map_err(|e| ForwardError::Upstream(super::upstream::classify(&e)))?;
        let id = handle.id.clone();
        self.relay.0.calls.lock().unwrap().insert(
            id.clone(),
            CallRoute {
                tx: tx.clone(),
                upstream_progress: handle.progress_token.clone(),
                agent_progress,
            },
        );
        let peer = self.peer.clone();
        let cancel_id = id.clone();
        let outcome = tokio::select! {
            r = handle.await_response() => match r {
                Ok(result) => serde_json::to_value(result)
                    .map_err(|_| ForwardError::Upstream(super::upstream::UpstreamError::Protocol)),
                Err(rmcp::ServiceError::McpError(e)) => Err(ForwardError::Rpc(e)),
                Err(rmcp::ServiceError::Timeout { .. }) => {
                    Err(ForwardError::Upstream(super::upstream::UpstreamError::Unavailable))
                }
                Err(e) => Err(ForwardError::Upstream(super::upstream::classify(&e))),
            },
            // The host gave up on the POST: the call has failed from the
            // agent's side, so upstream is told to stop. Never retried.
            _ = tx.closed() => {
                let _ = peer
                    .notify_cancelled(rmcp::model::CancelledNotificationParam::new(
                        Some(cancel_id),
                        Some("the agent's call ended".into()),
                    ))
                    .await;
                Err(ForwardError::Upstream(super::upstream::UpstreamError::Unavailable))
            }
        };
        self.relay.end_call(&id);
        outcome
    }

    /// Forwards an agent notification (`notifications/cancelled` and the
    /// like). Best effort.
    pub async fn notify(&self, notification: rmcp::model::ClientNotification) {
        let _ = self.peer.send_notification(notification).await;
    }
}

pub enum ForwardError {
    /// The upstream answered with a JSON-RPC error: passed through.
    Rpc(ErrorData),
    Upstream(super::upstream::UpstreamError),
}

/// The live upstream sessions and the §9 limits.
#[derive(Default)]
pub struct Sessions {
    live: Mutex<HashMap<(String, String), Arc<Upstream>>>,
    session_slots: Mutex<HashMap<String, Arc<tokio::sync::Semaphore>>>,
    connection_slots: Mutex<HashMap<String, Arc<tokio::sync::Semaphore>>>,
    session_rate: Mutex<HashMap<String, Vec<Instant>>>,
}

/// Held for a forwarded call's duration.
pub struct Permit {
    /// `None` for a listing (AM-G11).
    _session: Option<tokio::sync::OwnedSemaphorePermit>,
    _connection: tokio::sync::OwnedSemaphorePermit,
}

impl Sessions {
    pub fn get(&self, session: &str, connection: &str) -> Option<Arc<Upstream>> {
        self.live
            .lock()
            .unwrap()
            .get(&(session.to_string(), connection.to_string()))
            .cloned()
    }

    /// A fresh `initialize` replaces any previous upstream session.
    pub fn put(&self, session: &str, connection: &str, upstream: Upstream) -> Arc<Upstream> {
        let upstream = Arc::new(upstream);
        let previous = self.live.lock().unwrap().insert(
            (session.to_string(), connection.to_string()),
            upstream.clone(),
        );
        if let Some(previous) = previous {
            previous.close();
        }
        upstream
    }

    /// Closes every upstream session an agent session holds.
    pub fn close_session(&self, session: &str) {
        self.take(|(s, _)| s == session);
        self.session_slots.lock().unwrap().remove(session);
        self.session_rate.lock().unwrap().remove(session);
    }

    /// Closes every upstream session on a connection (disconnect, disable).
    pub fn close_connection(&self, connection: &str) {
        self.take(|(_, c)| c == connection);
    }

    /// Removes the matching entries and cancels them, outside the lock.
    fn take(&self, matches: impl Fn(&(String, String)) -> bool) {
        let removed: Vec<Arc<Upstream>> = {
            let mut live = self.live.lock().unwrap();
            let keys: Vec<(String, String)> = live.keys().filter(|k| matches(k)).cloned().collect();
            keys.iter().filter_map(|k| live.remove(k)).collect()
        };
        for upstream in removed {
            upstream.close();
        }
    }

    /// The agent sessions that hold upstream sessions, for sweeping.
    pub fn sessions(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .live
            .lock()
            .unwrap()
            .keys()
            .map(|(s, _)| s.clone())
            .collect();
        out.sort();
        out.dedup();
        out
    }

    /// Takes a slot for one forwarded call of `method`, or `None` when a
    /// limit is hit (`gateway_rate_limited`). Never waits: an agent over its
    /// budget is told at once rather than queued. A listing takes only its
    /// connection's slot ([`counts_against_session`]).
    pub fn permit(&self, session: &str, connection: &str, method: &str) -> Option<Permit> {
        let c = self
            .connection_slots
            .lock()
            .unwrap()
            .entry(connection.to_string())
            .or_insert_with(|| Arc::new(tokio::sync::Semaphore::new(PER_CONNECTION_CONCURRENT)))
            .clone();
        if !counts_against_session(method) {
            return Some(Permit {
                _session: None,
                _connection: c.try_acquire_owned().ok()?,
            });
        }
        let s = self
            .session_slots
            .lock()
            .unwrap()
            .entry(session.to_string())
            .or_insert_with(|| Arc::new(tokio::sync::Semaphore::new(PER_SESSION_CONCURRENT)))
            .clone();
        let permit = Permit {
            _session: Some(s.try_acquire_owned().ok()?),
            _connection: c.try_acquire_owned().ok()?,
        };
        // Only a call that will run counts against the per-second budget.
        let mut rate = self.session_rate.lock().unwrap();
        let window = rate.entry(session.to_string()).or_default();
        let now = Instant::now();
        window.retain(|t| now.duration_since(*t) < Duration::from_secs(1));
        if window.len() >= PER_SESSION_PER_SECOND {
            return None;
        }
        window.push(now);
        Some(permit)
    }
}

/// An id as the agent sent it, for echoing back.
pub fn agent_id(message: &Value) -> Value {
    message.get("id").cloned().unwrap_or(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sampling_roots_and_url_elicitation_never_go_upstream() {
        // Claude Code 2.1.289's initialize (G2).
        let claude = json!({"roots": {"listChanged": true}, "elicitation": {"form": {}, "url": {}}, "sampling": {}});
        assert_eq!(
            upstream_capabilities(&claude),
            json!({"elicitation": {"form": {}}})
        );
        // OpenCode 1.18.31's (G2): nothing is added.
        assert_eq!(upstream_capabilities(&json!({"roots": {}})), json!({}));
        // URL-only: no elicitation at all.
        assert_eq!(
            upstream_capabilities(&json!({"elicitation": {"url": {}}})),
            json!({})
        );
        // The legacy empty object means form.
        assert_eq!(
            upstream_capabilities(&json!({"elicitation": {}})),
            json!({"elicitation": {}})
        );
    }

    #[test]
    fn the_limits_refuse_rather_than_queue() {
        let s = Sessions::default();
        let held: Vec<Permit> = (0..PER_SESSION_CONCURRENT)
            .map(|_| s.permit("ags_A", "mcc_A", "tools/call").unwrap())
            .collect();
        assert!(
            s.permit("ags_A", "mcc_A", "tools/call").is_none(),
            "a fifth concurrent call"
        );
        drop(held);
        // Ten per second: four used above, six more, then refused.
        for _ in 0..(PER_SESSION_PER_SECOND - PER_SESSION_CONCURRENT) {
            assert!(s.permit("ags_A", "mcc_A", "tools/call").is_some());
        }
        assert!(
            s.permit("ags_A", "mcc_A", "tools/call").is_none(),
            "an eleventh call this second"
        );
        // Another session has its own budget.
        assert!(s.permit("ags_B", "mcc_A", "tools/call").is_some());
    }

    #[test]
    fn an_agents_startup_listings_are_never_refused_by_the_session_budget() {
        // AM-G11: what Claude Code does as it starts. Three connections, each
        // listed three ways in parallel, while the session's budget is spent.
        let s = Sessions::default();
        let held: Vec<Permit> = (0..PER_SESSION_CONCURRENT)
            .map(|_| s.permit("ags_A", "storm", "tools/call").unwrap())
            .collect();
        let mut listings = Vec::new();
        for connection in ["storm", "mcc_A", "mcc_B"] {
            for method in ["tools/list", "prompts/list", "resources/list"] {
                listings.push(
                    s.permit("ags_A", connection, method)
                        .unwrap_or_else(|| panic!("{method} on {connection} was refused")),
                );
            }
        }
        // ...and they took nothing from it: execution is still refused.
        assert!(s.permit("ags_A", "mcc_A", "tools/call").is_none());
        drop(held);
        drop(listings);
    }

    #[test]
    fn listings_are_still_bounded_per_connection() {
        let s = Sessions::default();
        let held: Vec<Permit> = (0..PER_CONNECTION_CONCURRENT)
            .map(|_| s.permit("ags_A", "mcc_A", "tools/list").unwrap())
            .collect();
        assert!(s.permit("ags_B", "mcc_A", "tools/list").is_none());
        assert!(s.permit("ags_A", "mcc_B", "tools/list").is_some());
        drop(held);
    }

    #[test]
    fn only_the_listings_are_exempt() {
        for method in [
            "tools/list",
            "prompts/list",
            "resources/list",
            "resources/templates/list",
        ] {
            assert!(!counts_against_session(method), "{method}");
        }
        for method in [
            "tools/call",
            "resources/read",
            "resources/subscribe",
            "resources/unsubscribe",
            "prompts/get",
            "completion/complete",
        ] {
            assert!(counts_against_session(method), "{method}");
        }
    }
}
