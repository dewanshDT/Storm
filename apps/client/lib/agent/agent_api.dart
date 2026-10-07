import 'dart:convert';
import 'dart:typed_data';

import 'package:http/http.dart' as http;

import '../api/models.dart';
import 'agent_models.dart';

/// REST client for the Agent Runtime routes (`/v1/agent/*`, owner only;
/// decisions 77c and 77d).
///
/// Direct network only in V1 (AM21), so it talks to the server's base URL with
/// the session's bearer, exactly as the MCP keys screen does.
class AgentApi {
  AgentApi({required this.baseUrl, required this.token, http.Client? client})
    : _client = client ?? http.Client();

  final String baseUrl;
  final String token;
  final http.Client _client;

  Map<String, String> get _headers => {
    'Authorization': 'Bearer $token',
    'Content-Type': 'application/json',
  };

  Uri _uri(String path, [Map<String, String>? q]) =>
      Uri.parse('$baseUrl$path').replace(queryParameters: q);

  String _s(String id) => '/v1/agent/sessions/${Uri.encodeComponent(id)}';

  dynamic _decode(http.Response r) {
    if (r.statusCode < 200 || r.statusCode >= 300) {
      String message = switch (r.statusCode) {
        403 => 'Agents are available to the server owner only.',
        503 => 'The host is offline.',
        _ => 'HTTP ${r.statusCode}',
      };
      try {
        final body = jsonDecode(r.body);
        if (body is Map && body['error'] != null && r.statusCode != 503) {
          message = '${body['error']}';
        }
      } catch (_) {}
      throw StormApiException(r.statusCode, message);
    }
    if (r.bodyBytes.isEmpty) return null;
    return jsonDecode(utf8.decode(r.bodyBytes));
  }

  /// Whether this caller may use agents at all. The server's owner check is
  /// the answer: the client never infers a role (decision 77d).
  Future<bool> canUseAgents() async {
    final r = await _client.get(_uri('/v1/config/agent'), headers: _headers);
    if (r.statusCode == 403 || r.statusCode == 401 || r.statusCode == 404) {
      return false;
    }
    _decode(r);
    return true;
  }

  Future<String> defaultProvider() async =>
      (_decode(await _client.get(_uri('/v1/config/agent'), headers: _headers))
              as Map)['default_provider']
          as String;

  Future<void> setDefaultProvider(String id) async {
    _decode(
      await _client.put(
        _uri('/v1/config/agent'),
        headers: _headers,
        body: jsonEncode({'default_provider': id}),
      ),
    );
  }

  Future<List<AgentHost>> hosts() async {
    final json = _decode(
      await _client.get(_uri('/v1/agent/hosts'), headers: _headers),
    );
    return [
      for (final h in json as List)
        AgentHost.fromJson(h as Map<String, dynamic>),
    ];
  }

  /// Issues an enrollment. The string carries a single-use secret and is
  /// shown once; nothing stores it.
  Future<({String enrollment, String expires})> enroll() async {
    final json =
        _decode(
              await _client.post(
                _uri('/v1/agent/hosts/enrollments'),
                headers: _headers,
                body: jsonEncode({'server_url': baseUrl}),
              ),
            )
            as Map;
    return (
      enrollment: json['enrollment'] as String,
      expires: json['expires'] as String,
    );
  }

  Future<void> renameHost(String id, String name) async {
    _decode(
      await _client.patch(
        _uri('/v1/agent/hosts/${Uri.encodeComponent(id)}'),
        headers: _headers,
        body: jsonEncode({'name': name}),
      ),
    );
  }

  Future<void> revokeHost(String id) async {
    _decode(
      await _client.delete(
        _uri('/v1/agent/hosts/${Uri.encodeComponent(id)}'),
        headers: _headers,
      ),
    );
  }

  Future<List<AgentWorkspace>> workspaces(String hostId) async {
    final json = _decode(
      await _client.get(
        _uri('/v1/agent/hosts/${Uri.encodeComponent(hostId)}/workspaces'),
        headers: _headers,
      ),
    );
    return [
      for (final w in json as List)
        AgentWorkspace.fromJson(w as Map<String, dynamic>),
    ];
  }

  Future<List<AgentSession>> sessions() async {
    final json = _decode(
      await _client.get(_uri('/v1/agent/sessions'), headers: _headers),
    );
    return [
      for (final s in json as List)
        AgentSession.fromJson(s as Map<String, dynamic>),
    ];
  }

  Future<AgentSession> session(String id) async => AgentSession.fromJson(
    _decode(await _client.get(_uri(_s(id)), headers: _headers))
        as Map<String, dynamic>,
  );

  Future<AgentSession> launch({
    required String hostId,
    required String workspace,
    String? provider,
    required int cols,
    required int rows,
    bool allowVaultWrites = false,
  }) async => AgentSession.fromJson(
    _decode(
          await _client.post(
            _uri('/v1/agent/sessions'),
            headers: _headers,
            body: jsonEncode({
              'host_id': hostId,
              'workspace': workspace,
              'provider': ?provider,
              'interaction': 'terminal',
              'terminal': {'cols': cols, 'rows': rows},
              // The launch toggle (G-D5): off unless asked for.
              'allow_vault_writes': allowVaultWrites,
            }),
          ),
        )
        as Map<String, dynamic>,
  );

  Future<void> end(String id) async {
    _decode(await _client.post(_uri('${_s(id)}/end'), headers: _headers));
  }

  Future<void> dismiss(String id) async {
    _decode(await _client.delete(_uri(_s(id)), headers: _headers));
  }

  /// Raw input bytes. At most once: never retried (freeze §11.3).
  Future<void> input(String id, Uint8List bytes) async {
    _decode(
      await _client.post(
        _uri('${_s(id)}/terminal/input'),
        headers: {'Authorization': 'Bearer $token'},
        body: bytes,
      ),
    );
  }

  Future<void> resize(
    String id,
    int cols,
    int rows, {
    bool focus = false,
  }) async {
    _decode(
      await _client.post(
        _uri('${_s(id)}/terminal/resize'),
        headers: _headers,
        body: jsonEncode({'cols': cols, 'rows': rows, 'focus': focus}),
      ),
    );
  }

  /// A single-use ticket for a browser stream, which cannot carry a header.
  Future<String> streamTicket() async =>
      (_decode(
                await _client.post(
                  _uri('/v1/auth/ws-ticket'),
                  headers: _headers,
                ),
              )
              as Map)['ticket']
          as String;

  void dispose() => _client.close();
}
