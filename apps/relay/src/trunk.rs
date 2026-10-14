//! Socket plumbing shared by both trunk kinds: the writer task, the send
//! handle, and frame classification.
//!
//! Registration was a strictly sequential exchange, so it could own the socket
//! outright. Routing cannot: a server trunk's socket is written by *every*
//! client task, and a client trunk's socket is written by the server trunk's
//! reader task. So each socket is split once, the write half is owned by one
//! task, and everybody else holds a [`Tx`] into a channel that feeds it.
//!
//! The two send methods are not interchangeable and the difference is the whole
//! reason this file exists — see [`Tx::try_send_json`].

use std::collections::HashSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use axum::extract::ws::{Message, WebSocket};
use futures_util::{SinkExt, StreamExt, stream::SplitStream};
use tokio::sync::{mpsc, watch};

use crate::proto::{self, ErrorCode, Frame};

/// The read half of a trunk's socket.
pub type Rx = SplitStream<WebSocket>;

/// How much a peer may fall behind before it costs somebody else.
///
/// Client-ward is the larger of the two because overrunning it kills the client
/// trunk (see [`Tx::try_send_json`]), while overrunning the server-ward queue
/// only makes one client wait.
///
/// **A count is not a memory bound.** A frame can be anything up to the
/// WebSocket reader's 16 MiB frame limit, and an attachment is one frame the
/// size of the file, so 256 queued frames could be 4 GiB. The client-ward queue
/// also carries a byte budget (`max_client_buffer_bytes`); these counts stay
/// only as the channel's capacity.
pub const SERVER_WARD_QUEUE: usize = 64;
pub const CLIENT_WARD_QUEUE: usize = 256;

/// Room for stream-ending `ERROR`s that overtake the ordinary queue (see
/// [`Tx::end_stream`]). Overflowing it drops the whole trunk.
const STREAM_END_QUEUE: usize = 256;

/// Bytes queued toward one peer and not yet written to its socket.
///
/// Charged when a frame is queued, refunded by the writer task once the frame
/// has been written. So it measures what the relay is actually holding for a
/// peer that has stopped reading, not what has passed through it.
#[derive(Debug)]
struct Budget {
    queued: AtomicUsize,
    limit: usize,
}

impl Budget {
    /// Reserves `len` bytes if they fit.
    ///
    /// **An empty queue admits any one frame**, however large. A response body
    /// is usually a single frame the size of the whole resource, so without
    /// this a budget smaller than the largest attachment could never deliver
    /// it, even to a client that is reading as fast as it can. The budget
    /// bounds the backlog, not the size of one frame; the frame-size limit
    /// bounds that.
    fn try_reserve(&self, len: usize) -> bool {
        self.queued
            .try_update(Ordering::AcqRel, Ordering::Acquire, |queued| {
                (queued == 0 || queued.saturating_add(len) <= self.limit)
                    .then(|| queued.saturating_add(len))
            })
            .is_ok()
    }

    /// Counts `len` bytes unconditionally. For the blocking path, which is
    /// bounded by waiting rather than by refusing.
    fn charge(&self, len: usize) {
        self.queued.fetch_add(len, Ordering::AcqRel);
    }

    fn refund(&self, len: usize) {
        self.queued.fetch_sub(len, Ordering::AcqRel);
    }
}

/// What a queued frame costs against a [`Budget`]. Control frames other than
/// text are free: `Close` in particular must never be refused for want of room.
fn cost(message: &Message) -> usize {
    match message {
        Message::Text(text) => text.len(),
        Message::Binary(bytes) => bytes.len(),
        _ => 0,
    }
}

/// How a connection stopped being useful.
///
/// `Disconnected` is separate from the error codes because there is nobody left
/// to tell: sending an `ERROR` down a closed socket is how a handler ends up
/// logging a spurious failure for an ordinary hang-up.
#[derive(Debug, Clone, Copy)]
pub enum Fault {
    Code(ErrorCode),
    Disconnected,
}

impl From<ErrorCode> for Fault {
    fn from(code: ErrorCode) -> Self {
        Self::Code(code)
    }
}

/// The peer could not keep up. The caller ends the stream it was for with
/// `client_backlogged`, told only to the client that fell behind.
#[derive(Debug, Clone, Copy)]
pub struct Backlogged;

/// A cloneable handle for writing to one trunk's socket.
#[derive(Debug, Clone)]
pub struct Tx {
    inner: mpsc::Sender<Message>,
    budget: Arc<Budget>,
    ends: Arc<StreamEnds>,
}

/// The lane that ends one stream on a peer whose ordinary queue may be full.
///
/// A stream that loses its client to backpressure has to say so (§5.4), and
/// the frame that says it cannot wait behind the backlog that caused it. So it
/// bypasses the queue: the writer takes these first and discards the ended
/// stream's queued body frames, while every other stream's frames go on in
/// order.
#[derive(Debug)]
struct StreamEnds {
    lane: mpsc::Sender<Message>,
    /// Streams whose queued `0x01`/`0x02` frames the writer drops.
    ended: Mutex<HashSet<u32>>,
    /// Set when even `lane` is full: the writer and reader stop, and the peer
    /// learns from the closed socket that every stream on it is gone.
    killed: watch::Sender<bool>,
}

impl StreamEnds {
    fn new(lane: mpsc::Sender<Message>) -> Self {
        Self {
            lane,
            ended: Mutex::new(HashSet::new()),
            killed: watch::Sender::new(false),
        }
    }

    fn discards(&self, message: &Message) -> bool {
        let Message::Binary(bytes) = message else {
            return false;
        };
        let Ok((_, stream_id)) = proto::parse_body_header(bytes) else {
            return false;
        };
        self.ended.lock().expect("ended mutex").contains(&stream_id)
    }

    /// Safe once the queue is empty: nothing queued before an end is left, and
    /// nothing is queued after one, since the stream has left the routing table.
    fn forget_ended(&self) {
        self.ended.lock().expect("ended mutex").clear();
    }
}

impl Tx {
    /// Blocks the caller while the peer is behind.
    ///
    /// Correct **only** where the calling task serves that one peer — a client
    /// trunk's reader pushing a request at the origin. The client waiting for
    /// its own slow server is backpressure; nobody else is delayed.
    pub async fn send_json(&self, json: String) -> Result<(), Fault> {
        self.send(Message::Text(json.into())).await
    }

    pub async fn send_raw(&self, bytes: Vec<u8>) -> Result<(), Fault> {
        self.send(Message::Binary(bytes.into())).await
    }

    async fn send(&self, message: Message) -> Result<(), Fault> {
        let len = cost(&message);
        self.budget.charge(len);
        self.inner.send(message).await.map_err(|_| {
            self.budget.refund(len);
            Fault::Disconnected
        })
    }

    /// Never blocks; reports a peer that has fallen too far behind.
    ///
    /// Required on the **server → client** hop, and the reason is structural
    /// rather than a tuning preference: one server trunk's reader task feeds
    /// every client on that trunk. Awaiting a full queue there would let a
    /// single client that stopped reading its socket stall the responses of
    /// every other client sharing the server — head-of-line blocking across
    /// tenants, from one slow reader. The caller drops that client instead.
    pub fn try_send_json(&self, json: String) -> Result<(), Backlogged> {
        self.try_send(Message::Text(json.into()))
    }

    pub fn try_send_raw(&self, bytes: Vec<u8>) -> Result<(), Backlogged> {
        self.try_send(Message::Binary(bytes.into()))
    }

    fn try_send(&self, message: Message) -> Result<(), Backlogged> {
        // Over budget, a closed channel and a full one are all "this peer is
        // not getting it"; the caller's response to each is to stop routing to
        // it.
        let len = cost(&message);
        if !self.budget.try_reserve(len) {
            return Err(Backlogged);
        }
        self.inner.try_send(message).map_err(|_| {
            self.budget.refund(len);
            Backlogged
        })
    }

    /// Ends `stream_id` with `error_json`, a stream-scoped `ERROR`, even when
    /// this peer's queue is full.
    ///
    /// The `ERROR` overtakes what is queued, and the stream's queued body
    /// frames are discarded. Toward a client, only ever an `ERROR`: a `CLOSE`
    /// overtaking the body it ends would present a truncated body as complete.
    pub fn end_stream(&self, stream_id: u32, error_json: String) {
        self.ends
            .ended
            .lock()
            .expect("ended mutex")
            .insert(stream_id);
        if self
            .ends
            .lane
            .try_send(Message::Text(error_json.into()))
            .is_err()
        {
            self.kill();
        }
    }

    /// Ends `stream_id` at the origin with `CLOSE{stream_id}`, for every end
    /// the origin did not ask for: client gone, ack timeout, client backlogged
    /// (§5.4, decision 116b). Toward the origin a `CLOSE` is a teardown, so it
    /// may overtake the stream's own queued request body, which is discarded.
    pub fn close_toward_origin(&self, stream_id: u32) {
        self.end_stream(stream_id, proto::close_stream(stream_id));
    }

    /// Queues a control frame ahead of everything this peer has queued, for
    /// the `PONG` that must not wait behind a backlog (§5.2).
    pub fn send_control(&self, json: String) {
        if self.ends.lane.try_send(Message::Text(json.into())).is_err() {
            self.kill();
        }
    }

    /// Drops the connection without queueing anything, for a peer too far
    /// behind to be told even through [`Tx::end_stream`].
    pub fn kill(&self) {
        self.ends.killed.send_replace(true);
    }

    /// Resolves once [`Tx::kill`] has been called.
    pub async fn killed(&self) {
        let mut killed = self.ends.killed.subscribe();
        let _ = killed.wait_for(|killed| *killed).await;
    }

    /// Best-effort close. Used on teardown paths where there is nothing useful
    /// to do about a peer that is already gone.
    pub fn close(&self) {
        let _ = self.inner.try_send(Message::Close(None));
    }

    /// A `Tx` with a plain receiver instead of a socket, for unit tests that
    /// exercise the routing tables without a WebSocket behind them.
    #[cfg(test)]
    pub fn detached(queue: usize) -> (Self, mpsc::Receiver<Message>) {
        Self::detached_with_budget(queue, usize::MAX)
    }

    #[cfg(test)]
    fn detached_with_budget(queue: usize, limit: usize) -> (Self, mpsc::Receiver<Message>) {
        let (tx, rx) = mpsc::channel(queue);
        let budget = Arc::new(Budget {
            queued: AtomicUsize::new(0),
            limit,
        });
        let (lane, _) = mpsc::channel(STREAM_END_QUEUE);
        let ends = Arc::new(StreamEnds::new(lane));
        (
            Self {
                inner: tx,
                budget,
                ends,
            },
            rx,
        )
    }
}

/// Splits a socket and spawns the task that owns its write half.
///
/// The writer drains in order, so an `ERROR` queued immediately before a
/// `Close` is still delivered — which is what makes "tell the client why, then
/// hang up" work at all.
///
/// `byte_limit` caps what [`Tx::try_send_json`] / [`Tx::try_send_raw`] will
/// queue. Pass `usize::MAX` for a peer that is only ever written with the
/// blocking sends, where waiting is the bound.
pub fn split(socket: WebSocket, queue: usize, byte_limit: usize) -> (Rx, Tx) {
    let (mut sink, stream) = socket.split();
    let (tx, mut rx) = mpsc::channel::<Message>(queue);
    let (lane, mut lane_rx) = mpsc::channel::<Message>(STREAM_END_QUEUE);
    let budget = Arc::new(Budget {
        queued: AtomicUsize::new(0),
        limit: byte_limit,
    });
    let ends = Arc::new(StreamEnds::new(lane));
    let writer_budget = budget.clone();
    let writer_ends = ends.clone();
    let mut killed = ends.killed.subscribe();
    tokio::spawn(async move {
        loop {
            // `Ok(_)`: a dropped sender is not a kill, and must not cut off the
            // `ERROR` and `Close` an ordinary teardown queued last.
            let (message, queued) = tokio::select! {
                biased;
                Ok(_) = killed.wait_for(|killed| *killed) => return,
                Some(message) = lane_rx.recv() => (message, false),
                message = rx.recv() => match message {
                    Some(message) => (message, true),
                    None => break,
                },
            };
            let len = if queued { cost(&message) } else { 0 };
            if queued {
                let discard = writer_ends.discards(&message);
                if rx.is_empty() {
                    writer_ends.forget_ended();
                }
                if discard {
                    writer_budget.refund(len);
                    continue;
                }
            }
            let sent = tokio::select! {
                biased;
                Ok(_) = killed.wait_for(|killed| *killed) => return,
                sent = sink.send(message) => sent,
            };
            // Refunded after the write, not on dequeue: until the socket has
            // taken the frame, the relay is still holding its bytes.
            writer_budget.refund(len);
            if sent.is_err() {
                break;
            }
        }
        let _ = sink.close().await;
    });
    (
        stream,
        Tx {
            inner: tx,
            budget,
            ends,
        },
    )
}

/// One frame, classified.
pub enum Incoming {
    Control(Frame),
    /// A body chunk. `raw` is the frame exactly as it arrived, because the
    /// relay MUST NOT rewrite payload bytes (§5.2) — it re-sends these bytes
    /// rather than re-encoding a header it parsed.
    Body {
        kind: u8,
        stream_id: u32,
        raw: Vec<u8>,
    },
}

/// Reads the next frame, skipping transport-level ping/pong.
///
/// Transport ping/pong is the WebSocket layer's own keepalive and is unrelated
/// to SRP's `PING`/`PONG` control messages; axum answers the former itself.
pub async fn recv(rx: &mut Rx) -> Result<Incoming, Fault> {
    loop {
        let Some(message) = rx.next().await else {
            return Err(Fault::Disconnected);
        };
        match message {
            Ok(Message::Text(text)) => {
                return Frame::parse(text.as_str())
                    .map(Incoming::Control)
                    .map_err(Fault::Code);
            }
            Ok(Message::Binary(bytes)) => {
                let (kind, stream_id) = proto::parse_body_header(&bytes).map_err(Fault::Code)?;
                return Ok(Incoming::Body {
                    kind,
                    stream_id,
                    raw: bytes.into(),
                });
            }
            Ok(Message::Ping(_) | Message::Pong(_)) => continue,
            Ok(Message::Close(_)) | Err(_) => return Err(Fault::Disconnected),
        }
    }
}

/// Reads the next frame, refusing body chunks.
///
/// Used before any stream exists on a trunk: a `0x01`/`0x02` frame names a
/// `stream_id`, and during a handshake there is no id it could name.
pub async fn recv_control(rx: &mut Rx) -> Result<Frame, Fault> {
    match recv(rx).await? {
        Incoming::Control(frame) => Ok(frame),
        Incoming::Body { .. } => Err(ErrorCode::ProtocolError.into()),
    }
}

/// The next control frame of a handshake. A `PING` may arrive at any point on
/// a trunk and is answered in order (§4.2, decision 108c), so one sent before
/// `REGISTERED` or `READY` is answered here rather than refused.
pub async fn recv_handshake_control(rx: &mut Rx, tx: &Tx) -> Result<Frame, Fault> {
    loop {
        let frame = recv_control(rx).await?;
        match frame.ty() {
            "PING" => tx.send_json(proto::pong()).await?,
            "PONG" => {}
            _ => return Ok(frame),
        }
    }
}

/// Reports the fault, if there is still anyone to report it to, and closes.
pub async fn close(tx: &Tx, fault: Fault) {
    if let Fault::Code(code) = fault {
        let _ = tx.send_json(proto::error(code)).await;
    }
    tx.close();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_queue_admits_one_frame_larger_than_the_whole_budget() {
        // An attachment is one frame the size of the file. A budget that
        // refused it outright could never deliver it to anyone.
        let (tx, _rx) = Tx::detached_with_budget(8, 100);
        assert!(tx.try_send_raw(vec![0; 1000]).is_ok());
        // ...but nothing queues behind it.
        assert!(tx.try_send_raw(vec![0; 1]).is_err());
    }

    #[test]
    fn frames_queue_until_the_next_would_pass_the_budget() {
        let (tx, _rx) = Tx::detached_with_budget(8, 100);
        assert!(tx.try_send_raw(vec![0; 60]).is_ok());
        assert!(tx.try_send_raw(vec![0; 40]).is_ok());
        assert!(tx.try_send_raw(vec![0; 1]).is_err());
    }

    #[test]
    fn a_refused_frame_costs_nothing() {
        // A refusal that still charged the budget would leak it, and the peer
        // would be backlogged for ever after one refusal.
        let (tx, _rx) = Tx::detached_with_budget(1, 100);
        assert!(tx.try_send_raw(vec![0; 10]).is_ok());
        // Channel full: refused, and the 10 bytes must not be charged.
        assert!(tx.try_send_raw(vec![0; 10]).is_err());
        assert_eq!(tx.budget.queued.load(Ordering::Acquire), 10);
    }

    #[tokio::test]
    async fn a_stream_end_that_finds_no_room_drops_the_trunk() {
        // Overflowing the lane must not lose the end silently, which is the
        // hang this lane exists to prevent.
        let (inner, _rx) = mpsc::channel(1);
        let (lane, _lane_rx) = mpsc::channel(1);
        let tx = Tx {
            inner,
            budget: Arc::new(Budget {
                queued: AtomicUsize::new(0),
                limit: usize::MAX,
            }),
            ends: Arc::new(StreamEnds::new(lane)),
        };
        tx.end_stream(1, "{}".into());
        assert!(!*tx.ends.killed.borrow());
        tx.end_stream(2, "{}".into());
        tokio::time::timeout(std::time::Duration::from_secs(1), tx.killed())
            .await
            .expect("an overflowing stream end did not drop the trunk");
    }

    #[test]
    fn only_an_ended_streams_body_frames_are_discarded() {
        let (lane, _lane_rx) = mpsc::channel(1);
        let ends = StreamEnds::new(lane);
        ends.ended.lock().unwrap().insert(7);
        let body = |id: u32| {
            let mut frame = vec![proto::BODY_RESPONSE];
            frame.extend_from_slice(&id.to_be_bytes());
            Message::Binary(frame.into())
        };
        assert!(ends.discards(&body(7)));
        assert!(!ends.discards(&body(8)));
        assert!(!ends.discards(&Message::Text(r#"{"stream_id":7}"#.into())));
        ends.forget_ended();
        assert!(!ends.discards(&body(7)));
    }

    #[test]
    fn close_is_never_refused_for_want_of_room() {
        let (tx, mut rx) = Tx::detached_with_budget(8, 10);
        assert!(tx.try_send_raw(vec![0; 10]).is_ok());
        tx.close();
        let _ = rx.try_recv().unwrap();
        assert!(matches!(rx.try_recv().unwrap(), Message::Close(None)));
    }
}
