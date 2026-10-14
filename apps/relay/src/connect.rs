//! The client trunk (§5): `HELLO`, `OPEN_STREAM`, and the client→server hop of
//! a tunnelled HTTP exchange.
//!
//! ```text
//! Client → Relay:  HELLO { server_id, pubkey }
//! Relay → Client:  READY { client_trunk_id }
//! Client → Relay:  OPEN_STREAM { attempt_id }
//! Relay → Client:  STREAM_READY { attempt_id, stream_id }
//! Relay → Server:  STREAM_OPEN { stream_id }
//! ```
//!
//! **The relay authenticates no clients (R12).** There is no challenge here and
//! there must never be one: a client's device credential, session token or
//! `stk_` key rides *inside* the tunnelled request and is checked by the origin
//! exactly as on the LAN. Any client speaking valid SRP gets a trunk. What
//! client connections need is resource limits, not identity.
//!
//! Two rules in this file are security rather than plumbing, and both are about
//! the same thing — the relay multiplexes many mutually distrusting clients
//! onto **one** server trunk:
//!
//! - a client never names a `stream_id` it was not given ([`route_client_ward`]);
//! - a client never sets `relay_peer_ip` ([`forward_request_head`]).

use std::net::SocketAddr;
use std::sync::Arc;

use crate::Relay;
use crate::auth::{KeyBytes, decode_key, validate_server_id};
use crate::config::Mode;
use crate::proto::{self, ErrorCode, Frame, Hello};
use crate::state::{HeadClaim, Only, Registration, ServerTrunk};
use crate::trunk::{self, Fault, Incoming, Rx, Tx};

/// The largest `attempt_id` the relay will echo.
///
/// §6 leaves the field untyped, so the relay accepts any JSON — but it holds
/// the value until `STREAM_READY` goes out, and an unbounded field on an
/// unauthenticated connection is memory an anonymous caller controls.
const MAX_ATTEMPT_ID_BYTES: usize = 256;

/// Serves one client trunk from upgrade to close.
///
/// `peer` is the socket's own address, taken from the connection and never from
/// a header — it becomes `relay_peer_ip`, and a header-derived value would make
/// this `X-Forwarded-For` with extra steps.
pub async fn serve(
    socket: axum::extract::ws::WebSocket,
    relay: Arc<Relay>,
    path_server_id: String,
    query_key: Option<String>,
    peer: SocketAddr,
) {
    // The byte budget applies here and only here: this is the queue the
    // server trunk's shared reader fills without waiting, so it is the one a
    // client that stops reading can make grow.
    let (mut rx, tx) = trunk::split(
        socket,
        trunk::CLIENT_WARD_QUEUE,
        relay.config.max_client_buffer_bytes,
    );

    // Bounded like the server-side handshake: the relay authenticates nobody at
    // the door, so anyone who can connect can start one and leave it hanging.
    let opened = match tokio::time::timeout(
        relay.config.handshake_timeout,
        hello(
            &mut rx,
            &tx,
            &relay,
            &path_server_id,
            query_key.as_deref(),
            peer.ip(),
        ),
    )
    .await
    {
        Ok(opened) => opened,
        Err(_) => {
            tracing::debug!("client trunk handshake timed out");
            Err(Fault::Disconnected)
        }
    };

    let session = match opened {
        Ok(session) => session,
        Err(fault) => return trunk::close(&tx, fault).await,
    };

    tracing::info!(
        server_id = %path_server_id,
        client_trunk_id = %session.client_trunk_id,
        "client trunk opened"
    );

    let fault = trunk_loop(&mut rx, &tx, &relay, &session, peer).await;

    // Whatever ended the trunk, its streams must not outlive it on the server
    // side (§5.4). Freed here rather than by a timeout so a client that hangs
    // up mid-request does not leave the origin holding a request that will
    // never finish.
    session.trunk.detach_client(&session.client_trunk_id);
    let orphaned = session.trunk.close_streams_of(&session.client_trunk_id);
    let server_tx = session.trunk.tx();
    for stream_id in orphaned {
        server_tx.close_toward_origin(stream_id);
    }

    tracing::info!(
        client_trunk_id = %session.client_trunk_id,
        "client trunk closed"
    );
    trunk::close(&tx, fault).await;
}

struct Session {
    client_trunk_id: String,
    /// The specific trunk this client is bound to, not the `server_id`. A
    /// supersession replaces the registration; this client stays on the trunk
    /// its streams are scoped to and learns about the change as `trunk_lost`.
    trunk: Arc<ServerTrunk>,
    /// This client's own lane toward the origin (§5.5, decision 116d). Its
    /// opens, heads and bodies wait only behind its own earlier frames.
    to_origin: Tx,
}

async fn hello(
    rx: &mut Rx,
    tx: &Tx,
    relay: &Relay,
    path_server_id: &str,
    query_key: Option<&str>,
    peer_ip: std::net::IpAddr,
) -> Result<Session, Fault> {
    // Rate limit HELLO attempts per source IP.
    if !relay.check_hello_rate_limit(peer_ip) {
        tracing::info!(peer_ip = %peer_ip, "rate limiting HELLO");
        return Err(ErrorCode::RateLimited.into());
    }

    // The path segment reaches the signed relay-auth message and the derived
    // `public_address` in exactly the charset `validate_server_id` allows, so a
    // value outside it cannot have come from a real registration.
    validate_server_id(path_server_id).map_err(|why| {
        tracing::debug!(reason = why, "rejecting /connect: bad server_id in path");
        ErrorCode::ProtocolError
    })?;

    let frame = trunk::recv_handshake_control(rx, tx).await?;
    if frame.ty() != "HELLO" {
        return Err(ErrorCode::ProtocolError.into());
    }
    let key = hello_key(&frame, query_key)?;
    let hello: Hello = frame.body()?;

    // §5: `HELLO.server_id` must equal the path. Refused rather than resolved:
    // picking one silently would let a client dial one server's address and be
    // routed to another.
    if hello.server_id != path_server_id {
        tracing::debug!("rejecting HELLO: server_id disagrees with the path");
        return Err(ErrorCode::ProtocolError.into());
    }

    let server_id = hello.server_id.as_str();
    let found = match (relay.config.mode(), key) {
        (_, Some(key)) => await_trunk(relay, || relay.registrations.get(server_id, &key)).await,
        // Storm's open relays refuse every keyless HELLO at once (§5, the 114f
        // addendum): without a key there is nothing to address.
        (Mode::Open, None) => None,
        // Before 115: the one registration, whatever its key.
        (_, None) => {
            await_trunk(relay, || match relay.registrations.only(server_id) {
                Only::One(registration) => Some(registration),
                Only::None | Only::Several => None,
            })
            .await
        }
    };
    let Some(registration) = found else {
        tracing::debug!(server_id, "no live server trunk for HELLO");
        return Err(ErrorCode::ServerUnreachable.into());
    };

    let client_trunk_id = new_client_trunk_id();
    registration
        .trunk
        .attach_client(&client_trunk_id, tx.clone());
    tx.send_json(proto::ready(client_trunk_id.clone())).await?;

    let to_origin = registration.trunk.tx().lane();
    Ok(Session {
        client_trunk_id,
        trunk: registration.trunk,
        to_origin,
    })
}

/// The key a `HELLO` names (§5), if any: `HELLO.pubkey`, which must decode to
/// 32 bytes and agree with `?pubkey=` when both are present. A query without
/// a `HELLO.pubkey` names nothing; the table in §5 is keyed on the `HELLO`.
fn hello_key(frame: &Frame, query_key: Option<&str>) -> Result<Option<KeyBytes>, Fault> {
    let refuse = |why: &str| {
        tracing::debug!(reason = why, "rejecting HELLO: bad pubkey");
        Fault::from(ErrorCode::ProtocolError)
    };
    let query = query_key
        .map(|q| decode_key(q).map_err(|_| refuse("query")))
        .transpose()?;
    let named = match frame.body.get("pubkey") {
        None => None,
        Some(serde_json::Value::String(text)) => {
            Some(decode_key(text).map_err(|_| refuse("HELLO"))?)
        }
        Some(_) => return Err(refuse("HELLO.pubkey is not a string")),
    };
    if let (Some(named), Some(query)) = (named, query)
        && named != query
    {
        return Err(refuse("HELLO and query disagree"));
    }
    Ok(named)
}

/// Waits up to `hello_wait` for a live trunk, covering a server mid-restart.
///
/// Polled rather than notified. A notification bus would have to be woken by
/// registration, which means registration would have to know that client trunks
/// exist — and the wait is bounded and rare enough that the coupling costs more
/// than the polling does.
async fn await_trunk(
    relay: &Relay,
    find: impl Fn() -> Option<Registration>,
) -> Option<Registration> {
    const POLL: std::time::Duration = std::time::Duration::from_millis(20);
    let deadline = tokio::time::Instant::now() + relay.config.hello_wait;
    loop {
        if let Some(registration) = find() {
            return Some(registration);
        }
        let now = tokio::time::Instant::now();
        if now >= deadline {
            return None;
        }
        tokio::time::sleep(POLL.min(deadline - now)).await;
    }
}

async fn trunk_loop(
    rx: &mut Rx,
    tx: &Tx,
    relay: &Relay,
    session: &Session,
    peer: SocketAddr,
) -> Fault {
    loop {
        // §5 client heartbeat: a client sends `PING` at least every 15 s, so a
        // trunk silent for the deadline is gone, and the caller's teardown
        // ends its streams at the origin as if it had closed. A timeout on the
        // read itself, as on the server side, or a silent client is never
        // noticed at all.
        let incoming = tokio::select! {
            biased;
            () = tx.killed() => return Fault::Disconnected,
            incoming = tokio::time::timeout(relay.config.heartbeat_deadline, trunk::recv(rx)) => match incoming {
                Ok(Ok(incoming)) => incoming,
                Ok(Err(fault)) => return fault,
                Err(_) => {
                    tracing::info!(
                        client_trunk_id = %session.client_trunk_id,
                        "client silent past the heartbeat deadline; closing its trunk"
                    );
                    return Fault::Disconnected;
                }
            },
        };

        let outcome = match incoming {
            Incoming::Control(frame) => match frame.ty() {
                "OPEN_STREAM" => open_stream(tx, relay, session, frame).await,
                "HTTP_REQUEST_HEAD" => forward_request_head(tx, session, frame, peer).await,
                "CLOSE" => match close_message(tx, session, frame).await {
                    Ok(true) => return Fault::Disconnected,
                    Ok(false) => Ok(()),
                    Err(fault) => Err(fault),
                },
                "PING" => {
                    tx.send_control(proto::pong());
                    Ok(())
                }
                "PONG" => Ok(()),
                // Including `STREAM_ACK`, `HTTP_RESPONSE_HEAD` and everything
                // else that is server→relay: a client sending one is not a
                // no-op to ignore, it is a client speaking the wrong role.
                _ => Err(ErrorCode::ProtocolError.into()),
            },
            Incoming::Body {
                kind,
                stream_id,
                raw,
            } => forward_request_body(tx, session, kind, stream_id, raw).await,
        };

        if let Err(fault) = outcome {
            return fault;
        }
    }
}

async fn open_stream(tx: &Tx, relay: &Relay, session: &Session, frame: Frame) -> Result<(), Fault> {
    let Some(attempt_id) = frame.body.get("attempt_id").cloned() else {
        return Err(ErrorCode::ProtocolError.into());
    };
    if attempt_id.to_string().len() > MAX_ATTEMPT_ID_BYTES {
        return Err(ErrorCode::ProtocolError.into());
    }

    let stream_id = match session.trunk.open_stream(
        &session.client_trunk_id,
        tx.clone(),
        relay.config.max_in_flight_streams,
        relay.config.max_total_streams,
    ) {
        Ok(stream_id) => stream_id,
        Err(refused) => {
            // Refused **immediately**, never queued (§5.1). Queueing would make
            // an overloaded relay look exactly like a slow server, and the two
            // want opposite responses from a client.
            tracing::debug!(
                client_trunk_id = %session.client_trunk_id,
                ?refused,
                "refusing OPEN_STREAM"
            );
            // One code for both bounds. Which limit a caller hit is the relay's
            // business, and §6 has exactly one code for "a limit was hit".
            // The `attempt_id` names the open this refuses; without it a client
            // can only guess, and the trunk carries on either way (§5.1).
            return tx
                .send_json(proto::error_on_attempt(ErrorCode::RateLimited, attempt_id))
                .await;
        }
    };

    // `attempt_id` is echoed here and never again: it is how a client
    // correlates concurrent opens before it learns the relay-assigned
    // `stream_id`, and the relay never routes on it (§5.1).
    tx.send_json(proto::stream_ready(attempt_id, stream_id))
        .await?;
    session
        .to_origin
        .send_json(proto::stream_open(stream_id))
        .await?;

    arm_ack_timeout(session.trunk.clone(), tx.clone(), stream_id, relay);
    Ok(())
}

/// Fails a stream the origin never acknowledged.
///
/// A task per open rather than one sweeper: the deadline is per stream and the
/// task is asleep for all of it. Note what it does *not* bound — once the ack
/// arrives, nothing here fires again, so a response that takes minutes to
/// produce its first byte, or produces none at all, is untouched (§5.2).
fn arm_ack_timeout(trunk: Arc<ServerTrunk>, client: Tx, stream_id: u32, relay: &Relay) {
    let timeout = relay.config.stream_ack_timeout;
    tokio::spawn(async move {
        tokio::time::sleep(timeout).await;
        if !trunk.is_awaiting_ack(stream_id) {
            return;
        }
        trunk.close_stream(stream_id);
        // No retry (§5.1). The client owns the decision to try again; a relay
        // retrying would duplicate a request whose side effects it cannot see.
        client.end_stream(
            stream_id,
            proto::error_on_stream(ErrorCode::ServerTimeout, stream_id),
        );
        trunk.tx().close_toward_origin(stream_id);
    });
}

/// The client→server hop of `HTTP_REQUEST_HEAD`, and where `relay_peer_ip` is
/// decided.
///
/// Both halves of §5.2's rule live here, and **either one alone is worthless**:
///
/// 1. the field is set from the socket on every server-ward request, whatever
///    the client sent;
/// 2. a client that sent it at all is refused with `protocol_error`.
///
/// Without (2), (1) is only as good as the relay's own diligence — one forward
/// path that forgets to overwrite and the origin is trusting a client-supplied
/// address. Refusing the field outright means the relay never has to be careful
/// about it again: a `relay_peer_ip` on a server-ward head is, structurally,
/// one the relay wrote.
async fn forward_request_head(
    tx: &Tx,
    session: &Session,
    frame: Frame,
    peer: SocketAddr,
) -> Result<(), Fault> {
    let mut body = frame.body;
    if body.contains_key("relay_peer_ip") {
        tracing::info!(
            client_trunk_id = %session.client_trunk_id,
            "refusing HTTP_REQUEST_HEAD: client set relay_peer_ip"
        );
        return Err(ErrorCode::ProtocolError.into());
    }

    let stream_id = proto::stream_id_of(&body)?;
    // The trunk toward the origin is shared by every client of the server, so
    // a head the origin could not interpret is refused here, to its sender
    // only, and never forwarded (§5.2).
    if !proto::is_request_head(&body) {
        return Err(ErrorCode::ProtocolError.into());
    }

    match session
        .trunk
        .claim_head(stream_id, &session.client_trunk_id)
    {
        HeadClaim::First => {}
        HeadClaim::NotOwned => {
            route_client_ward(tx, session, stream_id).await?;
            return Ok(());
        }
        HeadClaim::Repeat => {
            tracing::debug!(
                client_trunk_id = %session.client_trunk_id,
                stream_id,
                "refusing a second HTTP_REQUEST_HEAD"
            );
            session.trunk.close_stream(stream_id);
            tx.end_stream(
                stream_id,
                proto::error_on_stream(ErrorCode::StreamClosed, stream_id),
            );
            return session
                .to_origin
                .send_json(proto::close_stream(stream_id))
                .await;
        }
    }
    let server_tx = &session.to_origin;

    // `.ip()`, not the full socket address: the field is what the origin buckets
    // rate limits by, and an ephemeral source port would split one client into a
    // fresh bucket per connection.
    body.insert(
        "relay_peer_ip".to_string(),
        serde_json::Value::String(peer.ip().to_string()),
    );
    server_tx
        .send_json(proto::relayed("HTTP_REQUEST_HEAD", body))
        .await
}

async fn forward_request_body(
    tx: &Tx,
    session: &Session,
    kind: u8,
    stream_id: u32,
    raw: Vec<u8>,
) -> Result<(), Fault> {
    // `0x02` is the response direction. A client sending one is speaking the
    // server's half of the protocol.
    if kind != proto::BODY_REQUEST {
        return Err(ErrorCode::ProtocolError.into());
    }
    let Some(server_tx) = route_client_ward(tx, session, stream_id).await? else {
        return Ok(());
    };
    // Forwarded as the bytes arrived, unbuffered and unrewritten (§5.2). The
    // relay does not reassemble, re-chunk or look inside a body.
    server_tx.send_raw(raw).await
}

/// Resolves a `stream_id` a **client** named, or answers `stream_closed`.
///
/// This is the check that makes the multiplexing safe. A `stream_id` maps to
/// exactly one client trunk; a client naming an id it does not own — another
/// client's live stream, an id that was never issued, one that has already
/// closed — gets the same `stream_closed` for all three, and the frame is
/// dropped rather than routed.
///
/// It answers `Ok(None)` rather than a fault: naming a stream that has closed
/// under you is an ordinary race, not grounds to tear down a trunk carrying
/// other healthy streams.
async fn route_client_ward(
    tx: &Tx,
    session: &Session,
    stream_id: u32,
) -> Result<Option<Tx>, Fault> {
    if session
        .trunk
        .client_owns(stream_id, &session.client_trunk_id)
    {
        return Ok(Some(session.to_origin.clone()));
    }
    tracing::debug!(
        client_trunk_id = %session.client_trunk_id,
        stream_id,
        "refusing a stream_id this client does not own"
    );
    tx.send_json(proto::error_on_stream(ErrorCode::StreamClosed, stream_id))
        .await?;
    Ok(None)
}

/// `CLOSE`. Returns whether the whole trunk is going away.
async fn close_message(tx: &Tx, session: &Session, frame: Frame) -> Result<bool, Fault> {
    let Some(value) = frame.body.get("stream_id") else {
        // `CLOSE {}` — the whole trunk (§5.4). Its streams are freed by the
        // caller's teardown, which runs on every exit path rather than only
        // this one.
        return Ok(true);
    };
    if value.is_null() {
        return Err(ErrorCode::ProtocolError.into());
    }
    let stream_id = proto::stream_id_of(&frame.body)?;

    if !session
        .trunk
        .client_owns(stream_id, &session.client_trunk_id)
    {
        // Closing a stream that is already gone is idempotent, and closing one
        // that belongs to someone else must be indistinguishable from it.
        tx.send_json(proto::error_on_stream(ErrorCode::StreamClosed, stream_id))
            .await?;
        return Ok(false);
    }
    session.trunk.close_stream(stream_id);
    session
        .trunk
        .tx()
        .send_json(proto::close_stream(stream_id))
        .await?;
    Ok(false)
}

/// A fresh client trunk id.
///
/// Random and prefixed distinctly from a server `trk_`, so a log line or a
/// `stream_id` owner never reads as the wrong kind of trunk. Random rather than
/// a counter for the same reason `new_trunk_id` is: a counter leaks how many
/// clients have ever connected.
fn new_client_trunk_id() -> String {
    use rand::Rng;
    let mut bytes = [0u8; 16];
    rand::rng().fill_bytes(&mut bytes);
    format!("ctk_{}", data_encoding::BASE64URL_NOPAD.encode(&bytes))
}
