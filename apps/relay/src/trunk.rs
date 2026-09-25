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

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use axum::extract::ws::{Message, WebSocket};
use futures_util::{SinkExt, StreamExt, stream::SplitStream};
use tokio::sync::mpsc;

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
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |queued| {
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

/// The peer could not keep up. Never widened into an error code: which peer is
/// too slow is the relay's business, and §6 has no honest code for it beyond
/// `rate_limited`.
#[derive(Debug, Clone, Copy)]
pub struct Backlogged;

/// A cloneable handle for writing to one trunk's socket.
#[derive(Debug, Clone)]
pub struct Tx {
    inner: mpsc::Sender<Message>,
    budget: Arc<Budget>,
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
        (Self { inner: tx, budget }, rx)
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
    let budget = Arc::new(Budget {
        queued: AtomicUsize::new(0),
        limit: byte_limit,
    });
    let writer_budget = budget.clone();
    tokio::spawn(async move {
        while let Some(message) = rx.recv().await {
            let len = cost(&message);
            let sent = sink.send(message).await;
            // Refunded after the write, not on dequeue: until the socket has
            // taken the frame, the relay is still holding its bytes.
            writer_budget.refund(len);
            if sent.is_err() {
                break;
            }
        }
        let _ = sink.close().await;
    });
    (stream, Tx { inner: tx, budget })
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

    #[test]
    fn close_is_never_refused_for_want_of_room() {
        let (tx, mut rx) = Tx::detached_with_budget(8, 10);
        assert!(tx.try_send_raw(vec![0; 10]).is_ok());
        tx.close();
        let _ = rx.try_recv().unwrap();
        assert!(matches!(rx.try_recv().unwrap(), Message::Close(None)));
    }
}
