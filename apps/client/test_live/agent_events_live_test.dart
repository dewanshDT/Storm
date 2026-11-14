import 'dart:convert';
import 'dart:io';

import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;

import 'package:storm/agent/agent_events.dart';
import 'package:storm/agent/agent_state.dart';
import 'package:storm/api/auth_models.dart';
import 'package:storm/api/storm_api.dart';
import 'package:storm/state/app_state.dart';

/// `GET /v1/agent/events` against the real server (decision 154, 154-3): a
/// WebSocket when direct, SSE through the real `storm-relay`, both carrying
/// a real host's session from launch to dismissal. A-154-6's shape on
/// loopback; the LIVE gate is the same over a LAN and relay-cf.
void main() {
  const origin = String.fromEnvironment('STORM_RELAY_ORIGIN');
  const session = String.fromEnvironment('STORM_SESSION');
  const hostName = String.fromEnvironment('STORM_AGENT_HOST');
  const deadline = Duration(seconds: 20);

  late ServerInfo info;
  late String token;

  Future<T> until<T>(Future<T?> Function() probe, String what) async {
    final end = DateTime.now().add(deadline);
    while (DateTime.now().isBefore(end)) {
      final got = await probe();
      if (got != null) return got;
      await Future<void>.delayed(const Duration(milliseconds: 50));
    }
    fail('timed out waiting for $what');
  }

  setUpAll(() async {
    if (origin.isEmpty || session.isEmpty || hostName.isEmpty) {
      fail(
        'Run through `make test-live`: it starts storm-relay and a runtime '
        'host and passes STORM_RELAY_ORIGIN, STORM_SESSION and '
        'STORM_AGENT_HOST.',
      );
    }
    token = session.replaceFirst('Bearer ', '');
    info = await until(() async {
      final i = ServerInfo.fromJson(
        jsonDecode((await http.get(Uri.parse('$origin/v1/server'))).body)
            as Map<String, dynamic>,
      );
      return i.relays.isEmpty ? null : i;
    }, 'the server to register with the relay');
  });

  Future<ProviderContainer> signedIn(String baseUrl) async {
    final direct = StormApi(baseUrl: origin, token: token);
    final vaultId = (await direct.vaults()).first.id;
    direct.dispose();
    final c = ProviderContainer(
      overrides: [
        settingsProvider.overrideWith(
          () => _Fixed(
            Settings(
              baseUrl: baseUrl,
              accessToken: token,
              serverId: info.serverId,
              serverKeyId: info.keyId,
              serverPublicKey: info.publicKey,
              activeVault: vaultId,
              relays: info.relays,
            ),
          ),
        ),
      ],
    );
    await c.read(settingsProvider.future);
    return c;
  }

  Future<void> follows(ProviderContainer c, {required bool relayed}) async {
    final connection = c.read(connectionProvider)!;
    await connection.connect();
    expect(connection.isRelayed, relayed);
    final factory = c.read(agentApiFactoryProvider)!;
    final api = factory();
    final host = await until(() async {
      final hosts = await api.hosts();
      return hosts.where((h) => h.name == hostName && h.online).firstOrNull;
    }, 'the runtime host to come online');

    final events = AgentEvents(() => openAgentEvents(factory, connection))
      ..start(followLifecycle: false);
    final arrived = <(AgentEventUpdate, DateTime)>[];
    final sub = events.updates.listen((u) => arrived.add((u, DateTime.now())));
    await until(() async => events.isOpen ? true : null, 'the snapshot');
    expect(arrived.first.$1.event, isA<AgentSnapshot>());
    expect(
      events.sessions!.map((s) => s.id).toSet(),
      (await api.sessions()).map((s) => s.id).toSet(),
      reason: 'the snapshot is the list',
    );

    final s = await api.launch(
      hostId: host.id,
      workspace: 'storm',
      provider: 'fake',
      cols: 80,
      rows: 24,
    );
    await until(
      () async => events.byId(s.id)?.status == 'running' ? true : null,
      'the launch, pushed',
    );

    final endedAt = DateTime.now();
    await api.end(s.id);
    await until(
      () async => events.byId(s.id)?.ended == true ? true : null,
      'the end, pushed',
    );
    final pushed = arrived.lastWhere(
      (a) => switch (a.$1.event) {
        AgentSessionChanged(:final session) =>
          session.id == s.id && session.ended,
        _ => false,
      },
    );
    expect(pushed.$2.difference(endedAt), lessThan(const Duration(seconds: 2)));

    await api.dismiss(s.id);
    await until(
      () async =>
          arrived.any(
            (a) => switch (a.$1.event) {
              AgentSessionRemoved(:final id) => id == s.id,
              _ => false,
            },
          )
          ? true
          : null,
      'the dismissal, pushed',
    );
    expect(events.byId(s.id), isNull);

    await sub.cancel();
    events.dispose();
    api.dispose();
  }

  test('a direct client follows a session over a WebSocket', () async {
    final c = await signedIn(origin);
    addTearDown(() {
      c.read(connectionProvider)?.close();
      c.dispose();
    });
    await follows(c, relayed: false);
  });

  test(
    'a relayed client follows a session over SSE through the relay',
    () async {
      // A port nothing listens on: the LAN address is "not here".
      final socket = await ServerSocket.bind(InternetAddress.loopbackIPv4, 0);
      final closed = socket.port;
      await socket.close();
      final c = await signedIn('http://127.0.0.1:$closed');
      addTearDown(() {
        c.read(connectionProvider)?.close();
        c.dispose();
      });
      await follows(c, relayed: true);
    },
  );
}

class _Fixed extends SettingsNotifier {
  _Fixed(this.value);
  final Settings value;

  @override
  Future<Settings> build() async => value;

  @override
  Future<void> save(Settings next) async => state = AsyncData(next);
}
