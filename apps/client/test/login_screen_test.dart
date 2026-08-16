import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:storm/api/auth_api.dart';
import 'package:storm/router.dart';
import 'package:storm/state/app_state.dart';

import 'shell_harness.dart';

/// Signing in on a device that is already paired.
///
/// Two things carry this slice and both are here: the redirect that tells a
/// paired-but-signed-out device to go to /login rather than /pairing, and the
/// mapping from wire code to a sentence, which is what stops every auth
/// refusal reading as "couldn't reach the server".
void main() {
  /// Settings for a device that has paired but holds no session.
  const pairedNoSession = Settings(
    baseUrl: 'http://test',
    deviceId: 'dev-1',
    deviceSecret: 'sec-1',
    serverId: 'srv-1',
  );

  group('the redirect', () {
    String pathOf(ProviderContainer c) =>
        c.read(routerProvider).routerDelegate.currentConfiguration.uri.path;

    testWidgets('a paired device with no session lands on Sign in', (
      tester,
    ) async {
      // The gap this slice closes. Before it, the same state went to /pairing
      // and asked for a QR the user does not have and does not need.
      final c = shellContainer(configured: false, settings: pairedNoSession);
      await pumpShell(tester, c);

      expect(pathOf(c), Routes.login);
      expect(find.text('Sign in'), findsWidgets);
      await disposeShell(tester, c);
    });

    testWidgets('an unpaired device still lands on pairing', (tester) async {
      final c = shellContainer(configured: false, settings: const Settings());
      await pumpShell(tester, c);

      expect(pathOf(c), Routes.pairing);
      await disposeShell(tester, c);
    });

    testWidgets('a signed-in device is sent away from Sign in', (tester) async {
      final c = shellContainer(
        settings: pairedNoSession.copyWith(accessToken: 'access-1'),
      );
      await pumpShell(tester, c);

      c.read(routerProvider).go(Routes.login);
      await tester.pumpAndSettle();
      expect(pathOf(c), Routes.dashboard);
      await disposeShell(tester, c);
    });

    testWidgets('a legacy token install never sees Sign in', (tester) async {
      // It has no device credential, so /login could not succeed even if it
      // were shown. Same invariant as decision 52b: auth stays additive.
      final c = shellContainer(
        settings: const Settings(baseUrl: 'http://test', token: 't'),
      );
      await pumpShell(tester, c);

      c.read(routerProvider).go(Routes.login);
      await tester.pumpAndSettle();
      expect(pathOf(c), Routes.dashboard);
      await disposeShell(tester, c);
    });
  });

  group('authFailureMessage', () {
    String messageFor(String code, {int? retryAfter, int status = 401}) =>
        authFailureMessage(
          AuthApiException(status, code, retryAfter: retryAfter),
        );

    test('every code in the protocol table gets its own sentence', () {
      const codes = [
        'invalid_credentials',
        'session_expired',
        'session_revoked',
        'device_revoked',
        'not_paired',
        'user_disabled',
        'forbidden',
        'already_initialized',
        'pairing_consumed',
        'pairing_expired',
      ];
      final seen = <String>{};
      for (final code in codes) {
        final message = messageFor(code);
        expect(
          message.contains(code),
          isFalse,
          reason: '$code leaked the wire code into the message',
        );
        expect(
          seen.add(message),
          isTrue,
          reason: '$code shares a message with another code',
        );
      }
    });

    test('no auth refusal claims the server was unreachable', () {
      // The M9/M10 bug in one assertion: an HTTP refusal described as a
      // network failure makes every following request look offline too.
      for (final code in [
        'invalid_credentials',
        'session_expired',
        'device_revoked',
        'rate_limited',
        'something_new_and_unknown',
      ]) {
        expect(
          messageFor(code).toLowerCase(),
          isNot(contains('reach')),
          reason: '$code must not read as a connectivity problem',
        );
      }
    });

    test('rate limiting says how long, when the server said', () {
      expect(
        messageFor('rate_limited', retryAfter: 30),
        contains('30 seconds'),
      );
      expect(messageFor('rate_limited', retryAfter: 60), contains('a minute'));
      expect(
        messageFor('rate_limited', retryAfter: 240),
        contains('4 minutes'),
      );
      // Rounded up: 90s is "2 minutes", never "1" — telling someone to come
      // back before the lock lifts earns a second refusal.
      expect(messageFor('rate_limited', retryAfter: 90), contains('2 minutes'));
    });

    test('rate limiting without a number still says something useful', () {
      final message = messageFor('rate_limited');
      expect(message, contains('Too many attempts'));
      expect(message, isNot(contains('null')));
    });

    test('an unknown code names the server as the refuser', () {
      expect(messageFor('brand_new_code'), contains('brand_new_code'));
      expect(messageFor('brand_new_code').toLowerCase(), contains('refused'));
    });
  });
}
