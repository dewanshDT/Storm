import 'dart:convert';
import 'dart:typed_data';

import 'package:http/http.dart' as http;

import '../api/models.dart';
import 'agent_models.dart';

/// A refusal from `/v1/agent/*`, with the server's stable code when it gave
/// one.
class AgentRefusal extends StormApiException {
  AgentRefusal(super.statusCode, super.message, this.code);

  final String? code;
}

/// REST client for the Agent Runtime routes (`/v1/agent/*`, owner only;
/// decisions 77c and 77d).
///
/// Addressed to the server's saved address with the session's bearer. In the
/// app its client is the session's connection, so it is relayed off the LAN
/// like everything else (decision 112).
class AgentApi {
  AgentApi({required this.baseUrl, required this.token, http.Client? client})
    : _client = client ?? http.Client();

  final String baseUrl;
  final String token;
  final http.Client _client;

  /// What the terminal stream is sent on.
  http.Client get client => _client;

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
        503 => 'The host is offline.',
        _ => 'HTTP ${r.statusCode}',
      };
      String? code;
      try {
        final body = jsonDecode(r.body);
        if (body is Map && body['error'] != null) {
          code = '${body['error']}';
          if (r.statusCode != 503) message = code;
        }
      } catch (_) {}
      throw AgentRefusal(r.statusCode, message, code);
    }
    if (r.bodyBytes.isEmpty) return null;
    return jsonDecode(utf8.decode(r.bodyBytes));
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

  String _agentOn(String hostId, String agent) =>
      '/v1/agent/hosts/${Uri.encodeComponent(hostId)}/agents/'
      '${Uri.encodeComponent(agent)}';

  /// Starts an install, update or removal (`op`) of [agent] on a host
  /// (decision 133). The answer is the job, still queued or running.
  Future<AgentJob> startAgentJob(
    String hostId,
    String agent,
    String op,
  ) async => AgentJob.fromJson(
    _decode(
          await _client.post(
            _uri(_agentOn(hostId, agent)),
            headers: _headers,
            body: jsonEncode({'op': op}),
          ),
        )
        as Map<String, dynamic>,
  );

  Future<AgentJob> agentJob(String hostId, String jobId) async =>
      AgentJob.fromJson(
        _decode(
              await _client.get(
                _uri(
                  '/v1/agent/hosts/${Uri.encodeComponent(hostId)}/agents/jobs/'
                  '${Uri.encodeComponent(jobId)}',
                ),
                headers: _headers,
              ),
            )
            as Map<String, dynamic>,
      );

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
    ({String vaultId, String noteId})? context,
    String? writeVaultId,
    String? resumeOf,
    LaunchCrew? crew,
    ({String vaultId, String noteId})? role,
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
              if (context != null)
                'context': {
                  'vault_id': context.vaultId,
                  'note_id': context.noteId,
                },
              // Absent is read only (decision 82); `allow_vault_writes` is
              // the pre-v2 field and is never sent.
              'write_vault_id': ?writeVaultId,
              // The server takes the rest from that session (decision 89).
              'resume_of': ?resumeOf,
              if (crew != null)
                'crew': crew.id != null
                    ? {'id': crew.id}
                    : {'new_name': crew.newName},
              if (role != null)
                'role': {'vault_id': role.vaultId, 'note_id': role.noteId},
            }),
          ),
        )
        as Map<String, dynamic>,
  );

  /// The notes a session created or edited, newest write first.
  Future<List<SessionWrite>> writes(String id) async {
    final json = _decode(
      await _client.get(_uri('${_s(id)}/writes'), headers: _headers),
    );
    return [
      for (final w in json as List)
        SessionWrite.fromJson(w as Map<String, dynamic>),
    ];
  }

  /// Settings › AI access › "Allow writes when chosen at launch". Absent
  /// (an older server) reads as off.
  Future<bool> agentWrites() async =>
      (_decode(await _client.get(_uri('/v1/config'), headers: _headers))
          as Map)['agent_writes'] ==
      true;

  Future<void> end(String id) async {
    _decode(await _client.post(_uri('${_s(id)}/end'), headers: _headers));
  }

  /// The crew its dismissal emptied and so deleted (138c); null from a
  /// server before decision 138, which answers with no body.
  Future<Crew?> dismiss(String id) async {
    final body = _decode(await _client.delete(_uri(_s(id)), headers: _headers));
    final crew = body is Map ? body['removed_crew'] : null;
    return crew is Map<String, dynamic> ? Crew.fromJson(crew) : null;
  }

  // ---- crews and role labels (decision 138; plan §3.2) -------------------

  String _c(String id) => '/v1/agent/crews/${Uri.encodeComponent(id)}';

  Future<List<Crew>> crews() async {
    final json = _decode(
      await _client.get(_uri('/v1/agent/crews'), headers: _headers),
    );
    return [
      for (final c in json as List) Crew.fromJson(c as Map<String, dynamic>),
    ];
  }

  /// Makes a crew and moves [sessionIds] into it, in that order. Null
  /// [color] takes the next free palette word.
  Future<CrewChange> createCrew(
    String name, {
    String? color,
    List<String> sessionIds = const [],
  }) async => CrewChange.fromJson(
    _decode(
          await _client.post(
            _uri('/v1/agent/crews'),
            headers: _headers,
            body: jsonEncode({
              'name': name,
              'color': ?color,
              'session_ids': sessionIds,
            }),
          ),
        )
        as Map<String, dynamic>?,
  );

  Future<Crew> updateCrew(String id, {String? name, String? color}) async =>
      Crew.fromJson(
        _decode(
              await _client.patch(
                _uri(_c(id)),
                headers: _headers,
                body: jsonEncode({'name': ?name, 'color': ?color}),
              ),
            )
            as Map<String, dynamic>,
      );

  /// [ids] must be every crew, in the new order.
  Future<void> reorderCrews(List<String> ids) async {
    _decode(
      await _client.put(
        _uri('/v1/agent/crews/order'),
        headers: _headers,
        body: jsonEncode({'ids': ids}),
      ),
    );
  }

  /// [sessionIds] must be every member of the crew, in the new order.
  Future<void> reorderCrewMembers(String id, List<String> sessionIds) async {
    _decode(
      await _client.put(
        _uri('${_c(id)}/members/order'),
        headers: _headers,
        body: jsonEncode({'ids': sessionIds}),
      ),
    );
  }

  /// Deletes a crew; its members become loose.
  Future<void> deleteCrew(String id) async {
    _decode(await _client.delete(_uri(_c(id)), headers: _headers));
  }

  /// Joins, moves, reorders or (with a null [crewId]) leaves. [before] is a
  /// member to go in front of; last when absent.
  Future<CrewChange> moveToCrew(
    String sessionId,
    String? crewId, {
    String? before,
  }) async => CrewChange.fromJson(
    _decode(
          await _client.put(
            _uri('${_s(sessionId)}/crew'),
            headers: _headers,
            body: jsonEncode({'crew_id': crewId, 'before': ?before}),
          ),
        )
        as Map<String, dynamic>?,
  );

  /// Sets the role label only; the launch role never changes (138f).
  Future<void> setRole(String sessionId, String vaultId, String noteId) async {
    _decode(
      await _client.put(
        _uri('${_s(sessionId)}/role'),
        headers: _headers,
        body: jsonEncode({'vault_id': vaultId, 'note_id': noteId}),
      ),
    );
  }

  Future<void> clearRole(String sessionId) async {
    _decode(
      await _client.delete(_uri('${_s(sessionId)}/role'), headers: _headers),
    );
  }

  /// Kit's role notes, read live (138e).
  Future<List<RoleNote>> roles() async {
    final json = _decode(
      await _client.get(_uri('/v1/agent/roles'), headers: _headers),
    );
    return [
      for (final r in json as List)
        RoleNote.fromJson(r as Map<String, dynamic>),
    ];
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

  /// Stages a pasted image on the session's host; the path to paste
  /// (D15 AM46). Never retried.
  Future<String> stageImage(String id, Uint8List bytes, String mime) async {
    final body = _decode(
      await _client.post(
        _uri('${_s(id)}/terminal/images'),
        headers: {'Authorization': 'Bearer $token', 'Content-Type': mime},
        body: bytes,
      ),
    );
    final path = body is Map ? body['path'] : null;
    if (path is! String) {
      throw StormApiException(502, 'the server answered without a path');
    }
    return path;
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

  /// One request to an owner-only route, decoded: what the spawning
  /// client's calls (`spawn_api.dart`) are built on.
  Future<dynamic> send(
    String method,
    String path, {
    Object? body,
    Map<String, String>? query,
  }) async {
    final req = http.Request(method, _uri(path, query))
      ..headers.addAll(_headers);
    if (body != null) req.body = jsonEncode(body);
    return _decode(await http.Response.fromStream(await _client.send(req)));
  }

  void dispose() => _client.close();
}
