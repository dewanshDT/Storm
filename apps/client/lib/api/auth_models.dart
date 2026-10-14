/// Wire types for the auth protocol (server identity, pairing, sessions).
///
/// Same philosophy as `models.dart`: these mirror the server's JSON exactly.
library;

/// One relay the server is **registered with** — not merely configured.
///
/// The server drops relays it failed to register with before answering, so
/// every entry here is a path worth dialling. A configured-but-dead one would
/// spend the connection race's whole budget going nowhere.
class RelayAdvert {
  const RelayAdvert({required this.url, required this.publicAddress});

  /// The relay itself, e.g. `wss://relay.example.com`.
  final String url;

  /// Where *this* server answers on it —
  /// `wss://<relay-host>/connect/<server_id>`. Derived, never allocated.
  final String publicAddress;

  factory RelayAdvert.fromJson(Map<String, dynamic> j) => RelayAdvert(
    url: j['url'] as String,
    publicAddress: j['public_address'] as String,
  );

  /// The advert for [url], deriving `public_address` exactly as the server's
  /// `registry::public_address` does.
  factory RelayAdvert.derive(String url, String serverId) {
    final base = url.replaceFirst(RegExp(r'/+$'), '');
    return RelayAdvert(url: base, publicAddress: '$base/connect/$serverId');
  }
}

/// Server identity, from `GET /v1/server`.
class ServerInfo {
  const ServerInfo({
    required this.serverId,
    required this.keyId,
    required this.algorithm,
    required this.publicKey,
    this.relays = const [],
    this.relaysSig,
  });

  final String serverId;
  final String keyId;
  final String algorithm;
  final String publicKey;

  /// The live registered-relay set. Empty on a server with no tunnel client,
  /// and **absent entirely** on one predating the field — hence the default
  /// rather than a required key, since this endpoint is the one thing a
  /// stranded client can still reach.
  final List<RelayAdvert> relays;

  /// The server's signature over [relays]' URLs (§4.3); absent from a server
  /// that predates it.
  final String? relaysSig;

  factory ServerInfo.fromJson(Map<String, dynamic> j) => ServerInfo(
    serverId: j['server_id'] as String,
    keyId: j['key_id'] as String,
    algorithm: j['algorithm'] as String,
    publicKey: j['public_key'] as String,
    relays:
        (j['relays'] as List?)
            ?.map((r) => RelayAdvert.fromJson(r as Map<String, dynamic>))
            .toList() ??
        const [],
    relaysSig: j['relays_sig'] as String?,
  );
}

/// Challenge signature, from `POST /v1/server/challenge`.
class ChallengeAnswer {
  const ChallengeAnswer({required this.signature});

  final String signature;

  factory ChallengeAnswer.fromJson(Map<String, dynamic> j) =>
      ChallengeAnswer(signature: j['signature'] as String);
}

/// Result of consuming a pairing nonce, from `POST /v1/pair`.
class PairingResult {
  const PairingResult({
    required this.deviceId,
    required this.deviceSecret,
    required this.serverId,
    required this.publicKey,
    required this.keyId,
  });

  final String deviceId;
  final String deviceSecret;
  final String serverId;
  final String publicKey;
  final String keyId;

  factory PairingResult.fromJson(Map<String, dynamic> j) => PairingResult(
    deviceId: j['device_id'] as String,
    deviceSecret: j['device_secret'] as String,
    serverId: j['server_id'] as String,
    publicKey: j['public_key'] as String,
    keyId: j['key_id'] as String,
  );
}

/// A pairing invite minted by `POST /v1/pairings`, for a new device.
///
/// The mirror image of [PairingUri]: the server hands back the same five
/// fields, and [toUri] renders them into the string a new device consumes.
/// **The format is shared with the server's `encode_qr`** — the two are a wire
/// commitment, so a field added on one side and not the other produces a URI
/// its own client cannot read.
class PairingInvite {
  const PairingInvite({
    required this.serverId,
    required this.publicKey,
    required this.nonce,
    required this.expires,
    required this.address,
    this.relays = const [],
  });

  final String serverId;
  final String publicKey;
  final String nonce;
  final String expires;
  final String address;

  /// Relay URLs the server was registered with when it minted this invite.
  final List<String> relays;

  factory PairingInvite.fromJson(Map<String, dynamic> j) => PairingInvite(
    serverId: j['sid'] as String,
    publicKey: j['pk'] as String,
    nonce: j['n'] as String,
    expires: j['exp'] as String,
    address: j['addr'] as String,
    relays: [for (final r in (j['relays'] as List? ?? const [])) r as String],
  );

  /// The `storm://pair` URI a new device scans or pastes.
  String toUri() =>
      'storm://pair?v=1&sid=$serverId&pk=$publicKey&n=$nonce'
      '&exp=$expires&addr=$address'
      '${relays.map((r) => '&relay=${Uri.encodeQueryComponent(r)}').join()}';
}

/// A parsed `storm://pair` URI.
class PairingUri {
  const PairingUri({
    required this.version,
    required this.serverId,
    required this.publicKey,
    required this.nonce,
    required this.expires,
    required this.address,
    this.relays = const [],
  });

  final String version;
  final String serverId;
  final String publicKey;
  final String nonce;
  final String expires;

  /// The `addr` field exactly as the QR carries it: a bare `host:port`.
  final String address;

  /// The `relay` keys: the relays the server was registered with when the QR
  /// was issued. Optional — a QR from an older server, or one with no relays,
  /// has none. A relay URL here grants no trust: whatever answers through it
  /// still has to pass the challenge against [publicKey].
  final List<String> relays;

  /// [relays] as connection candidates, seeding `Settings.relays` until the
  /// first `GET /v1/server` replaces them.
  List<RelayAdvert> get relayAdverts => [
    for (final r in relays) RelayAdvert.derive(r, serverId),
  ];

  /// [address] as something an HTTP client can actually be given.
  ///
  /// **A bare `host:port` is not a URL.** `Uri.parse('192.168.91.51:8585/…')`
  /// reads `192.168.91.51` as the *scheme*, and a scheme may not start with a
  /// digit, so it throws `Scheme not starting with alphabetic character` — the
  /// error a real phone showed after scanning a perfectly good code. Every
  /// caller was passing `address` straight in as `baseUrl`, including the one
  /// that persists it into `Settings`, so this broke the whole app after
  /// pairing and not merely the pairing screen.
  ///
  /// The live tests missed it because the harness hands them a full
  /// `http://host:port` and they never read this field — the same blind spot
  /// that hid the wildcard bind address.
  String get baseUrl =>
      address.startsWith('http://') || address.startsWith('https://')
      ? address
      : 'http://$address';

  /// Parses `storm://pair?v=1&sid=...&pk=...&n=...&exp=...&addr=...`.
  ///
  /// Returns `null` on any parse failure rather than throwing, because the
  /// input comes from a QR scan or paste and is expected to be malformed
  /// sometimes.
  ///
  /// **Whitespace is stripped first.** This URI is copied out of a terminal by
  /// hand, and a wrapped line or a phone keyboard helpfully breaking a long
  /// token puts a space inside it — one real paste arrived with `&a ddr=`,
  /// which silently produced an empty address. A URI cannot contain raw
  /// whitespace, so removing it can only ever repair the input.
  ///
  /// **Every field must be present and non-empty.** Defaulting a missing key
  /// to `''` is what made that paste fail three screens later as "couldn't
  /// reach the server" instead of "this URI is incomplete": an empty `addr`
  /// became an empty base URL, and the first request had nowhere to go.
  static PairingUri? parse(String uri) {
    try {
      final url = Uri.parse(uri.replaceAll(RegExp(r'\s+'), ''));
      if (url.scheme != 'storm' || url.host != 'pair') return null;
      final q = url.queryParameters;
      final parsed = PairingUri(
        version: q['v'] ?? '',
        serverId: q['sid'] ?? '',
        publicKey: q['pk'] ?? '',
        nonce: q['n'] ?? '',
        expires: q['exp'] ?? '',
        address: q['addr'] ?? '',
        // A malformed relay is dropped, not fatal: the LAN address is what
        // pairing dials, and a relay is only ever an extra candidate.
        relays: [
          for (final r in url.queryParametersAll['relay'] ?? const <String>[])
            if (_isRelayUrl(r)) r,
        ],
      );
      if (parsed.version.isEmpty ||
          parsed.serverId.isEmpty ||
          parsed.publicKey.isEmpty ||
          parsed.nonce.isEmpty ||
          parsed.expires.isEmpty ||
          parsed.address.isEmpty) {
        return null;
      }
      // The address is dialled as a base URL, so it has to carry a host. A
      // bare port or a stray fragment gets caught here rather than at the
      // first request.
      if (Uri.tryParse('http://${parsed.address}')?.host.isEmpty ?? true) {
        return null;
      }
      return parsed;
    } catch (_) {
      return null;
    }
  }

  /// `ws://` or `wss://` and a host, nothing else — the server's
  /// `normalize_relay_url` rule.
  static bool _isRelayUrl(String r) {
    final u = Uri.tryParse(r);
    return u != null &&
        (u.scheme == 'wss' || u.scheme == 'ws') &&
        u.host.isNotEmpty &&
        (u.path.isEmpty || u.path == '/') &&
        !u.hasQuery &&
        !u.hasFragment;
  }

  bool get isExpired {
    try {
      final exp = DateTime.parse(expires).toUtc();
      return DateTime.now().toUtc().isAfter(exp);
    } catch (_) {
      return true;
    }
  }
}

/// The relay set a newly paired device starts with (D3): what `GET /v1/server`
/// reported during pairing — live, from the same server that then passed the
/// challenge — or, when that names none, the QR's snapshot. Either way it is
/// only a set of candidates for the connection race to try.
List<RelayAdvert> pairedRelays(PairingUri uri, ServerInfo info) =>
    info.relays.isNotEmpty ? info.relays : uri.relayAdverts;

/// Session tokens, from `POST /v1/auth/login` or `POST /v1/auth/refresh`.
class SessionTokens {
  const SessionTokens({
    required this.accessToken,
    required this.refreshToken,
    required this.expires,
    required this.refreshExpires,
    required this.userId,
    required this.deviceId,
  });

  final String accessToken;
  final String refreshToken;

  /// When the access token dies — an **absolute RFC3339 instant**, the wire
  /// contract in *Storm Auth Protocol* and the `sessions` table's own `expires`
  /// column projected unchanged.
  ///
  /// This used to be `accessExpiresIn`, an integer count of seconds read from
  /// `access_expires_in` — a field no server has ever sent and no note in the
  /// vault ever described. `fromJson` threw `Null is not a subtype of num` on
  /// every real login, and the mock-backed tests could not see it because they
  /// were built to the same wrong assumption.
  final String expires;

  /// When the refresh token dies. Absolute RFC3339, as [expires].
  final String refreshExpires;
  final String userId;
  final String deviceId;

  factory SessionTokens.fromJson(Map<String, dynamic> j) => SessionTokens(
    accessToken: j['access_token'] as String,
    refreshToken: j['refresh_token'] as String,
    expires: j['expires'] as String,
    refreshExpires: j['refresh_expires'] as String,
    userId: j['user_id'] as String,
    deviceId: j['device_id'] as String,
  );
}
