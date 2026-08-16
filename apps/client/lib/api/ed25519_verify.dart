/// Ed25519 signature verification for the server challenge step.
///
/// The client receives the server's public key (base64url) from the pairing QR
/// or from `GET /v1/server`, and must verify that the server holds the
/// corresponding private key before trusting the pairing response. The
/// challenge step (`POST /v1/server/challenge`) signs a domain-separated
/// message, and this module verifies it.
library;

import 'dart:convert';

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
    // The server sends base64url without padding; dart:convert expects padding.
    final paddedPk = _addPadding(publicKeyB64);
    final publicKeyBytes = base64Url.decode(paddedPk);
    if (publicKeyBytes.length != 32) return false;

    final paddedSig = _addPadding(signatureB64);
    final signatureBytes = base64Url.decode(paddedSig);
    if (signatureBytes.length != 64) return false;

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

/// Adds `=` padding to a base64url string so dart:convert can decode it.
String _addPadding(String s) {
  final mod = s.length % 4;
  if (mod == 0) return s;
  return s + '=' * (4 - mod);
}
