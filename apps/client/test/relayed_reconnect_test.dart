import 'dart:async';
import 'dart:convert';

import 'package:cryptography/cryptography.dart';
import 'package:drift/native.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:storm/api/auth_models.dart';
import 'package:storm/api/connection_log.dart';
import 'package:storm/api/ed25519_verify.dart';
import 'package:storm/api/relay/srp_trunk.dart';
import 'package:storm/api/storm_connection.dart';
import 'package:storm/cache/cache_db.dart';
import 'package:storm/state/app_state.dart';

import 'fake_relay.dart';
import 'shell_harness.dart' show FixedSettings;

/// A phone on mobile data, signed in, off the LAN (decision 113): every
/// request goes through a relay several hundred milliseconds away, and the app
/// must stay connected rather than tear its trunk down and re-race in a loop.
void main() {
  const vaultId = 'vlt_1';
  const token = 'sat_live';
  late SimpleKeyPair keyPair;
  late String pubkey;

  setUpAll(() async {
    keyPair = await Ed25519().newKeyPair();
    final pk = await keyPair.extractPublicKey();
    pubkey = base64Url.encode(pk.bytes).replaceAll('=', '');
  });

  late FakeRelay relay;
  late List<String> paths;
  late StreamController<List<int>> feed;
  var slowCreate = Duration.zero;

  FakeHandler origin() => (method, path, headers, body) async {
    paths.add('$method ${path.split('?').first}');
    Object? json;
    switch (path.split('?').first) {
      case '/v1/server':
        json = {
          'server_id': 'srv_test',
          'key_id': 'key_1',
          'algorithm': 'ed25519',
          'public_key': pubkey,
          'relays': [
            {
              'url': 'ws://127.0.0.1:${relay.port}',
              'public_address': relay.url.toString(),
            },
          ],
        };
      case '/v1/server/challenge':
        final nonce = jsonDecode(utf8.decode(body))['nonce'] as String;
        final sig = await Ed25519().sign(
          utf8.encode(challengeMessage('srv_test', nonce)),
          keyPair: keyPair,
        );
        json = {'signature': base64Url.encode(sig.bytes).replaceAll('=', '')};
      case '/v1/vaults':
        json = {
          'vaults': [
            {'id': vaultId, 'name': 'Notes'},
          ],
        };
      case '/v1/vaults/$vaultId/sync':
        json = {'changes': const [], 'seq': 0};
      case '/v1/vaults/$vaultId/notes':
        await Future<void>.delayed(slowCreate);
        json = {
          'note': {'id': 'n1', 'path': 'a.md', 'version': 1},
          'content': 'hi',
          'seq': 1,
        };
      case '/v1/auth/ws-ticket':
        json = {'ticket': 'tkt_1'};
      case '/v1/stream':
        return FakeResponse.streamed(200, {
          'content-type': 'text/event-stream',
        }, feed.stream);
    }
    if (json == null) return FakeResponse(404, const {}, const []);
    return FakeResponse(200, {
      'content-type': 'application/json',
    }, utf8.encode(jsonEncode(json)));
  };

  Future<ProviderContainer> signedInOffLan() async {
    final c = ProviderContainer(
      overrides: [
        settingsProvider.overrideWith(
          () => FixedSettings(
            Settings(
              // Nothing listens on port 1: the LAN address is "not here".
              baseUrl: 'http://127.0.0.1:1',
              accessToken: token,
              deviceId: 'dev_1',
              deviceSecret: 'dvs_1',
              serverId: 'srv_test',
              serverKeyId: 'key_1',
              serverPublicKey: pubkey,
              activeVault: vaultId,
              relays: [
                RelayAdvert(
                  url: 'ws://127.0.0.1:${relay.port}',
                  publicAddress: relay.url.toString(),
                ),
              ],
            ),
          ),
        ),
        cacheProvider.overrideWithValue(CacheDb(NativeDatabase.memory())),
      ],
    );
    addTearDown(() {
      c.read(connectionProvider)?.reset();
      c.dispose();
    });
    await c.read(settingsProvider.future);
    return c;
  }

  Future<void> until(bool Function() ok, String what) async {
    final end = DateTime.now().add(const Duration(seconds: 15));
    while (!ok()) {
      if (DateTime.now().isAfter(end)) fail('timed out waiting for $what');
      await Future<void>.delayed(const Duration(milliseconds: 20));
    }
  }

  setUp(() async {
    paths = [];
    feed = StreamController<List<int>>.broadcast();
    slowCreate = Duration.zero;
    relay = FakeRelay();
    relay.handler = origin();
    await relay.start();
    ConnectionLog.instance.clear();
  });
  tearDown(() async {
    unawaited(feed.close());
    await relay.close();
  });

  test('a relay a phone reaches in 2.5 s connects on the first race, and '
      'the vaults load through it', () async {
    // Each frame back takes 800 ms: READY, STREAM_READY and the answer put
    // the first probe at about 2.4 s, past the old 2 s race window.
    relay.latency = const Duration(milliseconds: 800);
    final c = await signedInOffLan();

    final vaults = await c.read(apiProvider)!.vaults();

    expect(vaults.single.id, vaultId);
    final connection = c.read(connectionProvider)!;
    expect(connection.status, ConnectionStatus.connected);
    expect(connection.isRelayed, isTrue);
    expect(relay.hellos, 1, reason: '${ConnectionLog.instance}');
  });

  test('the sync engine connects through a slow relay and creates a note, '
      'on one trunk', () async {
    relay.latency = const Duration(milliseconds: 800);
    final c = await signedInOffLan();
    final engine = c.read(syncEngineProvider);
    await until(
      () => paths.contains('GET /v1/stream'),
      'the change feed to open',
    );

    final created = await engine.create(path: 'a.md', content: 'hi');

    expect(created.error, isNull);
    expect(engine.isOnline, isTrue);
    expect(engine.connectionStatus, ConnectionStatus.connected);
    expect(relay.hellos, 1, reason: '${ConnectionLog.instance}');
  });

  test('a note created while the connection re-races waits for the race, '
      'rather than going to the saved address', () async {
    relay.latency = const Duration(milliseconds: 100);
    final c = await signedInOffLan();
    final engine = c.read(syncEngineProvider);
    await until(
      () => paths.contains('GET /v1/stream'),
      'the change feed to open',
    );
    final connection = c.read(connectionProvider)!;

    // What a network change does: the winner is forgotten and a new race
    // starts. The request below lands in the middle of it.
    connection.reset('test');
    unawaited(connection.connect());
    expect(connection.status, ConnectionStatus.connecting);
    final created = await engine.create(path: 'a.md', content: 'hi');

    expect(created.error, isNull);
    expect(engine.isOnline, isTrue);
    expect(paths.where((p) => p == 'POST /v1/vaults/$vaultId/notes'), [
      'POST /v1/vaults/$vaultId/notes',
    ]);
  });

  test('the change feed ending re-opens it on the same trunk, without '
      'failing a request in flight', () async {
    relay.latency = const Duration(milliseconds: 100);
    final c = await signedInOffLan();
    final engine = c.read(syncEngineProvider);
    await until(
      () => paths.contains('GET /v1/stream'),
      'the change feed to open',
    );
    expect(relay.hellos, 1);

    // A save that is still on the trunk when the feed ends, and when the
    // engine's reconnect timer (1 s) fires.
    slowCreate = const Duration(milliseconds: 1500);
    final created = engine.create(path: 'a.md', content: 'hi');
    await Future<void>.delayed(const Duration(milliseconds: 200));
    unawaited(feed.close());
    feed = StreamController<List<int>>.broadcast();

    expect((await created).error, isNull);
    await until(
      () => paths.where((p) => p == 'GET /v1/stream').length == 2,
      'the change feed to re-open',
    );
    expect(engine.isOnline, isTrue);
    expect(relay.hellos, 1, reason: '${ConnectionLog.instance}');
  });

  test('a trunk the carrier drops ends the connection, and the next request '
      're-races onto a new trunk', () async {
    final c = await signedInOffLan();
    final api = c.read(apiProvider)!;
    await api.vaults();
    final connection = c.read(connectionProvider)!;
    expect(connection.status, ConnectionStatus.connected);

    await relay.dropTrunk();
    await until(
      () => connection.status != ConnectionStatus.connected,
      'the connection to notice the dead trunk',
    );

    expect((await api.vaults()).single.id, vaultId);
    expect(connection.status, ConnectionStatus.connected);
    expect(relay.hellos, 2);
  });

  test(
    'the connection log names hops and reasons, never a credential',
    () async {
      relay.latency = const Duration(milliseconds: 50);
      final c = await signedInOffLan();
      await c.read(apiProvider)!.vaults();
      await relay.dropTrunk();
      final connection = c.read(connectionProvider)!;
      await until(
        () => connection.status != ConnectionStatus.connected,
        'the dead trunk',
      );

      final log = ConnectionLog.instance.toString();
      expect(log, contains('race start'));
      expect(log, contains('trunk ready'));
      expect(log, contains('race won by selfHosted'));
      expect(log, contains('status connecting -> connected'));
      expect(log, contains('trunk lost: socket_closed'));
      expect(log, contains('reset: active trunk lost: socket_closed'));
      for (final secret in [token, pubkey, 'srv_test', 'tkt_1', '/v1/']) {
        expect(log, isNot(contains(secret)), reason: secret);
      }
    },
  );

  test('one failed request does not latch the engine offline: it re-proves '
      'and comes back with no feed event', () async {
    final c = await signedInOffLan();
    final engine = c.read(syncEngineProvider);
    await until(
      () => paths.contains('GET /v1/stream'),
      'the change feed to open',
    );
    relay.streamErrors['/v1/vaults/$vaultId/notes'] = 'server_timeout';

    final failed = await engine.create(path: 'a.md', content: 'hi');
    expect(failed.error, isNotNull);
    expect(engine.isOnline, isFalse);
    expect(
      engine.connectionStatus,
      ConnectionStatus.connected,
      reason: 'one stream timing out says nothing about the connection',
    );

    relay.streamErrors.clear();
    await until(() => engine.isOnline, 'the engine to come back online');
    expect((await engine.create(path: 'a.md', content: 'hi')).error, isNull);
    expect(relay.hellos, 1, reason: '${ConnectionLog.instance}');
  });

  test('a stream-scoped server_timeout is not remembered as "server '
      'offline" by the next race', () async {
    final c = await signedInOffLan();
    final api = c.read(apiProvider)!;
    await api.vaults();
    final connection = c.read(connectionProvider)!;
    relay.streamErrors['/v1/vaults'] = 'server_timeout';
    await expectLater(api.vaults(), throwsA(anything));
    expect(connection.status, ConnectionStatus.connected);

    // Now nothing answers at all: that is offline, not "server offline,
    // relay reachable".
    await relay.close();
    await until(
      () => connection.status != ConnectionStatus.connected,
      'the closed relay to end the connection',
    );
    await connection.connect();
    expect(connection.status, ConnectionStatus.offline);
  });

  test(
    'saving an unrelated setting keeps the connection and its trunk',
    () async {
      final c = await signedInOffLan();
      await c.read(apiProvider)!.vaults();
      final before = c.read(connectionProvider)!;
      final notifier = c.read(settingsProvider.notifier);

      await notifier.save(
        c.read(settingsProvider).value!.copyWith(fontSize: 20),
      );

      expect(identical(c.read(connectionProvider), before), isTrue);
      expect(before.hasLiveTrunk, isTrue);
      expect(relay.hellos, 1);
    },
  );

  test('a rebuilt connection closes the old one\'s trunk', () async {
    final c = await signedInOffLan();
    await c.read(apiProvider)!.vaults();
    final before = c.read(connectionProvider)!;
    expect(before.hasLiveTrunk, isTrue);

    await c
        .read(settingsProvider.notifier)
        .save(c.read(settingsProvider).value!.copyWith(accessToken: 'sat_new'));
    final after = c.read(connectionProvider)!;

    expect(identical(after, before), isFalse);
    expect(before.hasLiveTrunk, isFalse);
    await until(() => relay.socketClosed, 'the old trunk to close');
  });

  test(
    'two callers during one handshake share one socket and one HELLO',
    () async {
      relay.latency = const Duration(milliseconds: 300);
      final trunk = SrpTrunk(url: relay.url, serverId: 'srv_test');
      addTearDown(trunk.dispose);

      await Future.wait([trunk.connect(), trunk.connect(), trunk.openStream()]);

      expect(relay.hellos, 1);
      expect(trunk.isConnected, isTrue);
    },
  );
}
