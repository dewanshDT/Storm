import 'dart:convert';

import 'package:cryptography/cryptography.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:storm/api/ed25519_verify.dart';

void main() {
  test('challengeMessage formats correctly', () {
    expect(
      challengeMessage('srv_abc123', 'nonce_xyz'),
      'storm-challenge:v1:srv_abc123:nonce_xyz',
    );
  });

  test('verifyChallenge accepts a valid signature', () async {
    final algorithm = Ed25519();
    final keyPair = await algorithm.newKeyPair();
    final publicKey = await keyPair.extractPublicKey();

    final serverId = 'srv_test000000000000000000001';
    final nonce = 'AAAAAAAAAAAAAAAAAAAAAAAAAAAA';
    final message = challengeMessage(serverId, nonce);
    final messageBytes = utf8.encode(message);

    // Sign with the private key.
    final sig = await algorithm.sign(messageBytes, keyPair: keyPair);

    // Encode public key and signature as base64url without padding.
    final pkB64 = base64Url.encode(publicKey.bytes).replaceAll('=', '');
    final sigB64 = base64Url.encode(sig.bytes).replaceAll('=', '');

    final result = await verifyChallenge(
      publicKeyB64: pkB64,
      signatureB64: sigB64,
      serverId: serverId,
      nonce: nonce,
    );
    expect(result, isTrue);
  });

  test('verifyChallenge rejects a wrong signature', () async {
    final algorithm = Ed25519();
    final keyPair = await algorithm.newKeyPair();
    final otherKeyPair = await algorithm.newKeyPair();
    final publicKey = await keyPair.extractPublicKey();

    final serverId = 'srv_test000000000000000000001';
    final nonce = 'AAAAAAAAAAAAAAAAAAAAAAAAAAAA';
    final message = challengeMessage(serverId, nonce);
    final messageBytes = utf8.encode(message);

    // Sign with a *different* private key.
    final sig = await algorithm.sign(messageBytes, keyPair: otherKeyPair);

    final pkB64 = base64Url.encode(publicKey.bytes).replaceAll('=', '');
    final sigB64 = base64Url.encode(sig.bytes).replaceAll('=', '');

    final result = await verifyChallenge(
      publicKeyB64: pkB64,
      signatureB64: sigB64,
      serverId: serverId,
      nonce: nonce,
    );
    expect(result, isFalse);
  });

  test('verifyChallenge rejects a tampered message', () async {
    final algorithm = Ed25519();
    final keyPair = await algorithm.newKeyPair();
    final publicKey = await keyPair.extractPublicKey();

    final serverId = 'srv_test000000000000000000001';
    final nonce = 'AAAAAAAAAAAAAAAAAAAAAAAAAAAA';
    final message = challengeMessage(serverId, nonce);
    final messageBytes = utf8.encode(message);

    final sig = await algorithm.sign(messageBytes, keyPair: keyPair);

    final pkB64 = base64Url.encode(publicKey.bytes).replaceAll('=', '');
    final sigB64 = base64Url.encode(sig.bytes).replaceAll('=', '');

    // Verify with a different nonce (the message is now different).
    final result = await verifyChallenge(
      publicKeyB64: pkB64,
      signatureB64: sigB64,
      serverId: serverId,
      nonce: 'BBBBBBBBBBBBBBBBBBBBBBBBBBBB',
    );
    expect(result, isFalse);
  });

  test('verifyChallenge rejects truncated public key', () async {
    final result = await verifyChallenge(
      publicKeyB64: base64Url.encode([1, 2, 3]).replaceAll('=', ''),
      signatureB64: base64Url.encode(List.filled(64, 0)).replaceAll('=', ''),
      serverId: 'srv_test',
      nonce: 'nonce',
    );
    expect(result, isFalse);
  });

  test('verifyChallenge rejects truncated signature', () async {
    final algorithm = Ed25519();
    final keyPair = await algorithm.newKeyPair();
    final publicKey = await keyPair.extractPublicKey();
    final pkB64 = base64Url.encode(publicKey.bytes).replaceAll('=', '');

    final result = await verifyChallenge(
      publicKeyB64: pkB64,
      signatureB64: base64Url.encode(List.filled(32, 0)).replaceAll('=', ''),
      serverId: 'srv_test',
      nonce: 'nonce',
    );
    expect(result, isFalse);
  });
}
