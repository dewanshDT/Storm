import 'dart:convert';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:storm/api/auth_models.dart';
import 'package:storm/api/pairing_route.dart';
import 'package:storm/api/storm_api.dart';
import 'package:storm/state/app_state.dart';
import 'package:storm/ui/login_screen.dart';
import 'package:storm/ui/theme.dart';

/// A device added away from home signs in (decision 111), against the
/// real `storm-relay` and storm-server `make test-live` starts.
///
/// Its own file, not `relay_live_test.dart`: a `testWidgets` there would make
/// the whole file run under the widget binding, whose `HttpClient` answers 400
/// to everything outside `runAsync`, including that suite's plain `test`s.
void main() {
  const origin = String.fromEnvironment('STORM_RELAY_ORIGIN');
  const session = String.fromEnvironment('STORM_SESSION');
  const password = 'correct horse battery staple';
  const deadline = Duration(seconds: 15);

  // The widget binding swaps in an HttpClient that answers 400 to everything.
  setUpAll(() => HttpOverrides.global = null);

  testWidgets('an extra device pairs and signs in through the relay with the '
      'address unreachable', (tester) async {
    if (origin.isEmpty || session.isEmpty) {
      fail(
        'Run through `make test-live`: it starts storm-relay and passes '
        'STORM_RELAY_ORIGIN and STORM_SESSION.',
      );
    }
    final token = session.replaceFirst('Bearer ', '');
    late Settings paired;

    await tester.runAsync(() async {
      // The server registers with the relay in the background.
      String? relayUrl;
      final until = DateTime.now().add(deadline);
      while (relayUrl == null && DateTime.now().isBefore(until)) {
        final client = HttpClient();
        final req = await client.getUrl(Uri.parse('$origin/v1/server'));
        final body = await (await req.close()).transform(utf8.decoder).join();
        client.close();
        final relays = (jsonDecode(body)['relays'] as List? ?? const [])
            .cast<Map<String, dynamic>>();
        if (relays.isNotEmpty) {
          relayUrl = relays.first['url'] as String;
        } else {
          await Future<void>.delayed(const Duration(milliseconds: 200));
        }
      }
      if (relayUrl == null) fail('The server never registered with the relay.');

      final closed = await ServerSocket.bind(InternetAddress.loopbackIPv4, 0);
      final port = closed.port;
      await closed.close();

      final direct = StormApi(baseUrl: origin, token: token);
      final invite = await direct.issuePairing();
      direct.dispose();
      final uri = PairingUri.parse(
        PairingInvite(
          serverId: invite.serverId,
          publicKey: invite.publicKey,
          nonce: invite.nonce,
          expires: invite.expires,
          address: '127.0.0.1:$port',
          relays: [relayUrl],
        ).toUri(),
      )!;

      // What the pairing screen's "account already exists" branch does.
      final route = await openPairingRoute(uri).timeout(deadline);
      expect(route.viaRelay, isTrue);
      final device = await route.authApi.pair(
        nonce: uri.nonce,
        deviceName: 'relay live',
        platform: 'test',
      );
      expect(
        await route.authApi.accountExists(
          deviceId: device.deviceId,
          deviceSecret: device.deviceSecret,
        ),
        isTrue,
      );
      paired = Settings(
        baseUrl: uri.baseUrl,
        deviceId: device.deviceId,
        deviceSecret: device.deviceSecret,
        serverId: route.serverInfo.serverId,
        serverKeyId: route.serverInfo.keyId,
        serverPublicKey: route.serverInfo.publicKey,
        relays: pairedRelays(uri, route.serverInfo),
      );
      route.close();
    });

    final c = ProviderContainer(
      overrides: [settingsProvider.overrideWith(() => _FixedSettings(paired))],
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
    Future<void> waitFor(bool Function() done) async {
      for (var i = 0; i < 750 && !done(); i++) {
        await tester.runAsync(
          () => Future<void>.delayed(const Duration(milliseconds: 20)),
        );
        await tester.pump();
      }
    }

    String screenText() => tester
        .widgetList<Text>(find.byType(Text))
        .map((t) => t.data)
        .join('\n');

    final field = find.byKey(const Key('login-password'));
    await waitFor(() => field.evaluate().isNotEmpty);
    expect(field, findsOneWidget, reason: screenText());
    await tester.enterText(field, password);
    await tester.pump();
    await tester.runAsync(
      () => tester.tap(find.byKey(const Key('login-submit'))),
    );
    await waitFor(
      () => c.read(settingsProvider).value?.accessToken.isNotEmpty ?? false,
    );

    final signedIn = c.read(settingsProvider).value!;
    expect(signedIn.accessToken, isNotEmpty, reason: screenText());
    await tester.runAsync(() async {
      final api = StormApi(baseUrl: origin, token: signedIn.accessToken);
      expect(await api.vaults(), isNotEmpty);
      api.dispose();
    });
    await tester.pumpWidget(const SizedBox());
    await tester.runAsync(
      () => Future<void>.delayed(const Duration(milliseconds: 50)),
    );
  });
}

class _FixedSettings extends SettingsNotifier {
  _FixedSettings(this.value);
  final Settings value;

  @override
  Future<Settings> build() async => value;

  @override
  Future<void> save(Settings next) async => state = AsyncData(next);
}
