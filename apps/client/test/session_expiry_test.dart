import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'package:storm/state/app_state.dart';

/// `accessTokenExpiresAt` was written by three screens and read by none.
///
/// So a session that lapsed stayed "configured" forever: the router kept you
/// on the dashboard, every request answered 401, and the screens filled with
/// failures that read as the app breaking rather than as a sign-in that had
/// run out. These pin the answers the app now acts on.
void main() {
  String stamp(Duration fromNow) =>
      DateTime.now().toUtc().add(fromNow).toIso8601String();

  Settings withExpiry(String at) => Settings(
    baseUrl: 'http://server',
    accessToken: 'a',
    accessTokenExpiresAt: at,
  );

  test('a live token is neither expired nor due for renewal', () {
    final s = withExpiry(stamp(const Duration(days: 20)));
    expect(s.accessTokenExpired, isFalse);
    expect(s.accessTokenNeedsRefresh, isFalse);
  });

  test('a token close to expiry is renewed before it dies', () {
    // The case that matters for an offline-first client: a phone that has been
    // away comes back and must not find its session already gone.
    final s = withExpiry(stamp(const Duration(hours: 2)));
    expect(s.accessTokenExpired, isFalse);
    expect(s.accessTokenNeedsRefresh, isTrue);
  });

  test('a lapsed token is expired', () {
    final s = withExpiry(stamp(const Duration(minutes: -1)));
    expect(s.accessTokenExpired, isTrue);
  });

  test('an unknown expiry is never treated as expired', () {
    // Covers an install from before this was stored, and a stamp we cannot
    // parse. Guessing "expired" would sign someone out over a parse error,
    // which is a worse failure than running a little past the end.
    expect(withExpiry('').accessTokenExpired, isFalse);
    expect(withExpiry('not a date').accessTokenExpired, isFalse);
    expect(withExpiry('').accessTokenNeedsRefresh, isFalse);
  });

  test('hasSession still means only that tokens are present', () {
    // Deliberately unchanged: the refresh path needs to send a stale session,
    // so "has tokens" and "the tokens work" are different questions.
    final s = withExpiry(stamp(const Duration(minutes: -5)));
    expect(s.hasSession, isTrue);
    expect(s.accessTokenExpired, isTrue);
  });

  test('no session means nothing to renew or retire', () {
    const s = Settings(baseUrl: 'http://server');
    expect(s.hasSession, isFalse);
    expect(s.accessTokenExpired, isFalse);
    expect(s.accessTokenNeedsRefresh, isFalse);
  });

  /// The startup race, which is what made every getter above unreachable.
  ///
  /// `ensureSession()` used to open `final s = state.value; if (s == null)
  /// return;` and was called from `addPostFrameCallback` — which fires after
  /// the first frame, while `build()` is still awaiting
  /// `SharedPreferences.getInstance()` and the keychain. During `AsyncLoading`
  /// `state.value` is null, so it returned having done nothing, and
  /// `didChangeAppLifecycleState` only fires on a *change*, so `resumed` never
  /// arrived at launch either. The answers above were all correct and nothing
  /// ever asked for them.
  group('ensureSession at launch', () {
    /// A container whose settings load has **not** been awaited — which is the
    /// state the app is genuinely in when startup calls this.
    ///
    /// The tokens go into the **keychain**, not prefs: slice 9 moved every
    /// credential into `SecretStore`, so seeding `storm.accessToken` in prefs
    /// alone leaves `hasSession` false and `ensureSession` returns early — a
    /// test that would then pass without exercising anything.
    ProviderContainer unsettled({
      required Map<String, Object> prefs,
      required Map<String, String> secrets,
    }) {
      FlutterSecureStorage.setMockInitialValues(secrets);
      SharedPreferences.setMockInitialValues(prefs);
      final container = ProviderContainer();
      addTearDown(container.dispose);
      expect(
        container.read(settingsProvider).value,
        isNull,
        reason:
            'precondition: the provider must still be loading, or this test '
            'proves nothing about the race it exists for',
      );
      return container;
    }

    test(
      'an expired session is retired even before settings have loaded',
      () async {
        final container = unsettled(
          prefs: {
            'storm.baseUrl': 'http://127.0.0.1:1',
            // Long dead, and the address above refuses connections, so the
            // refresh cannot succeed — the only correct outcome is that the
            // session is cleared and the router falls to /login.
            'storm.accessTokenExpiresAt': DateTime.now()
                .toUtc()
                .subtract(const Duration(days: 2))
                .toIso8601String(),
            'storm.userId': 'u1',
          },
          secrets: {
            'storm.accessToken': 'stale',
            'storm.refreshToken': 'also-stale',
          },
        );

        await container.read(settingsProvider.notifier).ensureSession();

        final after = container.read(settingsProvider).value!;
        expect(
          after.accessToken,
          isEmpty,
          reason: 'a dead session must not survive',
        );
        expect(after.hasSession, isFalse);
      },
    );

    test('a live session is left alone', () async {
      final container = unsettled(
        prefs: {
          'storm.baseUrl': 'http://127.0.0.1:1',
          'storm.accessTokenExpiresAt': DateTime.now()
              .toUtc()
              .add(const Duration(days: 20))
              .toIso8601String(),
        },
        secrets: {'storm.accessToken': 'good'},
      );

      await container.read(settingsProvider.notifier).ensureSession();

      expect(container.read(settingsProvider).value!.accessToken, 'good');
    });
  });
}
