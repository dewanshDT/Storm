//! The client half of an SRP trunk (§5): one WebSocket to a relay carrying
//! every HTTP exchange with one server, multiplexed by relay-assigned
//! `stream_id`.
//!
//! A port of `apps/client/lib/api/relay/srp_trunk.dart`. The client presents
//! no credential to the relay (R12); credentials ride inside each request.

use std::collections::{HashMap, VecDeque};
use std::net::IpAddr;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard};
use std::task::{Context, Poll};
use std::time::Duration;

use bytes::Bytes;
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, Stream, StreamExt};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot, watch};
use tokio::time::Instant;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::http::Uri;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use super::codec::{self, Inbound, WireHeaders};
use super::error::SrpError;

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// Frames waiting for the socket. Bounded, so an upload waits for a relay
/// that is not reading instead of buffering the body.
const OUTBOUND_CAPACITY: usize = 64;

const WRITER_DRAIN: Duration = Duration::from_secs(1);

/// Timings, injectable so tests need not wait out real heartbeats.
#[derive(Debug, Clone)]
pub struct TrunkConfig {
    /// Dial plus `HELLO`/`READY`. Longer than the relay's 5 s `HELLO` hold, so
    /// a relay without the server gets to say `server_unreachable` first.
    pub handshake_timeout: Duration,
    /// `PING` cadence (§4.2).
    pub heartbeat: Duration,
    /// How long the relay may say nothing before the trunk is dead. The relay
    /// answers every `PING`, so silence this long is a half-open socket
    /// (decision 97).
    pub silence_limit: Duration,
}

impl Default for TrunkConfig {
    fn default() -> Self {
        Self {
            handshake_timeout: Duration::from_secs(8),
            heartbeat: Duration::from_secs(15),
            silence_limit: Duration::from_secs(45),
        }
    }
}

/// A response whose head has arrived. The body streams as `0x02` frames
/// arrive, so an SSE response is readable before it ends.
#[derive(Debug)]
pub struct SrpResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: SrpBody,
}

/// A response body. Ends with `None` only on the origin's plain `CLOSE`; an
/// abort, a stream `ERROR` or a lost trunk is an `Err`, never a short body
/// (§5.4). Dropping it early closes the stream at the relay.
#[derive(Debug)]
pub struct SrpBody {
    rx: mpsc::UnboundedReceiver<Result<Bytes, SrpError>>,
    done: bool,
    _guard: StreamGuard,
}

impl SrpBody {
    pub async fn chunk(&mut self) -> Option<Result<Bytes, SrpError>> {
        std::future::poll_fn(|cx| self.poll_chunk(cx)).await
    }

    pub async fn collect(mut self) -> Result<Vec<u8>, SrpError> {
        let mut out = Vec::new();
        while let Some(chunk) = self.chunk().await {
            out.extend_from_slice(&chunk?);
        }
        Ok(out)
    }

    fn poll_chunk(&mut self, cx: &mut Context<'_>) -> Poll<Option<Result<Bytes, SrpError>>> {
        if self.done {
            return Poll::Ready(None);
        }
        let next = std::task::ready!(self.rx.poll_recv(cx));
        if !matches!(next, Some(Ok(_))) {
            self.done = true;
        }
        Poll::Ready(next)
    }
}

impl Stream for SrpBody {
    type Item = Result<Bytes, SrpError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.get_mut().poll_chunk(cx)
    }
}

/// One client trunk. Cheap to clone; the trunk closes when the last clone is
/// dropped or [`SrpTrunk::close`] is called.
#[derive(Clone)]
pub struct SrpTrunk {
    shared: Arc<Shared>,
}

struct Shared {
    core: Arc<Core>,
    client_trunk_id: String,
}

impl Drop for Shared {
    fn drop(&mut self) {
        self.core.shut_down();
    }
}

impl std::fmt::Debug for SrpTrunk {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SrpTrunk")
            .field("client_trunk_id", &self.shared.client_trunk_id)
            .field("loss_reason", &self.loss_reason())
            .finish()
    }
}

impl SrpTrunk {
    /// Dials `relay_url` (the relay's base URL, or the server's full
    /// `public_address`) and completes `HELLO`/`READY` (§5).
    pub async fn connect(relay_url: &str, server_id: &str) -> Result<SrpTrunk, SrpError> {
        Self::connect_with(relay_url, server_id, TrunkConfig::default()).await
    }

    pub async fn connect_with(
        relay_url: &str,
        server_id: &str,
        config: TrunkConfig,
    ) -> Result<SrpTrunk, SrpError> {
        let url = connect_url(relay_url, server_id)?;
        // Tungstenite's `wss://` uses the process default provider, and rustls
        // panics if none is installed. Idempotent; `ring` as everywhere else.
        let _ = rustls::crypto::ring::default_provider().install_default();

        let deadline = Instant::now() + config.handshake_timeout;
        let ws = match tokio::time::timeout_at(
            deadline,
            tokio_tungstenite::connect_async_with_config(url.as_str(), None, true),
        )
        .await
        {
            Err(_) => return Err(SrpError::RelayDown("timed out dialling".into())),
            Ok(Err(e)) => return Err(SrpError::RelayDown(e.to_string())),
            Ok(Ok((ws, _))) => ws,
        };
        let (mut sink, mut stream) = ws.split();
        let client_trunk_id = handshake(&mut sink, &mut stream, server_id, deadline).await?;

        let (out, out_rx) = mpsc::channel(OUTBOUND_CAPACITY);
        let (lost, _) = watch::channel(None);
        let core = Arc::new(Core {
            out,
            state: Mutex::new(State::default()),
            lost,
            open_order: tokio::sync::Mutex::new(()),
            last_heard: Mutex::new(Instant::now()),
        });
        tokio::spawn(write_loop(core.clone(), sink, out_rx));
        tokio::spawn(read_loop(core.clone(), stream));
        tokio::spawn(heartbeat_loop(core.clone(), config));

        Ok(SrpTrunk {
            shared: Arc::new(Shared {
                core,
                client_trunk_id,
            }),
        })
    }

    /// One HTTP exchange (§5.1, §5.2): `OPEN_STREAM`, the head, the body in
    /// frames of at most 256 KiB, then always one empty end-of-body frame.
    /// Returns once the response head arrives.
    pub async fn request(
        &self,
        method: &str,
        path_and_query: &str,
        headers: &[(String, String)],
        body: Bytes,
    ) -> Result<SrpResponse, SrpError> {
        let core = &self.shared.core;
        let (ready_tx, ready_rx) = oneshot::channel();
        let (head_tx, head_rx) = oneshot::channel();
        let (body_tx, body_rx) = mpsc::unbounded_channel();

        let guard = {
            // Held across the send: the relay answers opens in order, and a
            // refusal names no attempt, so pending order must be wire order.
            let _order = core.open_order.lock().await;
            let attempt_id = {
                let mut state = core.lock();
                if state.dead {
                    return Err(SrpError::TrunkLost);
                }
                state.next_attempt += 1;
                let attempt_id = format!("a{}", state.next_attempt);
                state.pending.push_back(Pending {
                    attempt_id: attempt_id.clone(),
                    ready: ready_tx,
                    slot: Slot {
                        attempt_id: attempt_id.clone(),
                        head: Some(head_tx),
                        body: body_tx,
                    },
                });
                attempt_id
            };
            let guard = StreamGuard {
                core: core.clone(),
                attempt_id: attempt_id.clone(),
            };
            core.send(Message::text(codec::open_stream(&attempt_id)))
                .await?;
            guard
        };

        let stream_id = ready_rx.await.unwrap_or(Err(SrpError::TrunkLost))?;

        let headers = WireHeaders(
            headers
                .iter()
                .filter(|(name, _)| !name.eq_ignore_ascii_case("relay_peer_ip"))
                .cloned()
                .collect(),
        );
        core.send(Message::text(codec::request_head(
            stream_id,
            method,
            path_and_query,
            &headers,
        )))
        .await?;

        let mut offset = 0;
        while offset < body.len() && core.stream_live(stream_id) {
            let end = (offset + codec::MAX_BODY_FRAME_PAYLOAD).min(body.len());
            core.send(body_frame(stream_id, &body[offset..end])).await?;
            offset = end;
        }
        if core.stream_live(stream_id) {
            core.send(body_frame(stream_id, &[])).await?;
        }

        let (status, headers) = head_rx.await.unwrap_or(Err(SrpError::TrunkLost))?;
        Ok(SrpResponse {
            status,
            headers,
            body: SrpBody {
                rx: body_rx,
                done: false,
                _guard: guard,
            },
        })
    }

    pub fn client_trunk_id(&self) -> &str {
        &self.shared.client_trunk_id
    }

    pub fn is_closed(&self) -> bool {
        self.shared.core.lost.borrow().is_some()
    }

    /// Why the trunk died, once it has.
    pub fn loss_reason(&self) -> Option<SrpError> {
        self.shared.core.lost.borrow().clone()
    }

    /// Resolves when the trunk dies, with the reason.
    pub async fn closed(&self) -> SrpError {
        let mut rx = self.shared.core.lost.subscribe();
        match rx.wait_for(Option::is_some).await {
            Ok(reason) => reason.clone().unwrap_or(SrpError::Closed),
            Err(_) => SrpError::Closed,
        }
    }

    /// Sends `CLOSE {}` and ends every stream with `trunk_lost`.
    pub fn close(&self) {
        self.shared.core.shut_down();
    }
}

fn body_frame(stream_id: u32, payload: &[u8]) -> Message {
    Message::Binary(codec::encode_body_frame(codec::FRAME_REQUEST_BODY, stream_id, payload).into())
}

/// `<relay>/connect/<server_id>` (§4.3). Plaintext only to this machine:
/// without E2E the path must be private, and a `ws://` relay elsewhere is not.
fn connect_url(relay_url: &str, server_id: &str) -> Result<String, SrpError> {
    let invalid = |why: &str| SrpError::InvalidTarget(why.to_string());
    codec::validate_server_id(server_id).map_err(invalid)?;
    let uri: Uri = relay_url.parse().map_err(|_| invalid("not a URL"))?;
    let scheme = uri.scheme_str().unwrap_or_default();
    if scheme != "ws" && scheme != "wss" {
        return Err(invalid("a relay URL is ws:// or wss://"));
    }
    let authority = uri.authority().ok_or_else(|| invalid("no host"))?;
    if scheme == "ws" && !is_loopback(authority.host()) {
        return Err(invalid(
            "plaintext ws:// is only for a relay on this machine",
        ));
    }
    let own = format!("/connect/{server_id}");
    match uri.path() {
        "" | "/" => {}
        path if path == own => {}
        _ => return Err(invalid("a relay URL has no path")),
    }
    if uri.query().is_some() {
        return Err(invalid("a relay URL has no query"));
    }
    Ok(format!("{scheme}://{authority}{own}"))
}

fn is_loopback(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host
            .trim_start_matches('[')
            .trim_end_matches(']')
            .parse::<IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

async fn handshake(
    sink: &mut SplitSink<Ws, Message>,
    stream: &mut SplitStream<Ws>,
    server_id: &str,
    deadline: Instant,
) -> Result<String, SrpError> {
    let closed = || SrpError::RelayDown("the relay closed the connection".into());
    sink.send(Message::text(codec::hello(server_id)))
        .await
        .map_err(|e| SrpError::RelayDown(e.to_string()))?;
    loop {
        let message = match tokio::time::timeout_at(deadline, stream.next()).await {
            Err(_) => return Err(SrpError::HandshakeTimeout),
            Ok(None | Some(Err(_)) | Some(Ok(Message::Close(_)))) => return Err(closed()),
            Ok(Some(Ok(message))) => message,
        };
        let text = match message {
            Message::Text(text) => text,
            Message::Binary(_) => return Err(SrpError::ProtocolError),
            _ => continue,
        };
        match codec::parse_text(text.as_str()).map_err(|_| SrpError::ProtocolError)? {
            Inbound::Ready { client_trunk_id } => return Ok(client_trunk_id),
            Inbound::Error(e) => return Err(SrpError::from_relay_code(&e.code)),
            Inbound::Close {
                stream_id: None, ..
            } => return Err(closed()),
            Inbound::Ping => {
                let _ = sink.send(Message::text(codec::pong())).await;
            }
            _ => {}
        }
    }
}

type Head = (u16, Vec<(String, String)>);

struct Core {
    out: mpsc::Sender<Message>,
    state: Mutex<State>,
    lost: watch::Sender<Option<SrpError>>,
    open_order: tokio::sync::Mutex<()>,
    last_heard: Mutex<Instant>,
}

#[derive(Default)]
struct State {
    dead: bool,
    next_attempt: u64,
    /// Opens the relay has not answered yet, in the order they were sent.
    pending: VecDeque<Pending>,
    streams: HashMap<u32, Slot>,
    /// A stream-scoped `trunk_lost`/`trunk_superseded`: the relay telling us
    /// why it is about to hang up.
    relay_said: Option<SrpError>,
}

struct Pending {
    attempt_id: String,
    ready: oneshot::Sender<Result<u32, SrpError>>,
    slot: Slot,
}

struct Slot {
    attempt_id: String,
    head: Option<oneshot::Sender<Result<Head, SrpError>>>,
    body: mpsc::UnboundedSender<Result<Bytes, SrpError>>,
}

impl Slot {
    /// Ends the stream: `None` is the origin's plain `CLOSE`.
    fn end(mut self, error: Option<SrpError>) {
        match (self.head.take(), error) {
            (Some(head), error) => {
                let _ = head.send(Err(error.unwrap_or(SrpError::StreamClosed)));
            }
            (None, Some(error)) => {
                let _ = self.body.send(Err(error));
            }
            (None, None) => {}
        }
    }
}

impl Core {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    async fn send(&self, message: Message) -> Result<(), SrpError> {
        self.out
            .send(message)
            .await
            .map_err(|_| SrpError::TrunkLost)
    }

    fn heard(&self) {
        *self.last_heard.lock().unwrap_or_else(|e| e.into_inner()) = Instant::now();
    }

    fn silent_for(&self) -> Duration {
        self.last_heard
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .elapsed()
    }

    fn stream_live(&self, stream_id: u32) -> bool {
        self.lock().streams.contains_key(&stream_id)
    }

    fn end_stream(&self, stream_id: u32, error: Option<SrpError>) {
        let slot = self.lock().streams.remove(&stream_id);
        if let Some(slot) = slot {
            slot.end(error);
        }
    }

    /// Terminal. Every open or pending stream ends with `trunk_lost`.
    fn fail(&self, reason: SrpError) {
        let (pending, streams) = {
            let mut state = self.lock();
            if state.dead {
                return;
            }
            state.dead = true;
            (
                std::mem::take(&mut state.pending),
                std::mem::take(&mut state.streams),
            )
        };
        for p in pending {
            let _ = p.ready.send(Err(SrpError::TrunkLost));
        }
        for (_, slot) in streams {
            slot.end(Some(SrpError::TrunkLost));
        }
        tracing::debug!(reason = reason.code(), "srp trunk lost");
        self.lost.send_replace(Some(reason));
    }

    fn shut_down(&self) {
        if !self.lock().dead {
            let _ = self.out.try_send(Message::text(codec::close_trunk()));
        }
        self.fail(SrpError::Closed);
    }

    fn relay_hung_up(&self) -> SrpError {
        self.lock()
            .relay_said
            .clone()
            .unwrap_or_else(|| SrpError::RelayDown("the relay closed the connection".into()))
    }

    /// Called when a request is abandoned: its future or body was dropped.
    fn abandon(&self, attempt_id: &str) {
        let mut state = self.lock();
        // Still pending: the reader closes it when its STREAM_READY finds no
        // one waiting. Removing it here would break the FIFO that attributes
        // a `rate_limited` refusal.
        if state.pending.iter().any(|p| p.attempt_id == attempt_id) {
            return;
        }
        let Some(stream_id) = state
            .streams
            .iter()
            .find(|(_, slot)| slot.attempt_id == attempt_id)
            .map(|(id, _)| *id)
        else {
            return;
        };
        state.streams.remove(&stream_id);
        drop(state);
        let _ = self
            .out
            .try_send(Message::text(codec::close_stream(stream_id)));
    }

    async fn on_control(&self, text: &str) -> Result<(), SrpError> {
        match codec::parse_text(text).map_err(|_| SrpError::ProtocolError)? {
            Inbound::StreamReady {
                attempt_id,
                stream_id,
            } => self.on_stream_ready(attempt_id, stream_id).await,
            Inbound::ResponseHead(head) => {
                let mut state = self.lock();
                if let Some(slot) = state.streams.get_mut(&head.stream_id) {
                    match slot.head.take() {
                        Some(tx) => {
                            let _ = tx.send(Ok((head.status, head.headers.0)));
                        }
                        None => {
                            let slot = state.streams.remove(&head.stream_id);
                            drop(state);
                            if let Some(slot) = slot {
                                slot.end(Some(SrpError::ProtocolError));
                            }
                        }
                    }
                }
                Ok(())
            }
            Inbound::Close {
                stream_id: None, ..
            } => Err(self.relay_hung_up()),
            Inbound::Close {
                stream_id: Some(stream_id),
                aborted,
            } => {
                self.end_stream(stream_id, aborted.then_some(SrpError::StreamAborted));
                Ok(())
            }
            Inbound::Ping => self.send(Message::text(codec::pong())).await,
            Inbound::Error(e) => {
                let error = SrpError::from_relay_code(&e.code);
                match e.stream_id {
                    Some(stream_id) => {
                        if matches!(error, SrpError::TrunkLost | SrpError::TrunkSuperseded) {
                            self.lock().relay_said = Some(error.clone());
                        }
                        self.end_stream(stream_id, Some(error));
                        Ok(())
                    }
                    // The relay refuses an over-cap OPEN_STREAM and keeps the
                    // trunk. Since decision 106 it names the attempt; an older
                    // relay does not, but answers opens in order, so then it
                    // is the oldest pending one.
                    None if error == SrpError::RateLimited => {
                        let refused =
                            {
                                let mut state = self.lock();
                                match &e.attempt_id {
                                    Some(id) => {
                                        match state.pending.iter().position(|p| {
                                            id.as_str() == Some(p.attempt_id.as_str())
                                        }) {
                                            Some(i) => state.pending.remove(i),
                                            // Already answered or abandoned.
                                            None => return Ok(()),
                                        }
                                    }
                                    None => state.pending.pop_front(),
                                }
                            };
                        match refused {
                            Some(p) => {
                                let _ = p.ready.send(Err(SrpError::RateLimited));
                                Ok(())
                            }
                            None => Err(error),
                        }
                    }
                    None => Err(error),
                }
            }
            Inbound::Ready { .. } | Inbound::Pong | Inbound::Unknown(_) => Ok(()),
        }
    }

    async fn on_stream_ready(
        &self,
        attempt_id: serde_json::Value,
        stream_id: u32,
    ) -> Result<(), SrpError> {
        let pending = {
            let mut state = self.lock();
            if state.streams.contains_key(&stream_id) {
                return Err(SrpError::ProtocolError);
            }
            let index = state
                .pending
                .iter()
                .position(|p| attempt_id.as_str() == Some(p.attempt_id.as_str()));
            let pending = index.and_then(|i| state.pending.remove(i));
            if let Some(p) = pending {
                state.streams.insert(stream_id, p.slot);
                Some(p.ready)
            } else {
                None
            }
        };
        let delivered = match pending {
            Some(ready) => ready.send(Ok(stream_id)).is_ok(),
            None => false,
        };
        if !delivered {
            self.lock().streams.remove(&stream_id);
            self.send(Message::text(codec::close_stream(stream_id)))
                .await?;
        }
        Ok(())
    }

    fn on_body(&self, frame: Bytes) -> Result<(), SrpError> {
        let Some((kind, stream_id, payload)) = codec::decode_body_frame(&frame) else {
            return Err(SrpError::ProtocolError);
        };
        if kind != codec::FRAME_RESPONSE_BODY {
            return Err(SrpError::ProtocolError);
        }
        if payload.is_empty() {
            return Ok(());
        }
        let mut state = self.lock();
        let Some(slot) = state.streams.get(&stream_id) else {
            return Ok(());
        };
        if slot.head.is_some() {
            // A body before its head names no response to belong to.
            let slot = state.streams.remove(&stream_id);
            drop(state);
            if let Some(slot) = slot {
                slot.end(Some(SrpError::ProtocolError));
            }
            let _ = self
                .out
                .try_send(Message::text(codec::close_stream(stream_id)));
            return Ok(());
        }
        let _ = slot.body.send(Ok(frame.slice(codec::BINARY_HEADER_LEN..)));
        Ok(())
    }
}

/// Closes a request's stream at the relay if it is dropped before its body
/// ended.
#[derive(Debug)]
struct StreamGuard {
    core: Arc<Core>,
    attempt_id: String,
}

impl Drop for StreamGuard {
    fn drop(&mut self) {
        self.core.abandon(&self.attempt_id);
    }
}

impl std::fmt::Debug for Core {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Core").finish_non_exhaustive()
    }
}

async fn read_loop(core: Arc<Core>, mut stream: SplitStream<Ws>) {
    let mut lost = core.lost.subscribe();
    loop {
        let message = tokio::select! {
            message = stream.next() => message,
            _ = lost.wait_for(Option::is_some) => return,
        };
        let message = match message {
            Some(Ok(Message::Close(_))) | Some(Err(_)) | None => {
                core.fail(core.relay_hung_up());
                return;
            }
            Some(Ok(message)) => message,
        };
        core.heard();
        let outcome = match message {
            Message::Text(text) => core.on_control(text.as_str()).await,
            Message::Binary(frame) => core.on_body(frame),
            _ => Ok(()),
        };
        if let Err(reason) = outcome {
            core.fail(reason);
            return;
        }
    }
}

async fn write_loop(
    core: Arc<Core>,
    mut sink: SplitSink<Ws, Message>,
    mut out: mpsc::Receiver<Message>,
) {
    let mut lost = core.lost.subscribe();
    loop {
        let message = tokio::select! {
            biased;
            message = out.recv() => message,
            _ = lost.wait_for(Option::is_some) => None,
        };
        let Some(message) = message else { break };
        if let Err(e) = sink.send(message).await {
            core.fail(SrpError::RelayDown(e.to_string()));
            return;
        }
    }
    // A `CLOSE {}` queued by `close()` still goes out.
    let _ = tokio::time::timeout(WRITER_DRAIN, async {
        while let Ok(message) = out.try_recv() {
            if sink.send(message).await.is_err() {
                return;
            }
        }
        let _ = sink.close().await;
    })
    .await;
}

async fn heartbeat_loop(core: Arc<Core>, config: TrunkConfig) {
    let mut lost = core.lost.subscribe();
    let mut tick = tokio::time::interval_at(Instant::now() + config.heartbeat, config.heartbeat);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            _ = tick.tick() => {}
            _ = lost.wait_for(Option::is_some) => return,
        }
        if core.silent_for() >= config.silence_limit {
            core.fail(SrpError::RelaySilent);
            return;
        }
        let _ = core.out.try_send(Message::text(codec::ping()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_relay_base_url_becomes_the_servers_connect_address() {
        assert_eq!(
            connect_url("wss://relay.example.com", "srv_A").unwrap(),
            "wss://relay.example.com/connect/srv_A"
        );
        assert_eq!(
            connect_url("wss://relay.example.com:8443/", "srv_A").unwrap(),
            "wss://relay.example.com:8443/connect/srv_A"
        );
        assert_eq!(
            connect_url("wss://relay.example.com/connect/srv_A", "srv_A").unwrap(),
            "wss://relay.example.com/connect/srv_A"
        );
    }

    #[test]
    fn plaintext_is_only_for_a_relay_on_this_machine() {
        for ok in [
            "ws://127.0.0.1:18860",
            "ws://localhost:18860",
            "ws://[::1]:18860",
            "ws://127.9.9.9",
        ] {
            assert!(connect_url(ok, "srv_A").is_ok(), "{ok}");
        }
        for refused in [
            "ws://relay.example.com",
            "ws://192.168.1.10:8080",
            "ws://localhost.example.com",
        ] {
            assert_eq!(
                connect_url(refused, "srv_A").unwrap_err().code(),
                "invalid_relay_target",
                "{refused}"
            );
        }
    }

    #[test]
    fn a_bad_target_is_refused_before_dialling() {
        for (url, id) in [
            ("https://relay.example.com", "srv_A"),
            ("wss://relay.example.com/prefix", "srv_A"),
            ("wss://relay.example.com/connect/srv_B", "srv_A"),
            ("wss://relay.example.com?x=1", "srv_A"),
            ("wss://relay.example.com", "srv:A"),
            ("wss://relay.example.com", ""),
            ("not a url", "srv_A"),
        ] {
            assert!(connect_url(url, id).is_err(), "{url} {id}");
        }
    }

    #[test]
    fn every_relay_code_maps_to_a_stable_one() {
        for (relay, ours) in [
            ("server_unreachable", "server_offline"),
            ("server_timeout", "server_timeout"),
            ("trunk_lost", "trunk_lost"),
            ("trunk_superseded", "trunk_superseded"),
            ("stream_aborted", "stream_aborted"),
            ("client_backlogged", "client_backlogged"),
            ("rate_limited", "rate_limited"),
            ("stream_closed", "stream_closed"),
            ("protocol_error", "protocol_error"),
            ("auth_failed", "relay_error"),
        ] {
            assert_eq!(SrpError::from_relay_code(relay).code(), ours, "{relay}");
        }
    }
}
