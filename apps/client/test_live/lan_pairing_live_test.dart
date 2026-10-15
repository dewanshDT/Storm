import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';

import 'package:storm/api/lan_discovery.dart';
import 'package:storm/api/lan_pairing.dart';
import 'package:storm/api/storm_api.dart';

/// LAN discovery pairing (decision 118) through the client's own route,
/// against the real, claimed server `make test-live` starts on loopback. The
/// advertisement is built from what the server's TXT carries
/// (`lan_discovery_e2e.py` reads the real one over mDNS); the identity check,
/// the password pairing and the sign-in after it are the real ones.
void main() {
  const base = String.fromEnvironment('STORM_BASE');
  const password = 'correct horse battery staple';

  if (base.isEmpty) {
    test('LAN pairing (skipped: no server)', () {
      markTestSkipped('Run through `make test-live`.');
    });
    return;
  }

  test('a server found on the LAN pairs and signs in by password', () async {
    final client = HttpClient();
    final req = await client.getUrl(Uri.parse('$base/v1/server'));
    final info =
        jsonDecode(await (await req.close()).transform(utf8.decoder).join())
            as Map<String, dynamic>;
    client.close();

    final uri = Uri.parse(base);
    final server = DiscoveredServer.fromAdvert(
      name: 'live',
      addresses: [uri.host],
      port: uri.port,
      txt: {
        'v': '1',
        'sid': info['server_id'] as String,
        'pk': info['public_key'] as String,
        'ver': '0.0.0',
        'claimed': '1',
        'lan': '1',
      },
    )!;

    final route = await openLanRoute(server);
    final paired = await pairOverLan(
      route,
      password: password,
      deviceName: 'live lan device',
      platform: 'linux',
    );
    route.close();

    expect(paired.device.serverId, server.serverId);
    final session = paired.session;
    expect(session, isNotNull, reason: 'the sign-in after pairing failed');

    final api = StormApi(baseUrl: server.baseUrl, token: session!.accessToken);
    expect(await api.vaults(), isNotEmpty);
    api.dispose();
  });
}
