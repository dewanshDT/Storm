/// REST client for decision 162's feedback endpoints (session tier).
///
/// Split from `StormApi` for the same reason `AuthApi` is: a distinct, small
/// surface with its own request/response shapes, normative in *Decision 162
/// — feedback API contract*. Build against the contract, not against a
/// running server — 162-3 (the server side) lands in parallel.
library;

import 'dart:convert';

import 'package:http/http.dart' as http;

import 'feedback_models.dart';
import 'models.dart' show StormApiException;

/// REST client for `/v1/feedback*`.
class FeedbackApi {
  FeedbackApi({required this.baseUrl, required this.token, http.Client? client})
    : _client = client ?? http.Client();

  /// e.g. `http://192.168.1.20:8484` — no trailing slash.
  final String baseUrl;
  final String token;
  final http.Client _client;

  Map<String, String> get _headers => {
    'Authorization': 'Bearer $token',
    'Content-Type': 'application/json',
  };

  Uri _uri(String path) => Uri.parse('$baseUrl$path');

  dynamic _decode(http.Response r) {
    if (r.statusCode < 200 || r.statusCode >= 300) {
      String message = 'HTTP ${r.statusCode}';
      try {
        final body = jsonDecode(r.body);
        if (body is Map && body['error'] != null) message = '${body['error']}';
      } catch (_) {
        // Non-JSON error body — keep the status line.
      }
      throw FeedbackApiException(r.statusCode, message);
    }
    return jsonDecode(utf8.decode(r.bodyBytes));
  }

  /// `GET /v1/feedback` — whether the sheet can send on this server.
  Future<FeedbackStatus> status() async {
    final json = _decode(
      await _client.get(_uri('/v1/feedback'), headers: _headers),
    );
    return FeedbackStatus.fromJson(json as Map<String, dynamic>);
  }

  /// `POST /v1/feedback/preview` — builds the bundle and returns exactly
  /// what a send with the returned `preview_id` will forward. Kept by the
  /// server for 10 minutes (162b); a stale one is `preview_expired` (410).
  Future<FeedbackPreview> preview({
    required String kind,
    required bool includeDiagnostics,
    required FeedbackClientInfo client,
  }) async {
    final json = _decode(
      await _client.post(
        _uri('/v1/feedback/preview'),
        headers: _headers,
        body: jsonEncode({
          'kind': kind,
          'include_diagnostics': includeDiagnostics,
          'client': client.toJson(),
        }),
      ),
    );
    return FeedbackPreview.fromJson(json as Map<String, dynamic>);
  }

  /// `POST /v1/feedback` — sends exactly the bytes [preview] showed.
  Future<FeedbackSendResult> send({
    required String previewId,
    required String kind,
    required String text,
    String? email,
    String? screenshotPngB64,
  }) async {
    final json = _decode(
      await _client.post(
        _uri('/v1/feedback'),
        headers: _headers,
        body: jsonEncode({
          'preview_id': previewId,
          'kind': kind,
          'text': text,
          'email': email,
          'screenshot_png_b64': screenshotPngB64,
        }),
      ),
    );
    return FeedbackSendResult.fromJson(json as Map<String, dynamic>);
  }

  void dispose() => _client.close();
}

class FeedbackApiException extends StormApiException {
  FeedbackApiException(super.statusCode, super.message);

  bool get isDisabled => message == 'feedback_disabled';
  bool get isPreviewExpired =>
      statusCode == 410 || message == 'preview_expired';
}
