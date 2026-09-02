/// Ed25519 signature verification for the server challenge step.
///
/// The client receives the server's public key (base64url) from the pairing QR
/// or from `GET /v1/server`, and must verify that the server holds the
/// corresponding private key before trusting the pairing response. The
/// challenge step (`POST /v1/server/challenge`) signs a domain-separated
/// message, and this module verifies it.
library;

import 'dart:convert';
import 'dart:typed_data';

import 'package:cryptography/cryptography.dart';

/// The domain-separated challenge message format, matching the server.
///
/// `storm-challenge:v1:<server_id>:<nonce>`
String challengeMessage(String serverId, String nonce) =>
    'storm-challenge:v1:$serverId:$nonce';

/// Verifies an Ed25519 signature over the challenge message.
///
/// [publicKeyB64] is the base64url-no-pad encoded 32-byte public key from the
/// server. [signatureB64] is the base64url-no-pad encoded 64-byte signature.
/// [serverId] and [nonce] reconstruct the signed message.
///
/// Returns `true` if the signature is valid, `false` otherwise. Never throws —
/// a verification failure is a return value, not an exception, because the
/// caller's response to an invalid signature is the same regardless of *why*
/// it is invalid.
Future<bool> verifyChallenge({
  required String publicKeyB64,
  required String signatureB64,
  required String serverId,
  required String nonce,
}) async {
  try {
    final publicKeyBytes = decodeBase64UrlNoPad(publicKeyB64);
    if (publicKeyBytes == null || publicKeyBytes.length != 32) return false;

    final signatureBytes = decodeBase64UrlNoPad(signatureB64);
    if (signatureBytes == null || signatureBytes.length != 64) return false;

    final message = utf8.encode(challengeMessage(serverId, nonce));

    final publicKey = SimplePublicKey(
      publicKeyBytes,
      type: KeyPairType.ed25519,
    );
    final signature = Signature(signatureBytes, publicKey: publicKey);

    final algorithm = Ed25519();
    return await algorithm.verify(message, signature: signature);
  } catch (_) {
    return false;
  }
}

/// Decodes a base64url-no-pad field, strictly. Returns `null` if it is not one.
///
/// The strictness is the point, and it is a wire rule rather than fussiness:
/// **one key has one spelling.** `dart:convert`'s `base64Url` decoder also
/// accepts the standard `+/` alphabet and accepts padding, while the Rust side
/// (`data_encoding`'s `BASE64URL_NOPAD`, used by both `apps/server` and
/// `apps/relay`) refuses both. Left lenient, the same 32 bytes would have three
/// valid spellings on the client and one everywhere else — so a value the relay
/// rejects outright would verify here, and the two implementations would
/// disagree about whether a key had changed.
///
/// `docs/srp-vectors.json` pins this, and `test/srp_vectors_test.dart` is what
/// caught it: the padded and standard-alphabet cases passed before this
/// function existed.
Uint8List? decodeBase64UrlNoPad(String s) {
  if (s.isEmpty) return null;
  for (final c in s.codeUnits) {
    final isUnreserved =
        (c >= 0x41 && c <= 0x5A) || // A-Z
        (c >= 0x61 && c <= 0x7A) || // a-z
        (c >= 0x30 && c <= 0x39) || // 0-9
        c == 0x2D || // -
        c == 0x5F; // _
    if (!isUnreserved) return null;
  }
  // A base64 group is 2, 3 or 4 characters; a remainder of 1 encodes nothing.
  final mod = s.length % 4;
  if (mod == 1) return null;
  try {
    return base64Url.decode(mod == 0 ? s : s + '=' * (4 - mod));
  } catch (_) {
    return null;
  }
}
