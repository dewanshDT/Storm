//! SRP v1 wire messages, from the origin server's side.
//!
//! The mirror of `apps/relay/src/proto.rs`. Both halves are written from
//! `docs/srp-v1.md` and never share code, so everything here that the spec
//! pins is spelled out rather than inferred: the `v: 1` envelope on every
//! outbound control frame, and the envelope-first parse on every inbound one.

use serde::{Deserialize, Serialize};

/// Hard-pinned (§3). No in-band negotiation — an incompatible change is a new
/// relay deployment.
pub const PROTOCOL_VERSION: u64 = 1;

/// Heartbeat cadence when `REGISTERED` does not name one (§5, §4.2).
pub const DEFAULT_HEARTBEAT_SECS: u64 = 15;

/// Binary frame kinds (§3). `type(1) | stream_id(4, big-endian) | payload`.
pub const FRAME_REQUEST_BODY: u8 = 0x01;
pub const FRAME_RESPONSE_BODY: u8 = 0x02;

/// The fixed size of a binary frame's header: one kind byte, four id bytes.
pub const BINARY_HEADER_LEN: usize = 5;

// ---------------------------------------------------------------------------
// Headers
// ---------------------------------------------------------------------------

/// One request's or response's headers, as they cross the wire.
///
/// **The spec does not pin this shape.** §5.2 and §6 both say `headers` and
/// stop there, so an origin and a relay written independently can disagree
/// about whether it is a JSON object or a list of pairs — and disagree
/// silently, because a wrong-shaped `headers` deserializes as "no headers"
/// rather than as an error. This type therefore *accepts both* and emits the
/// object form, so interop does not rest on a coin flip that the document
/// never called. See the report accompanying this change.
///
/// Order is preserved on the pair form and sorted on the object form, which is
/// fine: HTTP gives no meaning to header order between different field names.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WireHeaders(pub Vec<(String, String)>);

impl WireHeaders {
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.0.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum HeadersRepr {
    /// `{"accept": "application/json"}` — the object form.
    Map(std::collections::BTreeMap<String, String>),
    /// `[["accept", "application/json"]]` — the pair form, which is the only
    /// one that can carry a repeated field name.
    Pairs(Vec<(String, String)>),
}

impl<'de> Deserialize<'de> for WireHeaders {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(match HeadersRepr::deserialize(d)? {
            HeadersRepr::Map(map) => WireHeaders(map.into_iter().collect()),
            HeadersRepr::Pairs(pairs) => WireHeaders(pairs),
        })
    }
}

impl Serialize for WireHeaders {
    /// Emits the object form, folding a repeated field name into one
    /// comma-joined value the way RFC 9110 §5.3 defines a field list.
    ///
    /// `Set-Cookie` is the one field that rule does not hold for — it must
    /// never be joined. Storm authenticates with bearer tokens and sets no
    /// cookies, so nothing here produces one; if that ever changes this must
    /// move to the pair form rather than grow a special case, because a joined
    /// `Set-Cookie` is two broken cookies rather than a loud failure.
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut folded: Vec<(String, String)> = Vec::with_capacity(self.0.len());
        for (name, value) in &self.0 {
            match folded.iter_mut().find(|(n, _)| n == name) {
                Some((_, existing)) => {
                    existing.push_str(", ");
                    existing.push_str(value);
                }
                None => folded.push((name.clone(), value.clone())),
            }
        }
        let mut map = s.serialize_map(Some(folded.len()))?;
        for (name, value) in &folded {
            map.serialize_entry(name, value)?;
        }
        map.end()
    }
}

// ---------------------------------------------------------------------------
// Inbound: relay → server
// ---------------------------------------------------------------------------

/// A control frame, parsed **envelope first** (§3).
///
/// Same shape as the relay's `Frame` and for the same reason: `v` has to be
/// checked before the body is interpreted, and leaving the body an unparsed
/// map is what makes that structural rather than a matter of statement order.
#[derive(Debug, Deserialize)]
struct Frame {
    #[serde(default)]
    v: Option<u64>,
    #[serde(rename = "type", default)]
    ty: Option<String>,
    #[serde(flatten)]
    body: serde_json::Map<String, serde_json::Value>,
}

/// What the relay can say to this server.
///
/// `Unknown` rather than a parse failure: §3 pins one version, so an
/// unrecognised `type` at `v: 1` is a relay that has grown a message this
/// build does not know. Ignoring it keeps the trunk up, where treating it as
/// fatal would take a working tunnel down over a message that was never
/// addressed to this slice.
#[derive(Debug)]
pub enum Inbound {
    Challenge { nonce: String },
    Registered(Registered),
    StreamOpen { stream_id: u32 },
    RequestHead(RequestHead),
    Close { stream_id: Option<u32> },
    Ping,
    Pong,
    Error(RelayError),
    Unknown(String),
}

#[derive(Debug, Clone, Deserialize)]
pub struct Registered {
    pub trunk_id: String,
    pub public_address: String,
    /// Optional here though the relay always sends it: a missing cadence is a
    /// reason to fall back to §4.2's 15 s, not a reason to refuse a
    /// registration that otherwise succeeded.
    #[serde(default)]
    pub heartbeat_interval_secs: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RequestHead {
    pub stream_id: u32,
    pub method: String,
    pub path: String,
    #[serde(default)]
    pub headers: WireHeaders,
    /// The client trunk's real source address, attached by the relay on the
    /// server-ward hop (§5.2).
    ///
    /// **A field, deliberately not a header.** It is the one piece of caller
    /// identity a client cannot forge, precisely because it never travels in
    /// header space — see `dispatch.rs`, which turns it into the request's
    /// `ConnectInfo` and strips every forwarding header in the same pass.
    #[serde(default)]
    pub relay_peer_ip: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RelayError {
    pub code: String,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub stream_id: Option<u32>,
}

/// Deserializes a frame body, collapsing any failure to [`ProtocolError`].
///
/// A free function rather than a closure inside `parse_text`: several arms name
/// their body type with a turbofish, which a closure cannot accept.
fn typed<T: serde::de::DeserializeOwned>(v: serde_json::Value) -> Result<T, ProtocolError> {
    serde_json::from_value(v).map_err(|_| ProtocolError)
}

/// Parses one text frame.
///
/// `Err` is a protocol error: not an object, wrong or missing `v`, missing
/// `type`, or a known `type` whose body does not fit.
pub fn parse_text(text: &str) -> Result<Inbound, ProtocolError> {
    let frame: Frame = serde_json::from_str(text).map_err(|_| ProtocolError)?;
    if frame.v != Some(PROTOCOL_VERSION) {
        return Err(ProtocolError);
    }
    let ty = frame.ty.clone().ok_or(ProtocolError)?;
    let body = serde_json::Value::Object(frame.body);
    Ok(match ty.as_str() {
        "CHALLENGE" => {
            #[derive(Deserialize)]
            struct B {
                nonce: String,
            }
            Inbound::Challenge {
                nonce: typed::<B>(body)?.nonce,
            }
        }
        "REGISTERED" => Inbound::Registered(typed(body)?),
        "STREAM_OPEN" => {
            #[derive(Deserialize)]
            struct B {
                stream_id: u32,
            }
            Inbound::StreamOpen {
                stream_id: typed::<B>(body)?.stream_id,
            }
        }
        "HTTP_REQUEST_HEAD" => Inbound::RequestHead(typed(body)?),
        "CLOSE" => {
            #[derive(Deserialize)]
            struct B {
                #[serde(default)]
                stream_id: Option<u32>,
            }
            Inbound::Close {
                stream_id: typed::<B>(body)?.stream_id,
            }
        }
        "PING" => Inbound::Ping,
        "PONG" => Inbound::Pong,
        "ERROR" => Inbound::Error(typed(body)?),
        _ => Inbound::Unknown(ty),
    })
}

/// A frame this server refuses to interpret. Carries no detail, for the same
/// reason the relay's does (§6): a scanner learns nothing about which part of
/// its framing was wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProtocolError;

// ---------------------------------------------------------------------------
// Outbound: server → relay
// ---------------------------------------------------------------------------

/// Stamps `v` and `type` onto every outbound control message.
///
/// One place mints the envelope, so a new message type cannot ship without it
/// — the relay rejects any frame missing either field.
#[derive(Serialize)]
struct Control<T> {
    v: u64,
    #[serde(rename = "type")]
    ty: &'static str,
    #[serde(flatten)]
    body: T,
}

fn control<T: Serialize>(ty: &'static str, body: T) -> String {
    serde_json::to_string(&Control {
        v: PROTOCOL_VERSION,
        ty,
        body,
    })
    // Serializing owned scalars cannot fail; the fallback keeps a panic out of
    // a connection handler. An empty object fails the relay's envelope check,
    // which drops the trunk — the honest outcome for a frame we could not
    // build.
    .unwrap_or_else(|_| String::from("{}"))
}

#[derive(Serialize)]
struct RegisterServer<'a> {
    server_id: &'a str,
    pubkey: &'a str,
}

pub fn register_server(server_id: &str, pubkey: &str) -> String {
    control("REGISTER_SERVER", RegisterServer { server_id, pubkey })
}

#[derive(Serialize)]
struct ChallengeResponse<'a> {
    sig: &'a str,
}

pub fn challenge_response(sig: &str) -> String {
    control("CHALLENGE_RESPONSE", ChallengeResponse { sig })
}

#[derive(Serialize)]
struct StreamId {
    stream_id: u32,
}

/// §5.1. Says only that this trunk accepted the id — not that the request has
/// been dispatched, and not that a response is coming.
pub fn stream_ack(stream_id: u32) -> String {
    control("STREAM_ACK", StreamId { stream_id })
}

#[derive(Serialize)]
struct ResponseHead {
    stream_id: u32,
    status: u16,
    headers: WireHeaders,
}

pub fn response_head(stream_id: u32, status: u16, headers: WireHeaders) -> String {
    control(
        "HTTP_RESPONSE_HEAD",
        ResponseHead {
            stream_id,
            status,
            headers,
        },
    )
}

/// `CLOSE { stream_id }` ends one stream; `CLOSE {}` ends the whole trunk
/// (§5.4).
pub fn close_stream(stream_id: u32) -> String {
    control("CLOSE", StreamId { stream_id })
}

pub fn close_trunk() -> String {
    control("CLOSE", serde_json::Map::new())
}

pub fn deregister() -> String {
    control("DEREGISTER", serde_json::Map::new())
}

pub fn ping() -> String {
    control("PING", serde_json::Map::new())
}

pub fn pong() -> String {
    control("PONG", serde_json::Map::new())
}

#[derive(Serialize)]
struct ErrorBody<'a> {
    code: &'a str,
    message: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream_id: Option<u32>,
}

/// §5.1: a `stream_id` already open is refused rather than acknowledged twice.
pub fn error_stream_closed(stream_id: u32) -> String {
    control(
        "ERROR",
        ErrorBody {
            code: "stream_closed",
            message: "stream closed",
            stream_id: Some(stream_id),
        },
    )
}

pub fn error_protocol() -> String {
    control(
        "ERROR",
        ErrorBody {
            code: "protocol_error",
            message: "protocol error",
            stream_id: None,
        },
    )
}

// ---------------------------------------------------------------------------
// Binary frames
// ---------------------------------------------------------------------------

pub fn encode_body_frame(kind: u8, stream_id: u32, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(BINARY_HEADER_LEN + payload.len());
    out.push(kind);
    out.extend_from_slice(&stream_id.to_be_bytes());
    out.extend_from_slice(payload);
    out
}

/// `None` for anything shorter than the fixed header — a truncated frame names
/// no stream, so there is nothing to attribute it to.
pub fn decode_body_frame(frame: &[u8]) -> Option<(u8, u32, &[u8])> {
    if frame.len() < BINARY_HEADER_LEN {
        return None;
    }
    let kind = frame[0];
    let stream_id = u32::from_be_bytes([frame[1], frame[2], frame[3], frame[4]]);
    Some((kind, stream_id, &frame[BINARY_HEADER_LEN..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_outbound_control_frame_carries_the_version_envelope() {
        // The relay's `Frame::parse` refuses anything without both fields, so
        // a message that forgets the envelope does not fail its own test — it
        // drops the trunk at runtime.
        for json in [
            register_server("srv_A", "cHVia2V5"),
            challenge_response("c2ln"),
            stream_ack(7),
            response_head(7, 200, WireHeaders::default()),
            close_stream(7),
            close_trunk(),
            deregister(),
            ping(),
            pong(),
            error_stream_closed(7),
            error_protocol(),
        ] {
            let value: serde_json::Value = serde_json::from_str(&json).unwrap();
            assert_eq!(value["v"], 1, "{json}");
            assert!(value["type"].is_string(), "{json}");
        }
    }

    #[test]
    fn the_registration_frames_match_what_the_relay_parses() {
        let value: serde_json::Value =
            serde_json::from_str(&register_server("srv_A", "cHVia2V5")).unwrap();
        assert_eq!(value["type"], "REGISTER_SERVER");
        assert_eq!(value["server_id"], "srv_A");
        assert_eq!(value["pubkey"], "cHVia2V5");

        let value: serde_json::Value = serde_json::from_str(&challenge_response("c2ln")).unwrap();
        assert_eq!(value["type"], "CHALLENGE_RESPONSE");
        assert_eq!(value["sig"], "c2ln");
    }

    #[test]
    fn a_wrong_or_missing_version_is_a_protocol_error() {
        assert!(parse_text(r#"{"type":"PING"}"#).is_err());
        assert!(parse_text(r#"{"v":2,"type":"PING"}"#).is_err());
        assert!(parse_text(r#"{"v":"1","type":"PING"}"#).is_err());
        assert!(parse_text("not json").is_err());
        assert!(parse_text(r#"{"v":1}"#).is_err());
    }

    #[test]
    fn the_version_is_checked_before_the_body() {
        // A `v: 2` frame whose body is a perfectly good v1 CHALLENGE still
        // fails on the envelope.
        assert!(
            parse_text(r#"{"v":2,"type":"CHALLENGE","nonce":"0123456789abcdef0123"}"#).is_err()
        );
    }

    #[test]
    fn registration_messages_round_trip() {
        let Inbound::Challenge { nonce } =
            parse_text(r#"{"v":1,"type":"CHALLENGE","nonce":"abc"}"#).unwrap()
        else {
            panic!("expected CHALLENGE")
        };
        assert_eq!(nonce, "abc");

        let Inbound::Registered(r) = parse_text(
            r#"{"v":1,"type":"REGISTERED","trunk_id":"trk_1",
                "public_address":"wss://r/connect/srv_A","heartbeat_interval_secs":15}"#,
        )
        .unwrap() else {
            panic!("expected REGISTERED")
        };
        assert_eq!(r.trunk_id, "trk_1");
        assert_eq!(r.heartbeat_interval_secs, Some(15));
    }

    #[test]
    fn a_registered_without_a_cadence_still_parses() {
        // The relay always sends one. A missing cadence must not fail a
        // registration that otherwise succeeded — it falls back to §4.2's 15s.
        let Inbound::Registered(r) = parse_text(
            r#"{"v":1,"type":"REGISTERED","trunk_id":"t","public_address":"wss://r/c/s"}"#,
        )
        .unwrap() else {
            panic!("expected REGISTERED")
        };
        assert_eq!(r.heartbeat_interval_secs, None);
    }

    #[test]
    fn a_close_carries_a_stream_or_the_whole_trunk() {
        let Inbound::Close { stream_id } =
            parse_text(r#"{"v":1,"type":"CLOSE","stream_id":9}"#).unwrap()
        else {
            panic!("expected CLOSE")
        };
        assert_eq!(stream_id, Some(9));

        let Inbound::Close { stream_id } = parse_text(r#"{"v":1,"type":"CLOSE"}"#).unwrap() else {
            panic!("expected CLOSE")
        };
        assert_eq!(stream_id, None, "no stream_id means the whole trunk");
    }

    #[test]
    fn an_unknown_type_is_ignored_rather_than_fatal() {
        // Taking the trunk down over a message addressed to a later slice
        // would make every relay upgrade an outage.
        let Inbound::Unknown(ty) = parse_text(r#"{"v":1,"type":"WHAT_IS_THIS"}"#).unwrap() else {
            panic!("expected Unknown")
        };
        assert_eq!(ty, "WHAT_IS_THIS");
    }

    #[test]
    fn headers_parse_from_either_shape() {
        // The spec pins neither, so both have to work — see `WireHeaders`.
        let map = parse_head(r#"{"accept":"application/json","x-a":"1"}"#);
        assert_eq!(
            map.headers.0,
            vec![
                ("accept".to_string(), "application/json".to_string()),
                ("x-a".to_string(), "1".to_string())
            ]
        );

        let pairs = parse_head(r#"[["accept","application/json"],["x-a","1"]]"#);
        assert_eq!(pairs.headers.0, map.headers.0);
    }

    #[test]
    fn a_head_with_no_headers_field_parses_as_empty() {
        let head = parse_text(
            r#"{"v":1,"type":"HTTP_REQUEST_HEAD","stream_id":1,"method":"GET","path":"/v1/health"}"#,
        )
        .unwrap();
        let Inbound::RequestHead(head) = head else {
            panic!("expected HTTP_REQUEST_HEAD")
        };
        assert!(head.headers.0.is_empty());
        assert_eq!(head.relay_peer_ip, None);
    }

    fn parse_head(headers_json: &str) -> RequestHead {
        let frame = format!(
            r#"{{"v":1,"type":"HTTP_REQUEST_HEAD","stream_id":1,"method":"GET",
                 "path":"/v1/health","headers":{headers_json}}}"#
        );
        match parse_text(&frame).unwrap() {
            Inbound::RequestHead(head) => head,
            _ => panic!("expected HTTP_REQUEST_HEAD"),
        }
    }

    #[test]
    fn response_headers_serialize_as_an_object() {
        let json = response_head(
            3,
            200,
            WireHeaders(vec![("content-type".into(), "application/json".into())]),
        );
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["status"], 200);
        assert_eq!(value["headers"]["content-type"], "application/json");
    }

    #[test]
    fn a_repeated_field_name_is_joined_rather_than_dropped() {
        // The object form cannot carry two entries under one name. Dropping
        // one would lose a `Vary` or an `Accept-Encoding` silently; joining is
        // what RFC 9110 §5.3 says a field list means.
        let json = response_head(
            1,
            200,
            WireHeaders(vec![
                ("vary".into(), "origin".into()),
                ("vary".into(), "accept".into()),
            ]),
        );
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["headers"]["vary"], "origin, accept");
    }

    #[test]
    fn binary_frames_round_trip() {
        let frame = encode_body_frame(FRAME_RESPONSE_BODY, 0x01020304, b"hello");
        assert_eq!(frame[0], FRAME_RESPONSE_BODY);
        assert_eq!(&frame[1..5], &[1, 2, 3, 4], "stream_id is big-endian");
        let (kind, stream_id, payload) = decode_body_frame(&frame).unwrap();
        assert_eq!(kind, FRAME_RESPONSE_BODY);
        assert_eq!(stream_id, 0x01020304);
        assert_eq!(payload, b"hello");
    }

    #[test]
    fn an_empty_body_frame_is_valid_and_a_truncated_one_is_not() {
        let empty = encode_body_frame(FRAME_REQUEST_BODY, 5, b"");
        assert_eq!(decode_body_frame(&empty).unwrap().2, b"");
        assert!(decode_body_frame(&[FRAME_REQUEST_BODY, 0, 0]).is_none());
        assert!(decode_body_frame(&[]).is_none());
    }
}
