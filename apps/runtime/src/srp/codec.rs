//! SRP v1 wire messages, from the client's side.
//!
//! A copy, not a dependency: `apps/server/src/relay/proto.rs`,
//! `apps/relay/src/proto.rs` and `apps/client/lib/api/relay/srp_codec.dart`
//! are the other three, each written from `docs/srp-v1.md`, and no crate may
//! depend on another (R6). The envelope is checked before the body (§3), and
//! `tests/srp_vectors.rs` holds this copy to `docs/srp-vectors.json`.

use serde::{Deserialize, Serialize};

pub use crate::identity::validate_server_id;

/// Hard-pinned (§3).
pub const PROTOCOL_VERSION: u64 = 1;

/// Binary frame kinds (§3): `kind(1) | stream_id(4, big-endian) | payload`.
pub const FRAME_REQUEST_BODY: u8 = 0x01;
pub const FRAME_RESPONSE_BODY: u8 = 0x02;

pub const BINARY_HEADER_LEN: usize = 5;

/// The largest request-body payload this client puts in one frame (§5.2,
/// decision 100). Small frames let concurrent streams interleave.
pub const MAX_BODY_FRAME_PAYLOAD: usize = 256 * 1024;

/// One request's or response's headers as they cross the wire.
///
/// §6 does not pin the shape, so this accepts an object or a list of pairs
/// and emits the object form, which is the only one the relay forwards from a
/// client (`connect.rs` refuses a head whose `headers` is not an object).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WireHeaders(pub Vec<(String, String)>);

#[derive(Deserialize)]
#[serde(untagged)]
enum HeadersRepr {
    Map(std::collections::BTreeMap<String, String>),
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
    /// A repeated field name is folded into one comma-joined value (RFC 9110
    /// §5.3), because the object form cannot carry it twice.
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

#[derive(Debug, Deserialize)]
struct Frame {
    #[serde(default)]
    v: Option<u64>,
    #[serde(rename = "type", default)]
    ty: Option<String>,
    #[serde(flatten)]
    body: serde_json::Map<String, serde_json::Value>,
}

/// What a relay can say to a client.
///
/// `Unknown` rather than a parse failure: a `v: 1` type this build does not
/// know is a newer relay, not a broken one.
#[derive(Debug)]
pub enum Inbound {
    Ready {
        client_trunk_id: String,
    },
    /// `attempt_id` is echoed as whatever JSON was sent (§6 leaves it untyped).
    StreamReady {
        attempt_id: serde_json::Value,
        stream_id: u32,
    },
    ResponseHead(ResponseHead),
    /// `stream_id: None` is the whole trunk (§5.4).
    Close {
        stream_id: Option<u32>,
        aborted: bool,
    },
    Ping,
    Pong,
    Error(RelayError),
    Unknown(String),
}

#[derive(Debug, Clone, Deserialize)]
pub struct ResponseHead {
    pub stream_id: u32,
    pub status: u16,
    #[serde(default)]
    pub headers: WireHeaders,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RelayError {
    pub code: String,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub stream_id: Option<u32>,
    /// The refused `OPEN_STREAM`'s `attempt_id`, echoed as sent (decision
    /// 106). Absent from a relay older than that.
    #[serde(default)]
    pub attempt_id: Option<serde_json::Value>,
}

/// A frame this client refuses to interpret. No detail, like the relay's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProtocolError;

fn typed<T: serde::de::DeserializeOwned>(v: serde_json::Value) -> Result<T, ProtocolError> {
    serde_json::from_value(v).map_err(|_| ProtocolError)
}

/// Parses one text frame, envelope first.
pub fn parse_text(text: &str) -> Result<Inbound, ProtocolError> {
    let frame: Frame = serde_json::from_str(text).map_err(|_| ProtocolError)?;
    if frame.v != Some(PROTOCOL_VERSION) {
        return Err(ProtocolError);
    }
    let ty = frame.ty.ok_or(ProtocolError)?;
    let body = serde_json::Value::Object(frame.body);
    Ok(match ty.as_str() {
        "READY" => {
            #[derive(Deserialize)]
            struct B {
                client_trunk_id: String,
            }
            Inbound::Ready {
                client_trunk_id: typed::<B>(body)?.client_trunk_id,
            }
        }
        "STREAM_READY" => {
            #[derive(Deserialize)]
            struct B {
                attempt_id: serde_json::Value,
                stream_id: u32,
            }
            let b: B = typed(body)?;
            Inbound::StreamReady {
                attempt_id: b.attempt_id,
                stream_id: b.stream_id,
            }
        }
        "HTTP_RESPONSE_HEAD" => Inbound::ResponseHead(typed(body)?),
        "CLOSE" => {
            #[derive(Deserialize)]
            struct B {
                #[serde(default)]
                stream_id: Option<u32>,
                // §5.4: when present it MUST be a boolean.
                #[serde(default)]
                aborted: Option<bool>,
            }
            let b: B = typed(body)?;
            Inbound::Close {
                stream_id: b.stream_id,
                aborted: b.aborted.unwrap_or(false),
            }
        }
        "PING" => Inbound::Ping,
        "PONG" => Inbound::Pong,
        "ERROR" => Inbound::Error(typed(body)?),
        _ => Inbound::Unknown(ty),
    })
}

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
    .unwrap_or_else(|_| String::from("{}"))
}

pub fn hello(server_id: &str) -> String {
    #[derive(Serialize)]
    struct B<'a> {
        server_id: &'a str,
    }
    control("HELLO", B { server_id })
}

pub fn open_stream(attempt_id: &str) -> String {
    #[derive(Serialize)]
    struct B<'a> {
        attempt_id: &'a str,
    }
    control("OPEN_STREAM", B { attempt_id })
}

/// §5.2. There is no `relay_peer_ip` field: the relay refuses a client head
/// that carries one, and this struct cannot spell it.
pub fn request_head(stream_id: u32, method: &str, path: &str, headers: &WireHeaders) -> String {
    #[derive(Serialize)]
    struct B<'a> {
        stream_id: u32,
        method: &'a str,
        path: &'a str,
        headers: &'a WireHeaders,
    }
    control(
        "HTTP_REQUEST_HEAD",
        B {
            stream_id,
            method,
            path,
            headers,
        },
    )
}

pub fn close_stream(stream_id: u32) -> String {
    #[derive(Serialize)]
    struct B {
        stream_id: u32,
    }
    control("CLOSE", B { stream_id })
}

pub fn close_trunk() -> String {
    control("CLOSE", serde_json::Map::new())
}

pub fn ping() -> String {
    control("PING", serde_json::Map::new())
}

pub fn pong() -> String {
    control("PONG", serde_json::Map::new())
}

pub fn encode_body_frame(kind: u8, stream_id: u32, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(BINARY_HEADER_LEN + payload.len());
    out.push(kind);
    out.extend_from_slice(&stream_id.to_be_bytes());
    out.extend_from_slice(payload);
    out
}

/// `None` for anything shorter than the fixed header.
pub fn decode_body_frame(frame: &[u8]) -> Option<(u8, u32, &[u8])> {
    if frame.len() < BINARY_HEADER_LEN {
        return None;
    }
    let stream_id = u32::from_be_bytes([frame[1], frame[2], frame[3], frame[4]]);
    Some((frame[0], stream_id, &frame[BINARY_HEADER_LEN..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn json(text: &str) -> serde_json::Value {
        serde_json::from_str(text).unwrap()
    }

    #[test]
    fn every_outbound_control_frame_carries_the_version_envelope() {
        for text in [
            hello("srv_A"),
            open_stream("a1"),
            request_head(7, "GET", "/v1/health", &WireHeaders::default()),
            close_stream(7),
            close_trunk(),
            ping(),
            pong(),
        ] {
            let value = json(&text);
            assert_eq!(value["v"], 1, "{text}");
            assert!(value["type"].is_string(), "{text}");
        }
    }

    #[test]
    fn a_request_head_has_object_headers_and_never_a_peer_ip() {
        let head = json(&request_head(
            3,
            "POST",
            "/mcp?x=1",
            &WireHeaders(vec![
                ("authorization".into(), "Bearer stk_x".into()),
                ("accept".into(), "application/json".into()),
                ("accept".into(), "text/event-stream".into()),
            ]),
        ));
        assert_eq!(head["type"], "HTTP_REQUEST_HEAD");
        assert_eq!(head["stream_id"], 3);
        assert_eq!(head["path"], "/mcp?x=1");
        assert!(head["headers"].is_object());
        assert_eq!(
            head["headers"]["accept"],
            "application/json, text/event-stream"
        );
        assert!(head.get("relay_peer_ip").is_none());
    }

    #[test]
    fn a_wrong_or_missing_version_is_a_protocol_error() {
        assert!(parse_text(r#"{"type":"PING"}"#).is_err());
        assert!(parse_text(r#"{"v":2,"type":"PING"}"#).is_err());
        assert!(parse_text(r#"{"v":"1","type":"PING"}"#).is_err());
        assert!(parse_text(r#"{"v":1}"#).is_err());
        assert!(parse_text("[]").is_err());
        assert!(parse_text("not json").is_err());
        assert!(parse_text(r#"{"v":2,"type":"READY","client_trunk_id":"ctk_1"}"#).is_err());
    }

    #[test]
    fn the_relay_messages_parse() {
        assert!(matches!(
            parse_text(r#"{"v":1,"type":"READY","client_trunk_id":"ctk_1"}"#),
            Ok(Inbound::Ready { client_trunk_id }) if client_trunk_id == "ctk_1"
        ));
        assert!(matches!(
            parse_text(r#"{"v":1,"type":"STREAM_READY","attempt_id":"a1","stream_id":9}"#),
            Ok(Inbound::StreamReady { stream_id: 9, .. })
        ));
        let Ok(Inbound::ResponseHead(head)) = parse_text(
            r#"{"v":1,"type":"HTTP_RESPONSE_HEAD","stream_id":9,"status":200,
                "headers":[["vary","a"],["vary","b"]]}"#,
        ) else {
            panic!("expected HTTP_RESPONSE_HEAD")
        };
        assert_eq!(head.status, 200);
        assert_eq!(head.headers.0.len(), 2, "the pair form keeps repeats");
        let Ok(Inbound::Error(e)) = parse_text(
            r#"{"v":1,"type":"ERROR","code":"stream_aborted","message":"m","stream_id":9}"#,
        ) else {
            panic!("expected ERROR")
        };
        assert_eq!((e.code.as_str(), e.stream_id), ("stream_aborted", Some(9)));
        assert!(matches!(
            parse_text(r#"{"v":1,"type":"SOMETHING_NEW"}"#),
            Ok(Inbound::Unknown(_))
        ));
    }

    #[test]
    fn a_close_names_a_stream_or_the_trunk_and_aborted_must_be_a_boolean() {
        assert!(matches!(
            parse_text(r#"{"v":1,"type":"CLOSE","stream_id":4}"#),
            Ok(Inbound::Close {
                stream_id: Some(4),
                aborted: false
            })
        ));
        assert!(matches!(
            parse_text(r#"{"v":1,"type":"CLOSE","stream_id":4,"aborted":true}"#),
            Ok(Inbound::Close {
                stream_id: Some(4),
                aborted: true
            })
        ));
        assert!(matches!(
            parse_text(r#"{"v":1,"type":"CLOSE"}"#),
            Ok(Inbound::Close {
                stream_id: None,
                ..
            })
        ));
        assert!(parse_text(r#"{"v":1,"type":"CLOSE","stream_id":4,"aborted":"yes"}"#).is_err());
    }

    #[test]
    fn binary_frames_round_trip_with_a_big_endian_id() {
        let frame = encode_body_frame(FRAME_REQUEST_BODY, 0x01020304, b"hello");
        assert_eq!(&frame[..5], &[0x01, 1, 2, 3, 4]);
        assert_eq!(
            decode_body_frame(&frame),
            Some((FRAME_REQUEST_BODY, 0x01020304, &b"hello"[..]))
        );
        let end = encode_body_frame(FRAME_REQUEST_BODY, 5, b"");
        assert_eq!(end.len(), BINARY_HEADER_LEN);
        assert_eq!(decode_body_frame(&end).unwrap().2, b"");
        assert!(decode_body_frame(&[FRAME_RESPONSE_BODY, 0, 0, 0]).is_none());
    }
}
