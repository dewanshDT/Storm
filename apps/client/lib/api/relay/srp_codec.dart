/// SRP v1 wire framing and control messages, from the client's side.
///
/// The mirror of `apps/server/src/relay/proto.rs` and `apps/relay/src/proto.rs`.
/// All three are written from `docs/srp-v1.md` and never share code, so every
/// detail the spec pins is spelled out here rather than inferred — most of all
/// the `{ "v": 1 }` envelope on every control frame, checked **before** the
/// body is interpreted (§3).
///
/// Text frames carry JSON control messages; binary frames carry payload bytes,
/// and WebSocket already length-delimits them, so there is no base64 (§3).
library;

import 'dart:convert';
import 'dart:typed_data';

/// Hard-pinned (§3). An incompatible change is a new relay deployment, never
/// in-band negotiation.
const int srpVersion = 1;

/// Binary frame kinds (§3): `kind(1) | stream_id(4, big-endian) | payload`.
const int bodyRequest = 0x01; // HTTP_REQUEST_BODY_CHUNK
const int bodyResponse = 0x02; // HTTP_RESPONSE_BODY_CHUNK

/// The fixed size of a binary frame's header: one kind byte, four id bytes.
const int binaryHeaderLen = 5;

/// The largest `attempt_id` echoed by the relay (§5.1). The relay bounds it at
/// 256 bytes; the client picks values inside that, so this is a guard against
/// drift rather than a value a caller would normally hit.
const int maxAttemptIdLen = 64;

/// A JSON control message, `v` checked before the body is read.
///
/// Anything that is not a `v: 1` object with a string `type` is a protocol
/// error, exactly as the relay and origin treat it — a missing envelope means
/// the frame is not SRP at all.
class SrpControl {
  const SrpControl._(this.type, this.body);

  final String type;
  final Map<String, dynamic> body;

  /// Encodes a `v:1` control message for the wire.
  static String encode(String type, [Map<String, dynamic> body = const {}]) {
    return jsonEncode({'v': srpVersion, 'type': type, ...body});
  }

  /// Decodes one control message, envelope first.
  ///
  /// Throws [SrpProtocolError] if the frame is not a `v: 1` object with a
  /// string `type` — nothing about the body is looked at before that holds, so
  /// a malformed envelope can never be mistaken for a well-formed one.
  static SrpControl decode(String text) {
    final Object? decoded;
    try {
      decoded = jsonDecode(text);
    } on FormatException {
      throw const SrpProtocolError();
    }
    if (decoded is! Map<String, dynamic>) throw const SrpProtocolError();
    if (decoded['v'] != srpVersion) throw const SrpProtocolError();
    final type = decoded['type'];
    if (type is! String) throw const SrpProtocolError();
    return SrpControl._(type, decoded);
  }
}

/// A parsed binary body frame: `kind(1) | stream_id(4) | payload`.
class SrpBodyFrame {
  const SrpBodyFrame(this.kind, this.streamId, this.payload);

  final int kind;
  final int streamId;
  final Uint8List payload;
}

/// Encodes a binary body frame.
Uint8List encodeBodyFrame(int kind, int streamId, List<int> payload) {
  final out = Uint8List(5 + payload.length);
  final h = ByteData.sublistView(out);
  h.setUint8(0, kind);
  h.setUint32(1, streamId, Endian.big);
  out.setRange(5, out.length, payload);
  return out;
}

/// Decodes a binary body frame.
///
/// Returns null for anything shorter than the fixed header — a truncated frame
/// names no stream, so there is nothing to attribute it to.
SrpBodyFrame? decodeBodyFrame(List<int> frame) {
  if (frame.length < binaryHeaderLen) return null;
  final kind = frame[0];
  final streamId = ByteData.sublistView(
    Uint8List.fromList(frame),
    1,
    5,
  ).getUint32(0, Endian.big);
  return SrpBodyFrame(kind, streamId, Uint8List.fromList(frame.sublist(5)));
}

/// A frame this client refuses to interpret. Carries no detail, matching the
/// relay and origin sides: a scanner learns nothing about which part of its
/// framing was wrong (§6).
class SrpProtocolError implements Exception {
  const SrpProtocolError();
}
