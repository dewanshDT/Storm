//! The server-trunk handshake (§4).
//!
//! ```text
//! Server → Relay:  REGISTER_SERVER { server_id, pubkey }
//! Relay → Server:  CHALLENGE { nonce }                 // single-use, 30s TTL
//! Server → Relay:  CHALLENGE_RESPONSE { sig }
//! Relay → Server:  REGISTERED { trunk_id, public_address,
//!                               heartbeat_interval_secs: 15 }
//! ```
//!
//! **This is the only party the relay authenticates.** Clients present no
//! credential to the relay (R12); a client's device credential, session token
//! or `stk_` key rides *inside* the tunnelled HTTP request and is checked by
//! the origin server exactly as on the LAN. Nothing in this file may grow a
//! notion of a Storm user — the relay is never an authority (R5).
//!
//! Once registered, the trunk is also the **server→client** half of routing:
//! `STREAM_ACK`, `HTTP_RESPONSE_HEAD` and `0x02` body chunks arrive here and are
//! handed to the one client trunk that owns their `stream_id`.

use std::sync::Arc;
use std::time::{Instant, SystemTime};

use axum::extract::ws::WebSocket;

use crate::Relay;
use crate::auth::{PublicKey, validate_nonce, validate_server_id};
use crate::proto::{self, ChallengeResponse, ErrorCode, Frame, RegisterServer};
use crate::state::{Binding, Registration, ServerTrunk, new_trunk_id};
use crate::trunk::{self, Fault, Incoming, Rx, Tx};

/// Serves one server trunk from upgrade to close.
pub async fn serve(socket: WebSocket, relay: Arc<Relay>) {
    // Split before the handshake even though the handshake is strictly
    // sequential: the registration must be installed with a usable send handle
    // *before* `REGISTERED` goes out, or a client that raced in behind it would
    // find a trunk it cannot write to.
    let (mut rx, tx) = trunk::split(socket, trunk::SERVER_WARD_QUEUE);
    let server_trunk = Arc::new(ServerTrunk::new(tx.clone()));

    // A half-finished handshake must not hold a socket open indefinitely: the
    // relay authenticates nobody at the door, so anyone who can connect can
    // start one. Bounded separately from the nonce TTL — see `config`.
    let outcome = match tokio::time::timeout(
        relay.config.handshake_timeout,
        handshake(&mut rx, &tx, &relay, &server_trunk),
    )
    .await
    {
        Ok(outcome) => outcome,
        Err(_) => {
            tracing::debug!("server trunk handshake timed out");
            Err(Fault::Disconnected)
        }
    };

    let session = match outcome {
        Ok(session) => session,
        Err(fault) => return trunk::close(&tx, fault).await,
    };

    tracing::info!(
        server_id = %session.server_id,
        trunk_id = %session.trunk_id,
        "server trunk registered"
    );

    let fault = trunk_loop(&mut rx, &tx, &relay, &session).await;

    // Only ever releases a registration this trunk still owns. A superseded
    // trunk reaching here must not evict the trunk that replaced it.
    relay
        .registrations
        .release(&session.server_id, &session.trunk_id);
    // Streams do not survive trunk loss (§5.4): every client holding one is
    // told `trunk_lost` rather than left waiting for a response that can no
    // longer arrive.
    session.trunk.shut_down();
    tracing::info!(
        server_id = %session.server_id,
        trunk_id = %session.trunk_id,
        "server trunk closed"
    );

    trunk::close(&tx, fault).await;
}

struct Session {
    server_id: String,
    trunk_id: String,
    trunk: Arc<ServerTrunk>,
}

async fn handshake(
    rx: &mut Rx,
    tx: &Tx,
    relay: &Relay,
    server_trunk: &Arc<ServerTrunk>,
) -> Result<Session, Fault> {
    let frame = trunk::recv_control(rx).await?;
    if frame.ty() != "REGISTER_SERVER" {
        return Err(ErrorCode::ProtocolError.into());
    }
    let register: RegisterServer = frame.body()?;

    // Field *shape* problems are `protocol_error`; only the binding and the
    // signature produce `auth_failed`. Splitting them this way tells an
    // attacker nothing it does not already know — it knows whether it sent
    // valid base64 — while keeping every genuine authentication outcome under
    // one indistinguishable code.
    validate_server_id(&register.server_id).map_err(|why| {
        tracing::debug!(reason = why, "rejecting REGISTER_SERVER: bad server_id");
        ErrorCode::ProtocolError
    })?;
    let pubkey = PublicKey::from_b64(&register.pubkey).map_err(|why| {
        tracing::debug!(reason = why, "rejecting REGISTER_SERVER: bad pubkey");
        ErrorCode::ProtocolError
    })?;

    // **Binding first, challenge second** (§4.1). There is no reason to spend a
    // nonce on a registration that cannot succeed, and issuing one anyway would
    // let an unbound caller farm nonces from the relay for free.
    let binding = relay
        .bindings
        .check(relay.config.allowlist.as_ref(), &register.server_id, pubkey)
        .map_err(|_| {
            tracing::info!(
                server_id = %register.server_id,
                pubkey = %pubkey.to_b64(),
                "refusing registration: pubkey does not match the binding"
            );
            ErrorCode::AuthFailed
        })?;

    let nonce = relay.mint_nonce();
    // The nonce generator satisfies `validate_nonce` by construction, so this
    // can only fire if the generator (or a test's substitute for it) is wrong.
    // Refusing here rather than issuing an unsafe nonce is the point: a nonce
    // carrying `:` would let the signature cover a different
    // `(server_id, nonce)` split than the relay intends. There is no error code
    // for "the relay is misconfigured" and inventing one would say more than
    // `protocol_error` does.
    validate_nonce(&nonce).map_err(|why| {
        tracing::error!(reason = why, "refusing to issue an invalid nonce");
        ErrorCode::ProtocolError
    })?;
    relay
        .challenges
        .issue(&nonce, relay.config.challenge_ttl, Instant::now());
    tx.send_json(proto::challenge(nonce.clone())).await?;

    let frame = trunk::recv_control(rx).await?;
    if frame.ty() != "CHALLENGE_RESPONSE" {
        return Err(ErrorCode::ProtocolError.into());
    }
    let response: ChallengeResponse = frame.body()?;

    // Validated again on the way *into* the signed message, not only on the way
    // out (§4: an implementation cannot assume the only nonces it ever sees are
    // its own). Today this value came from the relay's own store; the guard is
    // here so that stays true no matter where a later slice sources it.
    if validate_nonce(&nonce).is_err() {
        tracing::error!("refusing to verify against an invalid nonce");
        return Err(ErrorCode::AuthFailed.into());
    }

    // Every one of the next three failures is the same `auth_failed` with the
    // same fixed message: a registration attacker must not learn whether the
    // signature was bad, the nonce expired or was replayed, or the binding was
    // refused. The reasons go to the relay's log, which the peer cannot read.
    if !relay.challenges.consume(&nonce, Instant::now()) {
        tracing::info!(server_id = %register.server_id, "nonce expired or already spent");
        return Err(ErrorCode::AuthFailed.into());
    }
    if !pubkey.verify_relay_auth(&register.server_id, &nonce, &response.sig) {
        tracing::info!(server_id = %register.server_id, "bad relay-auth signature");
        return Err(ErrorCode::AuthFailed.into());
    }
    if binding == Binding::RecordOnSuccess {
        // Recorded only now (§4.1): a pair recorded at REGISTER_SERVER time
        // would let anyone who can open a socket squat any unbound `server_id`
        // without holding a key at all.
        relay
            .bindings
            .record(&register.server_id, pubkey)
            .map_err(|_| {
                tracing::info!(
                    server_id = %register.server_id,
                    "lost the race to record a first-use binding"
                );
                ErrorCode::AuthFailed
            })?;
        tracing::info!(
            server_id = %register.server_id,
            pubkey = %pubkey.to_b64(),
            "recorded a trust-on-first-use binding"
        );
    }

    let trunk_id = new_trunk_id();
    let now = Instant::now();
    if let Some(displaced) = relay.registrations.install(
        &register.server_id,
        Registration {
            trunk_id: trunk_id.clone(),
            connected_at: SystemTime::now(),
            last_seen: now,
            trunk: server_trunk.clone(),
        },
    ) {
        // §4.2 supersession: whoever completed the challenge already holds the
        // private key, so this is a reconnect rather than a hijack. The graceful
        // 30 s drain and `ERROR{trunk_superseded}` belong to stream routing,
        // which this slice does not have.
        tracing::info!(
            server_id = %register.server_id,
            displaced = %displaced.trunk_id,
            replacement = %trunk_id,
            "superseded an existing server trunk"
        );
    }

    tx.send_json(proto::registered(
        trunk_id.clone(),
        relay.config.public_address(&register.server_id),
    ))
    .await?;

    Ok(Session {
        server_id: register.server_id,
        trunk_id,
        trunk: server_trunk.clone(),
    })
}

/// Holds the trunk open after registration, and routes everything the origin
/// sends back toward the client that asked for it.
///
/// Every route here goes through the stream table's recorded owner. **The relay
/// never reads a destination out of the frame it is forwarding** — a
/// `stream_id` names a stream, and the stream names its client. That is what
/// keeps one client's response from reaching another.
async fn trunk_loop(rx: &mut Rx, tx: &Tx, relay: &Relay, session: &Session) -> Fault {
    loop {
        let incoming = match trunk::recv(rx).await {
            Ok(incoming) => incoming,
            Err(fault) => return fault,
        };
        relay
            .registrations
            .touch(&session.server_id, &session.trunk_id, Instant::now());

        let outcome = match incoming {
            Incoming::Control(frame) => match frame.ty() {
                "PING" => tx.send_json(proto::pong()).await,
                // A server SHOULD send this before closing, so the relay frees
                // the `server_id` immediately rather than making clients wait
                // out a timeout for a deliberate restart (§4.2).
                "DEREGISTER" => return Fault::Disconnected,
                "PONG" => Ok(()),
                "STREAM_ACK" => stream_ack(session, frame),
                "HTTP_RESPONSE_HEAD" => forward_response_head(session, frame),
                "CLOSE" => match server_close(session, frame) {
                    Ok(true) => return Fault::Disconnected,
                    Ok(false) => Ok(()),
                    Err(fault) => Err(fault),
                },
                _ => Err(ErrorCode::ProtocolError.into()),
            },
            Incoming::Body {
                kind,
                stream_id,
                raw,
            } => forward_response_body(session, kind, stream_id, raw),
        };

        if let Err(fault) = outcome {
            return fault;
        }
    }
}

fn stream_ack(session: &Session, frame: Frame) -> Result<(), Fault> {
    let stream_id = proto::stream_id_of(&frame.body)?;
    if !session.trunk.ack_stream(stream_id) {
        // The stream timed out and was already torn down, or the origin
        // acknowledged an id the relay never issued. Neither is worth killing
        // a trunk that is carrying other healthy streams over.
        tracing::debug!(stream_id, "STREAM_ACK for a stream that is not open");
    }
    Ok(())
}

fn forward_response_head(session: &Session, frame: Frame) -> Result<(), Fault> {
    let stream_id = proto::stream_id_of(&frame.body)?;
    // Sent on as soon as status and headers are known (§5.2). Nothing waits
    // here for a body, which is what lets the change feed answer immediately
    // and then stay open and silent for hours.
    deliver(
        session,
        stream_id,
        proto::relayed("HTTP_RESPONSE_HEAD", frame.body),
    )
}

fn forward_response_body(
    session: &Session,
    kind: u8,
    stream_id: u32,
    raw: Vec<u8>,
) -> Result<(), Fault> {
    // `0x01` is the request direction; the origin sending one is speaking the
    // client's half of the protocol.
    if kind != proto::BODY_RESPONSE {
        return Err(ErrorCode::ProtocolError.into());
    }
    let Some(client) = session.trunk.client_for(stream_id) else {
        tracing::debug!(stream_id, "dropping a body chunk for a closed stream");
        return Ok(());
    };
    // Forwarded chunk by chunk as the origin produces them, never accumulated
    // (§5.2) — buffering a response whole would break streaming *and* let one
    // large download hold the relay's memory.
    if client.try_send_raw(raw).is_err() {
        drop_backlogged_client(session, stream_id);
    }
    Ok(())
}

fn deliver(session: &Session, stream_id: u32, json: String) -> Result<(), Fault> {
    let Some(client) = session.trunk.client_for(stream_id) else {
        // The client hung up, or the stream timed out. Dropped, not faulted:
        // the trunk is shared, and one dead stream must not disturb the others.
        tracing::debug!(stream_id, "dropping a frame for a closed stream");
        return Ok(());
    };
    if client.try_send_json(json).is_err() {
        drop_backlogged_client(session, stream_id);
    }
    Ok(())
}

/// A client that has stopped draining its socket loses its stream.
///
/// Never `.await` toward a client from this task: it is the *shared* reader for
/// one origin, so waiting on any single client would stall the responses of
/// every other client on that server. Dropping the stream is the bounded
/// alternative — the cost lands on the client that caused it.
fn drop_backlogged_client(session: &Session, stream_id: u32) {
    tracing::info!(stream_id, "client cannot keep up; dropping its stream");
    session.trunk.close_stream(stream_id);
    let _ = session
        .trunk
        .tx()
        .try_send_json(proto::close_stream(stream_id));
}

/// `CLOSE` from the origin. Returns whether the whole trunk is going away.
fn server_close(session: &Session, frame: Frame) -> Result<bool, Fault> {
    let Some(value) = frame.body.get("stream_id") else {
        return Ok(true);
    };
    if value.is_null() {
        return Err(ErrorCode::ProtocolError.into());
    }
    let stream_id = proto::stream_id_of(&frame.body)?;
    let closing = session.trunk.client_for(stream_id);
    session.trunk.close_stream(stream_id);
    if let Some(client) = closing {
        let _ = client.try_send_json(proto::close_stream(stream_id));
    }
    Ok(false)
}
