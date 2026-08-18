import 'package:flutter_test/flutter_test.dart';

import 'package:storm/api/auth_models.dart';

/// The pairing URI is typed or pasted by a human, from a terminal, onto a
/// phone. It is the single most mangled input in the product, and everything
/// after it depends on the `addr` field being right — the client dials it as
/// the base URL for every later call.
void main() {
  const good =
      'storm://pair?v=1&sid=srv_ABC&pk=KEY&n=NONCE'
      '&exp=2026-08-18T20:00:00Z&addr=192.168.91.51:8585';

  test('a well-formed URI parses', () {
    final p = PairingUri.parse(good)!;
    expect(p.serverId, 'srv_ABC');
    expect(p.address, '192.168.91.51:8585');
    expect(p.nonce, 'NONCE');
  });

  test('whitespace inside the URI is repaired, not rejected', () {
    // A real paste arrived as `&a ddr=` — a phone keyboard broke the long
    // token. The key never matched, `addr` came back empty, and the failure
    // surfaced three screens later as "couldn't reach the server".
    final mangled = good.replaceFirst('&addr=', '&a ddr=');
    final p = PairingUri.parse(mangled);
    expect(p, isNotNull, reason: 'a space cannot be part of a URI, so strip it');
    expect(p!.address, '192.168.91.51:8585');
  });

  test('a line break from a wrapped terminal line is repaired', () {
    final wrapped = good.replaceFirst('&exp=', '&exp\n=');
    expect(PairingUri.parse(wrapped)?.address, '192.168.91.51:8585');
  });

  test('a URI missing any field is refused rather than half-parsed', () {
    // Defaulting a missing key to '' is what let an empty address reach the
    // network layer and get reported as an unreachable server.
    for (final key in ['v', 'sid', 'pk', 'n', 'exp', 'addr']) {
      final without = good.replaceFirst(RegExp('[?&]$key=[^&]*'), '?x=1');
      expect(
        PairingUri.parse(without),
        isNull,
        reason: 'missing $key must not produce a PairingUri',
      );
    }
  });

  test('an address with no host is refused', () {
    // `addr=:8585` parses as a URI but has nowhere to dial.
    final hostless = good.replaceFirst('addr=192.168.91.51:8585', 'addr=');
    expect(PairingUri.parse(hostless), isNull);
  });

  test('a non-storm URI is refused', () {
    expect(PairingUri.parse('https://example.com/pair?v=1'), isNull);
    expect(PairingUri.parse('storm://other?v=1'), isNull);
    expect(PairingUri.parse('not a uri at all'), isNull);
  });
}
