import 'dart:convert';

import 'package:http/http.dart' as http;

import 'agent_fakes.dart';

/// Decision 134's routes on top of [FakeAgentServer], as 134-1…134-9 and
/// both integrations built them: the requests list and card, approve and
/// deny, the switch, descendants, a person's end and End all children, tree
/// dismissal, and a child's work with Apply and delete (137-3).

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
};

const _ended = ['completed', 'stopped', 'failed'];

class FakeSpawning {
  FakeSpawning(this.server) {
    server.extra = _handle;
  }

  final FakeAgentServer server;

  /// Every request agents made, oldest first, as the server keeps them.
  final requests = <Map<String, dynamic>>[];

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

  Map<String, dynamic> addRequest(
    String requester, {
    String kind = 'start',
    String? target,
    String? task,
    String? text,
    String? name,
    Duration left = const Duration(minutes: 8, seconds: 42),
  }) {
    final now = DateTime.now().toUtc();
    final r = <String, dynamic>{
      'id': 'agr_${requests.length + 1}',
      'kind': kind,
      'requester': requester,
      'target': ?target,
      'state': 'pending',
      'created_at': now.toIso8601String(),
      'expires_at': now.add(left).toIso8601String(),
      'task': ?task,
      'text': ?text,
      'name': ?name,
    };
    requests.add(r);
    _attention();
    return r;
  }

  void _attention() {
    for (final s in server.sessions) {
      final waits = requests.any(
        (r) => r['requester'] == s['id'] && r['state'] == 'pending',
      );
      if (waits) {
        s['needs_attention'] = 'approval';
      } else if (s['needs_attention'] == 'approval') {
        s['needs_attention'] = null;
      }
    }
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
    if (seg.length >= 4 && seg[1] == 'agent' && seg[2] == 'requests') {
      final r = requests.where((r) => r['id'] == seg[3]).firstOrNull;
      if (r == null) return _json({'error': 'no such request'}, 404);
      if (seg.length == 4) return _json(r);
      if (r['state'] == 'decided') {
        return _json({'error': 'already_decided'}, 409);
      }
      r['state'] = 'decided';
      r['decided_at'] = DateTime.now().toUtc().toIso8601String();
      if (seg[4] == 'deny') {
        r['outcome'] = 'denied';
      } else {
        final write = _body(req)['write_vault'] == true;
        r['write_vault_asked'] = write;
        switch (r['kind']) {
          case 'start':
            final parent = server.session(r['requester'] as String)!;
            final id = 'ags_child${server.sessions.length}';
            server.sessions.add(
              spawnSession(
                id,
                status: 'starting',
                name: r['name'] as String? ?? 'test-gateway',
                parent: parent['id'] as String,
                root: parent['root_session'] as String?,
                depth: (parent['depth'] as int? ?? 0) + 1,
                writeVaultId: write
                    ? parent['write_vault_id'] as String?
                    : null,
              ),
            );
            r['outcome'] = 'started';
            r['session'] = id;
          case 'message':
            r['outcome'] = 'delivered';
          case 'end':
            server.session(r['target'] as String)?['status'] = 'stopped';
            r['outcome'] = 'ended';
        }
      }
      _attention();
      return _json(r);
    }
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
            for (final r in requests.reversed)
              if (r['requester'] == id) r,
          ],
        });
      case ('PUT', 'spawn'):
        final on = _body(req)['enabled'] == true;
        final sw = s['spawn'] as Map<String, dynamic>?;
        if (on && sw?['refused'] != null) {
          return _json({'error': sw!['refused']}, 409);
        }
        var refused = 0;
        if (!on) {
          for (final r in requests) {
            if (r['requester'] == id && r['state'] == 'pending') {
              r['state'] = 'decided';
              r['outcome'] = 'refused';
              r['reason'] = 'revoked';
              refused++;
            }
          }
          _attention();
        }
        s['spawn'] = {'enabled': on};
        return _json({'enabled': on, 'refused_requests': refused});
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
      return _json({
        'removed_crew': null,
        'removed_crews': <Object>[],
        'dismissed': [for (final x in tree) x['id']],
      });
    }
    server.sessions.remove(s);
    return _json({'removed_crew': null});
  }
}
