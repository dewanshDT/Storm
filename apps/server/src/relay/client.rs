//! One trunk: connect, register, and serve tunnelled requests.
//!
//! **Nothing here may ever slow a LAN request down.** That is the load-bearing
//! constraint, and in-process dispatch is exactly where it goes wrong. Three
//! rules keep it true, and each is a place this file deliberately refuses a
//! simpler shape:
//!
//! 1. **Relayed work runs on its own spawned tasks.** The read loop parses a
//!    frame and hands off; it never awaits a router call, so one slow handler
//!    cannot stall the trunk and no relayed work is ever inlined onto a path a
//!    LAN request travels.
//! 2. **Every queue is bounded.** A relay that stops reading makes response
//!    tasks block on a full channel — backpressure — instead of buffering the
//!    vault into memory.
//! 3. **Reconnection is isolated from serving.** The supervisor owns the
//!    backoff; a connection that dies takes its own tasks with it and leaves
//!    the listener untouched.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use axum::Router;
use axum::body::{Body, Bytes};
use axum::http::HeaderValue;
use futures_util::{SinkExt, StreamExt};
use tokio::sync::{mpsc, watch};
use tokio_tungstenite::tungstenite::Message;

use super::dispatch;
use super::proto::{self, Inbound, RequestHead};
use crate::auth::ServerIdentity;
use crate::registry::RegisteredRelays;

/// Reconnect backoff bounds. Doubling, no jitter: a server has a handful of
/// relays, not a herd, and a deterministic schedule is one a test can assert.
const BACKOFF_MIN: Duration = Duration::from_secs(1);
const BACKOFF_MAX: Duration = Duration::from_secs(60);

/// Outbound control and body frames waiting for the socket.
///
/// Bounded, so a relay that stops reading applies backpressure to the response
/// tasks rather than letting them buffer without limit. 256 frames is a few
/// hundred KiB of body chunks in flight — enough that a healthy trunk never
/// touches the bound.
pub(super) const OUTBOUND_CAPACITY: usize = 256;

/// Request-body bytes one stream may have buffered and not yet read by its
/// handler before the stream is shed (decision 102).
///
/// A sender is never slowed down — awaiting in the trunk's read loop would
/// stall every other stream — so a client uploads at line rate and the
/// handler catches up from this buffer. It must therefore hold a whole
/// attachment: anything less sheds an ordinary upload whose handler starts
/// late. One more maximal WebSocket frame covers the frame that crosses the
/// line; a body that really is larger is refused by the route's own limit.
pub(super) const STREAM_BODY_BUDGET: usize = crate::api::MAX_ATTACHMENT_BYTES + 16 * 1024 * 1024;

/// Request-body bytes all of one trunk's streams may have buffered together.
///
/// Two maximal attachments in flight at once. Without a trunk-wide bound, the
/// per-stream one times the relay's stream cap (1000) is no bound at all.
const TRUNK_BODY_BUDGET: usize = 2 * crate::api::MAX_ATTACHMENT_BYTES;

/// How long registration may take before the attempt is abandoned. Covers the
/// whole handshake, not one frame, so a relay that accepts a socket and then
/// says nothing is a failed attempt rather than a stuck task.
const REGISTRATION_TIMEOUT: Duration = Duration::from_secs(20);

/// How long the TCP connect and WebSocket handshake together may take.
///
/// Separate from [`REGISTRATION_TIMEOUT`], which starts only once there is a
/// socket to send `REGISTER_SERVER` on. Shorter, because nothing here waits on
/// an Argon2 verify or any other real work — it is a handshake or it is a
/// wedged peer.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Unsent bytes the kernel may hold for a trunk socket (§5.2 "Control
/// before bulk", decision 116b). The priority lane only reorders what has not
/// reached the socket; on a slow uplink an autotuned send buffer holds
/// seconds of body, and a `STREAM_ACK` or `PING` written after it waits that
/// long. This bounds only what is unsent, not what is in flight, so it costs
/// no throughput. Linux and Android only; elsewhere the kernel default stands.
#[cfg(any(target_os = "linux", target_os = "android"))]
const UNSENT_LOWAT: u32 = 128 * 1024;

fn bound_unsent(socket: &tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>) {
    let tcp = match socket {
        tokio_tungstenite::MaybeTlsStream::Plain(tcp) => tcp,
        tokio_tungstenite::MaybeTlsStream::Rustls(tls) => tls.get_ref().0,
        _ => return,
    };
    #[cfg(any(target_os = "linux", target_os = "android"))]
    if let Err(e) = socket2::SockRef::from(tcp).set_tcp_notsent_lowat(UNSENT_LOWAT) {
        tracing::debug!(error = %e, "setting TCP_NOTSENT_LOWAT");
    }
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    let _ = tcp;
}

/// How long a closing trunk waits for queued frames to reach the socket.
const WRITER_DRAIN: Duration = Duration::from_secs(1);

/// How long an acknowledged stream may wait for its head before the origin
/// aborts it (§5.1). A client that died between `STREAM_READY` and its head
/// would otherwise hold the id for the life of the trunk.
#[cfg(not(test))]
pub(super) const HEAD_WAIT: Duration = Duration::from_secs(30);
#[cfg(test)]
pub(super) const HEAD_WAIT: Duration = Duration::from_secs(3);

/// Everything a trunk needs to serve. Cheap to clone — the router is `Router<()>`
/// over an `Arc` state, which is what makes in-process dispatch free.
#[derive(Clone)]
pub struct Tunnel {
    pub identity: Arc<ServerIdentity>,
    pub router: Router,
    pub registered: RegisteredRelays,
    /// The server's configured bind address, stamped onto every relayed
    /// request's `Host`. See `dispatch::build_request`.
    pub host_header: HeaderValue,
}

/// How far an attempt got, which is what decides whether backoff resets.
enum Outcome {
    /// Registered, then the trunk ended. The relay is healthy and reachable,
    /// so the next attempt starts from the minimum delay.
    ServedThenClosed,
    /// Never registered.
    Failed,
}

/// Runs one relay forever: connect, register, serve, reconnect.
///
/// Returns when `shutdown` fires, having sent `DEREGISTER` if a trunk was live
/// — the relay then frees the `server_id` immediately instead of making every
/// client wait out a heartbeat timeout.
pub async fn supervise(relay_url: String, tunnel: Tunnel, mut shutdown: watch::Receiver<bool>) {
    let mut backoff = BACKOFF_MIN;

    loop {
        if *shutdown.borrow() {
            break;
        }

        // No shutdown arm racing `run_once` here, deliberately. `serve_trunk`
        // watches the same channel and answers a shutdown by sending
        // DEREGISTER and CLOSE before returning — so a `select!` that also
        // watched it would cancel that future the moment shutdown fired, drop
        // the socket mid-handshake, and leave the relay holding the
        // `server_id` until a heartbeat timeout. Which is the outage
        // DEREGISTER exists to prevent.
        //
        // The cost is that a shutdown arriving *during* connect or
        // registration waits out that phase's timeout. That is bounded and
        // rare: a supervisor with no trunk spends its time in the backoff
        // sleep below, which does have a shutdown arm.
        let outcome = run_once(&relay_url, &tunnel, &mut shutdown).await;

        // Registration is a live fact about a connection. The moment the trunk
        // is gone the relay stops being advertised, or `/v1/server` sends
        // clients down a dead path and they burn their connection race on it.
        tunnel.registered.mark_unregistered(&relay_url);

        match outcome {
            Outcome::ServedThenClosed => {
                tracing::info!(relay = %relay_url, "relay trunk closed, reconnecting");
                backoff = BACKOFF_MIN;
            }
            Outcome::Failed => {
                tracing::warn!(
                    relay = %relay_url,
                    retry_in_secs = backoff.as_secs(),
                    "relay registration failed"
                );
            }
        }

        if *shutdown.borrow() {
            break;
        }
        tokio::select! {
            _ = shutdown.changed() => break,
            _ = tokio::time::sleep(backoff) => {}
        }
        backoff = (backoff * 2).min(BACKOFF_MAX);
    }

    tunnel.registered.mark_unregistered(&relay_url);
    tracing::info!(relay = %relay_url, "relay supervisor stopped");
}

/// The `ws(s)://<relay-host>/register` endpoint (§4.0).
fn register_url(relay_url: &str) -> String {
    format!("{}/register", relay_url.trim_end_matches('/'))
}

async fn run_once(
    relay_url: &str,
    tunnel: &Tunnel,
    shutdown: &mut watch::Receiver<bool>,
) -> Outcome {
    let url = register_url(relay_url);

    // Bounded, because `connect_async` is not. A host that completes the TCP
    // handshake and then says nothing leaves it pending for ever — the
    // supervisor never retries, never backs off, and never returns, so a clean
    // shutdown waits on it indefinitely. Unreachable and silent must cost the
    // same.
    //
    // Shutdown may cancel this await, and that is the rule the whole supervisor
    // turns on: **a shutdown may cancel the phases before registration, never
    // the serving phase.** Nothing here has been announced to the relay, so
    // dropping it costs nothing. `serve_trunk` is the opposite — it owes the
    // relay a DEREGISTER — which is why nothing races *it*.
    let connected = tokio::select! {
        biased;
        _ = shutdown.changed() => return Outcome::Failed,
        // Nagle off: a relayed exchange is several small writes, and with it on
        // each waits out the relay's delayed ACK, ~40 ms a request (decision 91).
        result = tokio::time::timeout(
            CONNECT_TIMEOUT,
            tokio_tungstenite::connect_async_with_config(&url, None, true),
        ) => {
            match result {
                Ok(result) => result,
                Err(_) => {
                    tracing::warn!(relay = %relay_url, "connecting to relay timed out");
                    return Outcome::Failed;
                }
            }
        }
    };

    let socket = match connected {
        Ok((socket, _)) => {
            bound_unsent(socket.get_ref());
            socket
        }
        Err(e) => {
            // `wss://` without a TLS feature fails here with a message that
            // does not obviously name the cause, so it is called out.
            if matches!(
                e,
                tokio_tungstenite::tungstenite::Error::Url(
                    tokio_tungstenite::tungstenite::error::UrlError::TlsFeatureNotEnabled
                )
            ) {
                tracing::error!(
                    relay = %relay_url,
                    "this build has no TLS support, so it cannot dial a wss:// relay"
                );
            } else {
                tracing::warn!(relay = %relay_url, error = %e, "connecting to relay");
            }
            return Outcome::Failed;
        }
    };

    let (mut sink, mut stream) = socket.split();

    // ---- registration (§4.1) ------------------------------------------
    // Cancellable for the same reason as the connect above: the relay has not
    // yet answered `REGISTERED`, so there is no trunk to take down politely.
    let attempt = tokio::select! {
        biased;
        _ = shutdown.changed() => return Outcome::Failed,
        result = tokio::time::timeout(
            REGISTRATION_TIMEOUT,
            register(&mut sink, &mut stream, tunnel),
        ) => result,
    };

    let registered = match attempt {
        Ok(Ok(registered)) => registered,
        Ok(Err(e)) => {
            tracing::warn!(relay = %relay_url, reason = %e, "relay refused registration");
            return Outcome::Failed;
        }
        Err(_) => {
            tracing::warn!(relay = %relay_url, "relay registration timed out");
            return Outcome::Failed;
        }
    };

    if let Err(e) = tunnel.registered.mark_registered(relay_url) {
        // A URL the registry will not normalise cannot be advertised, so the
        // trunk is useless even though the relay accepted it.
        tracing::error!(relay = %relay_url, error = %e, "refusing to advertise relay");
        return Outcome::Failed;
    }
    tracing::info!(
        relay = %relay_url,
        trunk_id = %registered.trunk_id,
        public_address = %registered.public_address,
        "registered with relay"
    );

    serve_trunk(
        sink,
        stream,
        tunnel,
        shutdown,
        registered
            .heartbeat_interval_secs
            .unwrap_or(proto::DEFAULT_HEARTBEAT_SECS),
    )
    .await;

    Outcome::ServedThenClosed
}

type Sink = futures_util::stream::SplitSink<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    Message,
>;
type Stream = futures_util::stream::SplitStream<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
>;

/// `REGISTER_SERVER` → `CHALLENGE` → `CHALLENGE_RESPONSE` → `REGISTERED`.
async fn register(
    sink: &mut Sink,
    stream: &mut Stream,
    tunnel: &Tunnel,
) -> Result<proto::Registered, String> {
    let identity = &tunnel.identity;
    sink.send(Message::text(proto::register_server(
        &identity.server_id,
        &identity.public_key_b64(),
    )))
    .await
    .map_err(|e| format!("sending REGISTER_SERVER: {e}"))?;

    loop {
        let message = stream
            .next()
            .await
            .ok_or_else(|| "relay closed during registration".to_string())?
            .map_err(|e| format!("reading during registration: {e}"))?;

        let text = match message {
            Message::Text(text) => text,
            Message::Ping(_) | Message::Pong(_) => continue,
            Message::Close(_) => return Err("relay closed during registration".into()),
            // Nothing binary is defined before a trunk exists.
            _ => continue,
        };

        match proto::parse_text(&text) {
            Ok(Inbound::Challenge { nonce }) => {
                // `sign_relay_auth` validates both the nonce *and* our own
                // `server_id`: they are two colon-delimited fields in one
                // signed string, so constraining one leaves the split
                // ambiguous. A refusal here is a relay sending something this
                // server will not sign, which is a failed registration and not
                // something to work around.
                let sig = identity
                    .sign_relay_auth(&nonce)
                    .map_err(|e| format!("refusing to sign the relay's challenge: {e}"))?;
                sink.send(Message::text(proto::challenge_response(&sig)))
                    .await
                    .map_err(|e| format!("sending CHALLENGE_RESPONSE: {e}"))?;
            }
            Ok(Inbound::Registered(registered)) => return Ok(registered),
            Ok(Inbound::Error(e)) => {
                return Err(format!("relay error {}: {:?}", e.code, e.message));
            }
            Ok(Inbound::Ping) => {
                sink.send(Message::text(proto::pong()))
                    .await
                    .map_err(|e| format!("sending PONG: {e}"))?;
            }
            Ok(_) => {} // Nothing else is meaningful before REGISTERED.
            Err(_) => return Err("malformed frame during registration".into()),
        }
    }
}

/// One live stream's hold on the trunk.
struct StreamEntry {
    /// `None` once the request head has arrived and the body is complete, or
    /// for a stream whose request carries no body at all.
    body: Option<BodySender>,
    /// Dropped on close, which aborts the dispatch task with it.
    task: Option<tokio::task::AbortHandle>,
    /// A head is accepted once per stream.
    head_seen: bool,
    /// When `STREAM_ACK` was queued, for [`HEAD_WAIT`].
    acked_at: tokio::time::Instant,
    /// Tells this stream's own end apart from a later stream reusing its id.
    generation: u64,
}

/// A dispatch task that has sent its stream's last frame, or been dropped.
/// The read loop forgets the stream (§5.1) only if the id still belongs to
/// that dispatch.
struct Finished {
    stream_id: u32,
    generation: u64,
    tx: mpsc::UnboundedSender<(u32, u64)>,
}

impl Drop for Finished {
    fn drop(&mut self) {
        let _ = self.tx.send((self.stream_id, self.generation));
    }
}

async fn serve_trunk(
    sink: Sink,
    mut stream: Stream,
    tunnel: &Tunnel,
    shutdown: &mut watch::Receiver<bool>,
    heartbeat_secs: u64,
) {
    let (outbound, outbound_rx) = mpsc::channel::<Message>(OUTBOUND_CAPACITY);
    let control = Arc::new(Control::default());

    // The socket has exactly one writer. Every other task reaches it through
    // the bounded channel, which is what turns a slow relay into backpressure
    // instead of an unbounded buffer.
    let writer = tokio::spawn(write_loop(sink, outbound_rx, control.clone()));

    let period = Duration::from_secs(heartbeat_secs.max(1));
    let heartbeat = tokio::spawn({
        let control = control.clone();
        async move {
            let mut ticker = tokio::time::interval(period);
            ticker.tick().await; // fires immediately; skip it
            loop {
                ticker.tick().await;
                control.ping();
            }
        }
    });

    let mut streams: HashMap<u32, StreamEntry> = HashMap::new();
    let (finished_tx, mut finished) = mpsc::unbounded_channel::<(u32, u64)>();
    let mut generation = 0u64;
    let silence_limit = liveness_deadline(heartbeat_secs);
    let mut liveness_check = tokio::time::interval(period.min(Duration::from_secs(1)));
    liveness_check.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    control.heard();

    let budget = BodyBudget::new(STREAM_BODY_BUDGET, TRUNK_BODY_BUDGET);

    loop {
        let message = tokio::select! {
            biased;
            _ = shutdown.changed() => {
                // Clean shutdown: tell the relay to free the `server_id` now
                // rather than after a heartbeat timeout.
                control.push(Message::text(proto::deregister()));
                control.push(Message::text(proto::close_trunk()));
                // Give the writer a moment to flush before the socket drops.
                tokio::time::sleep(Duration::from_millis(50)).await;
                break;
            }
            // The relay answers every PING, so silence this long is a relay
            // that is gone without the socket saying so: a network change, a
            // sleep, a NAT that dropped the mapping (decision 94). Waiting for
            // TCP to notice keeps the relay advertised for many minutes.
            _ = liveness_check.tick() => {
                if control.is_silent(silence_limit, period) {
                    tracing::warn!(
                        silent_secs = silence_limit.as_secs(),
                        "relay silent past the heartbeat deadline; dropping the trunk"
                    );
                    break;
                }
                abort_headless(&mut streams, &control);
                continue;
            }
            // Ahead of the socket: a dispatch queues its last frame only after
            // saying so here, so the id is forgotten before the relay can see
            // that frame and reuse it.
            Some((stream_id, generation)) = finished.recv() => {
                if streams.get(&stream_id).is_some_and(|e| e.generation == generation) {
                    streams.remove(&stream_id);
                }
                continue;
            }
            message = stream.next() => match message {
                Some(Ok(message)) => message,
                Some(Err(e)) => {
                    tracing::warn!(error = %e, "relay trunk read failed");
                    break;
                }
                None => {
                    tracing::warn!("relay closed the trunk");
                    break;
                }
            },
        };
        control.heard();

        match message {
            Message::Text(text) => {
                let trunk = Trunk {
                    outbound: &outbound,
                    control: &control,
                    tunnel,
                    budget: &budget,
                    finished: &finished_tx,
                };
                if !handle_control(&text, &mut streams, &trunk, &mut generation).await {
                    break;
                }
            }
            Message::Binary(bytes) => handle_body_frame(&bytes, &mut streams, &control),
            Message::Close(_) => break,
            // tungstenite answers protocol-level pings itself.
            Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => {}
        }
    }

    // Dropping the map aborts every dispatch task and closes every body
    // channel, so no relayed work outlives the trunk that asked for it.
    for (_, entry) in streams.drain() {
        if let Some(task) = entry.task {
            task.abort();
        }
    }
    heartbeat.abort();
    drop(outbound);
    // A writer blocked on a peer that stopped reading would hold the
    // supervisor here, and with it the reconnect.
    let writer_abort = writer.abort_handle();
    if tokio::time::timeout(WRITER_DRAIN, writer).await.is_err() {
        writer_abort.abort();
    }
}

/// Three missed heartbeats, the same allowance the relay gives a server
/// (§4.2's 45 s at the default 15 s interval).
fn liveness_deadline(heartbeat_secs: u64) -> Duration {
    Duration::from_secs(heartbeat_secs.max(1) * 3)
}

/// Frames that go ahead of every queued body frame and are never dropped for
/// want of room (SRP §5.2 "Control before bulk", decision 116b): `PING`,
/// `PONG`, `STREAM_ACK`, `CLOSE`, `DEREGISTER`.
///
/// Kept outside the bounded queue because the bounded queue is the backlog: a
/// `STREAM_ACK` behind 64 MiB of download misses the relay's 5 s timeout, and
/// a `PING` behind it lets the silence deadline fire on a relay that is fine.
/// The end of a response is not here; its `CLOSE` stays behind its body.
#[derive(Default)]
struct Control {
    state: std::sync::Mutex<ControlState>,
    notify: tokio::sync::Notify,
}

#[derive(Default)]
struct ControlState {
    /// Each frame, and whether it is the heartbeat `PING`.
    queue: VecDeque<(Message, bool)>,
    ping_unwritten: bool,
    last_heard: Option<tokio::time::Instant>,
    /// When the first `PING` since the relay last spoke reached the socket.
    ping_written: Option<tokio::time::Instant>,
}

impl Control {
    fn lock(&self) -> std::sync::MutexGuard<'_, ControlState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn push(&self, message: Message) {
        self.lock().queue.push_back((message, false));
        self.notify.notify_one();
    }

    /// At most one `PING` waits at a time.
    fn ping(&self) {
        let mut state = self.lock();
        if state.ping_unwritten {
            return;
        }
        state.ping_unwritten = true;
        state.queue.push_back((Message::text(proto::ping()), true));
        drop(state);
        self.notify.notify_one();
    }

    fn pop(&self) -> Option<(Message, bool)> {
        self.lock().queue.pop_front()
    }

    fn ping_written(&self) {
        let mut state = self.lock();
        state.ping_unwritten = false;
        state
            .ping_written
            .get_or_insert_with(tokio::time::Instant::now);
    }

    fn heard(&self) {
        let mut state = self.lock();
        state.last_heard = Some(tokio::time::Instant::now());
        state.ping_written = None;
    }

    /// §5.2: never silent while our own `PING` has not reached the socket,
    /// and silence counts from when it did. A relay cannot answer a `PING`
    /// that is still in this process.
    fn is_silent(&self, limit: Duration, heartbeat: Duration) -> bool {
        let state = self.lock();
        let (Some(heard), Some(written)) = (state.last_heard, state.ping_written) else {
            return false;
        };
        if state.ping_unwritten {
            return false;
        }
        heard.elapsed() >= limit && written.elapsed() >= limit.saturating_sub(heartbeat)
    }
}

async fn write_loop(mut sink: Sink, mut outbound: mpsc::Receiver<Message>, control: Arc<Control>) {
    loop {
        while let Some((message, is_ping)) = control.pop() {
            if sink.send(message).await.is_err() {
                return;
            }
            if is_ping {
                control.ping_written();
            }
        }
        let message = tokio::select! {
            biased;
            _ = control.notify.notified() => continue,
            message = outbound.recv() => match message {
                Some(message) => message,
                None => break,
            },
        };
        if sink.send(message).await.is_err() {
            return;
        }
    }
    // A clean shutdown queues DEREGISTER and the trunk's CLOSE last.
    while let Some((message, _)) = control.pop() {
        if sink.send(message).await.is_err() {
            return;
        }
    }
    let _ = sink.close().await;
}

/// Aborts every stream still without a head [`HEAD_WAIT`] after its ack.
fn abort_headless(streams: &mut HashMap<u32, StreamEntry>, control: &Control) {
    let stale: Vec<u32> = streams
        .iter()
        .filter(|(_, e)| !e.head_seen && e.acked_at.elapsed() >= HEAD_WAIT)
        .map(|(id, _)| *id)
        .collect();
    for stream_id in stale {
        tracing::debug!(stream_id, "no head within the wait; aborting the stream");
        refuse_stream(streams, control, stream_id);
    }
}

/// What a trunk's control handling reaches besides its stream table.
struct Trunk<'a> {
    outbound: &'a mpsc::Sender<Message>,
    control: &'a Control,
    tunnel: &'a Tunnel,
    budget: &'a BodyBudget,
    finished: &'a mpsc::UnboundedSender<(u32, u64)>,
}

/// Returns `false` when the trunk should end.
async fn handle_control(
    text: &str,
    streams: &mut HashMap<u32, StreamEntry>,
    trunk: &Trunk<'_>,
    generation: &mut u64,
) -> bool {
    let control = trunk.control;
    // An origin never sends `ERROR` (§6): a relay before 116a drops the whole
    // trunk on one. A stream is refused with `CLOSE{aborted}`, and only a
    // frame the relay itself got wrong ends the trunk.
    let message = match proto::parse_text(text) {
        Ok(message) => message,
        Err(_) => {
            tracing::warn!("relay sent a frame this server cannot parse; closing the trunk");
            control.push(Message::text(proto::close_trunk()));
            return false;
        }
    };

    match message {
        Inbound::StreamOpen { stream_id } => {
            if streams.contains_key(&stream_id) {
                // §5.1: an id already open is never acknowledged twice.
                refuse_stream(streams, control, stream_id);
                return true;
            }
            *generation += 1;
            streams.insert(
                stream_id,
                StreamEntry {
                    body: None,
                    task: None,
                    head_seen: false,
                    acked_at: tokio::time::Instant::now(),
                    generation: *generation,
                },
            );
            // The ACK says only that this trunk accepted the id — not that a
            // request has been dispatched, and not that a response is coming.
            control.push(Message::text(proto::stream_ack(stream_id)));
        }
        Inbound::RequestHead(head) => {
            let stream_id = head.stream_id;
            match streams.get_mut(&stream_id) {
                Some(entry) if !entry.head_seen => {
                    entry.head_seen = true;
                    if !start_dispatch(entry, head, trunk) {
                        streams.remove(&stream_id);
                    }
                }
                // A second head, or one for a stream not open here.
                _ => refuse_stream(streams, control, stream_id),
            }
        }
        Inbound::BadHead { stream_id } => match streams.get(&stream_id) {
            Some(entry) if !entry.head_seen => {
                // Nothing has been answered yet, so the honest answer is a
                // complete one: 400, and the stream ends.
                streams.remove(&stream_id);
                control.push(Message::text(proto::response_head(
                    stream_id,
                    400,
                    proto::WireHeaders::default(),
                )));
                control.push(Message::text(proto::close_stream(stream_id)));
            }
            _ => refuse_stream(streams, control, stream_id),
        },
        Inbound::Close {
            stream_id: Some(id),
        } => {
            close_stream(streams, id);
        }
        Inbound::Close { stream_id: None } => return false,
        Inbound::Ping => control.push(Message::text(proto::pong())),
        Inbound::Pong => {}
        Inbound::Error(e) => match e.stream_id {
            Some(id) => {
                tracing::debug!(code = %e.code, stream_id = id, "relay closed a stream");
                close_stream(streams, id);
            }
            None => tracing::warn!(code = %e.code, message = ?e.message, "relay error"),
        },
        // A relay that has grown a message this build does not know is not a
        // reason to drop a working trunk.
        Inbound::Unknown(ty) => tracing::debug!(%ty, "ignoring unknown relay message"),
        // Registration messages after REGISTERED are meaningless here.
        Inbound::Challenge { .. } | Inbound::Registered(_) => {}
    }
    true
}

/// Ends one stream as aborted (§5.1, §5.2): whatever it had started is
/// abandoned, and the trunk's other streams carry on.
fn refuse_stream(streams: &mut HashMap<u32, StreamEntry>, control: &Control, stream_id: u32) {
    tracing::debug!(stream_id, "refusing a stream");
    close_stream(streams, stream_id);
    control.push(Message::text(proto::abort_stream(stream_id)));
}

fn close_stream(streams: &mut HashMap<u32, StreamEntry>, stream_id: u32) {
    if let Some(entry) = streams.remove(&stream_id)
        && let Some(task) = entry.task
    {
        task.abort();
    }
}

/// Methods that never carry a request body.
///
/// SRP §5.2 lets an origin dispatch these at the head without waiting for the
/// end-of-body frame, so a client from before that rule (which sent none)
/// still gets its `GET`s answered. Every other method waits for the
/// zero-length `0x01` frame.
fn is_bodyless(method: &str) -> bool {
    matches!(
        method.to_ascii_uppercase().as_str(),
        "GET" | "HEAD" | "OPTIONS" | "TRACE"
    )
}

/// Returns `false` when the stream ended here, answered without a dispatch.
fn start_dispatch(entry: &mut StreamEntry, head: RequestHead, trunk: &Trunk<'_>) -> bool {
    let (control, tunnel, budget) = (trunk.control, trunk.tunnel, trunk.budget);
    let stream_id = head.stream_id;

    let body = if is_bodyless(&head.method) {
        Body::empty()
    } else {
        let (tx, body) = budget.channel();
        entry.body = Some(tx);
        body
    };

    let request = match dispatch::build_request(&head, &tunnel.host_header, body) {
        Ok(request) => request,
        Err(bad) => {
            entry.body = None;
            control.push(Message::text(proto::response_head(
                stream_id,
                bad.status().as_u16(),
                proto::WireHeaders::default(),
            )));
            control.push(Message::text(proto::close_stream(stream_id)));
            return false;
        }
    };

    let router = tunnel.router.clone();
    let outbound = trunk.outbound.clone();
    let finished = Finished {
        stream_id,
        generation: entry.generation,
        tx: trunk.finished.clone(),
    };
    // **Spawned, never awaited here.** The read loop returns to the socket
    // immediately, so one slow handler cannot stall the trunk's other streams
    // and no relayed work is ever inlined onto a path a LAN request travels.
    let task = tokio::spawn(async move {
        serve_one(router, request, stream_id, outbound, finished).await;
    });
    entry.task = Some(task.abort_handle());
    true
}

async fn serve_one(
    router: Router,
    request: axum::http::Request<Body>,
    stream_id: u32,
    outbound: mpsc::Sender<Message>,
    finished: Finished,
) {
    use tower::ServiceExt;

    // The same `Service` a LAN request reaches: same `require_auth`, same tier
    // routers, same error mapping. There is no second code path to keep in
    // step, which is the whole point of R13.
    let response = match router.oneshot(request).await {
        Ok(response) => response,
        Err(e) => match e {},
    };

    let status = response.status().as_u16();
    let headers = proto::WireHeaders(
        response
            .headers()
            .iter()
            .filter_map(|(name, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|value| (name.as_str().to_string(), value.to_string()))
            })
            .collect(),
    );

    if outbound
        .send(Message::text(proto::response_head(
            stream_id, status, headers,
        )))
        .await
        .is_err()
    {
        return;
    }

    // Chunks go out as the handler produces them, unbuffered (§5.2) — which is
    // what lets `/v1/stream` be tunnelled as an ordinary SSE response rather
    // than needing a second, unauthenticated path of its own.
    let mut body = response.into_body().into_data_stream();
    while let Some(chunk) = body.next().await {
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(e) => {
                // A plain `CLOSE` here would tell the client the body is
                // complete, and it would keep a truncated note (decision 101).
                tracing::warn!(stream_id, error = %e, "response body failed mid-stream");
                drop(finished);
                let _ = outbound
                    .send(Message::text(proto::abort_stream(stream_id)))
                    .await;
                return;
            }
        };
        if chunk.is_empty() {
            continue;
        }
        // `send`, not `try_send`: a full queue must slow this task down, not
        // drop a chunk out of the middle of a response body.
        for piece in chunk.chunks(proto::MAX_BODY_FRAME_PAYLOAD) {
            if outbound
                .send(Message::binary(proto::encode_body_frame(
                    proto::FRAME_RESPONSE_BODY,
                    stream_id,
                    piece,
                )))
                .await
                .is_err()
            {
                return;
            }
        }
    }

    drop(finished);
    let _ = outbound
        .send(Message::text(proto::close_stream(stream_id)))
        .await;
}

fn handle_body_frame(bytes: &[u8], streams: &mut HashMap<u32, StreamEntry>, control: &Control) {
    let Some((kind, stream_id, payload)) = proto::decode_body_frame(bytes) else {
        return;
    };
    if kind != proto::FRAME_REQUEST_BODY {
        return;
    }
    let Some(entry) = streams.get_mut(&stream_id) else {
        return;
    };

    // A zero-length frame ends the body (§5.2).
    if payload.is_empty() {
        entry.body = None;
        return;
    }

    let Some(sender) = entry.body.as_ref() else {
        return;
    };
    // **Never awaited, and shed past the byte budget.** Awaiting here would
    // block the trunk's read loop on one slow handler, which is head-of-line
    // blocking for every other stream on the trunk (decision 102).
    //
    // **Shed means abort, never end the body.** Dropping the sender would read
    // as end-of-body, and the handler would serve the prefix as the whole
    // request: a truncated note or attachment saved as if complete (decision 95).
    match sender.push(payload) {
        Ok(()) => {}
        // The handler answered without reading the rest; its response stands.
        Err(Refused::Closed) => entry.body = None,
        Err(Refused::OverBudget) => {
            tracing::warn!(
                stream_id,
                "request body over its buffer budget; closing the stream"
            );
            close_stream(streams, stream_id);
            control.push(Message::text(proto::close_stream(stream_id)));
        }
    }
}

/// Byte accounting for the request bodies one trunk has buffered for its
/// handlers (decision 102). Only the trunk's read loop adds; only a body's
/// reader, or its drop, takes away.
struct BodyBudget {
    per_stream: usize,
    per_trunk: usize,
    trunk: Arc<AtomicUsize>,
}

impl BodyBudget {
    fn new(per_stream: usize, per_trunk: usize) -> Self {
        Self {
            per_stream,
            per_trunk,
            trunk: Arc::default(),
        }
    }

    fn channel(&self) -> (BodySender, Body) {
        let (tx, rx) = mpsc::unbounded_channel();
        let queued = Arc::new(AtomicUsize::new(0));
        let reader = BodyReader {
            rx,
            queued: queued.clone(),
            trunk: self.trunk.clone(),
        };
        let sender = BodySender {
            tx,
            queued,
            trunk: self.trunk.clone(),
            per_stream: self.per_stream,
            per_trunk: self.per_trunk,
        };
        (sender, Body::from_stream(reader))
    }
}

#[derive(Debug, PartialEq)]
enum Refused {
    /// The handler dropped its body: it has answered.
    Closed,
    OverBudget,
}

struct BodySender {
    tx: mpsc::UnboundedSender<Bytes>,
    queued: Arc<AtomicUsize>,
    trunk: Arc<AtomicUsize>,
    per_stream: usize,
    per_trunk: usize,
}

impl BodySender {
    fn push(&self, chunk: &[u8]) -> Result<(), Refused> {
        let len = chunk.len();
        if self.queued.load(Ordering::SeqCst) + len > self.per_stream
            || self.trunk.load(Ordering::SeqCst) + len > self.per_trunk
        {
            return Err(Refused::OverBudget);
        }
        self.queued.fetch_add(len, Ordering::SeqCst);
        self.trunk.fetch_add(len, Ordering::SeqCst);
        if self.tx.send(Bytes::copy_from_slice(chunk)).is_err() {
            // `swap`, as the reader's drop does, so the bytes leave the trunk's
            // count exactly once whichever side got there first.
            let back = self.queued.swap(0, Ordering::SeqCst);
            self.trunk.fetch_sub(back, Ordering::SeqCst);
            return Err(Refused::Closed);
        }
        Ok(())
    }
}

struct BodyReader {
    rx: mpsc::UnboundedReceiver<Bytes>,
    queued: Arc<AtomicUsize>,
    trunk: Arc<AtomicUsize>,
}

impl futures_util::Stream for BodyReader {
    type Item = Result<Bytes, std::io::Error>;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.rx.poll_recv(cx).map(|chunk| {
            chunk.map(|chunk| {
                self.queued.fetch_sub(chunk.len(), Ordering::SeqCst);
                self.trunk.fetch_sub(chunk.len(), Ordering::SeqCst);
                Ok(chunk)
            })
        })
    }
}

impl Drop for BodyReader {
    /// A handler that stops reading, or a stream that is aborted, gives its
    /// unread bytes back to the trunk.
    fn drop(&mut self) {
        let left = self.queued.swap(0, Ordering::SeqCst);
        self.trunk.fetch_sub(left, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn drain(body: Body) -> usize {
        axum::body::to_bytes(body, usize::MAX).await.unwrap().len()
    }

    #[tokio::test]
    async fn a_stream_is_refused_past_its_own_budget_not_truncated() {
        let budget = BodyBudget::new(10, 100);
        let (tx, body) = budget.channel();
        assert_eq!(tx.push(&[0; 6]), Ok(()));
        assert_eq!(tx.push(&[0; 5]), Err(Refused::OverBudget));
        assert_eq!(budget.trunk.load(Ordering::SeqCst), 6);
        drop(tx);
        assert_eq!(drain(body).await, 6);
        assert_eq!(budget.trunk.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn the_trunk_budget_sheds_only_the_stream_that_would_exceed_it() {
        let budget = BodyBudget::new(60, 100);
        let (a, a_body) = budget.channel();
        let (b, b_body) = budget.channel();
        let (c, _c_body) = budget.channel();
        assert_eq!(a.push(&[0; 50]), Ok(()));
        assert_eq!(b.push(&[0; 40]), Ok(()));
        assert_eq!(c.push(&[0; 20]), Err(Refused::OverBudget));
        assert_eq!(c.push(&[0; 10]), Ok(()), "what fits still fits");

        drop((a, b));
        assert_eq!(drain(a_body).await, 50);
        assert_eq!(drain(b_body).await, 40);
        assert_eq!(budget.trunk.load(Ordering::SeqCst), 10);
    }

    #[tokio::test]
    async fn read_bytes_free_the_budget_for_more() {
        let budget = BodyBudget::new(10, 10);
        let (tx, body) = budget.channel();
        let mut reader = body.into_data_stream();
        for _ in 0..5 {
            assert_eq!(tx.push(&[0; 10]), Ok(()));
            assert_eq!(reader.next().await.unwrap().unwrap().len(), 10);
        }
        assert_eq!(budget.trunk.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn an_abandoned_body_gives_its_bytes_back_to_the_trunk() {
        let budget = BodyBudget::new(100, 100);
        let (tx, body) = budget.channel();
        assert_eq!(tx.push(&[0; 30]), Ok(()));
        drop(body);
        assert_eq!(budget.trunk.load(Ordering::SeqCst), 0);
        assert_eq!(tx.push(&[0; 30]), Err(Refused::Closed));
        assert_eq!(budget.trunk.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn the_register_endpoint_is_derived_from_the_relay_url() {
        assert_eq!(
            register_url("wss://relay.example.com"),
            "wss://relay.example.com/register"
        );
        // `normalize_relay_url` strips a trailing slash, but a caller that
        // bypassed it must not produce a double slash.
        assert_eq!(
            register_url("ws://127.0.0.1:9000/"),
            "ws://127.0.0.1:9000/register"
        );
    }

    #[test]
    fn backoff_doubles_and_caps_at_a_minute() {
        let mut backoff = BACKOFF_MIN;
        let mut seen = vec![backoff];
        for _ in 0..10 {
            backoff = (backoff * 2).min(BACKOFF_MAX);
            seen.push(backoff);
        }
        assert_eq!(seen[0], Duration::from_secs(1));
        assert_eq!(seen[1], Duration::from_secs(2));
        assert_eq!(seen[6], Duration::from_secs(60));
        assert_eq!(
            *seen.last().unwrap(),
            BACKOFF_MAX,
            "it caps rather than growing without bound"
        );
    }

    #[test]
    fn only_the_bodyless_methods_skip_the_body_channel() {
        for method in ["GET", "HEAD", "OPTIONS", "TRACE", "get"] {
            assert!(is_bodyless(method), "{method}");
        }
        for method in ["POST", "PUT", "PATCH", "DELETE"] {
            assert!(!is_bodyless(method), "{method}");
        }
    }

    /// This build can dial `wss://` (decision 71). Before it, `connect_async`
    /// refused every `wss://` URL with `TlsFeatureNotEnabled`, so a TLS relay
    /// was unreachable from every server.
    ///
    /// **Something must be listening.** `connect_async` opens the TCP
    /// connection *before* it decides whether it can do TLS, so against a
    /// closed port both builds fail identically with an IO error, and the first
    /// version of this test passed with the feature removed. Here the TCP
    /// connect succeeds and the peer hangs up, so a build without TLS fails
    /// with `TlsFeatureNotEnabled` and a build with it fails in the handshake.
    #[tokio::test]
    async fn a_wss_relay_is_dialled_rather_than_refused_for_want_of_tls() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((socket, _)) = listener.accept().await {
                drop(socket);
            }
        });
        let err = tokio_tungstenite::connect_async(format!("wss://127.0.0.1:{port}/register"))
            .await
            .expect_err("the peer hangs up before any handshake");
        assert!(
            !matches!(
                err,
                tokio_tungstenite::tungstenite::Error::Url(
                    tokio_tungstenite::tungstenite::error::UrlError::TlsFeatureNotEnabled
                )
            ),
            "{err}"
        );
    }
}
