//! One WebSocket to the relay, as the suite sees it: frames in, frames out,
//! and every frame from the relay checked against §3's envelope on arrival.
//!
//! A reader and a writer task own the socket, so a server's heartbeat can go
//! out while a check is waiting on something else, and a client can stop
//! reading its socket on purpose (`pause`), which is how a slow client is
//! made.

use std::net::IpAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail, ensure};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;

pub const BODY_REQUEST: u8 = 0x01;
pub const BODY_RESPONSE: u8 = 0x02;

#[derive(Debug)]
pub enum Frame {
    Text(Value),
    Body {
        kind: u8,
        stream_id: u32,
        payload: Vec<u8>,
    },
    /// The relay sent something §3 does not allow: not JSON, no `v: 1`, no
    /// `type`, or a binary frame shorter than its header.
    Invalid(String),
    Closed,
}

impl Frame {
    pub fn describe(&self) -> String {
        match self {
            Self::Text(value) => value.to_string(),
            Self::Body {
                kind,
                stream_id,
                payload,
            } => format!(
                "binary 0x{kind:02x} stream {stream_id}, {} bytes",
                payload.len()
            ),
            Self::Invalid(why) => format!("an invalid frame ({why})"),
            Self::Closed => "the connection closing".into(),
        }
    }

    pub fn is_type(&self, ty: &str) -> bool {
        matches!(self, Self::Text(value) if value["type"] == ty)
    }
}

type Outgoing = (Message, oneshot::Sender<bool>);

pub struct Conn {
    label: String,
    out: mpsc::UnboundedSender<Outgoing>,
    inbox: mpsc::UnboundedReceiver<Frame>,
    paused: watch::Sender<bool>,
    show_heartbeats: Arc<AtomicBool>,
    answer_pings: Arc<AtomicBool>,
    tasks: Vec<JoinHandle<()>>,
    pinger: Option<JoinHandle<()>>,
    closed: bool,
}

impl Drop for Conn {
    fn drop(&mut self) {
        self.kill();
    }
}

fn ws_config() -> WebSocketConfig {
    // Room above §5.2's 16 MiB frame ceiling, so the suite is never the side
    // that refuses a frame.
    WebSocketConfig::default()
        .max_message_size(Some(64 << 20))
        .max_frame_size(Some(32 << 20))
}

pub async fn connect(url: &str, label: &str, source: Option<IpAddr>) -> Result<Conn> {
    let socket = match source {
        None => {
            tokio_tungstenite::connect_async_with_config(url, Some(ws_config()), true)
                .await
                .with_context(|| format!("{label}: connecting to {url}"))?
                .0
        }
        Some(source) => {
            let request = url.into_client_request()?;
            ensure!(
                request.uri().scheme_str() == Some("ws"),
                "a source address needs a ws:// relay"
            );
            let host = request.uri().host().ok_or_else(|| anyhow!("no host"))?;
            let port = request.uri().port_u16().unwrap_or(80);
            let addr = tokio::net::lookup_host((host, port))
                .await?
                .find(|addr| addr.is_ipv4() == source.is_ipv4())
                .ok_or_else(|| anyhow!("{host} has no address of {source}'s family"))?;
            let socket = match source {
                IpAddr::V4(_) => tokio::net::TcpSocket::new_v4()?,
                IpAddr::V6(_) => tokio::net::TcpSocket::new_v6()?,
            };
            socket
                .bind((source, 0).into())
                .with_context(|| format!("binding {source}"))?;
            let stream = socket.connect(addr).await?;
            stream.set_nodelay(true)?;
            tokio_tungstenite::client_async_with_config(
                request,
                tokio_tungstenite::MaybeTlsStream::Plain(stream),
                Some(ws_config()),
            )
            .await
            .with_context(|| format!("{label}: connecting to {url} from {source}"))?
            .0
        }
    };
    Ok(Conn::spawn(socket, label))
}

impl Conn {
    pub(crate) fn spawn<S>(socket: tokio_tungstenite::WebSocketStream<S>, label: &str) -> Self
    where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
    {
        let (mut sink, mut stream) = socket.split();
        let (out, mut out_rx) = mpsc::unbounded_channel::<Outgoing>();
        let (inbox_tx, inbox) = mpsc::unbounded_channel();
        let (paused, mut paused_rx) = watch::channel(false);
        let show_heartbeats = Arc::new(AtomicBool::new(false));
        let answer_pings = Arc::new(AtomicBool::new(true));

        let writer = tokio::spawn(async move {
            while let Some((message, done)) = out_rx.recv().await {
                let sent = sink.send(message).await.is_ok();
                let _ = done.send(sent);
                if !sent {
                    break;
                }
            }
        });

        let pong_out = out.clone();
        let show = show_heartbeats.clone();
        let answer = answer_pings.clone();
        let reader = tokio::spawn(async move {
            loop {
                if *paused_rx.borrow() {
                    if paused_rx.changed().await.is_err() {
                        return;
                    }
                    continue;
                }
                let message = tokio::select! {
                    message = stream.next() => message,
                    _ = paused_rx.changed() => continue,
                };
                let frame = match message {
                    Some(Ok(Message::Text(text))) => parse_text(&text),
                    Some(Ok(Message::Binary(bytes))) => parse_binary(&bytes),
                    Some(Ok(Message::Ping(_) | Message::Pong(_) | Message::Frame(_))) => continue,
                    Some(Ok(Message::Close(_))) | Some(Err(_)) | None => {
                        let _ = inbox_tx.send(Frame::Closed);
                        return;
                    }
                };
                if let Frame::Text(value) = &frame {
                    let heartbeat = value["type"] == "PING" || value["type"] == "PONG";
                    if value["type"] == "PING" && answer.load(Ordering::Relaxed) {
                        let (done, _) = oneshot::channel();
                        let pong = json!({ "v": 1, "type": "PONG" }).to_string();
                        let _ = pong_out.send((Message::Text(pong.into()), done));
                    }
                    if heartbeat && !show.load(Ordering::Relaxed) {
                        continue;
                    }
                }
                if inbox_tx.send(frame).is_err() {
                    return;
                }
            }
        });

        Self {
            label: label.to_string(),
            out,
            inbox,
            paused,
            show_heartbeats,
            answer_pings,
            tasks: vec![writer, reader],
            pinger: None,
            closed: false,
        }
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    /// Drops the socket without a close handshake: a process that died.
    pub fn kill(&mut self) {
        if let Some(pinger) = self.pinger.take() {
            pinger.abort();
        }
        for task in &self.tasks {
            task.abort();
        }
    }

    pub fn pause(&self) {
        self.paused.send_replace(true);
    }

    pub fn resume(&self) {
        self.paused.send_replace(false);
    }

    pub fn show_heartbeats(&self, show: bool) {
        self.show_heartbeats.store(show, Ordering::Relaxed);
    }

    /// `PING` every `interval`, the way a real origin heartbeats (§4.2).
    pub fn start_heartbeat(&mut self, interval: Duration) {
        let out = self.out.clone();
        self.pinger = Some(tokio::spawn(async move {
            let mut tick = tokio::time::interval(interval);
            tick.tick().await;
            loop {
                tick.tick().await;
                let (done, _) = oneshot::channel();
                let ping = json!({ "v": 1, "type": "PING" }).to_string();
                if out.send((Message::Text(ping.into()), done)).is_err() {
                    return;
                }
            }
        }));
    }

    /// Stops its own heartbeat but keeps answering the relay's `PING`s: an
    /// origin that only ever replies.
    pub fn stop_heartbeat(&mut self) {
        if let Some(pinger) = self.pinger.take() {
            pinger.abort();
        }
    }

    /// Stops sending anything of its own accord, heartbeats and their answers
    /// both: a server that is still connected and has stopped talking.
    pub fn go_silent(&mut self) {
        if let Some(pinger) = self.pinger.take() {
            pinger.abort();
        }
        self.answer_pings.store(false, Ordering::Relaxed);
    }

    async fn send(&self, message: Message) -> Result<()> {
        let (done, sent) = oneshot::channel();
        self.out
            .send((message, done))
            .map_err(|_| anyhow!("{}: the connection is closed", self.label))?;
        match sent.await {
            Ok(true) => Ok(()),
            _ => bail!("{}: the connection is closed", self.label),
        }
    }

    pub async fn send_json(&self, value: Value) -> Result<()> {
        self.send(Message::Text(value.to_string().into())).await
    }

    pub async fn send_text(&self, text: &str) -> Result<()> {
        self.send(Message::Text(text.to_string().into())).await
    }

    pub async fn send_binary(&self, bytes: Vec<u8>) -> Result<()> {
        self.send(Message::Binary(bytes.into())).await
    }

    pub async fn send_body(&self, kind: u8, stream_id: u32, payload: &[u8]) -> Result<()> {
        self.send_binary(body_frame(kind, stream_id, payload)).await
    }

    /// The next frame, or an error naming what was being waited for.
    pub async fn next(&mut self, within: Duration) -> Result<Frame> {
        if self.closed {
            return Ok(Frame::Closed);
        }
        match tokio::time::timeout(within, self.inbox.recv()).await {
            Ok(Some(frame)) => {
                if matches!(frame, Frame::Closed) {
                    self.closed = true;
                }
                Ok(frame)
            }
            Ok(None) => {
                self.closed = true;
                Ok(Frame::Closed)
            }
            Err(_) => bail!("{}: nothing arrived within {within:?}", self.label),
        }
    }

    /// A frame already received, without waiting.
    pub fn try_next(&mut self) -> Option<Frame> {
        self.inbox.try_recv().ok()
    }

    /// The next frame, which must be a control message.
    pub async fn recv(&mut self, within: Duration) -> Result<Value> {
        match self.next(within).await? {
            Frame::Text(value) => Ok(value),
            other => bail!(
                "{}: expected a control message, got {}",
                self.label,
                other.describe()
            ),
        }
    }

    pub async fn recv_type(&mut self, ty: &str, within: Duration) -> Result<Value> {
        let value = self.recv(within).await?;
        ensure!(
            value["type"] == ty,
            "{}: expected {ty}, got {value}",
            self.label
        );
        Ok(value)
    }

    pub async fn expect_error(&mut self, code: &str, within: Duration) -> Result<Value> {
        let value = self.recv(within).await?;
        check_error(&value, code).with_context(|| self.label.clone())?;
        Ok(value)
    }

    pub async fn recv_body(&mut self, within: Duration) -> Result<(u8, u32, Vec<u8>)> {
        match self.next(within).await? {
            Frame::Body {
                kind,
                stream_id,
                payload,
            } => Ok((kind, stream_id, payload)),
            other => bail!(
                "{}: expected a body frame, got {}",
                self.label,
                other.describe()
            ),
        }
    }

    /// Skips frames until one satisfies `wanted`. Closing first is a failure.
    pub async fn wait_for(
        &mut self,
        what: &str,
        within: Duration,
        wanted: impl Fn(&Frame) -> bool,
    ) -> Result<Frame> {
        let deadline = tokio::time::Instant::now() + within;
        loop {
            let left = deadline.saturating_duration_since(tokio::time::Instant::now());
            let frame = self
                .next(left)
                .await
                .with_context(|| format!("{}: waiting for {what}", self.label))?;
            if wanted(&frame) {
                return Ok(frame);
            }
            if matches!(frame, Frame::Closed) {
                bail!("{}: closed while waiting for {what}", self.label);
            }
        }
    }

    /// Nothing at all arrives for `window`: the only way to prove a frame did
    /// not go somewhere.
    pub async fn expect_silence(&mut self, window: Duration) -> Result<()> {
        match tokio::time::timeout(window, self.inbox.recv()).await {
            Err(_) => Ok(()),
            Ok(Some(frame)) => bail!("{}: expected nothing, got {}", self.label, frame.describe()),
            Ok(None) => bail!("{}: expected nothing, and it closed", self.label),
        }
    }

    /// The relay closes this connection within `within`, sending nothing more
    /// first.
    pub async fn expect_closed(&mut self, within: Duration) -> Result<()> {
        match self.next(within).await {
            Ok(Frame::Closed) => Ok(()),
            Ok(other) => bail!(
                "{}: expected the relay to close, got {}",
                self.label,
                other.describe()
            ),
            Err(_) => bail!("{}: the relay did not close within {within:?}", self.label),
        }
    }
}

pub fn body_frame(kind: u8, stream_id: u32, payload: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(5 + payload.len());
    frame.push(kind);
    frame.extend_from_slice(&stream_id.to_be_bytes());
    frame.extend_from_slice(payload);
    frame
}

fn parse_text(text: &str) -> Frame {
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        return Frame::Invalid(format!("not JSON: {text}"));
    };
    // §3: every control message carries `v: 1` and a `type`.
    if value["v"] != 1 || !value["type"].is_string() {
        return Frame::Invalid(format!("no v:1 envelope: {text}"));
    }
    Frame::Text(value)
}

fn parse_binary(bytes: &[u8]) -> Frame {
    if bytes.len() < 5 {
        return Frame::Invalid(format!("a {}-byte binary frame", bytes.len()));
    }
    Frame::Body {
        kind: bytes[0],
        stream_id: u32::from_be_bytes([bytes[1], bytes[2], bytes[3], bytes[4]]),
        payload: bytes[5..].to_vec(),
    }
}

/// §6: an `ERROR` with a string `code` and a string `message`.
pub fn check_error(value: &Value, code: &str) -> Result<()> {
    ensure!(
        value["type"] == "ERROR",
        "expected ERROR {code}, got {value}"
    );
    ensure!(
        value["code"] == code,
        "expected ERROR {code}, got code {} in {value}",
        value["code"]
    );
    ensure!(
        value["message"].is_string(),
        "an ERROR must carry a string message: {value}"
    );
    Ok(())
}

pub fn check_trunk_scoped(value: &Value) -> Result<()> {
    ensure!(
        value.get("stream_id").is_none() && value.get("attempt_id").is_none(),
        "expected a trunk-scoped ERROR (no stream_id, no attempt_id): {value}"
    );
    Ok(())
}

pub fn check_on_stream(value: &Value, stream_id: u32) -> Result<()> {
    ensure!(
        value["stream_id"] == stream_id,
        "expected the ERROR to name stream {stream_id}: {value}"
    );
    Ok(())
}

pub fn stream_id_of(value: &Value) -> Result<u32> {
    value["stream_id"]
        .as_u64()
        .and_then(|id| u32::try_from(id).ok())
        .ok_or_else(|| anyhow!("stream_id is not a u32 in {value}"))
}
