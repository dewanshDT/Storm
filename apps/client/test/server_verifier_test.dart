import 'dart:convert';

import 'package:cryptography/cryptography.dart';
import 'package:drift/native.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';

import 'package:storm/api/auth_api.dart';
import 'package:storm/api/storm_connection.dart';
import 'package:storm/api/ed25519_verify.dart';
import 'package:storm/api/server_verifier.dart';
import 'package:storm/api/storm_api.dart';
import 'package:storm/cache/cache_db.dart';
import 'package:storm/sync/sync_engine.dart';

import 'fake_server.dart';

/// Challenge-on-connect: the check that lets a relay carry Storm's traffic
/// without being trusted with it.
///
/// The property under test is not "a signature verifies" — `ed25519_verify_test`
/// covers that. It is that the engine *refuses to talk to a server that fails
/// the check*, and that it runs the check again on every reconnect rather than
/// once at startup.
void main() {
  const serverId = 'SRV0123456789ABCDEF';

  late SimpleKeyPair keyPair;
  late String publicKeyB64;

  setUpAll(() async {
    keyPair = await Ed25519().newKeyPair();
    final pk = await keyPair.extractPublicKey();
    publicKeyB64 = base64Url.encode(pk.bytes).replaceAll('=', '');
  });

  /// Signs the real challenge message, exactly as the server does.
  Future<String> signFor(String nonce, {String? asServer}) async {
    final message = utf8.encode(challengeMessage(asServer ?? serverId, nonce));
    final sig = await Ed25519().sign(message, keyPair: keyPair);
    return base64Url.encode(sig.bytes).replaceAll('=', '');
  }

  /// An `AuthApi` whose challenge endpoint behaves however a test needs.
  ///
  /// [answer] receives the nonce the client generated, so a test can sign the
  /// right message, sign the wrong one, or refuse outright.
  AuthApi authApiThat(Future<http.Response> Function(String nonce) answer) =>
      AuthApi(
        baseUrl: 'http://test',
        client: MockClient((request) async {
          expect(request.url.path, '/v1/server/challenge');
          final nonce = jsonDecode(request.body)['nonce'] as String;
          return answer(nonce);
        }),
      );

  http.Response signed(String signature) =>
      http.Response(jsonEncode({'signature': signature}), 200);

  group('ServerVerifier', () {
    test('a correctly signed nonce verifies', () async {
      final verifier = ServerVerifier(
        authApi: authApiThat((nonce) async => signed(await signFor(nonce))),
        serverId: serverId,
        publicKeyB64: publicKeyB64,
      );
      expect(await verifier.check(), ServerProof.verified);
    });

    test('a signature over a different server id is an impostor', () async {
      // The server id is inside the signed message precisely so a signature
      // lifted from one server cannot be replayed for another.
      final verifier = ServerVerifier(
        authApi: authApiThat(
          (nonce) async => signed(await signFor(nonce, asServer: 'OTHER-SRV')),
        ),
        serverId: serverId,
        publicKeyB64: publicKeyB64,
      );
      expect(await verifier.check(), ServerProof.impostor);
    });

    test('a signature over a different nonce is an impostor', () async {
      // Replaying yesterday's valid answer must not work.
      final verifier = ServerVerifier(
        authApi: authApiThat(
          (_) async => signed(await signFor('some-old-nonce')),
        ),
        serverId: serverId,
        publicKeyB64: publicKeyB64,
      );
      expect(await verifier.check(), ServerProof.impostor);
    });

    test('garbage where a signature should be is an impostor', () async {
      final verifier = ServerVerifier(
        authApi: authApiThat((_) async => signed('not-a-signature')),
        serverId: serverId,
        publicKeyB64: publicKeyB64,
      );
      expect(await verifier.check(), ServerProof.impostor);
    });

    test('a socket failure is unreachable, never an impostor', () async {
      // The distinction the whole client is built on: nothing answered, so
      // nothing has been learned about who the server is. Calling this an
      // impostor would turn every tunnel outage into a security alarm.
      final verifier = ServerVerifier(
        authApi: AuthApi(
          baseUrl: 'http://test',
          client: MockClient((_) => throw const SocketishError()),
        ),
        serverId: serverId,
        publicKeyB64: publicKeyB64,
      );
      expect(await verifier.check(), ServerProof.unreachable);
    });

    test('an HTTP refusal is unreachable, not an impostor', () async {
      // A server predating the endpoint answers 404. That is not a forgery,
      // and treating it as one would brick an upgrade.
      final verifier = ServerVerifier(
        authApi: authApiThat((_) async => http.Response('{}', 404)),
        serverId: serverId,
        publicKeyB64: publicKeyB64,
      );
      expect(await verifier.check(), ServerProof.unreachable);
    });

    test('nothing pinned means nothing to verify', () async {
      // An install that has not paired has no key to check against. Refusing
      // to sync here would be refusing to work before setup.
      final verifier = ServerVerifier(
        authApi: authApiThat((_) async => fail('must not be called')),
        serverId: '',
        publicKeyB64: '',
      );
      expect(await verifier.check(), ServerProof.verified);
    });
  });

  group('SyncEngine gates on the check', () {
    late CacheDb cache;
    late FakeServer server;
    var challenges = 0;

    SyncEngine engineWith(ServerVerifier? verifier) => SyncEngine(
      connection: StormConnection.direct(
        api: StormApi(
          baseUrl: 'http://test',
          token: 't',
          client: server.client,
        ),
        verifier: verifier,
      ),
      cache: cache,
      vaultId: FakeServer.primaryVault,
    );

    ServerVerifier verifierThat(
      Future<http.Response> Function(String nonce) answer,
    ) => ServerVerifier(
      authApi: authApiThat((nonce) {
        challenges++;
        return answer(nonce);
      }),
      serverId: serverId,
      publicKeyB64: publicKeyB64,
    );

    setUp(() {
      cache = CacheDb(NativeDatabase.memory());
      server = FakeServer();
      challenges = 0;
    });

    tearDown(() async => cache.close());

    test('a verified server syncs normally', () async {
      final engine = engineWith(
        verifierThat((nonce) async => signed(await signFor(nonce))),
      );
      addTearDown(engine.dispose);

      await engine.start();

      expect(challenges, 1, reason: 'the check runs on connect');
      expect(engine.serverIdentityFailed, isFalse);
      expect(engine.isOnline, isTrue);
    });

    test('a failed challenge blocks sync and says so', () async {
      final engine = engineWith(
        verifierThat((_) async => signed(await signFor('wrong-nonce'))),
      );
      addTearDown(engine.dispose);

      await engine.start();

      expect(engine.serverIdentityFailed, isTrue);
      // Not folded into offline: the server answered, it just is not the
      // server. Reporting this as offline would send the user to check wifi.
      expect(
        engine.isOnline,
        isTrue,
        reason: 'identity failure is a third state, not offline',
      );
    });

    test('an unproven server is sent nothing at all', () async {
      // The point of the check is to not hand plaintext to whatever answered,
      // so it has to block every request, not only the pull.
      final engine = engineWith(
        verifierThat((_) async => signed('not-a-signature')),
      );
      addTearDown(engine.dispose);
      await engine.start();
      expect(engine.serverIdentityFailed, isTrue);

      final before = server.requests;

      await engine.sync();
      final created = await engine.create(path: 'Leaked.md');
      final saved = await engine.save(
        id: 'n1',
        baseVersion: 1,
        content: 'secret\n',
      );

      expect(
        server.requests,
        before,
        reason: 'not one request may reach an unproven server',
      );
      expect(created.meta, isNull);
      expect(created.error, contains('challenge'));
      // The edit is not lost — it queues, exactly as it would offline.
      expect(saved.status, SaveStatus.queued);
    });

    test(
      'an unreachable challenge is offline, not an identity failure',
      () async {
        final engine = engineWith(
          verifierThat((_) => throw const SocketishError()),
        );
        addTearDown(engine.dispose);

        await engine.start();

        expect(engine.serverIdentityFailed, isFalse);
        expect(engine.isOnline, isFalse);
      },
    );

    test('every reconnect re-runs the check', () async {
      // The reason the gate lives in the engine rather than at startup: a
      // reconnect is what a transport switch looks like from here — wifi
      // handover, a server restart, waking from sleep — and it is exactly
      // when a relay may have appeared in the path.
      final engine = engineWith(
        verifierThat((nonce) async => signed(await signFor(nonce))),
      );
      addTearDown(engine.dispose);

      await engine.start();
      expect(challenges, 1);

      engine.debugSimulateSocketDrop();
      // The first backoff is one second.
      await Future<void>.delayed(const Duration(milliseconds: 1400));

      expect(
        challenges,
        greaterThan(1),
        reason: 'a reconnect that skipped the check would leave the hole open',
      );
    });

    test('a server that proves itself later is trusted again', () async {
      // Recovery matters: an operator who removes a bad relay should not have
      // to re-pair every device. Because the check reruns on reconnect, the
      // recovery path is the reconnect path — nothing extra to build.
      var honest = false;
      final engine = engineWith(
        verifierThat(
          (nonce) async =>
              signed(await signFor(honest ? nonce : 'wrong-nonce')),
        ),
      );
      addTearDown(engine.dispose);

      await engine.start();
      expect(engine.serverIdentityFailed, isTrue);

      honest = true;
      await Future<void>.delayed(const Duration(milliseconds: 1400));

      expect(
        engine.serverIdentityFailed,
        isFalse,
        reason: 'a passing check clears the failure',
      );
    });

    test('no verifier means the old behaviour, unchanged', () async {
      final engine = engineWith(null);
      addTearDown(engine.dispose);

      await engine.start();

      expect(challenges, 0);
      expect(engine.serverIdentityFailed, isFalse);
      expect(engine.isOnline, isTrue);
    });
  });
}

/// Stands in for a socket-level failure — no HTTP response, ever.
class SocketishError implements Exception {
  const SocketishError();
  @override
  String toString() => 'connection refused';
}
