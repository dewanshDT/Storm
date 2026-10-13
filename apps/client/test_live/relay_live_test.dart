import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;

import 'package:storm/api/relay/srp_http_client.dart';
import 'package:storm/api/relay/srp_trunk.dart';
import 'package:storm/api/storm_api.dart';

/// The Dart SRP client against a real `storm-relay` and a real storm-server.
///
/// `test/srp_tunnel_test.dart` pins the framing against a fake relay; this is
/// the one place the three implementations meet. It exists because a relayed
/// write hung until timeout (P2, PLAN decision 92) while every unit test was
/// green: the fake answered after a fixed delay rather than at end of body.
///
/// `make test-live` starts the relay, points the server at it and passes
/// `STORM_RELAY_ORIGIN` (the server's direct URL) and `STORM_SESSION` as
/// `--dart-define`s.
void main() {
  const origin = String.fromEnvironment('STORM_RELAY_ORIGIN');
  const session = String.fromEnvironment('STORM_SESSION');
  // A hang is the failure under test; it must fail fast, not at the default.
  const deadline = Duration(seconds: 10);

  late String vaultId;
  late SrpTrunk trunk;
  late StormApi relayed;
  late StormApi direct;

  setUpAll(() async {
    if (origin.isEmpty || session.isEmpty) {
      fail(
        'Run through `make test-live`: it starts storm-relay and passes '
        'STORM_RELAY_ORIGIN and STORM_SESSION.',
      );
    }
    final token = session.replaceFirst('Bearer ', '');
    direct = StormApi(baseUrl: origin, token: token);

    // Registration happens in the background after the relay list is saved,
    // so wait for the server to report the relay rather than racing it.
    Map<String, dynamic>? relay;
    String? serverId;
    final until = DateTime.now().add(deadline);
    while (relay == null && DateTime.now().isBefore(until)) {
      final info =
          jsonDecode((await http.get(Uri.parse('$origin/v1/server'))).body)
              as Map<String, dynamic>;
      serverId = info['server_id'] as String;
      final relays = (info['relays'] as List? ?? const [])
          .cast<Map<String, dynamic>>();
      if (relays.isNotEmpty) {
        relay = relays.first;
      } else {
        await Future<void>.delayed(const Duration(milliseconds: 200));
      }
    }
    if (relay == null) fail('The server never registered with the relay.');

    trunk = SrpTrunk(
      url: Uri.parse(relay['public_address'] as String),
      serverId: serverId!,
    );
    relayed = StormApi(
      baseUrl: origin,
      token: token,
      client: SrpHttpClient(trunk),
    );
    vaultId = (await direct.vaults()).first.id;
  });

  tearDownAll(() {
    trunk.dispose();
    direct.dispose();
  });

  String scratch(String name) =>
      'RelayTest/${DateTime.now().microsecondsSinceEpoch}-$name.md';

  test('a relayed GET answers what the LAN answers', () async {
    final viaRelay = await relayed.vaults().timeout(deadline);
    final viaLan = await direct.vaults();
    expect(viaRelay.map((v) => v.id), viaLan.map((v) => v.id));
  });

  test('a relayed POST with a JSON body completes', () async {
    final path = scratch('create');
    final created = await relayed
        .createNote(vaultId: vaultId, path: path, content: '# Relayed\n')
        .timeout(deadline);
    expect(created.meta.path, path);

    final fetched = await direct.note(vaultId, created.meta.id);
    expect(fetched.content, contains('# Relayed'));
  });

  test('a relayed save (PUT with base_version) completes', () async {
    final created = await direct.createNote(
      vaultId: vaultId,
      path: scratch('save'),
      content: '# Before\n',
    );
    final saved = await relayed
        .saveNote(
          vaultId: vaultId,
          id: created.meta.id,
          baseVersion: created.meta.version,
          content: '# After\n',
        )
        .timeout(deadline);
    expect(saved.content, contains('# After'));
  });

  test('a relayed POST with no body completes', () async {
    final created = await direct.createNote(
      vaultId: vaultId,
      path: scratch('opened'),
      content: '# Opened\n',
    );
    await relayed.markOpened(vaultId, created.meta.id).timeout(deadline);
  });
}
