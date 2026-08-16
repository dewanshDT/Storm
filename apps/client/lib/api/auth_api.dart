/// Unauthenticated and device-tier API calls for the auth protocol.
///
/// Split from `StormApi` because these calls do not carry a bearer token and
/// have different error semantics: a failed challenge is a security event, not
/// a "wrong password".
library;

import 'dart:convert';

import 'package:http/http.dart' as http;

import 'auth_models.dart';

/// REST client for the auth protocol endpoints.
///
/// Thin transport only — verification logic lives in `ed25519_verify.dart`,
/// credential storage in `credentials.dart`.
class AuthApi {
  AuthApi({required this.baseUrl, http.Client? client})
    : _client = client ?? http.Client();

  final String baseUrl;
  final http.Client _client;

  Uri _uri(String path) => Uri.parse('$baseUrl$path');

  dynamic _decode(http.Response r) {
    if (r.statusCode < 200 || r.statusCode >= 300) {
      String message = 'HTTP ${r.statusCode}';
      int? retryAfter;
      try {
        final body = jsonDecode(r.body);
        if (body is Map) {
          if (body['error'] != null) message = '${body['error']}';
          retryAfter = (body['retry_after'] as num?)?.toInt();
        }
      } catch (_) {}
      throw AuthApiException(r.statusCode, message, retryAfter: retryAfter);
    }
    return jsonDecode(utf8.decode(r.bodyBytes));
  }

  /// `GET /v1/server` — server identity.
  Future<ServerInfo> serverInfo() async {
    final json = _decode(await _client.get(_uri('/v1/server')));
    return ServerInfo.fromJson(json as Map<String, dynamic>);
  }

  /// `POST /v1/server/challenge` — signs a nonce with the server's Ed25519 key.
  Future<ChallengeAnswer> challenge(String nonce) async {
    final json = _decode(
      await _client.post(
        _uri('/v1/server/challenge'),
        headers: {'Content-Type': 'application/json'},
        body: jsonEncode({'nonce': nonce}),
      ),
    );
    return ChallengeAnswer.fromJson(json as Map<String, dynamic>);
  }

  /// `POST /v1/pair` — consume a pairing nonce, receive device credentials.
  Future<PairingResult> pair({
    required String nonce,
    required String deviceName,
    String? platform,
    String? version,
  }) async {
    final json = _decode(
      await _client.post(
        _uri('/v1/pair'),
        headers: {'Content-Type': 'application/json'},
        body: jsonEncode({
          'n': nonce,
          'name': deviceName,
          'platform': ?platform,
          'version': ?version,
        }),
      ),
    );
    return PairingResult.fromJson(json as Map<String, dynamic>);
  }

  /// `POST /v1/users/first` — create the owner account (unauthenticated).
  ///
  /// Only works when the user table is empty.
  Future<void> createFirstUser({
    required String username,
    required String password,
  }) async {
    final r = await _client.post(
      _uri('/v1/users/first'),
      headers: {'Content-Type': 'application/json'},
      body: jsonEncode({'username': username, 'password': password}),
    );
    if (r.statusCode < 200 || r.statusCode >= 300) {
      final body = jsonDecode(r.body);
      final error = body is Map ? '${body['error']}' : 'HTTP ${r.statusCode}';
      throw AuthApiException(r.statusCode, error);
    }
  }

  /// `GET /v1/users` — the accounts on this server, for the login picker.
  ///
  /// Device tier: paired devices may see who exists, strangers on the LAN may
  /// not (A7/A8).
  Future<List<AuthUser>> listUsers({
    required String deviceId,
    required String deviceSecret,
  }) async {
    final json = _decode(
      await _client.get(
        _uri('/v1/users'),
        headers: {'Authorization': 'StormDevice $deviceId:$deviceSecret'},
      ),
    );
    return (json as List)
        .map((u) => AuthUser.fromJson(u as Map<String, dynamic>))
        .toList();
  }

  /// `POST /v1/auth/login` — exchange device credentials + password for
  /// session tokens.
  ///
  /// The `StormDevice` header is the device-tier credential from pairing.
  Future<SessionTokens> login({
    required String deviceId,
    required String deviceSecret,
    required String username,
    required String password,
  }) async {
    final json = _decode(
      await _client.post(
        _uri('/v1/auth/login'),
        headers: {
          'Content-Type': 'application/json',
          'Authorization': 'StormDevice $deviceId:$deviceSecret',
        },
        body: jsonEncode({'username': username, 'password': password}),
      ),
    );
    return SessionTokens.fromJson(json as Map<String, dynamic>);
  }

  /// `POST /v1/auth/refresh` — exchange a refresh token for a new pair.
  Future<SessionTokens> refresh(String refreshToken) async {
    final json = _decode(
      await _client.post(
        _uri('/v1/auth/refresh'),
        headers: {'Content-Type': 'application/json'},
        body: jsonEncode({'refresh_token': refreshToken}),
      ),
    );
    return SessionTokens.fromJson(json as Map<String, dynamic>);
  }

  void dispose() => _client.close();
}

class AuthApiException implements Exception {
  AuthApiException(this.statusCode, this.message, {this.retryAfter});

  final int statusCode;

  /// The server's `error` field — a wire code from *Storm Auth Protocol*'s
  /// table (`invalid_credentials`, `device_revoked`, …) for anything the auth
  /// endpoints refuse. Pass it to [authFailureMessage] rather than showing it.
  final String message;

  /// Seconds to wait, when the server said `rate_limited`.
  final int? retryAfter;

  bool get isUnauthorized => statusCode == 401;
  bool get isConflict => statusCode == 409;
  bool get isGone => statusCode == 410;
  bool get isRateLimited => statusCode == 429;

  @override
  String toString() => 'AuthApiException($statusCode): $message';
}

/// A sentence for the person holding the phone, from the server's wire code.
///
/// Every auth refusal gets its own message, which is the point: the protocol
/// keeps these distinct so the client can say the true thing, and collapsing
/// them into one "login failed" throws that away.
///
/// **"Couldn't reach the server" is never one of these.** An auth refusal is an
/// HTTP answer, not a socket failure — reporting it as a network problem is the
/// M9/M10 bug, where a local failure was described as a network one and
/// everything after it failed as offline too.
String authFailureMessage(AuthApiException e) {
  switch (e.message) {
    case 'invalid_credentials':
      return 'Wrong username or password.';
    case 'session_expired':
      return 'Signed out — sign in again.';
    case 'session_revoked':
      return 'This device was signed out remotely.';
    case 'device_revoked':
      return "This device's access was removed. Pair with the server again.";
    case 'not_paired':
      return 'Pair with the server first.';
    case 'user_disabled':
      return 'This account is disabled.';
    case 'forbidden':
      return "You don't have access to this vault.";
    case 'already_initialized':
      return 'This server already has an account.';
    case 'pairing_consumed':
      return 'That pairing code has already been used.';
    case 'pairing_expired':
      return 'That pairing code expired — show a new one.';
    case 'rate_limited':
      final wait = e.retryAfter;
      if (wait == null) return 'Too many attempts. Try again shortly.';
      return 'Too many attempts. Try again in ${_humanSeconds(wait)}.';
    default:
      // An unrecognised code is still the server refusing, not the network
      // failing, so say so plainly and show what it sent.
      return 'The server refused the request (${e.message}).';
  }
}

String _humanSeconds(int seconds) {
  if (seconds < 60) return '$seconds seconds';
  final minutes = (seconds / 60).ceil();
  return minutes == 1 ? 'a minute' : '$minutes minutes';
}
