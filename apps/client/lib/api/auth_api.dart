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
      try {
        final body = jsonDecode(r.body);
        if (body is Map && body['error'] != null) {
          message = '${body['error']}';
        }
      } catch (_) {}
      throw AuthApiException(r.statusCode, message);
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
  AuthApiException(this.statusCode, this.message);

  final int statusCode;
  final String message;

  bool get isUnauthorized => statusCode == 401;
  bool get isConflict => statusCode == 409;
  bool get isGone => statusCode == 410;
  bool get isRateLimited => statusCode == 429;

  @override
  String toString() => 'AuthApiException($statusCode): $message';
}
