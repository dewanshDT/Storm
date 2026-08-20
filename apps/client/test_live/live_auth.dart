import 'dart:io';

import 'package:storm/api/auth_api.dart';
import 'package:storm/api/auth_models.dart';

/// Getting a real session, for the live suites that used to send a shared
/// token.
///
/// Before the cutover both suites authenticated with `testtoken`, so neither
/// exercised authentication at all. There is no such credential now, so each
/// one has to walk the path a real client walks: read the bootstrap pairing
/// nonce out of the server's own log, pair, create the first account, log in.
///
/// One implementation, shared — three copies of a bootstrap sequence is three
/// things to update when the flow changes.
///
/// The bootstrap nonce is minted at boot **only when the user table is empty**,
/// so anything using this needs a fresh server. `make test-live` gives each
/// suite one.
class LiveSession {
  const LiveSession({
    required this.accessToken,
    required this.deviceId,
    required this.deviceSecret,
    required this.userId,
  });

  final String accessToken;
  final String deviceId;
  final String deviceSecret;
  final String userId;
}

const _password = 'correct horse battery staple';

/// Pairs a device against `baseUrl` and signs in, returning the session.
///
/// [logPath] is the server's own log — the only place the bootstrap nonce
/// appears, which is the point: claiming a fresh server costs console access
/// (A8).
Future<LiveSession> signIn({
  required String baseUrl,
  required String logPath,
  String username = 'live',
}) async {
  final log = await File(logPath).readAsString();
  final uri = RegExp(r'storm://pair\?\S+').firstMatch(log)?.group(0);
  if (uri == null) {
    throw StateError(
      'no bootstrap pairing URI in $logPath — this suite needs a server whose '
      'user table was empty at boot',
    );
  }
  final pairing = PairingUri.parse(uri.replaceAll('"', ''));
  if (pairing == null) throw StateError('unparseable pairing URI: $uri');

  final auth = AuthApi(baseUrl: baseUrl);
  try {
    final paired = await auth.pair(
      nonce: pairing.nonce,
      deviceName: 'live test',
      platform: 'test',
    );

    // The first account, if this server has none. A 409 means someone got
    // there first, which is fine — the login below is what matters.
    try {
      await auth.createFirstUser(
        username: username,
        password: _password,
        deviceId: paired.deviceId,
        deviceSecret: paired.deviceSecret,
      );
    } on AuthApiException catch (e) {
      if (e.statusCode != 409) rethrow;
    }

    final session = await auth.login(
      deviceId: paired.deviceId,
      deviceSecret: paired.deviceSecret,
      username: username,
      password: _password,
    );
    return LiveSession(
      accessToken: session.accessToken,
      deviceId: paired.deviceId,
      deviceSecret: paired.deviceSecret,
      userId: session.userId,
    );
  } finally {
    auth.dispose();
  }
}
