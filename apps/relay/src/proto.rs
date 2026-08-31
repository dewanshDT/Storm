//! SRP v1 control messages: the version envelope, the registration types, the
//! client-trunk and stream types, and the binary body-frame header.
//!
//! §6's catalog is the reference. The two messages the relay forwards rather
//! than originates — `HTTP_REQUEST_HEAD` and `HTTP_RESPONSE_HEAD` — have no
//! typed struct here on purpose: see [`relayed`].

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

#[derive(Debug, Deserialize)]
pub struct Hello {
    pub server_id: String,
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
pub struct Ready {
    pub client_trunk_id: String,
}

/// `attempt_id` is echoed as whatever JSON the client sent.
///
/// §6 types every other field and leaves this one open, and the relay has no
/// business narrowing it: it is client-generated, echoed exactly once so the
/// client can correlate concurrent opens, and **never routed on** (§5.1). It is
/// size-bounded on the way in rather than type-checked, because a bound is the
/// only property the relay actually needs from it.
#[derive(Debug, Serialize)]
pub struct StreamReady {
    pub attempt_id: serde_json::Value,
    pub stream_id: u32,
}

#[derive(Debug, Serialize)]
pub struct StreamOpen {
    pub stream_id: u32,
}

/// `CLOSE` with no `stream_id` ends the whole trunk (§5.4). The field is
/// skipped rather than serialized as `null` so the two cases are distinct on
/// the wire and not merely distinguishable by a reader that treats `null` as
/// absent.
#[derive(Debug, Serialize)]
pub struct Close {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_id: Option<u32>,
}

#[derive(Debug, Serialize)]
pub struct ErrorBody {
    pub code: &'static str,
    pub message: &'static str,
    /// Present when the failure belongs to one stream rather than the trunk.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_id: Option<u32>,
}

/// The §6 codes this relay can actually emit.
///
/// `trunk_superseded` is absent because the 30 s drain it belongs to is not
/// built (§4.2). An enum listing a code the crate never produces would read as
/// a promise it does not keep.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    ProtocolError,
    AuthFailed,
    ServerUnreachable,
    ServerTimeout,
    TrunkLost,
    RateLimited,
    StreamClosed,
}

impl ErrorCode {
    pub fn code(self) -> &'static str {
        match self {
            Self::ProtocolError => "protocol_error",
            Self::AuthFailed => "auth_failed",
            Self::ServerUnreachable => "server_unreachable",
            Self::ServerTimeout => "server_timeout",
            Self::TrunkLost => "trunk_lost",
            Self::RateLimited => "rate_limited",
            Self::StreamClosed => "stream_closed",
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
            Self::ServerUnreachable => "server unreachable",
            Self::ServerTimeout => "server timeout",
            Self::TrunkLost => "trunk lost",
            Self::RateLimited => "rate limited",
            Self::StreamClosed => "stream closed",
        }
    }

    pub fn body(self) -> ErrorBody {
        ErrorBody {
            code: self.code(),
            message: self.message(),
            stream_id: None,
        }
    }

    pub fn body_on_stream(self, stream_id: u32) -> ErrorBody {
        ErrorBody {
            code: self.code(),
            message: self.message(),
            stream_id: Some(stream_id),
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

pub fn ready(client_trunk_id: String) -> String {
    Control::new("READY", Ready { client_trunk_id }).to_json()
}

pub fn stream_ready(attempt_id: serde_json::Value, stream_id: u32) -> String {
    Control::new(
        "STREAM_READY",
        StreamReady {
            attempt_id,
            stream_id,
        },
    )
    .to_json()
}

pub fn stream_open(stream_id: u32) -> String {
    Control::new("STREAM_OPEN", StreamOpen { stream_id }).to_json()
}

pub fn close_stream(stream_id: u32) -> String {
    Control::new(
        "CLOSE",
        Close {
            stream_id: Some(stream_id),
        },
    )
    .to_json()
}

/// Re-emits a message the relay forwards rather than originates.
///
/// `HTTP_REQUEST_HEAD` and `HTTP_RESPONSE_HEAD` deliberately have no typed
/// struct. Deserializing into one and re-serializing would **silently drop any
/// field the relay does not know about**, which for a head message means
/// dropping part of a proxied HTTP exchange — and the relay's whole job is to
/// move that exchange unaltered (§5.2). Passing the body map through keeps
/// every field the peer sent; the relay only ever reads `stream_id` out of it
/// and, on the server-ward request hop, writes `relay_peer_ip` into it.
pub fn relayed(ty: &'static str, body: serde_json::Map<String, serde_json::Value>) -> String {
    Control::new(ty, serde_json::Value::Object(body)).to_json()
}

pub fn error(code: ErrorCode) -> String {
    Control::new("ERROR", code.body()).to_json()
}

pub fn error_on_stream(code: ErrorCode, stream_id: u32) -> String {
    Control::new("ERROR", code.body_on_stream(stream_id)).to_json()
}

pub fn pong() -> String {
    Control::new("PONG", serde_json::Map::new()).to_json()
}

/// `type(1) | stream_id(4, big-endian) | payload` (§3).
pub const BODY_HEADER_LEN: usize = 5;
/// `HTTP_REQUEST_BODY_CHUNK` — client → relay → server.
pub const BODY_REQUEST: u8 = 0x01;
/// `HTTP_RESPONSE_BODY_CHUNK` — server → relay → client. Also carries the
/// change feed, which is an ordinary streamed response and not a third type.
pub const BODY_RESPONSE: u8 = 0x02;

/// Reads a binary frame's header, leaving the payload untouched.
///
/// Deliberately returns no payload slice: the relay forwards the original
/// bytes, so nothing downstream has any reason to look past the header. It
/// MUST NOT inspect a body (§1).
pub fn parse_body_header(bytes: &[u8]) -> Result<(u8, u32), ErrorCode> {
    if bytes.len() < BODY_HEADER_LEN {
        return Err(ErrorCode::ProtocolError);
    }
    let kind = bytes[0];
    // "There are exactly two body types, and there MUST NOT be a third" (§3).
    if kind != BODY_REQUEST && kind != BODY_RESPONSE {
        return Err(ErrorCode::ProtocolError);
    }
    let stream_id = u32::from_be_bytes([bytes[1], bytes[2], bytes[3], bytes[4]]);
    Ok((kind, stream_id))
}

/// Reads a `stream_id` out of a control message's body.
///
/// Rejects anything that is not an in-range `u32`: a `stream_id` is a routing
/// key, and a value the relay cannot represent is a frame it cannot route.
pub fn stream_id_of(body: &serde_json::Map<String, serde_json::Value>) -> Result<u32, ErrorCode> {
    body.get("stream_id")
        .and_then(serde_json::Value::as_u64)
        .and_then(|id| u32::try_from(id).ok())
        .ok_or(ErrorCode::ProtocolError)
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

    #[test]
    fn an_error_carries_a_stream_id_only_when_it_belongs_to_one_stream() {
        let trunk: serde_json::Value = serde_json::from_str(&error(ErrorCode::TrunkLost)).unwrap();
        // Absent, not null: a client distinguishing "this stream failed" from
        // "the trunk failed" must not have to treat null as a third case.
        assert!(trunk.get("stream_id").is_none(), "{trunk}");

        let stream: serde_json::Value =
            serde_json::from_str(&error_on_stream(ErrorCode::ServerTimeout, 7)).unwrap();
        assert_eq!(stream["stream_id"], 7, "{stream}");
        assert_eq!(stream["code"], "server_timeout", "{stream}");
    }

    #[test]
    fn there_are_exactly_two_body_types() {
        assert_eq!(
            parse_body_header(&[BODY_REQUEST, 0, 0, 0, 1]).unwrap(),
            (BODY_REQUEST, 1)
        );
        // Big-endian, and the full u32 range.
        assert_eq!(
            parse_body_header(&[BODY_RESPONSE, 0xFF, 0xFF, 0xFF, 0xFF])
                .unwrap()
                .1,
            u32::MAX
        );
        // §3: "there MUST NOT be a third". A relay that forwarded an unknown
        // type would be carrying a payload neither side has agreed on.
        assert_eq!(
            parse_body_header(&[0x03, 0, 0, 0, 1]).unwrap_err(),
            ErrorCode::ProtocolError
        );
        // Shorter than the header itself: there is no `stream_id` to route on.
        assert_eq!(
            parse_body_header(&[BODY_REQUEST, 0, 0, 0]).unwrap_err(),
            ErrorCode::ProtocolError
        );
        assert_eq!(
            parse_body_header(&[]).unwrap_err(),
            ErrorCode::ProtocolError
        );
        // A header with no payload is legal — an empty chunk, not a bad frame.
        assert!(parse_body_header(&[BODY_RESPONSE, 0, 0, 0, 9]).is_ok());
    }

    #[test]
    fn a_stream_id_outside_u32_is_not_routable() {
        let ok = Frame::parse(r#"{"v":1,"type":"CLOSE","stream_id":7}"#).unwrap();
        assert_eq!(stream_id_of(&ok.body).unwrap(), 7);

        for body in [
            r#"{"v":1,"type":"CLOSE"}"#,
            r#"{"v":1,"type":"CLOSE","stream_id":null}"#,
            r#"{"v":1,"type":"CLOSE","stream_id":"7"}"#,
            r#"{"v":1,"type":"CLOSE","stream_id":-1}"#,
            r#"{"v":1,"type":"CLOSE","stream_id":4294967296}"#,
        ] {
            let frame = Frame::parse(body).unwrap();
            assert_eq!(
                stream_id_of(&frame.body).unwrap_err(),
                ErrorCode::ProtocolError,
                "{body}"
            );
        }
    }

    #[test]
    fn a_relayed_head_keeps_every_field_it_was_given() {
        // The reason `HTTP_REQUEST_HEAD` has no typed struct: a field the relay
        // does not know about is still part of the proxied exchange, and
        // dropping it silently would corrupt the request rather than fail it.
        let frame = Frame::parse(
            r#"{"v":1,"type":"HTTP_REQUEST_HEAD","stream_id":3,"method":"GET",
                "path":"/v1/notes","headers":{"a":"b"},"future_field":[1,2]}"#,
        )
        .unwrap();
        let json = relayed("HTTP_REQUEST_HEAD", frame.body);
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["v"], 1);
        assert_eq!(value["type"], "HTTP_REQUEST_HEAD");
        assert_eq!(value["headers"]["a"], "b");
        assert_eq!(value["future_field"], serde_json::json!([1, 2]));
    }
}
