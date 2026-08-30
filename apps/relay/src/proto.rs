//! SRP v1 control messages: the version envelope, the registration types, and
//! the two error codes registration can produce.
//!
//! Only the messages §4 needs are here. Client trunks (`HELLO`/`READY`),
//! `OPEN_STREAM`, the HTTP head messages and the binary body frames are the
//! next slice and are deliberately absent rather than stubbed.

use serde::{Deserialize, Serialize};

/// Hard-pinned (§3). There is no in-band negotiation and no multi-version
/// support: an incompatible change is a new relay deployment.
pub const PROTOCOL_VERSION: u64 = 1;

/// What the relay tells a server to send it: §4.2's `PING`/`PONG` cadence.
pub const HEARTBEAT_INTERVAL_SECS: u64 = 15;

/// A control frame, parsed **envelope first**.
///
/// The body is deliberately left as an unparsed map. §3 requires `v` to be
/// checked before the rest of the body is interpreted, and the only way to make
/// that structural rather than a matter of statement order is to have no typed
/// body until the version has been accepted. `#[serde(deny_unknown_fields)]`
/// would defeat the flatten, so the catch-all is the body itself.
#[derive(Debug, Deserialize)]
pub struct Frame {
    #[serde(default)]
    pub v: Option<u64>,
    #[serde(rename = "type", default)]
    pub ty: Option<String>,
    #[serde(flatten)]
    pub body: serde_json::Map<String, serde_json::Value>,
}

impl Frame {
    /// Parses a text frame and accepts its version.
    ///
    /// Returns `Err` for anything that is not a `v: 1` object carrying a
    /// `type`, without saying which — a malformed-frame scanner is told
    /// nothing about which part of its framing was wrong (§6).
    pub fn parse(text: &str) -> Result<Self, ErrorCode> {
        let frame: Frame = serde_json::from_str(text).map_err(|_| ErrorCode::ProtocolError)?;
        if frame.v != Some(PROTOCOL_VERSION) {
            return Err(ErrorCode::ProtocolError);
        }
        if frame.ty.is_none() {
            return Err(ErrorCode::ProtocolError);
        }
        Ok(frame)
    }

    pub fn ty(&self) -> &str {
        self.ty.as_deref().unwrap_or_default()
    }

    /// Interprets the body as `T`. Only reachable once `parse` accepted `v`.
    pub fn body<T: serde::de::DeserializeOwned>(self) -> Result<T, ErrorCode> {
        serde_json::from_value(serde_json::Value::Object(self.body))
            .map_err(|_| ErrorCode::ProtocolError)
    }
}

#[derive(Debug, Deserialize)]
pub struct RegisterServer {
    pub server_id: String,
    pub pubkey: String,
}

#[derive(Debug, Deserialize)]
pub struct ChallengeResponse {
    pub sig: String,
}

#[derive(Debug, Serialize)]
pub struct Challenge {
    pub nonce: String,
}

#[derive(Debug, Serialize)]
pub struct Registered {
    pub trunk_id: String,
    pub public_address: String,
    pub heartbeat_interval_secs: u64,
}

#[derive(Debug, Serialize)]
pub struct ErrorBody {
    pub code: &'static str,
    pub message: &'static str,
}

/// The subset of §6's codes registration can emit.
///
/// The other seven codes belong to stream routing and are not defined here:
/// an enum listing codes this slice can never produce would read as a promise
/// the crate does not keep.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    ProtocolError,
    AuthFailed,
}

impl ErrorCode {
    pub fn code(self) -> &'static str {
        match self {
            Self::ProtocolError => "protocol_error",
            Self::AuthFailed => "auth_failed",
        }
    }

    /// **One fixed message per code, never per cause.**
    ///
    /// This takes no argument on purpose. `auth_failed` must not say whether a
    /// signature was bad, a nonce expired or a binding was refused, and
    /// `protocol_error` must not say which part of the framing was wrong (§6).
    /// A `message(cause)` overload is how that guarantee gets lost one helpful
    /// diagnostic at a time, so there is no way to attach a cause here at all.
    /// Causes belong in the relay's own logs, which the peer cannot read.
    pub fn message(self) -> &'static str {
        match self {
            Self::ProtocolError => "protocol error",
            Self::AuthFailed => "authentication failed",
        }
    }

    pub fn body(self) -> ErrorBody {
        ErrorBody {
            code: self.code(),
            message: self.message(),
        }
    }
}

/// Stamps `v` and `type` onto every outbound control message.
///
/// One place mints the version envelope, so a new message type cannot ship
/// without it.
#[derive(Debug, Serialize)]
pub struct Control<T> {
    v: u64,
    #[serde(rename = "type")]
    ty: &'static str,
    #[serde(flatten)]
    body: T,
}

impl<T: Serialize> Control<T> {
    pub fn new(ty: &'static str, body: T) -> Self {
        Self {
            v: PROTOCOL_VERSION,
            ty,
            body,
        }
    }

    /// Serializing a `#[derive(Serialize)]` struct of owned scalars cannot
    /// fail; the fallback keeps a panic out of a connection handler.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| String::from("{}"))
    }
}

pub fn challenge(nonce: String) -> String {
    Control::new("CHALLENGE", Challenge { nonce }).to_json()
}

pub fn registered(trunk_id: String, public_address: String) -> String {
    Control::new(
        "REGISTERED",
        Registered {
            trunk_id,
            public_address,
            heartbeat_interval_secs: HEARTBEAT_INTERVAL_SECS,
        },
    )
    .to_json()
}

pub fn error(code: ErrorCode) -> String {
    Control::new("ERROR", code.body()).to_json()
}

pub fn pong() -> String {
    Control::new("PONG", serde_json::Map::new()).to_json()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_or_wrong_version_is_a_protocol_error() {
        assert_eq!(
            Frame::parse(r#"{"type":"REGISTER_SERVER"}"#).unwrap_err(),
            ErrorCode::ProtocolError
        );
        assert_eq!(
            Frame::parse(r#"{"v":2,"type":"REGISTER_SERVER"}"#).unwrap_err(),
            ErrorCode::ProtocolError
        );
        assert_eq!(
            Frame::parse(r#"{"v":"1","type":"REGISTER_SERVER"}"#).unwrap_err(),
            ErrorCode::ProtocolError
        );
    }

    #[test]
    fn the_version_is_checked_before_the_body_is_interpreted() {
        // A `v: 2` frame whose body would be a valid REGISTER_SERVER still
        // fails on the envelope. If the body were parsed first, a future
        // version's message could be interpreted as a v1 one.
        let frame = r#"{"v":2,"type":"REGISTER_SERVER","server_id":"srv_A","pubkey":"AAAA"}"#;
        assert_eq!(Frame::parse(frame).unwrap_err(), ErrorCode::ProtocolError);
    }

    #[test]
    fn a_typed_body_only_parses_after_the_envelope_is_accepted() {
        let frame =
            Frame::parse(r#"{"v":1,"type":"REGISTER_SERVER","server_id":"srv_A","pubkey":"k"}"#)
                .unwrap();
        assert_eq!(frame.ty(), "REGISTER_SERVER");
        let body: RegisterServer = frame.body().unwrap();
        assert_eq!(body.server_id, "srv_A");
        assert_eq!(body.pubkey, "k");
    }

    #[test]
    fn a_body_missing_a_required_field_is_a_protocol_error() {
        let frame =
            Frame::parse(r#"{"v":1,"type":"REGISTER_SERVER","server_id":"srv_A"}"#).unwrap();
        assert_eq!(
            frame.body::<RegisterServer>().unwrap_err(),
            ErrorCode::ProtocolError
        );
    }

    #[test]
    fn every_outbound_message_carries_the_version_envelope() {
        for json in [
            challenge("nonce-0123456789".into()),
            registered("trk_1".into(), "wss://relay/connect/srv_A".into()),
            error(ErrorCode::AuthFailed),
            pong(),
        ] {
            let value: serde_json::Value = serde_json::from_str(&json).unwrap();
            assert_eq!(value["v"], 1, "{json}");
            assert!(value["type"].is_string(), "{json}");
        }
    }

    #[test]
    fn an_error_message_is_fixed_by_its_code() {
        assert_eq!(ErrorCode::AuthFailed.message(), "authentication failed");
        assert_eq!(ErrorCode::ProtocolError.message(), "protocol error");
    }
}
