import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:cryptography/cryptography.dart';
import 'package:flutter/material.dart';
import 'package:flutter_lucide/flutter_lucide.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:storm/agent/agent_state.dart';
import 'package:storm/agent/integrations_screen.dart';
import 'package:storm/agent/terminal_events.dart';
import 'package:storm/agent/terminal_stream.dart';
import 'package:storm/api/auth_models.dart';
import 'package:storm/api/ed25519_verify.dart';
import 'package:storm/api/storm_connection.dart';
import 'package:storm/state/app_state.dart';
import 'package:storm/ui/add_device_screen.dart';
import 'package:storm/ui/attachment_strip.dart';
import 'package:storm/ui/theme.dart';

import 'agent_fakes.dart';
import 'fake_relay.dart';
import 'shell_harness.dart' show FixedSettings;

/// Off the LAN after sign-in (decision 112): the saved address is refused and
/// every feature that used to build its own client from it must reach the
/// server through the session's relay, after the identity check.
class Seen {
  Seen(this.method, this.path, this.headers);
  final String method;
  final String path;
  final Map<String, String> headers;
  String? get auth => headers['authorization'] ?? headers['Authorization'];
}

const vaultId = 'vlt_1';
const token = 'sat_live';

/// A 1×1 PNG.
final png = base64.decode(
  'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==',
);

void main() {
  late SimpleKeyPair keyPair;
  late String pubkey;

  setUpAll(() async {
    // Real sockets, so the saved address is refused rather than answered 400
    // by the widget binding's stand-in HttpClient.
    HttpOverrides.global = null;
    keyPair = await Ed25519().newKeyPair();
    final pk = await keyPair.extractPublicKey();
    pubkey = base64Url.encode(pk.bytes).replaceAll('=', '');
  });

  List<RelayAdvert> advertised(FakeRelay relay) => [
    RelayAdvert.derive('ws://127.0.0.1:${relay.port}', 'srv_test'),
  ];

  String sse(String event, String data, {String? id}) =>
      'event: $event\n${id == null ? '' : 'id: $id\n'}data: $data\n\n';

  /// The server behind the fake relay. [terminal] carries the terminal
  /// stream's body; it stays open until the test closes it.
  FakeHandler origin(
    List<Seen> seen,
    FakeRelay relay, {
    StreamController<List<int>>? terminal,
    bool impostor = false,
  }) => (method, path, headers, body) async {
    seen.add(Seen(method, path, headers));
    final route = path.split('?').first;
    Object? json;
    switch (route) {
      case '/v1/server':
        json = {
          'server_id': 'srv_test',
          'key_id': 'key_1',
          'algorithm': 'ed25519',
          'public_key': pubkey,
          // The relay it is registered with, as a real server reports it: an
          // empty set would be mirrored into settings and drop the relay.
          'relays': [
            for (final r in advertised(relay))
              {'url': r.url, 'public_address': r.publicAddress},
          ],
        };
      case '/v1/server/challenge':
        final nonce = jsonDecode(utf8.decode(body))['nonce'] as String;
        final sig = await Ed25519().sign(
          utf8.encode(challengeMessage('srv_test', nonce)),
          keyPair: impostor ? await Ed25519().newKeyPair() : keyPair,
        );
        json = {'signature': base64Url.encode(sig.bytes).replaceAll('=', '')};
      case '/v1/vaults':
        json = {
          'vaults': [
            {'id': vaultId, 'name': 'Notes'},
          ],
        };
      case '/v1/agent/hosts':
        json = [agentHost()];
      case '/v1/agent/sessions':
        json = [agentSession('ses_1')];
      case '/v1/agent/sessions/ses_1/terminal/stream':
        if (terminal != null) {
          return FakeResponse.streamed(200, {
            'content-type': 'text/event-stream',
          }, terminal.stream);
        }
      case '/v1/integrations/connections':
        json = [
          {
            'id': 'int_1',
            'slug': 'linear',
            'display_name': 'Linear',
            'auth_kind': 'oauth',
            'status': 'connected',
          },
        ];
      case '/v1/pairings':
        json = {
          'sid': 'srv_test',
          'pk': pubkey,
          'n': 'nonce_relayed',
          'exp': '2099-01-01T00:00:00Z',
          'addr': 'http://192.168.1.20:8484',
          'relays': const <String>[],
        };
      case '/v1/vaults/$vaultId/attachments/attachments/x.png':
        return FakeResponse(200, {'content-type': 'image/png'}, png);
    }
    if (json == null) return FakeResponse(404, const {}, const []);
    return FakeResponse(200, {
      'content-type': 'application/json',
    }, utf8.encode(jsonEncode(json)));
  };

  Settings signedInOffLan(FakeRelay relay) => Settings(
    // Nothing listens on port 1: the LAN address is "not here".
    baseUrl: 'http://127.0.0.1:1',
    accessToken: token,
    deviceId: 'dev_1',
    deviceSecret: 'dvs_1',
    serverId: 'srv_test',
    serverKeyId: 'key_1',
    serverPublicKey: pubkey,
    activeVault: vaultId,
    relays: advertised(relay),
  );

  Future<ProviderContainer> container(FakeRelay relay) async {
    final c = ProviderContainer(
      overrides: [
        settingsProvider.overrideWith(
          () => FixedSettings(signedInOffLan(relay)),
        ),
      ],
    );
    addTearDown(() {
      c.read(connectionProvider)?.reset();
      c.dispose();
    });
    await c.read(settingsProvider.future);
    return c;
  }

  /// Every credentialed request came after the challenge, carried this
  /// session's token, and went through the relay (the only way [seen] fills).
  void provenFirst(List<Seen> seen, String path) {
    final paths = [for (final r in seen) r.path];
    final challenge = paths.indexOf('/v1/server/challenge');
    expect(challenge, isNonNegative, reason: '$paths');
    expect(
      paths.indexWhere((p) => p.startsWith(path)),
      greaterThan(challenge),
      reason: '$paths',
    );
    for (final r in seen.take(challenge + 1)) {
      expect(r.auth, isNull, reason: '${r.path} before proof');
    }
    for (final r in seen.where((r) => r.path.startsWith(path))) {
      expect(r.auth, 'Bearer $token');
    }
  }

  group('with the saved address refused', () {
    late List<Seen> seen;
    late FakeRelay relay;

    setUp(() async {
      seen = [];
      relay = FakeRelay();
      relay.handler = origin(seen, relay);
      await relay.start();
    });
    tearDown(() => relay.close());

    test('Agents lists sessions and hosts through the relay', () async {
      final c = await container(relay);
      final api = c.read(agentApiFactoryProvider)!();
      final (sessions, hosts) = await (api.sessions(), api.hosts()).wait;
      expect(sessions.single.id, 'ses_1');
      expect(hosts.single.id, 'hst_1');
      provenFirst(seen, '/v1/agent/');
      // Still the saved address: it is what an enrollment tells a runtime.
      expect(api.baseUrl, 'http://127.0.0.1:1');
      api.dispose();
    });

    test('Integrations lists its connections through the relay', () async {
      final c = await container(relay);
      final api = c.read(integrationsApiFactoryProvider)!();
      final list = await api.list();
      expect(list.single.slug, 'linear');
      provenFirst(seen, '/v1/integrations/');
    });

    test('the dashboard reads its vaults through the relay before the sync '
        'engine has started', () async {
      final c = await container(relay);
      final vaults = await c.read(apiProvider)!.vaults();
      expect(vaults.single.id, vaultId);
      provenFirst(seen, '/v1/vaults');
    });

    test('a reconnect does not leave Agents on the dead trunk', () async {
      final c = await container(relay);
      final api = c.read(agentApiFactoryProvider)!();
      await api.hosts();
      final connection = c.read(connectionProvider)!;
      expect(connection.isRelayed, isTrue);

      // What the sync engine does when its feed drops: the trunk dies and
      // the next connect re-races.
      connection.reset();
      seen.clear();
      final hosts = await api.hosts();
      expect(hosts.single.id, 'hst_1');
      provenFirst(seen, '/v1/agent/hosts');
      expect(connection.isRelayed, isTrue);
    });

    test('callers arriving together share one race', () async {
      final c = await container(relay);
      final api = c.read(agentApiFactoryProvider)!();
      await Future.wait([
        api.hosts(),
        api.sessions(),
        c.read(apiProvider)!.vaults(),
        c.read(connectionProvider)!.connect(),
      ]);
      expect(seen.where((r) => r.path == '/v1/server/challenge'), hasLength(1));
    });

    test('nothing is sent to a host other than the server', () async {
      final c = await container(relay);
      final client = c.read(sessionClientProvider);
      await expectLater(
        client.get(
          Uri.parse('https://elsewhere.example/x'),
          headers: {'Authorization': 'Bearer $token'},
        ),
        throwsA(anything),
      );
      expect(seen, isEmpty);
    });
  });

  test('the terminal streams through the relay as it arrives, and '
      'closing it closes the relayed stream', () async {
    final seen = <Seen>[];
    final terminal = StreamController<List<int>>();
    final relay = FakeRelay();
    relay.handler = origin(seen, relay, terminal: terminal);
    await relay.start();
    addTearDown(relay.close);
    final c = await container(relay);
    final api = c.read(agentApiFactoryProvider)!();

    final events = <TerminalEvent>[];
    final got = StreamController<void>.broadcast();
    final sub = openTerminalStream(api, 'ses_1', 0).listen((e) {
      events.add(e);
      got.add(null);
    });

    // The body is still open: these must arrive unbuffered.
    terminal.add(utf8.encode(sse('status', jsonEncode(agentSession('ses_1')))));
    await got.stream.first.timeout(const Duration(seconds: 5));
    terminal.add(
      utf8.encode(sse('output', base64.encode(utf8.encode('hi')), id: '2')),
    );
    await got.stream.first.timeout(const Duration(seconds: 5));
    expect(events.first, isA<StatusEvent>());
    expect(utf8.decode((events.last as OutputEvent).bytes), 'hi');
    provenFirst(seen, '/v1/agent/sessions/ses_1/terminal/stream');
    expect(seen.last.path, contains('offset=0'));

    await sub.cancel();
    for (var i = 0; i < 100 && relay.closedByClient.isEmpty; i++) {
      await Future<void>.delayed(const Duration(milliseconds: 20));
    }
    expect(relay.closedByClient, isNotEmpty);
    await terminal.close();
  });

  group('widgets, with the saved address refused', () {
    Future<void> waitFor(WidgetTester tester, bool Function() done) async {
      for (var i = 0; i < 300 && !done(); i++) {
        await tester.runAsync(
          () => Future<void>.delayed(const Duration(milliseconds: 20)),
        );
        await tester.pump();
      }
    }

    Future<ProviderContainer> mount(
      WidgetTester tester,
      FakeRelay relay,
      Widget home,
    ) async {
      final c = ProviderContainer(
        overrides: [
          settingsProvider.overrideWith(
            () => FixedSettings(signedInOffLan(relay)),
          ),
        ],
      );
      addTearDown(c.dispose);
      await tester.runAsync(() => c.read(settingsProvider.future));
      await tester.runAsync(
        () => tester.pumpWidget(
          UncontrolledProviderScope(
            container: c,
            child: MaterialApp(theme: StormTheme.dark(), home: home),
          ),
        ),
      );
      return c;
    }

    Future<void> unmount(WidgetTester tester, ProviderContainer c) async {
      await tester.pumpWidget(const SizedBox());
      c.read(connectionProvider)?.reset();
      await tester.runAsync(
        () => Future<void>.delayed(const Duration(milliseconds: 50)),
      );
    }

    testWidgets('Add a device mints its invite through the relay', (
      tester,
    ) async {
      final seen = <Seen>[];
      final relay = FakeRelay();
      relay.handler = origin(seen, relay);
      await tester.runAsync(relay.start);
      try {
        final c = await mount(tester, relay, const AddDeviceScreen());
        await waitFor(
          tester,
          () => find.textContaining('nonce_relayed').evaluate().isNotEmpty,
        );
        expect(find.textContaining('nonce_relayed'), findsOneWidget);
        provenFirst(seen, '/v1/pairings');
        await unmount(tester, c);
      } finally {
        await tester.runAsync(relay.close);
      }
    });

    testWidgets('an attachment image loads through the relay', (tester) async {
      final seen = <Seen>[];
      final relay = FakeRelay();
      relay.handler = origin(seen, relay);
      await tester.runAsync(relay.start);
      try {
        final c = await mount(
          tester,
          relay,
          const Scaffold(
            body: AttachmentStrip(body: '![x](attachments/x.png)'),
          ),
        );
        const path = '/v1/vaults/$vaultId/attachments/attachments/x.png';
        await waitFor(tester, () => seen.any((r) => r.path == path));
        await tester.runAsync(
          () => Future<void>.delayed(const Duration(milliseconds: 200)),
        );
        await tester.pump();
        provenFirst(seen, path);
        expect(find.byIcon(LucideIcons.image_off), findsNothing);
        expect(tester.widget<RawImage>(find.byType(RawImage)).image, isNotNull);
        await unmount(tester, c);
      } finally {
        await tester.runAsync(relay.close);
      }
    });
  });

  test('an unproven connection is sent nothing', () async {
    final seen = <Seen>[];
    final relay = FakeRelay();
    relay.handler = origin(seen, relay, impostor: true);
    await relay.start();
    addTearDown(relay.close);
    final c = await container(relay);
    final connection = c.read(connectionProvider)!;
    expect(connection.status, ConnectionStatus.idle);
    final api = c.read(agentApiFactoryProvider)!();
    await expectLater(api.hosts(), throwsA(anything));
    expect(connection.status, ConnectionStatus.unproven);
    expect(seen.where((r) => r.auth != null), isEmpty);
  });
}
