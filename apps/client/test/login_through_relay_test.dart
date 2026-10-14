import 'dart:convert';

import 'package:cryptography/cryptography.dart';
import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'package:storm/api/auth_models.dart';
import 'package:storm/api/ed25519_verify.dart';
import 'package:storm/api/pairing_route.dart';
import 'package:storm/state/app_state.dart';
import 'package:storm/ui/login_screen.dart';
import 'package:storm/ui/theme.dart';

import 'fake_relay.dart';
import 'shell_harness.dart' show FixedSettings;

/// Signing in away from the server's network (decision 111): the saved address
/// is a LAN IP nothing answers, so sign-in must race the saved relays, prove
/// the server, and only then send the password.
const password = 'correct horse battery staple';

/// One request as the relay's origin saw it.
class Seen {
  Seen(this.path, this.headers, this.body);
  final String path;
  final Map<String, String> headers;
  final String body;
  bool get carriesPassword => body.contains(password);
}

void main() {
  late SimpleKeyPair keyPair;
  late String pubkey;

  setUpAll(() async {
    keyPair = await Ed25519().newKeyPair();
    final pk = await keyPair.extractPublicKey();
    pubkey = base64Url.encode(pk.bytes).replaceAll('=', '');
  });

  /// The server behind the fake relay. With [impostor] it answers the
  /// challenge with a signature its key cannot have made.
  FakeHandler origin(List<Seen> seen, {bool impostor = false}) =>
      (method, path, headers, body) async {
        seen.add(Seen(path, headers, utf8.decode(body)));
        Map<String, Object?>? json;
        switch (path) {
          case '/v1/server':
            json = {
              'server_id': 'srv_test',
              'key_id': 'key_1',
              'algorithm': 'ed25519',
              'public_key': pubkey,
              'relays': const [],
            };
          case '/v1/server/challenge':
            final nonce = jsonDecode(utf8.decode(body))['nonce'] as String;
            final signer = impostor ? await Ed25519().newKeyPair() : keyPair;
            final sig = await Ed25519().sign(
              utf8.encode(challengeMessage('srv_test', nonce)),
              keyPair: signer,
            );
            json = {
              'signature': base64Url.encode(sig.bytes).replaceAll('=', ''),
            };
          case '/v1/account':
            json = {'exists': true};
          case '/v1/auth/login' || '/v1/auth/refresh':
            json = {
              'session_id': 'ses_1',
              'access_token': 'sat_relayed',
              'refresh_token': 'srt_relayed',
              'expires': '2099-01-01T00:00:00Z',
              'refresh_expires': '2099-01-01T00:00:00Z',
              'user_id': 'usr_1',
              'device_id': 'dev_1',
            };
        }
        if (json == null) return FakeResponse(404, const {}, const []);
        return FakeResponse(200, {
          'content-type': 'application/json',
        }, utf8.encode(jsonEncode(json)));
      };

  Settings pairedOffLan(FakeRelay relay) => Settings(
    // Nothing listens on port 1: the LAN address is "not here".
    baseUrl: 'http://127.0.0.1:1',
    deviceId: 'dev_1',
    deviceSecret: 'dvs_1',
    serverId: 'srv_test',
    serverKeyId: 'key_1',
    serverPublicKey: pubkey,
    relays: [RelayAdvert.derive('ws://127.0.0.1:${relay.port}', 'srv_test')],
  );

  /// Real sockets need real time: every step that reaches the network runs in
  /// [WidgetTester.runAsync], and the frame is pumped between waits.
  Future<void> waitFor(WidgetTester tester, bool Function() done) async {
    for (var i = 0; i < 300 && !done(); i++) {
      await tester.runAsync(
        () => Future<void>.delayed(const Duration(milliseconds: 20)),
      );
      await tester.pump();
    }
  }

  Future<ProviderContainer> mount(WidgetTester tester, Settings s) async {
    final c = ProviderContainer(
      overrides: [settingsProvider.overrideWith(() => FixedSettings(s))],
    );
    addTearDown(c.dispose);
    await tester.runAsync(() => c.read(settingsProvider.future));
    await tester.runAsync(
      () => tester.pumpWidget(
        UncontrolledProviderScope(
          container: c,
          child: MaterialApp(
            theme: StormTheme.dark(),
            home: const LoginScreen(),
          ),
        ),
      ),
    );
    return c;
  }

  Future<void> unmount(WidgetTester tester) async {
    await tester.pumpWidget(const SizedBox());
    await tester.runAsync(
      () => Future<void>.delayed(const Duration(milliseconds: 50)),
    );
  }

  Future<void> signIn(WidgetTester tester) async {
    await tester.enterText(find.byKey(const Key('login-password')), password);
    await tester.pump();
    await tester.runAsync(
      () => tester.tap(find.byKey(const Key('login-submit'))),
    );
  }

  testWidgets('with the address refused, sign-in goes through the relay, '
      'and the password only after the challenge', (tester) async {
    final seen = <Seen>[];
    final relay = FakeRelay(handler: origin(seen));
    await tester.runAsync(relay.start);
    try {
      final c = await mount(tester, pairedOffLan(relay));
      await waitFor(
        tester,
        () => find.byKey(const Key('login-password')).evaluate().isNotEmpty,
      );
      await signIn(tester);
      await waitFor(
        tester,
        () => c.read(settingsProvider).value?.accessToken.isNotEmpty ?? false,
      );

      expect(c.read(settingsProvider).value?.accessToken, 'sat_relayed');
      final paths = [for (final r in seen) r.path];
      final challenge = paths.indexOf('/v1/server/challenge');
      final login = paths.indexOf('/v1/auth/login');
      expect(challenge, isNonNegative, reason: '$paths');
      expect(login, greaterThan(challenge), reason: '$paths');
      for (final r in seen.take(challenge + 1)) {
        expect(r.carriesPassword, isFalse, reason: '${r.path} before proof');
        expect(
          r.headers['authorization'] ?? r.headers['Authorization'],
          isNull,
        );
      }
      expect(
        seen.where((r) => r.carriesPassword).single.path,
        '/v1/auth/login',
      );
      await unmount(tester);
    } finally {
      await tester.runAsync(relay.close);
    }
  });

  testWidgets('an impostor on the relay gets an identity error, never the '
      'password', (tester) async {
    final seen = <Seen>[];
    final relay = FakeRelay(handler: origin(seen, impostor: true));
    await tester.runAsync(relay.start);
    try {
      final c = await mount(tester, pairedOffLan(relay));
      await waitFor(
        tester,
        () => find.byKey(const Key('login-password')).evaluate().isNotEmpty,
      );
      await signIn(tester);
      await waitFor(
        tester,
        () => find
            .textContaining('cryptographic challenge through the relay')
            .evaluate()
            .isNotEmpty,
      );

      expect(
        find.textContaining(
          'cryptographic challenge through the relay 127.0.0.1',
        ),
        findsOneWidget,
      );
      expect(find.textContaining('Your password was not sent'), findsOneWidget);
      expect(c.read(settingsProvider).value?.accessToken, isEmpty);
      expect(seen.where((r) => r.carriesPassword), isEmpty);
      expect([
        for (final r in seen) r.path,
      ], isNot(anyOf(contains('/v1/auth/login'), contains('/v1/account'))));
      await unmount(tester);
    } finally {
      await tester.runAsync(relay.close);
    }
  });

  group('openSavedRoute', () {
    test('a plain ws:// relay off loopback is never raced', () async {
      final route = openSavedRoute(
        baseUrl: 'http://127.0.0.1:1',
        serverId: 'srv_test',
        publicKey: pubkey,
        relays: [RelayAdvert.derive('ws://relay.example', 'srv_test')],
      );
      await expectLater(
        route,
        throwsA(
          isA<PairingRouteException>()
              .having(
                (e) => e.failure,
                'failure',
                PairingFailure.serverUnreachable,
              )
              .having((e) => e.relayHost, 'relayHost', isNull),
        ),
      );
    });

    test('a relay without the server is named', () async {
      final relay = FakeRelay(helloError: 'server_unreachable');
      await relay.start();
      try {
        await expectLater(
          openSavedRoute(
            baseUrl: 'http://127.0.0.1:1',
            serverId: 'srv_test',
            publicKey: pubkey,
            relays: [
              RelayAdvert.derive('ws://127.0.0.1:${relay.port}', 'srv_test'),
            ],
            raceTimeout: const Duration(seconds: 2),
          ),
          throwsA(
            isA<PairingRouteException>()
                .having(
                  (e) => e.failure,
                  'failure',
                  PairingFailure.serverNotOnRelay,
                )
                .having((e) => e.relayHost, 'relayHost', '127.0.0.1'),
          ),
        );
      } finally {
        await relay.close();
      }
    });
  });

  test('a session refresh off the LAN goes through the relay, the refresh '
      'token only after the challenge', () async {
    final seen = <Seen>[];
    final relay = FakeRelay(handler: origin(seen));
    await relay.start();
    try {
      FlutterSecureStorage.setMockInitialValues({
        'storm.deviceSecret': 'dvs_1',
        'storm.accessToken': 'sat_old',
        'storm.refreshToken': 'srt_old',
      });
      final advert = RelayAdvert.derive(
        'ws://127.0.0.1:${relay.port}',
        'srv_test',
      );
      SharedPreferences.setMockInitialValues({
        'storm.baseUrl': 'http://127.0.0.1:1',
        'storm.deviceId': 'dev_1',
        'storm.serverId': 'srv_test',
        'storm.serverPubKey': pubkey,
        'storm.relays': jsonEncode([
          {'url': advert.url, 'public_address': advert.publicAddress},
        ]),
      });
      final c = ProviderContainer();
      addTearDown(c.dispose);
      await c.read(settingsProvider.future);

      expect(await c.read(settingsProvider.notifier).refreshSession(), isTrue);
      expect(c.read(settingsProvider).value?.accessToken, 'sat_relayed');
      final paths = [for (final r in seen) r.path];
      expect(
        paths.indexOf('/v1/auth/refresh'),
        greaterThan(paths.indexOf('/v1/server/challenge')),
        reason: '$paths',
      );
      expect(paths.indexOf('/v1/server/challenge'), isNonNegative);
    } finally {
      await relay.close();
    }
  });
}
