import 'dart:io';

import 'package:flutter_test/flutter_test.dart';

import 'package:storm/api/auth_api.dart';
import 'package:storm/api/storm_api.dart';

/// The client's device-tier path, against a real server.
///
/// Everything else in `test_live/` authenticates with the legacy shared token,
/// which routes around the entire auth stack: `require_auth` takes the legacy
/// branch and no device credential, session token or tier check is ever
/// exercised. That is how the device tier reached slice 12 with a deadlock in
/// it that hung every request, and how `AuthApi.refresh` shipped without an
/// `Authorization` header at all.
///
/// So this walks the real thing, in order:
///
///   pair → device credential → users → first user → login → session →
///   authenticated vault request → refresh → logout → rejected afterwards
///
/// It needs **its own server**, because the bootstrap pairing nonce and the
/// first-user window are each once per server and `auth_e2e.py` has already
/// spent both on the main one. `make test-live` starts it on `AUTH_PORT` and
/// passes the address and log path in; without them the test is skipped rather
/// than failing, so a bare `flutter test test_live/` still works.
void main() {
  const base = String.fromEnvironment('STORM_AUTH_BASE');
  const logPath = String.fromEnvironment('STORM_AUTH_LOG');

  if (base.isEmpty || logPath.isEmpty) {
    test('device-tier auth flow (skipped: no dedicated auth server)', () {
      markTestSkipped(
        'Set STORM_AUTH_BASE and STORM_AUTH_LOG, or run `make test-live`.',
      );
    });
    return;
  }

  test('pair, log in, use the session, refresh, log out', () async {
    final auth = AuthApi(baseUrl: base);
    addTearDown(auth.dispose);

    // ---- the bootstrap QR -------------------------------------------------
    //
    // A fresh server logs a `storm://pair` URI at boot. This is the operator
    // reading it off the console, which is exactly what the first device does.
    final log = await File(logPath).readAsString();
    final uri = RegExp(r'storm://pair\?\S+').firstMatch(log)?.group(0);
    expect(uri, isNotNull, reason: 'no bootstrap pairing URI in $logPath');

    final nonce = Uri.parse(uri!.replaceFirst('storm://', 'https://'))
        .queryParameters['n'];
    expect(nonce, isNotNull);

    // The identity in the QR is what the client pins, so it must match what
    // the server publishes about itself.
    final info = await auth.serverInfo();
    final qr = Uri.parse(uri.replaceFirst('storm://', 'https://'));
    expect(qr.queryParameters['sid'], info.serverId);
    expect(qr.queryParameters['pk'], info.publicKey);

    // ---- pair -------------------------------------------------------------
    final paired = await auth.pair(
      nonce: nonce!,
      deviceName: 'integration test',
      platform: 'linux',
    );
    expect(paired.deviceId, isNotEmpty);
    expect(paired.deviceSecret, isNotEmpty);
    expect(paired.serverId, info.serverId);

    // ---- the device tier --------------------------------------------------
    final users = await auth.listUsers(
      deviceId: paired.deviceId,
      deviceSecret: paired.deviceSecret,
    );
    expect(users, isEmpty, reason: 'a fresh server has no accounts');

    const username = 'dewansh';
    const password = 'a-long-enough-password';

    // Device tier too — this call used to send no credential and 401.
    await auth.createFirstUser(
      username: username,
      password: password,
      deviceId: paired.deviceId,
      deviceSecret: paired.deviceSecret,
    );

    final after = await auth.listUsers(
      deviceId: paired.deviceId,
      deviceSecret: paired.deviceSecret,
    );
    expect(after.map((u) => u.username), contains(username));

    // ---- login ------------------------------------------------------------
    final session = await auth.login(
      deviceId: paired.deviceId,
      deviceSecret: paired.deviceSecret,
      username: username,
      password: password,
    );
    expect(session.accessToken, isNotEmpty);
    expect(session.refreshToken, isNotEmpty);
    expect(session.deviceId, paired.deviceId);
    // Absolute RFC3339, per Auth Protocol. Parsing it is the assertion: the
    // client used to read a seconds count from a field no server sends.
    expect(DateTime.parse(session.expires).isAfter(DateTime.now()), isTrue);
    expect(DateTime.parse(session.refreshExpires).isAfter(DateTime.now()), isTrue);

    // ---- an authenticated vault request -----------------------------------
    final api = StormApi(baseUrl: base, token: session.accessToken);
    addTearDown(api.dispose);
    final vaults = await api.vaults();
    expect(vaults, isNotEmpty, reason: 'the harness seeds one vault');
    final tree = await api.tree(vaults.first.id);
    expect(tree, isNotNull);

    // ---- refresh ----------------------------------------------------------
    //
    // Device tier, like login. This call used to send no credential at all and
    // was refused by every real server.
    final rotated = await auth.refresh(
      session.refreshToken,
      deviceId: paired.deviceId,
      deviceSecret: paired.deviceSecret,
    );
    expect(rotated.accessToken, isNot(session.accessToken));
    expect(rotated.refreshToken, isNot(session.refreshToken));

    final rotatedApi = StormApi(baseUrl: base, token: rotated.accessToken);
    addTearDown(rotatedApi.dispose);
    expect(await rotatedApi.vaults(), isNotEmpty);

    // ---- logout, and the door actually shuts ------------------------------
    await rotatedApi.logout();

    final dead = StormApi(baseUrl: base, token: rotated.accessToken);
    addTearDown(dead.dispose);
    await expectLater(
      dead.vaults(),
      throwsA(anything),
      reason: 'the access token must be dead after logout',
    );
  });
}
