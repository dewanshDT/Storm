import 'dart:async';
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
  List<String>? workspaceRoots,
  List<String>? resumes = const ['claude-code', 'opencode'],
  Map<String, bool> mcp = const {},
}) => {
  'id': id,
  'name': name,
  'status': status,
  'egress': 'host',
  'capabilities': {
    'providers': [
      for (final p in providers)
        {'id': p, 'kind': 'cli', 'available': true, 'mcp': ?mcp[p]},
    ],
    'workspaces': workspaces,
    'workspace_roots': ?workspaceRoots,
    'max_sessions': 8,
    'resume': ?resumes,
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
  List<Map<String, dynamic>>? integrations,
  String? harnessSession,
  String? resumedFrom,
  String? activity,
  (String, int)? crew,
  Map<String, dynamic>? role,
  Map<String, dynamic>? launchRole,
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
  'integrations': ?integrations,
  'activity': ?activity,
  'harness_session': harnessSession,
  'resumed_from': resumedFrom,
  if (crew != null) 'crew': {'id': crew.$1, 'order': crew.$2},
  'role': ?role,
  'launch_role': ?launchRole,
};

Map<String, dynamic> agentCrew(
  String id, {
  String name = 'Gateway release',
  String color = 'sage',
  int order = 0,
}) => {'id': id, 'name': name, 'color': color, 'order': order};

/// A role label or launch role: a Kit note by id, `title` null once deleted.
Map<String, dynamic> roleRef(String noteId, String? title) => {
  'vault_id': 'v-kit',
  'note_id': noteId,
  'title': title,
};

/// A Kit role note as `GET /v1/agent/roles` lists it.
Map<String, dynamic> kitRole(
  String noteId,
  String title, {
  String folder = 'agents',
  String? summary,
}) => {
  'vault_id': 'v-kit',
  'note_id': noteId,
  'title': title,
  'path': '$folder/$title.md',
  'folder': folder,
  'summary': summary,
};

class FakeAgentServer {
  FakeAgentServer({
    List<Map<String, dynamic>>? hosts,
    List<Map<String, dynamic>>? sessions,
    Map<String, List<Map<String, dynamic>>>? writes,
    this.agentWrites = true,
    this.defaultProvider = 'claude-code',
    this.down = false,
    List<Map<String, dynamic>>? crews,
    List<Map<String, dynamic>>? roles,
    this.crewsSupported = true,
  }) : hosts = hosts ?? [],
       sessions = sessions ?? [],
       writes = writes ?? {},
       crews = crews ?? [],
       roles = roles ?? [];

  final List<Map<String, dynamic>> hosts;

  /// Oldest first, as the server lists them.
  final List<Map<String, dynamic>> sessions;
  final Map<String, List<Map<String, dynamic>>> writes;
  bool agentWrites;
  String defaultProvider;
  bool down;

  /// Decision 138's crews, as `GET /v1/agent/crews` lists them.
  final List<Map<String, dynamic>> crews;

  /// Kit's role notes (`GET /v1/agent/roles`).
  final List<Map<String, dynamic>> roles;

  /// False: a server before decision 138, with no crew or role routes.
  bool crewsSupported;

  /// Refuses the next crew or role change with this status and code.
  (int, String)? refuseCrewChange;

  /// While set, crew and role changes are answered only once it completes:
  /// what a client draws before the server has answered.
  Completer<void>? holdCrewChanges;

  /// While set, the session list is answered only once it completes.
  Completer<void>? holdSessionList;

  final requests = <http.Request>[];

  /// Answers a route this fake does not know before it says 404, or null
  /// to fall through (the spawning routes, `spawn_fakes.dart`).
  Future<http.Response?> Function(http.Request req)? extra;

  /// Each agent's install, update or removal as the host would report it
  /// (decision 133): the job's states in order, the start answering the
  /// first and each poll the next; the last one repeats.
  final agentJobs = <String, List<Map<String, dynamic>>>{};

  /// A refused start, by agent: its status and stable code.
  final agentRefusals = <String, (int, String)>{};

  /// Called when a poll first sees a job done, to change the host the way
  /// the job would have.
  void Function(String hostId, String agent, String op)? onAgentJobDone;

  final _jobPolls = <String, int>{};
  final _jobOps = <String, String>{};

  /// The ops started, as `<host id>/<agent>:<op>`.
  List<String> get agentJobStarts => [
    for (final r in requests)
      if (r.method == 'POST' &&
          r.url.pathSegments.length == 6 &&
          r.url.pathSegments[4] == 'agents')
        '${r.url.pathSegments[3]}/${r.url.pathSegments[5]}:'
            '${(jsonDecode(r.body) as Map)['op']}',
  ];

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

  Map<String, dynamic> _job(
    String id,
    String hostId,
    String agent,
    String op,
    int n,
  ) {
    final states = agentJobs[agent] ?? const [];
    final at = states.isEmpty
        ? const {'state': 'queued'}
        : states[n.clamp(0, states.length - 1)];
    return {
      'id': id,
      'host_id': hostId,
      'agent': agent,
      'op': op,
      'created': '2026-10-24T12:00:00Z',
      'updated': '2026-10-24T12:00:00Z',
      ...at,
    };
  }

  http.Response _json(Object body, [int status = 200]) =>
      http.Response(jsonEncode(body), status);

  Map<String, dynamic>? crew(String id) =>
      crews.where((c) => c['id'] == id).firstOrNull;

  /// The crews the last change emptied, removed as the server would (138c).
  List<Map<String, dynamic>> _dropEmpty() {
    final emptied = [
      for (final c in crews)
        if (!sessions.any((s) => (s['crew'] as Map?)?['id'] == c['id'])) c,
    ];
    crews.removeWhere(emptied.contains);
    return emptied;
  }

  Future<http.Response>? _crewRoute(http.Request req) {
    final path = req.url.path;
    final seg = req.url.pathSegments;
    final crewPath = seg.length >= 3 && seg[2] == 'crews';
    final rolePath = path == '/v1/agent/roles';
    final sessionLabel =
        seg.length == 5 &&
        seg[2] == 'sessions' &&
        (seg[4] == 'crew' || seg[4] == 'role');
    if (!crewPath && !rolePath && !sessionLabel) return null;
    if (!crewsSupported) return Future.value(_json({'error': 'nope'}, 404));
    if (req.method != 'GET' && refuseCrewChange != null) {
      final (status, code) = refuseCrewChange!;
      refuseCrewChange = null;
      return Future.value(_json({'error': code}, status));
    }
    if (rolePath) return Future.value(_json(roles));
    if (sessionLabel) {
      final s = session(seg[3]);
      if (s == null) {
        return Future.value(_json({'error': 'no such session'}, 404));
      }
      if (seg[4] == 'role') {
        if (req.method == 'DELETE') {
          s.remove('role');
        } else {
          final body = jsonDecode(req.body) as Map<String, dynamic>;
          final note = roles
              .where((r) => r['note_id'] == body['note_id'])
              .firstOrNull;
          s['role'] = {...body, 'title': note?['title']};
        }
        return Future.value(http.Response('', 204));
      }
      final body = jsonDecode(req.body) as Map<String, dynamic>;
      final to = body['crew_id'] as String?;
      if (to == null) {
        s.remove('crew');
      } else {
        // In front of `before`, else last, renumbered as the server does.
        final members =
            [
              for (final m in sessions)
                if ((m['crew'] as Map?)?['id'] == to && m != s) m,
            ]..sort(
              (a, b) => ((a['crew'] as Map)['order'] as int).compareTo(
                (b['crew'] as Map)['order'] as int,
              ),
            );
        final at = members.indexWhere((m) => m['id'] == body['before']);
        members.insert(at < 0 ? members.length : at, s);
        for (final (i, m) in members.indexed) {
          m['crew'] = {'id': to, 'order': i};
        }
      }
      return Future.value(
        _json({
          'crew': to == null ? null : crew(to),
          'removed_crews': _dropEmpty(),
        }),
      );
    }
    if (path == '/v1/agent/crews' && req.method == 'GET') {
      return Future.value(_json(crews));
    }
    if (path == '/v1/agent/crews' && req.method == 'POST') {
      final body = jsonDecode(req.body) as Map<String, dynamic>;
      final made = agentCrew(
        'crw_new${crews.length}',
        name: body['name'] as String,
        color: body['color'] as String? ?? 'sky',
        order: crews.length,
      );
      crews.add(made);
      for (final (i, id) in (body['session_ids'] as List? ?? []).indexed) {
        session(id as String)?['crew'] = {'id': made['id'], 'order': i};
      }
      // A crew made empty stays; only the crews its sessions left go.
      final emptied = _dropEmpty()..remove(made);
      if (!crews.contains(made)) crews.add(made);
      return Future.value(_json({'crew': made, 'removed_crews': emptied}, 201));
    }
    if (path == '/v1/agent/crews/order') {
      final ids = (jsonDecode(req.body) as Map)['ids'] as List;
      for (final (i, id) in ids.indexed) {
        crew(id as String)?['order'] = i;
      }
      return Future.value(http.Response('', 204));
    }
    final c = crew(seg[3]);
    if (c == null) return Future.value(_json({'error': 'no such crew'}, 404));
    if (seg.length == 6 && seg[4] == 'members') {
      final ids = (jsonDecode(req.body) as Map)['ids'] as List;
      for (final (i, id) in ids.indexed) {
        session(id as String)?['crew'] = {'id': c['id'], 'order': i};
      }
      return Future.value(http.Response('', 204));
    }
    if (req.method == 'PATCH') {
      final body = jsonDecode(req.body) as Map<String, dynamic>;
      if (body['name'] != null) c['name'] = body['name'];
      if (body['color'] != null) c['color'] = body['color'];
      return Future.value(_json(c));
    }
    if (req.method == 'DELETE') {
      crews.remove(c);
      for (final s in sessions) {
        if ((s['crew'] as Map?)?['id'] == c['id']) s.remove('crew');
      }
      return Future.value(http.Response('', 204));
    }
    return Future.value(_json({'error': 'nope'}, 404));
  }

  Future<http.Response> _handle(http.Request req) async {
    requests.add(req);
    if (down) return http.Response('', 503);
    if (await extra?.call(req) case final answer?) return answer;
    final path = req.url.path;
    final seg = req.url.pathSegments;
    final crewAnswer = _crewRoute(req);
    if (crewAnswer != null) {
      if (req.method != 'GET') await holdCrewChanges?.future;
      return crewAnswer;
    }
    if (path == '/v1/agent/hosts') return _json(hosts);
    if (path == '/v1/config') return _json({'agent_writes': agentWrites});
    if (path == '/v1/config/agent') {
      return _json({'default_provider': defaultProvider});
    }
    if (seg.length == 6 &&
        seg[2] == 'hosts' &&
        seg[4] == 'agents' &&
        req.method == 'POST') {
      final agent = seg[5];
      if (agentRefusals[agent] case (final status, final code)) {
        return _json({'error': code}, status);
      }
      final op = (jsonDecode(req.body) as Map)['op'] as String;
      final id = 'job_$agent';
      _jobPolls[id] = 0;
      _jobOps[id] = op;
      return _json(_job(id, seg[3], agent, op, 0), 202);
    }
    if (seg.length == 7 && seg[4] == 'agents' && seg[5] == 'jobs') {
      final id = seg[6];
      final agent = id.substring('job_'.length);
      final n = _jobPolls[id] = (_jobPolls[id] ?? 0) + 1;
      final job = _job(id, seg[3], agent, _jobOps[id]!, n);
      final states = agentJobs[agent]!;
      if (job['state'] == 'done' && n == states.length - 1) {
        onAgentJobDone?.call(seg[3], agent, _jobOps[id]!);
      }
      return _json(job);
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
      await holdSessionList?.future;
      return _json(sessions);
    }
    if (path == '/v1/agent/sessions' && req.method == 'POST') {
      final body = jsonDecode(req.body) as Map<String, dynamic>;
      final ctx = body['context'] as Map<String, dynamic>?;
      final resumed = session(body['resume_of'] as String? ?? '');
      final made = agentSession(
        'ags_new${sessions.length}',
        status: 'starting',
        name: 'launched',
        hostId: body['host_id'] as String,
        workspace: body['workspace'] as String,
        provider: body['provider'] as String? ?? defaultProvider,
        context: ctx == null ? null : {...ctx, 'title': 'Context note'},
        writeVaultId: body['write_vault_id'] as String?,
        harnessSession: resumed?['harness_session'] as String?,
        resumedFrom: resumed?['id'] as String?,
      );
      // As 138-2's launch: join or make the crew, label the role, and keep
      // it as the launch role only for an agent that reads Storm.
      final crewAsk = body['crew'] as Map<String, dynamic>?;
      if (crewAsk != null) {
        var id = crewAsk['id'] as String?;
        if (id == null) {
          id = 'crw_new${crews.length}';
          crews.add(
            agentCrew(id, name: crewAsk['new_name'] as String, order: 99),
          );
        }
        made['crew'] = {'id': id, 'order': sessions.length};
      }
      final roleAsk = body['role'] as Map<String, dynamic>?;
      if (roleAsk != null) {
        final note = roles
            .where((r) => r['note_id'] == roleAsk['note_id'])
            .firstOrNull;
        made['role'] = {...roleAsk, 'title': note?['title']};
        if (made['provider'] != 'shell') made['launch_role'] = made['role'];
      }
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
      if (seg.length == 6 && seg[5] == 'images' && req.method == 'POST') {
        return _json({
          'image': 'img_1',
          'path': '/h/sessions/$id/inbox/img_1.png',
        });
      }
      if (seg.length == 5 && seg[4] == 'end' && req.method == 'POST') {
        s['status'] = 'stopped';
        return http.Response('', 204);
      }
      if (seg.length == 4 && req.method == 'DELETE') {
        sessions.remove(s);
        if (!crewsSupported) return http.Response('', 204);
        final removed = _dropEmpty();
        return _json({'removed_crew': removed.firstOrNull});
      }
      if (seg.length == 4) return _json(s);
      return http.Response('', 204);
    }
    return _json({'error': 'nope'}, 404);
  }
}
