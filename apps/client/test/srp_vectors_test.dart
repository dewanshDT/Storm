/// `docs/srp-vectors.json`, checked against the client's copy of the signed
/// bytes.
///
/// `challengeMessage` and the base64url rules in `ed25519_verify.dart` are a
/// third re-derivation of what `apps/server/src/auth/identity.rs` and
/// `apps/relay/src/auth.rs` implement in Rust. The three cannot depend on one
/// another, nothing in any build makes them agree, and drift does not fail a
/// compile — it surfaces as a failed challenge, which is also exactly what a
/// server being impersonated looks like.
///
/// This is the language boundary, so it is where drift is likeliest. It has
/// already paid for itself once: `dart:convert`'s `base64Url` decoder accepts
/// the standard `+/` alphabet and accepts padding, so the padded and
/// standard-alphabet vectors both verified here while the Rust side refused
/// them — one key with three spellings on the client and one everywhere else.
/// `decodeBase64UrlNoPad` exists because of these vectors.
///
/// The signatures come from an independent RFC 8032 implementation
/// (`tools/srp-vectors/`), so a positive vector verifying under `package:
/// cryptography` is a genuine cross-check rather than one library agreeing with
/// itself.
library;

import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:storm/api/ed25519_verify.dart';
import 'package:storm/api/server_verifier.dart';

/// The vectors live at the repo root, not in the client — three implementations
/// read the one file, and a copy per app is exactly the drift being guarded
/// against.
Map<String, dynamic> loadVectors() {
  // `flutter test` runs from the package root, so the repo root is two up.
  final file = File('../../docs/srp-vectors.json');
  if (!file.existsSync()) {
    fail(
      'cannot find ${file.absolute.path} — run `flutter test` from apps/client',
    );
  }
  return jsonDecode(file.readAsStringSync()) as Map<String, dynamic>;
}

List<dynamic> section(Map<String, dynamic> doc, String name) {
  final value = doc[name];
  if (value is! List) fail('section `$name` missing from srp-vectors.json');
  return value;
}

void main() {
  final doc = loadVectors();

  test('the file is the version this test understands', () {
    // A v2 file read by a v1 test would pass vacuously on whichever sections it
    // still happens to recognise.
    expect(doc['version'], 1);
  });

  test('the client challenge domain is the one this app builds', () {
    final domains = doc['domains'] as Map<String, dynamic>;
    final challenge = domains['client_challenge'] as String;
    final relay = domains['relay_auth'] as String;
    expect(challengeMessage('s', 'n'), startsWith(challenge));
    // The client must never build or accept the relay-registration domain: it
    // authenticates servers to relays, and the client is not a party to it.
    expect(challengeMessage('s', 'n'), isNot(startsWith(relay)));
  });

  test('every client challenge message vector matches byte for byte', () {
    final cases = section(doc, 'client_challenge_message');
    expect(cases, isNotEmpty, reason: 'no vectors — this would pass vacuously');
    for (final c in cases.cast<Map<String, dynamic>>()) {
      expect(
        challengeMessage(c['server_id'] as String, c['nonce'] as String),
        c['message_utf8'],
        reason: 'client_challenge_message vector `${c['name']}` disagrees',
      );
      // The UTF-8 encoding too: the server signs bytes, not a Dart string.
      expect(
        base64Url
            .encode(
              utf8.encode(
                challengeMessage(
                  c['server_id'] as String,
                  c['nonce'] as String,
                ),
              ),
            )
            .replaceAll('=', ''),
        c['message_b64'],
      );
    }
  });

  test('every signature vector verifies exactly as recorded', () async {
    final cases = section(doc, 'verify_client_challenge');
    expect(
      cases.any((c) => (c as Map)['verifies'] == true),
      isTrue,
      reason: 'no positive vector — a verifier stuck at false would pass',
    );
    for (final c in cases.cast<Map<String, dynamic>>()) {
      final actual = await verifyChallenge(
        publicKeyB64: c['public_key_b64'] as String,
        signatureB64: c['signature_b64'] as String,
        serverId: c['server_id'] as String,
        nonce: c['nonce'] as String,
      );
      expect(
        actual,
        c['verifies'],
        reason:
            'verify_client_challenge vector `${c['name']}`: '
            'expected verifies=${c['verifies']}',
      );
    }
  });

  test('every public key parses exactly as recorded', () {
    final cases = section(
      doc['public_key_parsing'] as Map<String, dynamic>,
      'cases',
    );
    for (final c in cases.cast<Map<String, dynamic>>()) {
      final decoded = decodeBase64UrlNoPad(c['public_key_b64'] as String);
      // The Rust side folds length into parsing; here the decoder returns bytes
      // and the caller checks the length, so accepted means both.
      final accepted = decoded != null && decoded.length == 32;
      expect(
        accepted,
        c['accepted'],
        reason:
            'public_key_parsing vector `${c['name']}`: '
            'expected accepted=${c['accepted']}',
      );
    }
  });

  test('the generated nonce satisfies the rule the server enforces', () {
    // `randomNonce` is the client's half of a constraint checked on the server
    // and on the relay. The vectors carry that rule, so assert against them
    // rather than against a comment: a nonce the server refuses turns into a
    // failed challenge, which is indistinguishable from an impostor.
    final invalid = section(
      doc,
      'validate_nonce',
    ).cast<Map<String, dynamic>>().where((c) => c['valid'] == false);

    for (var i = 0; i < 50; i++) {
      final nonce = randomNonce();
      expect(nonce.length, greaterThanOrEqualTo(16));
      expect(nonce.length, lessThanOrEqualTo(128));
      for (final rune in nonce.codeUnits) {
        expect(
          rune,
          greaterThan(0x20),
          reason: 'nonce must be printable ASCII',
        );
        expect(rune, lessThan(0x7F));
        expect(rune, isNot(0x3A), reason: "a ':' would forge a field boundary");
        expect(rune, isNot(0x22), reason: 'the nonce travels inside JSON');
      }
      expect(invalid.map((c) => c['nonce']), isNot(contains(nonce)));
    }
  });
}
