import 'package:flutter_test/flutter_test.dart';

import 'package:storm/api/auth_models.dart';

/// `PairingInvite.toUri` and `PairingUri.parse` are the two halves of one wire
/// format, and the server's `encode_qr` is the third. If the renderer and the
/// reader drift, Storm produces a QR its own client cannot read — and the only
/// place that would show up is a phone in someone's hand.
void main() {
  const invite = PairingInvite(
    serverId: 'srv_ABC',
    publicKey: 'PUBKEY',
    nonce: 'NONCE',
    expires: '2026-08-18T20:00:00Z',
    address: '192.168.91.51:8585',
  );

  test('an invite renders the URI the parser accepts', () {
    final parsed = PairingUri.parse(invite.toUri());
    expect(parsed, isNotNull, reason: 'we must be able to read what we emit');
    expect(parsed!.serverId, invite.serverId);
    expect(parsed.publicKey, invite.publicKey);
    expect(parsed.nonce, invite.nonce);
    expect(parsed.expires, invite.expires);
    expect(parsed.address, invite.address);
  });

  test('the URI carries the version the protocol fixes', () {
    expect(invite.toUri(), startsWith('storm://pair?v=1&'));
  });

  test('it parses from the server field names', () {
    // The keys are the server's `PairingQrPayload`, not the Dart names.
    final fromServer = PairingInvite.fromJson(const {
      'sid': 'srv_ABC',
      'pk': 'PUBKEY',
      'n': 'NONCE',
      'exp': '2026-08-18T20:00:00Z',
      'addr': '192.168.91.51:8585',
    });
    expect(fromServer.toUri(), invite.toUri());
  });
}
