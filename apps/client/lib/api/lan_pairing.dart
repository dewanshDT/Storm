/// Pairing with a server found on the LAN, by the account password
/// (decision 118).
///
/// The order is the security: the advertised key is pinned and the identity
/// challenge runs over the advertised **direct** address before the password
/// exists anywhere but this device. There is no relay candidate, so the
/// password can never cross one; the server refuses it through a relay too.
library;

import 'auth_api.dart';
import 'auth_models.dart';
import 'lan_discovery.dart';
import 'pairing_route.dart';
import 'storm_connection.dart';

/// A direct route to a discovered server that has proved it holds the
/// advertised key.
class LanRoute {
  LanRoute._(this._route, this.server, this.serverInfo);

  final ServerRoute _route;
  final DiscoveredServer server;

  /// `GET /v1/server`, read after the identity verified, for the relays.
  final ServerInfo serverInfo;

  AuthApi get authApi => _route.authApi;

  void close() => _route.close();
}

/// Runs the identity challenge against the advertised key over the
/// advertised address, and nothing else. Throws [PairingRouteException]:
/// `identityFailed` for a server that could not sign, `identityMismatch` for
/// one that describes itself differently from its advertisement.
Future<LanRoute> openLanRoute(
  DiscoveredServer server, {
  AuthApi Function(ConnectionCandidate)? buildAuthApi,
  Duration deadline = const Duration(seconds: 10),
}) async {
  final route = await openSavedRoute(
    baseUrl: server.baseUrl,
    serverId: server.serverId,
    publicKey: server.publicKey,
    relays: const [],
    publicRelayHosts: const {},
    buildAuthApi: buildAuthApi,
    candidateDeadline: deadline,
    deadline: deadline,
  );
  final ServerInfo info;
  try {
    info = await route.authApi.serverInfo();
  } catch (_) {
    route.close();
    rethrow;
  }
  if (info.serverId != server.serverId || info.publicKey != server.publicKey) {
    route.close();
    throw const PairingRouteException(PairingFailure.identityMismatch);
  }
  return LanRoute._(route, server, info);
}

/// What pairing produced: the device credential, and a session when the
/// sign-in that follows it worked.
class LanPairing {
  const LanPairing({required this.device, this.session});

  final PairingResult device;
  final SessionTokens? session;
}

/// Pairs over a verified [route], then signs in with the same password so the
/// person is not asked twice. A failed sign-in after a successful pairing
/// still returns the device: the router sends it to `/login`.
Future<LanPairing> pairOverLan(
  LanRoute route, {
  required String password,
  required String deviceName,
  String? platform,
  String? version,
}) async {
  final device = await route.authApi.pairWithPassword(
    password: password,
    deviceName: deviceName,
    platform: platform,
    version: version,
  );
  SessionTokens? session;
  try {
    session = await route.authApi.login(
      deviceId: device.deviceId,
      deviceSecret: device.deviceSecret,
      password: password,
    );
  } catch (_) {
    session = null;
  }
  return LanPairing(device: device, session: session);
}

/// The sheet's message for a refused pairing, or null when it is not one the
/// sheet words itself (`setup_required` switches the sheet instead).
String lanPairingError(AuthApiException e) {
  if (e.isRateLimited) {
    final wait = e.retryAfter;
    return wait == null
        ? 'Too many attempts, try again shortly.'
        : 'Too many attempts, try again in $wait s.';
  }
  if (e.message == 'lan_only') {
    return 'Pairing by password works only on the same network as the server.';
  }
  return authFailureMessage(e);
}

bool isSetupRequired(AuthApiException e) =>
    e.statusCode == 409 && e.message == 'setup_required';
