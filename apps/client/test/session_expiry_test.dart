import 'package:flutter_test/flutter_test.dart';

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
}
