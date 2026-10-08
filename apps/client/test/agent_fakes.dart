import 'dart:convert';

import 'package:http/http.dart' as http;
import 'package:http/testing.dart';

/// The Agent Manager's REST surface as a test needs it: hosts, sessions with
/// their launch record (decision 82, slice 5), writes, end, dismiss, the
/// config switch and launches, every request kept for asserting on.

Map<String, dynamic> agentHost({
  String id = 'hst_1',
  String name = 'build-vm',
  String status = 'online',
  List<String> providers = const ['claude-code', 'opencode', 'shell'],
  List<String> workspaces = const ['storm', 'site'],
}) => {
  'id': id,
  'name': name,
  'status': status,
  'egress': 'host',
  'capabilities': {
    'providers': [
      for (final p in providers) {'id': p, 'kind': 'cli', 'available': true},
    ],
    'workspaces': workspaces,
    'max_sessions': 8,
  },
};

Map<String, dynamic> agentSession(
  String id, {
  String status = 'running',
  String name = 'gateway-spec',
  String hostId = 'hst_1',
  String workspace = 'storm',
  String provider = 'claude-code',
  Map<String, dynamic>? context,
  String? writeVaultId,
  int wroteCount = 0,
  String? endReason,
  int? exitCode,
}) => {
  'id': id,
  'host_id': hostId,
  'workspace': workspace,
  'provider': provider,
  'status': status,
  'end_reason': endReason,
  'exit_code': exitCode,
  'signal': null,
  'cols': 80,
  'rows': 24,
  'created_at': DateTime.now().toUtc().toIso8601String(),
  'started_at': DateTime.now().toUtc().toIso8601String(),
  'ended_at': status == 'completed' || status == 'stopped' || status == 'failed'
      ? DateTime.now().toUtc().toIso8601String()
      : null,
  'provider_fallback': null,
  'name': name,
  'context': context,
  'write_vault_id': writeVaultId,
  'wrote_count': wroteCount,
};

class FakeAgentServer {
  FakeAgentServer({
    List<Map<String, dynamic>>? hosts,
    List<Map<String, dynamic>>? sessions,
    Map<String, List<Map<String, dynamic>>>? writes,
    this.agentWrites = true,
    this.defaultProvider = 'claude-code',
    this.down = false,
  }) : hosts = hosts ?? [],
       sessions = sessions ?? [],
       writes = writes ?? {};

  final List<Map<String, dynamic>> hosts;

  /// Oldest first, as the server lists them.
  final List<Map<String, dynamic>> sessions;
  final Map<String, List<Map<String, dynamic>>> writes;
  bool agentWrites;
  String defaultProvider;
  bool down;

  final requests = <http.Request>[];

  late final MockClient client = MockClient(_handle);

  Map<String, dynamic>? session(String id) =>
      sessions.where((s) => s['id'] == id).firstOrNull;

  int count(String method, String path) =>
      requests.where((r) => r.method == method && r.url.path == path).length;

  List<Map<String, dynamic>> get launches => [
    for (final r in requests)
      if (r.method == 'POST' && r.url.path == '/v1/agent/sessions')
        jsonDecode(r.body) as Map<String, dynamic>,
  ];

  http.Response _json(Object body, [int status = 200]) =>
      http.Response(jsonEncode(body), status);

  Future<http.Response> _handle(http.Request req) async {
    requests.add(req);
    if (down) return http.Response('', 503);
    final path = req.url.path;
    final seg = req.url.pathSegments;
    if (path == '/v1/agent/hosts') return _json(hosts);
    if (path == '/v1/config') return _json({'agent_writes': agentWrites});
    if (path == '/v1/config/agent') {
      return _json({'default_provider': defaultProvider});
    }
    if (seg.length == 5 && seg[2] == 'hosts' && seg[4] == 'workspaces') {
      final host = hosts.firstWhere((h) => h['id'] == seg[3]);
      final names = (host['capabilities'] as Map)['workspaces'] as List;
      return _json([
        for (final n in names)
          {
            'name': n,
            'live_sessions': sessions
                .where(
                  (s) =>
                      s['workspace'] == n &&
                      s['host_id'] == host['id'] &&
                      !['completed', 'stopped', 'failed'].contains(s['status']),
                )
                .length,
          },
      ]);
    }
    if (path == '/v1/agent/sessions' && req.method == 'GET') {
      return _json(sessions);
    }
    if (path == '/v1/agent/sessions' && req.method == 'POST') {
      final body = jsonDecode(req.body) as Map<String, dynamic>;
      final ctx = body['context'] as Map<String, dynamic>?;
      final made = agentSession(
        'ags_new${sessions.length}',
        status: 'starting',
        name: 'launched',
        hostId: body['host_id'] as String,
        workspace: body['workspace'] as String,
        provider: body['provider'] as String? ?? defaultProvider,
        context: ctx == null ? null : {...ctx, 'title': 'Context note'},
        writeVaultId: body['write_vault_id'] as String?,
      );
      sessions.add(made);
      return _json({
        ...made,
        'mcp': {'connections': <Object>[], 'notice': null},
      });
    }
    if (seg.length >= 4 && seg[1] == 'agent' && seg[2] == 'sessions') {
      final id = seg[3];
      final s = session(id);
      if (s == null) return _json({'error': 'no such session'}, 404);
      if (seg.length == 5 && seg[4] == 'writes') return _json(writes[id] ?? []);
      if (seg.length == 5 && seg[4] == 'end' && req.method == 'POST') {
        s['status'] = 'stopped';
        return http.Response('', 204);
      }
      if (seg.length == 4 && req.method == 'DELETE') {
        sessions.remove(s);
        return http.Response('', 204);
      }
      if (seg.length == 4) return _json(s);
      return http.Response('', 204);
    }
    return _json({'error': 'nope'}, 404);
  }
}
