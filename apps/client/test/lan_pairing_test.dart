import 'dart:convert';

import 'package:cryptography/cryptography.dart';
import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';

import 'package:storm/api/auth_api.dart';
import 'package:storm/api/ed25519_verify.dart';
import 'package:storm/api/lan_discovery.dart';
import 'package:storm/api/lan_pairing.dart';
import 'package:storm/api/pairing_route.dart';
import 'package:storm/api/storm_connection.dart';
import 'package:storm/state/app_state.dart';
import 'package:storm/state/client_version.dart';
import 'package:storm/ui/lan_servers.dart';
import 'package:storm/ui/pairing_screen.dart';
import 'package:storm/ui/theme.dart';

import 'lan_fakes.dart';
import 'shell_harness.dart' show FixedSettings;

/// Pairing with a server found on the LAN (decision 118): the identity
/// challenge against the advertised key comes first, the password only after
/// it, and only over the advertised direct address.
void main() {
  late SimpleKeyPair serverKey;
  late SimpleKeyPair impostorKey;
  late String pubkey;
  const sid = 'srv_LANTEST0000000000';
  const password = 'correct horse battery staple';

  setUpAll(() async {
    serverKey = await Ed25519().newKeyPair();
    impostorKey = await Ed25519().newKeyPair();
    final pk = await serverKey.extractPublicKey();
    pubkey = base64Url.encode(pk.bytes).replaceAll('=', '');
  });

  DiscoveredServer advertised({String host = '192.168.1.13'}) =>
      DiscoveredServer.fromAdvert(
        name: "Dewansh's Storm",
        addresses: [host],
        port: 8484,
        txt: {
          'v': '1',
          'sid': sid,
          'pk': pubkey,
          'ver': '0.0.0',
          'claimed': '1',
          'lan': '1',
        },
      )!;

  /// The server's side, recording `METHOD host path` in order. [signWith]
  /// lets a test make the challenge lie; [pairStatus] refuses the pairing.
  http.Client server(
    List<String> seen, {
    SimpleKeyPair? signWith,
    int pairStatus = 200,
    String pairError = '',
    int? retryAfter,
  }) => MockClient((request) async {
    final path = request.url.path;
    seen.add('${request.method} ${request.url.host} $path');
    final json = {'content-type': 'application/json'};
    switch (path) {
      case '/v1/server':
        return http.Response(
          jsonEncode({
            'server_id': sid,
            'key_id': 'key_1',
            'algorithm': 'ed25519',
            'public_key': pubkey,
            // A relay the route must never use for the password.
            'relays': [
              {
                'url': 'wss://relay.example',
                'public_address': 'wss://relay.example/connect/$sid',
              },
            ],
          }),
          200,
          headers: json,
        );
      case '/v1/server/challenge':
        final nonce = jsonDecode(request.body)['nonce'] as String;
        final sig = await Ed25519().sign(
          utf8.encode(challengeMessage(sid, nonce)),
          keyPair: signWith ?? serverKey,
        );
        return http.Response(
          jsonEncode({
            'server_id': sid,
            'key_id': 'key_1',
            'signature': base64Url.encode(sig.bytes).replaceAll('=', ''),
          }),
          200,
          headers: json,
        );
      case '/v1/pairings/password':
        if (pairStatus != 200) {
          return http.Response(
            jsonEncode({'error': pairError, 'retry_after': ?retryAfter}),
            pairStatus,
            headers: json,
          );
        }
        final body = jsonDecode(request.body) as Map<String, dynamic>;
        if (body['password'] != password) {
          return http.Response(
            '{"error":"invalid_credentials"}',
            401,
            headers: json,
          );
        }
        return http.Response(
          jsonEncode({
            'device_id': 'dev_lan',
            'device_secret': 'dvs_lan',
            'server_id': sid,
            'public_key': pubkey,
            'key_id': 'key_1',
          }),
          200,
          headers: json,
        );
      case '/v1/auth/login':
        return http.Response(
          jsonEncode({
            'access_token': 'sta_lan',
            'refresh_token': 'str_lan',
            'expires': '2099-01-01T00:00:00Z',
            'refresh_expires': '2099-01-01T00:00:00Z',
            'user_id': 'usr_1',
            'device_id': 'dev_lan',
            'session_id': 'ses_1',
          }),
          200,
          headers: json,
        );
    }
    return http.Response('{"error":"unexpected"}', 404);
  });

  /// Every candidate the route builds, and the one fake server behind them.
  AuthApi Function(ConnectionCandidate) recording(
    List<ConnectionCandidate> candidates,
    http.Client client,
  ) => (c) {
    candidates.add(c);
    return AuthApi(baseUrl: c.baseUrl, client: client);
  };

  group('the route', () {
    test('verifies the advertised key, then pairs and signs in', () async {
      final seen = <String>[];
      final candidates = <ConnectionCandidate>[];
      final route = await openLanRoute(
        advertised(),
        buildAuthApi: recording(candidates, server(seen)),
      );
      final paired = await pairOverLan(
        route,
        password: password,
        deviceName: 'Pixel 10',
        platform: 'android',
      );
      route.close();

      expect(paired.device.deviceId, 'dev_lan');
      expect(paired.session?.accessToken, 'sta_lan');
      expect(route.serverInfo.relays, hasLength(1));
      final challenge = seen.indexOf('POST 192.168.1.13 /v1/server/challenge');
      final pairing = seen.indexOf('POST 192.168.1.13 /v1/pairings/password');
      expect(challenge, greaterThanOrEqualTo(0));
      expect(pairing, greaterThan(challenge));
      expect(seen.last, 'POST 192.168.1.13 /v1/auth/login');
    });

    test('an impostor gets no password request', () async {
      final seen = <String>[];
      await expectLater(
        openLanRoute(
          advertised(),
          buildAuthApi: recording([], server(seen, signWith: impostorKey)),
        ),
        throwsA(
          isA<PairingRouteException>().having(
            (e) => e.failure,
            'failure',
            PairingFailure.identityFailed,
          ),
        ),
      );
      expect(seen.where((s) => s.contains('/v1/pairings/password')), isEmpty);
      expect(seen.where((s) => s.contains('/v1/auth/login')), isEmpty);
    });

    test('is never routed through a relay', () async {
      final seen = <String>[];
      final candidates = <ConnectionCandidate>[];
      final route = await openLanRoute(
        advertised(),
        buildAuthApi: recording(candidates, server(seen)),
      );
      await pairOverLan(route, password: password, deviceName: 'Pixel 10');
      route.close();

      // The server advertised a relay; nothing was dialled through it.
      expect(candidates, isNotEmpty);
      expect(
        candidates.every(
          (c) => c.tier == CandidateTier.direct && c.relayUrl == null,
        ),
        isTrue,
      );
      expect(
        seen.every((s) => s.contains(' 192.168.1.13 ')),
        isTrue,
        reason: '$seen',
      );
    });

    test('refusals are worded for the sheet', () {
      expect(
        lanPairingError(AuthApiException(401, 'invalid_credentials')),
        'Wrong password.',
      );
      expect(
        lanPairingError(AuthApiException(429, 'rate_limited', retryAfter: 42)),
        'Too many attempts, try again in 42 s.',
      );
      expect(
        lanPairingError(AuthApiException(403, 'lan_only')),
        'Pairing by password works only on the same network as the server.',
      );
      expect(isSetupRequired(AuthApiException(409, 'setup_required')), isTrue);
    });
  });

  group('the screen', () {
    late FakeLanBrowser browser;

    // Not `pumpAndSettle`: the list's spinner turns for as long as it browses.
    Future<void> settle(WidgetTester tester) async {
      for (var i = 0; i < 30; i++) {
        await tester.pump(const Duration(milliseconds: 50));
      }
    }

    Future<ProviderContainer> pump(
      WidgetTester tester, {
      bool supported = true,
      Settings settings = const Settings(),
      SimpleKeyPair? signWith,
      int pairStatus = 200,
      String pairError = '',
      List<String>? seen,
    }) async {
      browser = FakeLanBrowser();
      final client = server(
        seen ?? [],
        signWith: signWith,
        pairStatus: pairStatus,
        pairError: pairError,
      );
      final c = ProviderContainer(
        overrides: [
          settingsProvider.overrideWith(() => FixedSettings(settings)),
          clientVersionProvider.overrideWith((ref) async => '0.0.0'),
          lanDiscoverySupportedProvider.overrideWithValue(supported),
          lanBrowserFactoryProvider.overrideWithValue(() => browser),
          defaultDeviceNameProvider.overrideWith((ref) async => 'Pixel 10'),
          lanRouteOpenerProvider.overrideWithValue(
            (s) => openLanRoute(
              s,
              buildAuthApi: (cand) =>
                  AuthApi(baseUrl: cand.baseUrl, client: client),
            ),
          ),
        ],
      );
      await tester.pumpWidget(
        UncontrolledProviderScope(
          container: c,
          child: MaterialApp(
            theme: StormTheme.dark(),
            home: const PairingScreen(),
          ),
        ),
      );
      await tester.pump();
      return c;
    }

    testWidgets('lists servers with their state, live', (tester) async {
      await pump(tester);
      expect(find.byKey(const Key('lan-servers')), findsOneWidget);
      expect(find.byKey(const Key('lan-browsing')), findsOneWidget);
      final fresh = DiscoveredServer.fromAdvert(
        name: 'storm-test',
        addresses: ['192.168.1.40'],
        port: 8484,
        txt: {
          'v': '1',
          'sid': 'srv_FRESH',
          'pk': pubkey,
          'ver': '0.0.0',
          'claimed': '0',
          'lan': '1',
        },
      )!;
      browser.emit([advertised(), fresh]);
      await tester.pump();

      expect(find.text("Dewansh's Storm"), findsOneWidget);
      expect(find.text('192.168.1.13'), findsOneWidget);
      expect(find.text('${advertised().fingerprint} · v0.0.0'), findsOneWidget);
      expect(find.text('Needs setup — scan its setup QR'), findsOneWidget);
      await tester.pumpWidget(const SizedBox());
      expect(browser.stops, 1, reason: 'browsing stops when the screen goes');
    });

    testWidgets('says why the list is empty after 5 s', (tester) async {
      await pump(tester);
      expect(find.byKey(const Key('lan-empty')), findsNothing);
      await tester.pump(const Duration(seconds: 5));
      expect(find.byKey(const Key('lan-empty')), findsOneWidget);
      expect(find.textContaining('No Storm servers found'), findsOneWidget);
      await tester.pumpWidget(const SizedBox());
    });

    testWidgets('web hides the section and keeps QR and manual entry', (
      tester,
    ) async {
      await pump(tester, supported: false);
      expect(find.byKey(const Key('lan-servers')), findsNothing);
      expect(browser.starts, 0);
      expect(find.text('Verify & Pair'), findsOneWidget);
      await tester.pumpWidget(const SizedBox());
    });

    testWidgets('an impostor sees no password field', (tester) async {
      final seen = <String>[];
      await pump(tester, signWith: impostorKey, seen: seen);
      browser.emit([advertised()]);
      await tester.pump();
      await tester.tap(find.text("Dewansh's Storm"));
      await settle(tester);

      expect(find.byKey(const Key('lan-identity-failed')), findsOneWidget);
      expect(
        find.text(
          "This server didn't prove its identity. Don't enter your password.",
        ),
        findsOneWidget,
      );
      expect(find.byKey(const Key('lan-password')), findsNothing);
      expect(seen.where((s) => s.contains('/v1/pairings/password')), isEmpty);
      await tester.pumpWidget(const SizedBox());
    });

    testWidgets('verified: fingerprint, password, and the device is saved', (
      tester,
    ) async {
      final c = await pump(tester);
      browser.emit([advertised()]);
      await tester.pump();
      await tester.tap(find.text("Dewansh's Storm"));
      await settle(tester);

      expect(find.text("Pair with Dewansh's Storm"), findsOneWidget);
      expect(
        tester.widget<Text>(find.byKey(const Key('lan-fingerprint'))).data,
        advertised().fingerprint,
      );
      await tester.enterText(
        find.descendant(
          of: find.byKey(const Key('lan-password')),
          matching: find.byType(EditableText),
        ),
        password,
      );
      await tester.tap(find.byKey(const Key('lan-pair')));
      await settle(tester);

      final saved = c.read(settingsProvider).value!;
      expect(saved.baseUrl, 'http://192.168.1.13:8484');
      expect(saved.deviceId, 'dev_lan');
      expect(saved.serverId, sid);
      expect(saved.serverPublicKey, pubkey);
      expect(saved.accessToken, 'sta_lan');
      expect(saved.relays.single.url, 'wss://relay.example');
      await tester.pumpWidget(const SizedBox());
    });

    testWidgets('a wrong password is said inline, as sign-in says it', (
      tester,
    ) async {
      await pump(tester);
      browser.emit([advertised()]);
      await tester.pump();
      await tester.tap(find.text("Dewansh's Storm"));
      await settle(tester);
      await tester.enterText(
        find.descendant(
          of: find.byKey(const Key('lan-password')),
          matching: find.byType(EditableText),
        ),
        'not it',
      );
      await tester.tap(find.byKey(const Key('lan-pair')));
      await settle(tester);
      expect(find.text('Wrong password.'), findsOneWidget);
      await tester.pumpWidget(const SizedBox());
    });

    testWidgets('setup_required switches to the setup QR', (tester) async {
      await pump(tester, pairStatus: 409, pairError: 'setup_required');
      browser.emit([advertised()]);
      await tester.pump();
      await tester.tap(find.text("Dewansh's Storm"));
      await settle(tester);
      await tester.enterText(
        find.descendant(
          of: find.byKey(const Key('lan-password')),
          matching: find.byType(EditableText),
        ),
        password,
      );
      await tester.tap(find.byKey(const Key('lan-pair')));
      await settle(tester);
      expect(find.byKey(const Key('lan-setup-required')), findsOneWidget);
      await tester.pumpWidget(const SizedBox());
    });
  });
}
