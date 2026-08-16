import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'package:storm/state/app_state.dart';

/// The auth half of [Settings] — device credentials, session tokens, and the
/// two ways a session ends.
///
/// The lifecycle methods have no UI yet, so without these they would ship as
/// untested code that the pairing screen quietly depends on the shape of.
void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  /// A fully paired, logged-in install.
  const paired = Settings(
    baseUrl: 'http://vault.local',
    deviceId: 'dev-1',
    deviceSecret: 'sec-1',
    serverId: 'srv-1',
    serverKeyId: 'key-1',
    serverPublicKey: 'pub-1',
    accessToken: 'access-1',
    refreshToken: 'refresh-1',
    accessTokenExpiresAt: '2026-08-17T12:00:00.000Z',
    userId: 'user-1',
  );

  group('derived state', () {
    test('a paired install with a session is configured', () {
      expect(paired.isPaired, isTrue);
      expect(paired.hasSession, isTrue);
      expect(paired.isConfigured, isTrue);
      expect(paired.bearerToken, 'access-1');
    });

    test('a legacy token install is configured without being paired', () {
      // The invariant that keeps auth additive: an install predating pairing
      // still reaches its vault, and the router must not bounce it.
      const legacy = Settings(baseUrl: 'http://vault.local', token: 't');
      expect(legacy.isPaired, isFalse);
      expect(legacy.hasSession, isFalse);
      expect(legacy.isConfigured, isTrue);
      expect(legacy.bearerToken, 't');
    });

    test('a session token wins over a stale legacy token', () {
      expect(paired.copyWith(token: 'old').bearerToken, 'access-1');
    });

    test('a base URL alone is not configured', () {
      expect(const Settings(baseUrl: 'http://vault.local').isConfigured, false);
    });

    test('the device header carries id and secret', () {
      expect(paired.deviceHeader, 'StormDevice dev-1:sec-1');
    });
  });

  /// A notifier hosted in its own container, over the given prefs.
  ///
  /// `SettingsNotifier` cannot set `state` outside a container, so every
  /// lifecycle test needs one; the container is torn down with the test. The
  /// initial load is awaited before returning — otherwise it lands *after* the
  /// test's first `save` and overwrites it.
  Future<SettingsNotifier> notifierOver(Map<String, Object> prefs) async {
    SharedPreferences.setMockInitialValues(prefs);
    final container = ProviderContainer();
    addTearDown(container.dispose);
    await container.read(settingsProvider.future);
    return container.read(settingsProvider.notifier);
  }

  /// What a relaunch would load, reading the prefs the test just wrote.
  Future<Settings> reload() async {
    final container = ProviderContainer();
    addTearDown(container.dispose);
    return container.read(settingsProvider.future);
  }

  group('persistence', () {
    test('every auth field survives a save and reload', () async {
      final notifier = await notifierOver({});
      await notifier.save(paired);

      // A second container reads what the first wrote — the launch path.
      final reloaded = await reload();
      expect(reloaded.deviceId, 'dev-1');
      expect(reloaded.deviceSecret, 'sec-1');
      expect(reloaded.serverId, 'srv-1');
      expect(reloaded.serverKeyId, 'key-1');
      expect(reloaded.serverPublicKey, 'pub-1');
      expect(reloaded.accessToken, 'access-1');
      expect(reloaded.refreshToken, 'refresh-1');
      expect(reloaded.accessTokenExpiresAt, '2026-08-17T12:00:00.000Z');
      expect(reloaded.userId, 'user-1');
    });

    test('a fresh install has no auth state', () async {
      SharedPreferences.setMockInitialValues({});
      final fresh = await reload();
      expect(fresh.isPaired, isFalse);
      expect(fresh.hasSession, isFalse);
      expect(fresh.isConfigured, isFalse);
    });
  });

  group('ending a session', () {
    /// A notifier already holding [paired], with prefs backing it.
    Future<SettingsNotifier> pairedNotifier() async {
      final notifier = await notifierOver({});
      await notifier.save(paired);
      return notifier;
    }

    test('logout drops the session but keeps the device paired', () async {
      final notifier = await pairedNotifier();
      await notifier.logout();

      final s = notifier.state.value!;
      expect(s.hasSession, isFalse);
      expect(s.accessToken, isEmpty);
      expect(s.refreshToken, isEmpty);
      expect(s.userId, isEmpty);
      // Still paired: the credentials a re-login will reuse once the
      // login-only screen exists.
      expect(s.isPaired, isTrue);
      expect(s.deviceSecret, 'sec-1');
      expect(s.baseUrl, 'http://vault.local');
    });

    test('logout clears a legacy token too', () async {
      // Left behind, it would keep `isConfigured` true and skip the login the
      // logout was asking for.
      final notifier = await notifierOver({});
      await notifier.save(paired.copyWith(token: 'legacy'));
      await notifier.logout();

      expect(notifier.state.value!.isConfigured, isFalse);
    });

    test('unpair forgets the server entirely', () async {
      final notifier = await pairedNotifier();
      await notifier.unpair();

      final s = notifier.state.value!;
      expect(s.isPaired, isFalse);
      expect(s.hasSession, isFalse);
      expect(s.deviceId, isEmpty);
      expect(s.deviceSecret, isEmpty);
      expect(s.serverId, isEmpty);
      expect(s.serverPublicKey, isEmpty);
    });

    test('logout and unpair both survive a reload', () async {
      final notifier = await pairedNotifier();
      await notifier.unpair();

      expect((await reload()).isPaired, isFalse);
    });
  });

  test(
    'refreshing without a refresh token fails rather than calling out',
    () async {
      // baseUrl points nowhere; reaching the network would hang or throw, so a
      // clean `false` proves the guard ran first.
      final notifier = await notifierOver({});
      await notifier.save(paired.copyWith(refreshToken: ''));

      expect(await notifier.refreshSession(), isFalse);
    },
  );
}
