import 'dart:async';
import 'dart:convert';

import 'package:http/http.dart' as http;

import 'agent_fakes.dart';

/// Decision 134's routes on top of [FakeAgentServer], as 134-1…134-9, both
/// integrations and 145a left them: what a session's agent did (its acts,
/// on `…/requests`), the switch, descendants, a person's end and End all
/// children, tree dismissal, and a child's work with Apply and delete
/// (137-3). There is no approve or deny route: 145a removed them.

/// A session's view with what spawning adds to it.
Map<String, dynamic> spawnSession(
  String id, {
  String status = 'running',
  String name = 'gateway-impl',
  String workspace = 'storm',
  String? parent,
  String? root,
  int depth = 0,
  String? needsAttention,
  Map<String, dynamic>? spawn,
  Map<String, dynamic>? childWorkspace,
  String? writeVaultId,
  String? resumedFrom,
  String identity = 'verified',
  String isolation = 'verified',
  String? childActivity,
  int? pendingEvents,
  List<Map<String, dynamic>>? recentEvents,
  String? model,
  String? permissionMode,
}) => {
  ...agentSession(
    id,
    status: status,
    name: name,
    workspace: parent == null ? workspace : 'children/$id',
    writeVaultId: writeVaultId,
    resumedFrom: resumedFrom,
  ),
  'parent_session': parent,
  'root_session': root ?? parent ?? id,
  'depth': depth,
  'needs_attention': needsAttention,
  'spawn': ?spawn,
  'child_workspace': ?childWorkspace,
  'identity': {'state': identity},
  'isolation': {'state': isolation},
  'child_activity': ?childActivity,
  'pending_events': ?pendingEvents,
  'recent_events': ?recentEvents,
  'model': ?model,
  'permission_mode': ?permissionMode,
};

/// A child event as the server's queue holds it (144a).
Map<String, dynamic> childEvent(
  String parent,
  int seq,
  String child,
  String kind, {
  String? reason,
  String status = 'running',
  String activity = 'unknown',
  String? needsAttention,
  String source = 'storm',
  String confidence = 'certain',
  String at = '2026-10-27T10:00:00Z',
}) => {
  'id': 'evt_${parent}_$seq',
  'seq': seq,
  'child': child,
  'kind': kind,
  'reason': reason,
  'state': {
    'status': status,
    'activity': activity,
    'needs_attention': needsAttention,
  },
  'source': source,
  'confidence': confidence,
  'at': at,
};

const _ended = ['completed', 'stopped', 'failed'];

class FakeSpawning {
  FakeSpawning(this.server) {
    server.extra = _handle;
  }

  final FakeAgentServer server;

  /// Every act agents did, oldest first, as the server keeps them (145a).
  final acts = <Map<String, dynamic>>[];

  /// A child's work as its export answers, by child id.
  final work = <String, Map<String, dynamic>>{};

  /// Names that already exist in a parent's workspace (W3's refusal).
  final existing = <String>{};

  /// Branches made by Apply.
  final applied = <String>[];
  final deleted = <String>[];

  /// What each end and End all children was sent.
  final ends = <(String, Map<String, dynamic>)>[];

  /// Terminal input any spawning action sent: it must stay empty (138p).
  int get terminalWrites => server.requests
      .where((r) => r.url.path.endsWith('/terminal/input'))
      .length;

  /// An act as the server records it: decided at once (145a).
  Map<String, dynamic> addAct(
    String requester, {
    String kind = 'start',
    String? target,
    String? task,
    String? text,
    String? name,
    String? agent,
    String? model,
    String? session,
    int? round,
    String outcome = 'started',
    String? reason,
  }) {
    final now = DateTime.now().toUtc().toIso8601String();
    final r = <String, dynamic>{
      'id': 'agr_${acts.length + 1}',
      'kind': kind,
      'requester': requester,
      'target': ?target,
      'state': 'decided',
      'created_at': now,
      'expires_at': now,
      'decided_at': now,
      'outcome': outcome,
      'reason': ?reason,
      'session': ?session,
      'task': ?task,
      'text': ?text,
      'name': ?name,
      'agent': ?agent,
      'model': ?model,
      'round': ?round,
    };
    acts.add(r);
    return r;
  }

  /// [parent]'s agent starts a child at once, as `start_agent` does, and
  /// the act is recorded.
  Map<String, dynamic> startChild(
    String parent, {
    required String id,
    String name = 'test-gateway',
    String task = 'write the tests',
    String? model,
    String status = 'running',
  }) {
    final p = server.session(parent)!;
    final child = spawnSession(
      id,
      status: status,
      name: name,
      parent: parent,
      root: p['root_session'] as String? ?? parent,
      depth: (p['depth'] as int? ?? 0) + 1,
      model: model,
    );
    // 145i: a child is listed in its root's crew.
    final root = server.session(child['root_session'] as String);
    if (root?['crew'] != null) child['crew'] = root!['crew'];
    server.sessions.add(child);
    addAct(
      parent,
      task: task,
      name: name,
      agent: p['provider'] as String?,
      model: model,
      session: id,
    );
    return child;
  }

  bool _live(Map<String, dynamic> s) => !_ended.contains(s['status']);

  List<Map<String, dynamic>> _descendants(String id) {
    final out = <Map<String, dynamic>>[];
    void walk(String p) {
      for (final s in server.sessions) {
        if (s['parent_session'] == p) {
          out.add(s);
          walk(s['id'] as String);
        }
      }
    }

    walk(id);
    return out;
  }

  Map<String, dynamic> _ref(Map<String, dynamic> s) => {
    'id': s['id'],
    'name': s['name'],
    'status': s['status'],
    'parent_session': s['parent_session'],
    'root_session': s['root_session'],
    'depth': s['depth'],
  };

  // UTF-8, as the server answers: an agent's text is not Latin-1.
  http.Response _json(Object body, [int status = 200]) => http.Response.bytes(
    utf8.encode(jsonEncode(body)),
    status,
    headers: {'content-type': 'application/json; charset=utf-8'},
  );

  Map<String, dynamic> _body(http.Request req) =>
      req.body.isEmpty ? {} : jsonDecode(req.body) as Map<String, dynamic>;

  Future<http.Response?> _handle(http.Request req) async {
    final seg = req.url.pathSegments;
    if (seg.length < 5 || seg[1] != 'agent' || seg[2] != 'sessions') {
      if (seg.length == 4 && seg[2] == 'sessions' && req.method == 'DELETE') {
        return _dismiss(req, seg[3]);
      }
      return null;
    }
    final id = seg[3];
    final s = server.session(id);
    if (s == null) return null;
    switch ((req.method, seg.sublist(4).join('/'))) {
      case ('GET', 'requests'):
        return _json({
          'requests': [
            for (final r in acts.reversed)
              if (r['requester'] == id) r,
          ],
        });
      case ('PUT', 'spawn'):
        final on = _body(req)['enabled'] == true;
        final sw = s['spawn'] as Map<String, dynamic>?;
        if (on && sw?['refused'] != null) {
          return _json({'error': sw!['refused']}, 409);
        }
        s['spawn'] = {'enabled': on, 'launched_on': sw?['launched_on']};
        return _json({'enabled': on});
      case ('GET', 'descendants'):
        final d = _descendants(id);
        return _json({
          'running': [
            for (final x in d)
              if (_live(x)) _ref(x),
          ],
          'ended': [
            for (final x in d)
              if (!_live(x)) _ref(x),
          ],
        });
      case ('POST', 'end'):
        final body = _body(req);
        final live = [
          for (final x in _descendants(id))
            if (_live(x)) x['id'],
        ];
        if (live.isNotEmpty) {
          final confirmed = body['descendants'] as List?;
          if (confirmed == null) {
            return _json({'error': 'confirm_descendants'}, 409);
          }
          if (live.any((x) => !confirmed.contains(x))) {
            return _json({'error': 'descendants_changed'}, 409);
          }
        }
        ends.add(('end:$id', body));
        s['status'] = 'stopped';
        for (final x in _descendants(id)) {
          if (_live(x)) x['status'] = 'stopped';
        }
        return _json({'cascade': null}, 202);
      case ('POST', 'end-children'):
        final body = _body(req);
        ends.add(('children:$id', body));
        for (final x in _descendants(id)) {
          if (_live(x)) x['status'] = 'stopped';
        }
        return _json({'id': 'cas_1', 'done': true, 'targets': <Object>[]});
      case ('GET', 'child-work'):
        if (s['parent_session'] == null) {
          return _json({'error': 'not a spawned child'}, 404);
        }
        if (_live(s)) return _json({'error': 'child_live'}, 409);
        final w = work[id];
        return w == null ? _json({'error': 'no_child'}, 404) : _json(w);
      case ('POST', 'child-work/apply'):
        final w = work[id]!;
        final name = _body(req)['name'] as String?;
        if (w['mode'] == 'clone') {
          final branch = 'storm/$id';
          if (applied.contains(branch)) {
            return _json({'error': 'branch_exists'}, 409);
          }
          applied.add(branch);
          return _json({'branch': branch, 'commit': w['tip'], 'child': id});
        }
        if (name == null || existing.contains(name)) {
          return _json({'error': 'target_exists'}, 409);
        }
        existing.add(name);
        applied.add(name);
        return _json({'directory': '/w/$name', 'files': 1, 'child': id});
      case ('DELETE', 'child-work'):
        deleted.add(id);
        work.remove(id);
        return _json({'removed': true});
    }
    return null;
  }

  http.Response _dismiss(http.Request req, String id) {
    final s = server.session(id);
    if (s == null) return _json({'error': 'no such session'}, 404);
    final root = s['root_session'] ?? id;
    final tree = [
      for (final x in server.sessions)
        if ((x['root_session'] ?? x['id']) == root) x,
    ];
    if (tree.length > 1) {
      if (tree.any(_live)) return _json({'error': 'tree_live'}, 409);
      if (req.url.queryParameters['tree'] != 'true') {
        return _json({'error': 'confirm_tree'}, 409);
      }
      for (final x in tree) {
        server.sessions.remove(x);
      }
      final emptied = server.dropEmptyCrews();
      return _json({
        'removed_crew': emptied.firstOrNull,
        'removed_crews': emptied,
        'dismissed': [for (final x in tree) x['id']],
      });
    }
    server.sessions.remove(s);
    return _json({'removed_crew': server.dropEmptyCrews().firstOrNull});
  }
}
