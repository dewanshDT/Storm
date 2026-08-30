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

use std::sync::Arc;
use std::time::{Instant, SystemTime};

use axum::extract::ws::{Message, WebSocket};

use crate::Relay;
use crate::auth::{PublicKey, validate_nonce, validate_server_id};
use crate::proto::{self, ChallengeResponse, ErrorCode, Frame, RegisterServer};
use crate::state::{Binding, Registration, new_trunk_id};

/// How a connection stopped being useful.
///
/// `Disconnected` is separate from the two error codes because there is
/// nobody left to tell: sending an `ERROR` down a closed socket is how a
/// handler ends up logging a spurious failure for an ordinary hang-up.
#[derive(Debug, Clone, Copy)]
enum Fault {
    Code(ErrorCode),
    Disconnected,
}

impl From<ErrorCode> for Fault {
    fn from(code: ErrorCode) -> Self {
        Self::Code(code)
    }
}

/// Serves one server trunk from upgrade to close.
pub async fn serve(mut socket: WebSocket, relay: Arc<Relay>) {
    // A half-finished handshake must not hold a socket open indefinitely: the
    // relay authenticates nobody at the door, so anyone who can connect can
    // start one. Bounded separately from the nonce TTL — see `config`.
    let outcome = match tokio::time::timeout(
        relay.config.handshake_timeout,
        handshake(&mut socket, &relay),
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
        Err(fault) => return close(socket, fault).await,
    };

    tracing::info!(
        server_id = %session.server_id,
        trunk_id = %session.trunk_id,
        "server trunk registered"
    );

    let fault = trunk_loop(&mut socket, &relay, &session).await;

    // Only ever releases a registration this trunk still owns. A superseded
    // trunk reaching here must not evict the trunk that replaced it.
    relay
        .registrations
        .release(&session.server_id, &session.trunk_id);
    tracing::info!(
        server_id = %session.server_id,
        trunk_id = %session.trunk_id,
        "server trunk closed"
    );

    close(socket, fault).await;
}

struct Session {
    server_id: String,
    trunk_id: String,
}

async fn handshake(socket: &mut WebSocket, relay: &Relay) -> Result<Session, Fault> {
    let frame = recv_control(socket).await?;
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
    send(socket, proto::challenge(nonce.clone())).await?;

    let frame = recv_control(socket).await?;
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

    send(
        socket,
        proto::registered(
            trunk_id.clone(),
            relay.config.public_address(&register.server_id),
        ),
    )
    .await?;

    Ok(Session {
        server_id: register.server_id,
        trunk_id,
    })
}

/// Holds the trunk open after registration.
///
/// Answers `PING` and honours `DEREGISTER`. It does **not** route anything:
/// client trunks, `OPEN_STREAM` and the HTTP messages are the next slice, so
/// any other control type is a protocol error rather than a silent no-op.
async fn trunk_loop(socket: &mut WebSocket, relay: &Relay, session: &Session) -> Fault {
    loop {
        let frame = match recv_control(socket).await {
            Ok(frame) => frame,
            Err(fault) => return fault,
        };
        relay
            .registrations
            .touch(&session.server_id, &session.trunk_id, Instant::now());

        match frame.ty() {
            "PING" => {
                if let Err(fault) = send(socket, proto::pong()).await {
                    return fault;
                }
            }
            // A server SHOULD send this before closing, so the relay frees the
            // `server_id` immediately rather than making clients wait out a
            // timeout for a deliberate restart (§4.2).
            "DEREGISTER" => return Fault::Disconnected,
            "PONG" => {}
            _ => return Fault::Code(ErrorCode::ProtocolError),
        }
    }
}

/// Reads the next control frame, skipping transport-level ping/pong.
///
/// A binary frame is a body chunk (§3) and has no meaning on a trunk with no
/// open streams, so it is a protocol error here rather than something ignored.
async fn recv_control(socket: &mut WebSocket) -> Result<Frame, Fault> {
    loop {
        let Some(message) = socket.recv().await else {
            return Err(Fault::Disconnected);
        };
        match message {
            Ok(Message::Text(text)) => return Frame::parse(text.as_str()).map_err(Fault::Code),
            Ok(Message::Binary(_)) => return Err(ErrorCode::ProtocolError.into()),
            Ok(Message::Ping(_) | Message::Pong(_)) => continue,
            Ok(Message::Close(_)) | Err(_) => return Err(Fault::Disconnected),
        }
    }
}

async fn send(socket: &mut WebSocket, json: String) -> Result<(), Fault> {
    socket
        .send(Message::Text(json.into()))
        .await
        .map_err(|_| Fault::Disconnected)
}

/// Reports the fault, if there is still anyone to report it to, and closes.
async fn close(mut socket: WebSocket, fault: Fault) {
    if let Fault::Code(code) = fault {
        let _ = socket.send(Message::Text(proto::error(code).into())).await;
    }
    let _ = socket.send(Message::Close(None)).await;
}
