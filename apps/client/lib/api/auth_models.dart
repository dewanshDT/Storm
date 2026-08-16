/// Wire types for the auth protocol (server identity, pairing, sessions).
///
/// Same philosophy as `models.dart`: these mirror the server's JSON exactly.
library;

/// Server identity, from `GET /v1/server`.
class ServerInfo {
  const ServerInfo({
    required this.serverId,
    required this.keyId,
    required this.algorithm,
    required this.publicKey,
  });

  final String serverId;
  final String keyId;
  final String algorithm;
  final String publicKey;

  factory ServerInfo.fromJson(Map<String, dynamic> j) => ServerInfo(
    serverId: j['server_id'] as String,
    keyId: j['key_id'] as String,
    algorithm: j['algorithm'] as String,
    publicKey: j['public_key'] as String,
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

/// A parsed `storm://pair` URI.
class PairingUri {
  const PairingUri({
    required this.version,
    required this.serverId,
    required this.publicKey,
    required this.nonce,
    required this.expires,
    required this.address,
  });

  final String version;
  final String serverId;
  final String publicKey;
  final String nonce;
  final String expires;
  final String address;

  /// Parses `storm://pair?v=1&sid=...&pk=...&n=...&exp=...&addr=...`.
  ///
  /// Returns `null` on any parse failure rather than throwing, because the
  /// input comes from a QR scan or paste and is expected to be malformed
  /// sometimes.
  static PairingUri? parse(String uri) {
    try {
      final url = Uri.parse(uri);
      if (url.scheme != 'storm' || url.host != 'pair') return null;
      final q = url.queryParameters;
      return PairingUri(
        version: q['v'] ?? '',
        serverId: q['sid'] ?? '',
        publicKey: q['pk'] ?? '',
        nonce: q['n'] ?? '',
        expires: q['exp'] ?? '',
        address: q['addr'] ?? '',
      );
    } catch (_) {
      return null;
    }
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

/// Session tokens, from `POST /v1/auth/login` or `POST /v1/auth/refresh`.
class SessionTokens {
  const SessionTokens({
    required this.accessToken,
    required this.refreshToken,
    required this.accessExpiresIn,
    required this.refreshExpiresIn,
    required this.userId,
    required this.deviceId,
  });

  final String accessToken;
  final String refreshToken;
  final int accessExpiresIn;
  final int refreshExpiresIn;
  final String userId;
  final String deviceId;

  factory SessionTokens.fromJson(Map<String, dynamic> j) => SessionTokens(
    accessToken: j['access_token'] as String,
    refreshToken: j['refresh_token'] as String,
    accessExpiresIn: (j['access_expires_in'] as num).toInt(),
    refreshExpiresIn: (j['refresh_expires_in'] as num).toInt(),
    userId: j['user_id'] as String,
    deviceId: j['device_id'] as String,
  );
}

/// An account on the server, as the login picker sees it.
///
/// Behind device auth (A7/A8): a stranger on the LAN cannot enumerate account
/// names, but a paired device can offer a list instead of demanding you
/// remember a username.
class AuthUser {
  const AuthUser({
    required this.id,
    required this.username,
    required this.displayName,
    required this.role,
    required this.status,
  });

  final String id;
  final String username;

  /// Unrestricted, unlike [username], which is ASCII-only.
  final String? displayName;
  final String role;
  final String status;

  /// What to show in the picker. Falls back to the username, which always
  /// exists.
  String get label => (displayName != null && displayName!.isNotEmpty)
      ? displayName!
      : username;

  /// A disabled account cannot log in, so the picker shows it as unavailable
  /// rather than letting someone type a password that will always be refused.
  bool get isDisabled => status == 'disabled';

  factory AuthUser.fromJson(Map<String, dynamic> j) => AuthUser(
    id: j['id'] as String? ?? '',
    username: j['username'] as String? ?? '',
    displayName: j['display_name'] as String?,
    role: j['role'] as String? ?? 'member',
    status: j['status'] as String? ?? 'active',
  );
}
