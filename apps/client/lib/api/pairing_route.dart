/// How a device with no session reaches its server: the QR's or the saved
/// address, or — when that does not answer — one of the server's relays
/// (decisions 96 and 111).
///
/// Pairing, sign-in and session refresh run the same [StormConnection] race
/// the app uses afterwards, so there is one ordering rule (direct and
/// self-hosted first, a public relay only on failure) and one identity gate.
/// **Nothing sent before the challenge against the pinned key has passed is
/// sensitive:** the race sends `HELLO {server_id}` and `GET /v1/server`, the
/// challenge sends a fresh random nonce, and only a [ServerRoute] — which
/// exists only after [ServerProof.verified] — carries the pairing nonce, the
/// device secret, a refresh token or a password.
library;

import 'dart:async';

import 'auth_api.dart';
import 'auth_models.dart';
import 'relay/srp_trunk.dart';
import 'server_verifier.dart';
import 'storm_connection.dart';

/// Why a pairing QR's server could not be reached and proven.
enum PairingFailure {
  /// Nothing answered at the QR's address, and it names no usable relay.
  serverUnreachable,

  /// Neither the QR's address nor any of its relays answered.
  relayUnreachable,

  /// A relay answered, but the server is not connected to it.
  serverNotOnRelay,

  /// Something answered and could not prove it holds the QR's key.
  identityFailed,

  /// The server proved its key but describes itself differently from the QR.
  identityMismatch,
}

class PairingRouteException implements Exception {
  const PairingRouteException(this.failure, {this.relayHost});

  final PairingFailure failure;

  /// The relay involved, for the message — null when none was.
  final String? relayHost;

  @override
  String toString() =>
      'PairingRouteException(${failure.name}${relayHost == null ? '' : ' via $relayHost'})';
}

/// What a relay said when dialled on its own, to explain a failed race.
enum RelayReach {
  /// It completed the handshake: the server is connected to it.
  up,

  /// It answered `ERROR{server_unreachable}`: up, but the server is not there.
  serverAbsent,

  /// It did not answer at all.
  down,
}

/// A transport to the server that has passed the identity challenge.
class ServerRoute {
  ServerRoute._(this._connection);

  final StormConnection _connection;

  /// Device-tier and unauthenticated calls, over the same transport the
  /// challenge was answered on.
  AuthApi get authApi => _connection.authApi;

  bool get viaRelay => _connection.isRelayed;

  /// The relay carrying the route, or null when it is direct.
  String? get relayHost => _hostOf(_connection.active.relayUrl);

  /// Releases the relay trunk, if any. The app's own connection re-races.
  void close() => _connection.reset();
}

/// A [ServerRoute] opened from a pairing QR.
class PairingRoute extends ServerRoute {
  PairingRoute._(super.connection, this.serverInfo) : super._();

  /// `GET /v1/server`, fetched over the proven transport.
  final ServerInfo serverInfo;
}

/// The relays a route may send credentials through.
///
/// The challenge proves *who* answers; it does nothing for confidentiality. A
/// password crossing the internet over a plain `ws://` relay would be readable
/// by anyone on the path, so only `wss://` relays — and loopback ones, which
/// never leave the machine — are used.
List<RelayAdvert> confidentialRelays(List<RelayAdvert> relays) => [
  for (final relay in relays)
    if (_isConfidential(relay.url)) relay,
];

List<RelayAdvert> pairingRelays(PairingUri uri) =>
    confidentialRelays(uri.relayAdverts);

bool _isConfidential(String relayUrl) {
  final u = Uri.tryParse(relayUrl);
  if (u == null) return false;
  if (u.scheme == 'wss') return true;
  return u.host == 'localhost' || u.host == '127.0.0.1' || u.host == '::1';
}

String? _hostOf(String? url) {
  if (url == null) return null;
  final host = Uri.tryParse(url)?.host ?? '';
  return host.isEmpty ? null : host;
}

/// Dials [relay] alone and reports whether it, and the server behind it, are
/// there. Sends only `HELLO {server_id}`.
Future<RelayReach> probeRelay(RelayAdvert relay, String serverId) async {
  final trunk = SrpTrunk(
    url: Uri.parse(relay.publicAddress),
    serverId: serverId,
    // Longer than the relay's own hold for a server mid-restart (§5), so its
    // `server_unreachable` arrives before this gives up.
    handshakeTimeout: const Duration(seconds: 8),
  );
  try {
    await trunk.connect();
    return RelayReach.up;
  } on SrpHandshakeFailed catch (e) {
    return e.code == 'server_unreachable'
        ? RelayReach.serverAbsent
        : RelayReach.down;
  } catch (_) {
    return RelayReach.down;
  } finally {
    trunk.dispose();
  }
}

/// Races the QR's address against its relays and proves the winner.
///
/// Throws [PairingRouteException] for anything a person has to act on, and
/// [ArgumentError] for a QR address that cannot be a URL.
Future<PairingRoute> openPairingRoute(
  PairingUri uri, {
  Set<String> publicRelayHosts = kPublicRelayHosts,
  AuthApi Function(ConnectionCandidate)? buildAuthApi,
  Future<RelayReach> Function(RelayAdvert relay)? probe,
  Duration raceTimeout = const Duration(seconds: 10),
  Duration deadline = const Duration(seconds: 30),
}) async {
  if (Uri.parse(uri.baseUrl).host.isEmpty) {
    throw ArgumentError('no host in "${uri.address}"');
  }
  final connection = await _openProven(
    baseUrl: uri.baseUrl,
    serverId: uri.serverId,
    publicKey: uri.publicKey,
    relays: pairingRelays(uri),
    publicRelayHosts: publicRelayHosts,
    buildAuthApi: buildAuthApi,
    probe: probe,
    raceTimeout: raceTimeout,
    deadline: deadline,
  );
  final ServerInfo info;
  try {
    info = await connection.authApi.serverInfo();
  } catch (_) {
    connection.reset();
    rethrow;
  }
  if (info.serverId != uri.serverId || info.publicKey != uri.publicKey) {
    final host = _hostOf(connection.active.relayUrl);
    connection.reset();
    throw PairingRouteException(
      PairingFailure.identityMismatch,
      relayHost: host,
    );
  }
  return PairingRoute._(connection, info);
}

/// Races a paired device's saved address against its saved relays and proves
/// the winner against the identity pinned at pairing. Sign-in and session
/// refresh go through this, so a device away from the server's network can
/// still reach it — and never hand a password to anything that cannot sign.
///
/// Throws [PairingRouteException] for anything a person has to act on.
Future<ServerRoute> openSavedRoute({
  required String baseUrl,
  required String serverId,
  required String publicKey,
  required List<RelayAdvert> relays,
  Set<String> publicRelayHosts = kPublicRelayHosts,
  AuthApi Function(ConnectionCandidate)? buildAuthApi,
  Future<RelayReach> Function(RelayAdvert relay)? probe,
  Duration raceTimeout = const Duration(seconds: 10),
  Duration deadline = const Duration(seconds: 30),
}) async => ServerRoute._(
  await _openProven(
    baseUrl: baseUrl,
    serverId: serverId,
    publicKey: publicKey,
    relays: confidentialRelays(relays),
    publicRelayHosts: publicRelayHosts,
    buildAuthApi: buildAuthApi,
    probe: probe,
    raceTimeout: raceTimeout,
    deadline: deadline,
  ),
);

Future<StormConnection> _openProven({
  required String baseUrl,
  required String serverId,
  required String publicKey,
  required List<RelayAdvert> relays,
  required Set<String> publicRelayHosts,
  required AuthApi Function(ConnectionCandidate)? buildAuthApi,
  required Future<RelayReach> Function(RelayAdvert relay)? probe,
  required Duration raceTimeout,
  required Duration deadline,
}) async {
  final connection = StormConnection(
    localAddress: baseUrl,
    token: '',
    serverId: serverId,
    publicKeyB64: publicKey,
    relays: relays,
    publicRelayHosts: publicRelayHosts,
    buildAuthApi: buildAuthApi,
    // One deliberate attempt, often over a phone's mobile data: a relay's
    // TLS, handshake and first exchange get longer than a reconnect.
    publicRelayDelay: raceTimeout,
  );

  final ServerProof proof;
  try {
    proof = await connection.connect().timeout(
      deadline,
      onTimeout: () => ServerProof.unreachable,
    );
  } catch (_) {
    connection.reset();
    rethrow;
  }

  switch (proof) {
    case ServerProof.verified:
      return connection;
    case ServerProof.impostor:
      final host = _hostOf(connection.active.relayUrl);
      connection.reset();
      throw PairingRouteException(
        PairingFailure.identityFailed,
        relayHost: host,
      );
    case ServerProof.unreachable:
      connection.reset();
      if (relays.isEmpty) {
        throw const PairingRouteException(PairingFailure.serverUnreachable);
      }
      final reach = await Future.wait([
        for (final relay in relays)
          (probe ?? (r) => probeRelay(r, serverId))(relay),
      ]);
      final absent = reach.indexOf(RelayReach.serverAbsent);
      if (absent >= 0) {
        throw PairingRouteException(
          PairingFailure.serverNotOnRelay,
          relayHost: _hostOf(relays[absent].url),
        );
      }
      final up = reach.indexOf(RelayReach.up);
      if (up >= 0) {
        // The relay has the server, yet the exchange failed: the server is
        // what did not answer.
        throw PairingRouteException(
          PairingFailure.serverUnreachable,
          relayHost: _hostOf(relays[up].url),
        );
      }
      throw PairingRouteException(
        PairingFailure.relayUnreachable,
        relayHost: _hostOf(relays.first.url),
      );
  }
}
