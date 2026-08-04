import 'dart:convert';

import 'package:http/http.dart' as http;

import 'models.dart';

/// REST client for `storm-server`.
///
/// M2 is online-only: every read and write goes straight to the server with no
/// local cache and no outbox. Those arrive in M3, layered *above* this class
/// rather than inside it, so this stays a thin transport.
class StormApi {
  StormApi({required this.baseUrl, required this.token, http.Client? client})
      : _client = client ?? http.Client();

  /// e.g. `http://192.168.1.20:8484` — no trailing slash.
  final String baseUrl;
  final String token;
  final http.Client _client;

  Map<String, String> get _headers => {
        'Authorization': 'Bearer $token',
        'Content-Type': 'application/json',
      };

  Uri _uri(String path, [Map<String, String>? query]) =>
      Uri.parse('$baseUrl$path').replace(queryParameters: query);

  /// Decodes a response, converting any non-2xx into [StormApiException].
  dynamic _decode(http.Response r) {
    if (r.statusCode < 200 || r.statusCode >= 300) {
      String message = 'HTTP ${r.statusCode}';
      try {
        final body = jsonDecode(r.body);
        if (body is Map && body['error'] != null) message = '${body['error']}';
      } catch (_) {
        // Non-JSON error body (a proxy, say) — keep the status line.
      }
      throw StormApiException(r.statusCode, message);
    }
    // utf8.decode rather than r.body: the latter guesses latin-1 when the
    // server omits a charset, which mangles any non-ASCII note.
    return jsonDecode(utf8.decode(r.bodyBytes));
  }

  /// Verifies the server is reachable and the token is accepted.
  Future<void> checkConnection() async {
    _decode(await _client.get(_uri('/v1/tree'), headers: _headers));
  }

  Future<VaultTree> tree() async {
    final json = _decode(await _client.get(_uri('/v1/tree'), headers: _headers));
    return VaultTree.fromJson(json as Map<String, dynamic>);
  }

  /// Changes after [since] — the delta-sync primitive.
  ///
  /// Clients store the returned `seq` and pass it back next time, so catching
  /// up never requires diffing a full vault manifest.
  Future<SyncBatch> sync({int since = 0, int limit = 500}) async {
    final json = _decode(await _client.get(
      _uri('/v1/sync', {'since': '$since', 'limit': '$limit'}),
      headers: _headers,
    ));
    return SyncBatch.fromJson(json as Map<String, dynamic>);
  }

  Future<Note> note(String id) async {
    final json = _decode(await _client.get(_uri('/v1/notes/$id'), headers: _headers));
    return Note.fromJson(json as Map<String, dynamic>);
  }

  Future<WriteResult> createNote({required String path, String content = ''}) async {
    final json = _decode(await _client.post(
      _uri('/v1/notes'),
      headers: _headers,
      body: jsonEncode({'path': path, 'content': content}),
    ));
    return WriteResult.fromJson(json as Map<String, dynamic>);
  }

  /// Saves a note.
  ///
  /// [baseVersion] is the version this client last read. The server uses it to
  /// decide between a fast-forward and a 3-way merge — see the README.
  Future<WriteResult> saveNote({
    required String id,
    required int baseVersion,
    required String content,
    String? deviceId,
  }) async {
    final json = _decode(await _client.put(
      _uri('/v1/notes/$id'),
      headers: _headers,
      body: jsonEncode({
        'base_version': baseVersion,
        'content': content,
        'device_id': ?deviceId,
      }),
    ));
    return WriteResult.fromJson(json as Map<String, dynamic>);
  }

  Future<WriteResult> moveNote({required String id, required String newPath}) async {
    final json = _decode(await _client.post(
      _uri('/v1/notes/$id/move'),
      headers: _headers,
      body: jsonEncode({'new_path': newPath}),
    ));
    return WriteResult.fromJson(json as Map<String, dynamic>);
  }

  Future<void> deleteNote(String id) async {
    _decode(await _client.delete(_uri('/v1/notes/$id'), headers: _headers));
  }

  Future<List<SearchHit>> search(String query, {int limit = 50}) async {
    if (query.trim().isEmpty) return const [];
    final json = _decode(await _client.get(
      _uri('/v1/search', {'q': query, 'limit': '$limit'}),
      headers: _headers,
    ));
    return (json['hits'] as List)
        .map((h) => SearchHit.fromJson(h as Map<String, dynamic>))
        .toList();
  }

  Future<List<NoteMeta>> backlinks(String id) async {
    final json =
        _decode(await _client.get(_uri('/v1/notes/$id/backlinks'), headers: _headers));
    return (json['backlinks'] as List)
        .map((n) => NoteMeta.fromJson(n as Map<String, dynamic>))
        .toList();
  }

  void dispose() => _client.close();
}
