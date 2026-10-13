import 'dart:convert';

import 'package:cryptography/cryptography.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';

import 'package:storm/api/auth_api.dart';
import 'package:storm/api/auth_models.dart';
import 'package:storm/api/ed25519_verify.dart';
import 'package:storm/api/pairing_route.dart';
import 'package:storm/api/storm_connection.dart';

import 'fake_relay.dart';

/// Pairing through a relay (decision 96): a QR scanned away from the LAN
/// reaches its server through one of the QR's relays — and nothing sensitive
/// crosses any transport until the server has proved it holds the QR's key.
void main() {
  late SimpleKeyPair keyPair;
  late String pubkey;

  setUpAll(() async {
    keyPair = await Ed25519().newKeyPair();
    final pk = await keyPair.extractPublicKey();
    pubkey = base64Url.encode(pk.bytes).replaceAll('=', '');
  });

  Future<String> signFor(String serverId, String nonce) async {
    final sig = await Ed25519().sign(
      utf8.encode(challengeMessage(serverId, nonce)),
      keyPair: keyPair,
    );
    return base64Url.encode(sig.bytes).replaceAll('=', '');
  }

  PairingUri qr({
    String serverId = 'srv_TESTSERVER',
    String addr = '192.168.91.51:8585',
    List<String> relays = const [],
  }) => PairingUri.parse(
    'storm://pair?v=1&sid=$serverId&pk=$pubkey&n=PAIRNONCE0000000'
    '&exp=2099-01-01T00:00:00Z&addr=$addr'
    '${relays.map((r) => '&relay=${Uri.encodeQueryComponent(r)}').join()}',
  )!;

  /// The server's unauthenticated surface: `/v1/server`, the challenge (signed
  /// with [signAs], so a test can make it lie) and `/v1/pair`. Records every
  /// path it is asked for, in order.
  Future<FakeResponse> Function(String, String, Map<String, String>, List<int>)
  serverHandler(
    List<String> seen, {
    required String serverId,
    String? signAs,
    String? advertiseId,
  }) => (method, path, headers, body) async {
    seen.add('$method $path');
    switch (path) {
      case '/v1/server':
        return FakeResponse(
          200,
          {'content-type': 'application/json'},
          utf8.encode(
            jsonEncode({
              'server_id': advertiseId ?? serverId,
              'key_id': 'key_1',
              'algorithm': 'ed25519',
              'public_key': pubkey,
              'relays': const [],
            }),
          ),
        );
      case '/v1/server/challenge':
        final nonce = jsonDecode(utf8.decode(body))['nonce'] as String;
        return FakeResponse(
          200,
          {'content-type': 'application/json'},
          utf8.encode(
            jsonEncode({'signature': await signFor(signAs ?? serverId, nonce)}),
          ),
        );
      case '/v1/pair':
        return FakeResponse(
          200,
          {'content-type': 'application/json'},
          utf8.encode(
            jsonEncode({
              'device_id': 'dev_1',
              'device_secret': 'secret',
              'server_id': serverId,
              'public_key': pubkey,
              'key_id': 'key_1',
            }),
          ),
        );
    }
    return FakeResponse(404, const {}, const []);
  };

  /// [serverHandler] as an `http.Client`, for routing tests that need no trunk.
  http.Client asClient(
    Future<FakeResponse> Function(
      String,
      String,
      Map<String, String>,
      List<int>,
    )
    handler,
  ) => MockClient((request) async {
    final r = await handler(
      request.method,
      request.url.path,
      request.headers,
      request.bodyBytes,
    );
    return http.Response.bytes(r.body, r.status, headers: r.headers);
  });

  AuthApi Function(ConnectionCandidate) byHost(
    Map<String, http.Client> clients,
  ) => (c) {
    final host = Uri.parse(c.baseUrl).host;
    return AuthApi(
      baseUrl: c.baseUrl,
      client:
          clients[host] ??
          MockClient((_) async => throw http.ClientException('no route')),
    );
  };

  group('the route', () {
    test('direct wins when the QR address answers', () async {
      final seen = <String>[];
      final uri = qr(relays: ['wss://mine.example']);
      final route = await openPairingRoute(
        uri,
        buildAuthApi: byHost({
          '192.168.91.51': asClient(
            serverHandler(seen, serverId: uri.serverId),
          ),
        }),
      );
      expect(route.viaRelay, isFalse);
      expect(route.serverInfo.serverId, uri.serverId);
      route.close();
    });

    test('off the LAN, the QR relay carries pairing once proven', () async {
      final seen = <String>[];
      final uri = qr(relays: ['wss://mine.example']);
      final route = await openPairingRoute(
        uri,
        buildAuthApi: byHost({
          'mine.example': asClient(serverHandler(seen, serverId: uri.serverId)),
        }),
      );
      expect(route.viaRelay, isTrue);
      expect(route.relayHost, 'mine.example');
      expect(seen, [
        'GET /v1/server',
        'POST /v1/server/challenge',
        'GET /v1/server',
      ], reason: 'nothing but identity questions before the route exists');
      route.close();
    });

    test('a relay whose server fails the challenge is never paired through, '
        'and is sent nothing sensitive', () async {
      final seen = <String>[];
      final uri = qr(relays: ['wss://mine.example']);
      await expectLater(
        openPairingRoute(
          uri,
          buildAuthApi: byHost({
            'mine.example': asClient(
              serverHandler(seen, serverId: uri.serverId, signAs: 'srv_OTHER'),
            ),
          }),
        ),
        throwsA(
          isA<PairingRouteException>()
              .having(
                (e) => e.failure,
                'failure',
                PairingFailure.identityFailed,
              )
              .having((e) => e.relayHost, 'relayHost', 'mine.example'),
        ),
      );
      expect(seen, ['GET /v1/server', 'POST /v1/server/challenge']);
    });

    test(
      'a server that proves the key but names another id is a mismatch',
      () async {
        final seen = <String>[];
        final uri = qr();
        await expectLater(
          openPairingRoute(
            uri,
            buildAuthApi: byHost({
              '192.168.91.51': asClient(
                serverHandler(
                  seen,
                  serverId: uri.serverId,
                  advertiseId: 'srv_OTHER',
                ),
              ),
            }),
          ),
          throwsA(
            isA<PairingRouteException>().having(
              (e) => e.failure,
              'failure',
              PairingFailure.identityMismatch,
            ),
          ),
        );
      },
    );

    test('a plain ws:// relay off this machine is never used to pair', () {
      final uri = qr(
        relays: ['ws://relay.example:8787', 'ws://127.0.0.1:8787', 'wss://a.b'],
      );
      expect(pairingRelays(uri).map((r) => r.url), [
        'ws://127.0.0.1:8787',
        'wss://a.b',
      ]);
    });
  });

  group('when nothing is reached', () {
    Future<PairingRouteException> failureWith(
      PairingUri uri,
      RelayReach reach,
    ) async {
      try {
        await openPairingRoute(
          uri,
          buildAuthApi: byHost(const {}),
          probe: (_) async => reach,
        );
      } on PairingRouteException catch (e) {
        return e;
      }
      fail('expected a PairingRouteException');
    }

    test('no relays: the server is unreachable', () async {
      final e = await failureWith(qr(), RelayReach.down);
      expect(e.failure, PairingFailure.serverUnreachable);
      expect(e.relayHost, isNull);
    });

    test('a relay that does not answer is named as the problem', () async {
      final e = await failureWith(
        qr(relays: ['wss://mine.example']),
        RelayReach.down,
      );
      expect(e.failure, PairingFailure.relayUnreachable);
      expect(e.relayHost, 'mine.example');
    });

    test('a relay without the server says the server is offline', () async {
      final e = await failureWith(
        qr(relays: ['wss://mine.example']),
        RelayReach.serverAbsent,
      );
      expect(e.failure, PairingFailure.serverNotOnRelay);
    });

    test('a relay that has the server blames the server', () async {
      final e = await failureWith(
        qr(relays: ['wss://mine.example']),
        RelayReach.up,
      );
      expect(e.failure, PairingFailure.serverUnreachable);
      expect(e.relayHost, 'mine.example');
    });
  });

  group('through a real SRP trunk', () {
    test('pairing completes over the relay, the nonce only after the '
        'challenge', () async {
      final seen = <String>[];
      final relay = FakeRelay(
        handler: serverHandler(seen, serverId: 'srv_test'),
      );
      await relay.start();
      try {
        final uri = qr(
          serverId: 'srv_test',
          // Nothing listens on port 1: the LAN address is "not here".
          addr: '127.0.0.1:1',
          relays: ['ws://127.0.0.1:${relay.port}'],
        );
        final route = await openPairingRoute(uri);
        expect(route.viaRelay, isTrue);

        final paired = await route.authApi.pair(
          nonce: uri.nonce,
          deviceName: 'test',
        );
        expect(paired.deviceId, 'dev_1');
        expect(
          seen.indexOf('POST /v1/pair'),
          greaterThan(seen.indexOf('POST /v1/server/challenge')),
        );
        route.close();
      } finally {
        await relay.close();
      }
    });

    test(
      'probing a relay tells "server not there" from "relay not there"',
      () async {
        final absent = FakeRelay(helloError: 'server_unreachable');
        await absent.start();
        final up = FakeRelay();
        await up.start();
        try {
          RelayAdvert advert(int port) =>
              RelayAdvert.derive('ws://127.0.0.1:$port', 'srv_test');
          expect(
            await probeRelay(advert(absent.port), 'srv_test'),
            RelayReach.serverAbsent,
          );
          expect(await probeRelay(advert(up.port), 'srv_test'), RelayReach.up);
          expect(await probeRelay(advert(1), 'srv_test'), RelayReach.down);
        } finally {
          await absent.close();
          await up.close();
        }
      },
    );
  });
}
