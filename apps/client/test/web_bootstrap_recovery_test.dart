import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'package:storm/state/app_state.dart';

/// H1 and H2 from *Auth Review Findings (PR 28)*.
///
/// Web bootstrap has two ways to produce no device, and both were dead ends:
///
/// * **H1** — the router returned `Routes.starting` on every path through the
///   web branch, including "the attempt finished and failed", so a browser the
///   server declined to mint for waited on the brand mark forever. Declining is
///   *expected*: behind any reverse proxy, over the per-peer rate limit, over
///   the outstanding ceiling, or with no peer address to bind to.
/// * **H2** — the nonce was cleared in a `finally`, so a 500 or a dropped
///   connection threw away a still-live credential. The only way to get another
///   is a fresh document, so one bad second wedged the tab.
void main() {
  setUp(() {
    SharedPreferences.setMockInitialValues({});
    FlutterSecureStorage.setMockInitialValues({});
  });

  group('a refusal only counts as spent when the server says so', () {
    test('consumed, expired and wrong-peer are terminal', () {
      // The three the server uses to mean "this nonce will never work again":
      // 409 pairing_consumed, 410 pairing_expired, 403 pairing_wrong_peer.
      expect(webNonceIsSpent(409), isTrue);
      expect(webNonceIsSpent(410), isTrue);
      expect(webNonceIsSpent(403), isTrue);
    });

    test('a rate limit or a server fault leaves the nonce alive', () {
      // **The H2 case.** These say "not now", not "not ever" — discarding the
      // nonce here is what turned a transient failure into a permanently
      // un-bootstrappable tab.
      expect(webNonceIsSpent(429), isFalse);
      expect(webNonceIsSpent(500), isFalse);
      expect(webNonceIsSpent(502), isFalse);
      expect(webNonceIsSpent(503), isFalse);
      // A 401 here would be a bug rather than a spent nonce — `/v1/pair` is
      // the `none` tier and has no credential to reject.
      expect(webNonceIsSpent(401), isFalse);
    });
  });

  group('a bootstrap that produces nothing stops the router waiting', () {
    test('bootstrapFailed is false until an attempt has actually run', () async {
      final container = ProviderContainer();
      addTearDown(container.dispose);
      await container.read(settingsProvider.future);

      expect(
        container.read(settingsProvider.notifier).bootstrapFailed,
        isFalse,
        reason:
            'nothing has been tried yet, so the router should still be willing '
            'to wait rather than falling through to pairing',
      );
    });

    test('no nonce means failed, so the router can fall through', () async {
      // Off the web `readWebBootstrapNonce()` is the stub and returns null —
      // the same answer a browser gets when the server declines to mint one,
      // which is the case H1 stranded.
      final container = ProviderContainer();
      addTearDown(container.dispose);
      await container.read(settingsProvider.future);
      final notifier = container.read(settingsProvider.notifier);

      final paired = await notifier.bootstrapWebDevice();

      expect(paired, isFalse);
      expect(
        notifier.bootstrapFailed,
        isTrue,
        reason:
            'without this the redirect has no way to tell "not yet" from '
            '"never on this page load", and holds on /starting forever',
      );
      expect(
        notifier.bootstrapping,
        isFalse,
        reason: 'and it must not still claim to be in flight',
      );
    });

    test('an already-paired device is a success, not a failure', () async {
      // A returning browser short-circuits on `isPaired`. That is the *good*
      // outcome and must not set the failed flag, or the router would send a
      // perfectly good device to the pairing screen.
      SharedPreferences.setMockInitialValues({
        'storm.baseUrl': 'http://server',
      });
      FlutterSecureStorage.setMockInitialValues({
        'storm.deviceSecret': 'dvs_x',
      });
      final container = ProviderContainer();
      addTearDown(container.dispose);
      await container.read(settingsProvider.future);
      final notifier = container.read(settingsProvider.notifier);
      await notifier.save(
        container.read(settingsProvider).value!.copyWith(deviceId: 'dev_1'),
      );

      expect(await notifier.bootstrapWebDevice(), isTrue);
      expect(notifier.bootstrapFailed, isFalse);
    });
  });
}
