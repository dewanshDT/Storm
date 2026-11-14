import 'dart:async';
import 'dart:convert';

import 'package:http/http.dart' as http;
import 'package:http/testing.dart';

/// Minimal stand-in for storm-server, shared by the client test suites.
///
/// The distinction that matters is between [unreachable] — a socket-level
/// failure, what a dead server looks like — and [failWith], a real HTTP status
/// from a server that is up but refuses. The sync engine must treat those very
/// differently: the first is queued and retried, the second never is.
class ServerNote {
  ServerNote({
    required this.id,
    required this.path,
    required this.content,
    required this.version,
    this.title,
  });

  final String id;
  final String path;
  final String content;
  final int version;

  /// The indexed title; the id when unset, as these fakes always had.
  final String? title;

  ServerNote copyWith({String? content, int? version, String? path}) =>
      ServerNote(
        id: id,
        path: path ?? this.path,
        content: content ?? this.content,
        version: version ?? this.version,
        title: title,
      );

  Map<String, dynamic> get meta => {
    'id': id,
    'path': path,
    'title': title ?? id,
    'version': version,
    'content_hash': 'h',
    'created': '2026-08-05T10:00:00Z',
    'modified': '2026-08-05T10:00:00Z',
    'size': content.length,
  };
}

/// A vault as the fake server reports it.
class ServerVault {
  ServerVault({required this.id, required this.name, this.missing = false});

  final String id;
  final String name;
  final bool missing;
}

/// Minimal stand-in for storm-server, with a switch for connectivity.
class FakeServer {
  /// The vault every note-shaped route is served from unless a test says
  /// otherwise. Tests that only care about notes never have to mention it.
  static const primaryVault = 'v-primary';

  /// Notes, keyed by vault id then note id.
  final Map<String, Map<String, ServerNote>> byVault = {primaryVault: {}};

  final List<ServerVault> vaults = [
    ServerVault(id: primaryVault, name: 'Primary'),
  ];

  /// Explicitly created folders, per vault. Kept separate from note paths so
  /// an empty folder can exist, exactly as on the real server.
  final Map<String, Set<String>> folders = {};

  /// Each note's latest agent writer (`agent_write`), per vault: note id ->
  /// the field as `GET …/notes/{id}` serves it. Feeds `…/agent-writes` too.
  final Map<String, Map<String, Map<String, dynamic>>> agentWriteOf = {};

  /// Records [sessionId] as a note's latest agent writer.
  void agentWrote(
    String noteId, {
    required String sessionId,
    required int version,
    String sessionName = 'gateway-spec',
    String kind = 'edited',
    bool dismissed = false,
    String vaultId = primaryVault,
    String? at,
  }) => (agentWriteOf[vaultId] ??= {})[noteId] = {
    'session_id': sessionId,
    'session_name': sessionName,
    'session_dismissed': dismissed,
    'kind': kind,
    'version': version,
    'at': at ?? DateTime.now().toUtc().toIso8601String(),
  };

  /// Each vault's tags and their counts, as `…/tags` lists them.
  final Map<String, Map<String, int>> tags = {};

  /// Server-recorded opens, per vault: note id -> ISO timestamp.
  final Map<String, Map<String, String>> opened = {};

  String vaultRoot = '/srv/storm/vaults';

  /// What each vault's trash says its retention is (`retention_days`, null
  /// for never). A vault not here has no trash route, so its retention is
  /// unknown to the client.
  final Map<String, int?> trashRetention = {};

  /// The items each vault's trash lists, where [trashRetention] serves it.
  final Map<String, List<Map<String, dynamic>>> trashItems = {};

  /// Whether the server would serve `/mcp`. Off by default, as the real one is.
  bool mcpEnabled = false;

  /// Whether MCP may change the vault. Off unless explicitly turned on, and
  /// disarmed with the endpoint, exactly as the real server does it.
  bool mcpWritable = false;

  /// Answers `GET /v1/config` without `mcp_enabled`, the way a server built
  /// before the switch existed does.
  bool omitMcpField = false;

  /// Whether agent sessions may write; null answers as a server that
  /// predates the setting (no `agent_writes` field).
  bool? agentWrites;

  /// The last `PUT /v1/config/mcp` body, to check what the client asked for
  /// and not only what the server made of it.
  Map<String, dynamic>? lastMcpBody;

  /// The server's release version; null answers as an older server.
  String? version;

  /// The configured relay URLs, as `PUT /v1/config/relays` stores them.
  List<String> relays = [];

  /// LAN discovery's setting (decision 118); null answers as an older server.
  bool? discoverable;

  /// `--no-discovery` on the server.
  bool discoveryForcedOff = false;

  /// Paired devices, as `GET /v1/auth/devices` returns them.
  final List<Map<String, dynamic>> devices = [];

  /// The account password, for `POST /v1/auth/password`.
  String password = 'correct horse';

  /// Access keys, as `GET /v1/keys` returns them.
  final List<Map<String, dynamic>> keys = [];

  /// Notes in the primary vault. The shorthand most tests use.
  Map<String, ServerNote> get notes => byVault[primaryVault]!;
  final List<Map<String, dynamic>> changeLog = [];
  int seq = 0;

  /// Simulates a dead server: a socket error, not an HTTP status.
  bool unreachable = false;

  /// Simulates a reachable server that refuses.
  int? failWith;

  /// Thrown by the next requests, one each: a relay stream that dropped or
  /// timed out, as the SRP client reports it.
  final List<Object> throwNext = [];

  String? mergeInstead;
  String? conflictInstead;
  int? lastBaseVersion;

  /// How many times /v1/tree was fetched — lets a test assert that status
  /// notifications don't cause refetch storms.
  int treeRequests = 0;

  void pushChange(
    String noteId,
    String kind,
    int version, {
    String vaultId = primaryVault,
  }) {
    seq++;
    changeLog.add({
      'seq': seq,
      'vault_id': vaultId,
      'note_id': noteId,
      'kind': kind,
      'version': version,
      'at': '2026-08-05T10:00:00Z',
    });
  }

  /// Registers another vault, so a test can exercise switching.
  Map<String, ServerNote> addVault(String id, String name) {
    vaults.add(ServerVault(id: id, name: name));
    return byVault[id] ??= {};
  }

  /// Marks a note as opened, feeding `/v1/recents`.
  void markOpened(String vaultId, String noteId, String at) =>
      (opened[vaultId] ??= {})[noteId] = at;

  /// Vaults `/v1/search` reports in `skipped` instead of searching.
  final Set<String> searchSkipped = {};

  /// Answers `/v1/search` with 404, as a server older than decision 156 does.
  bool noCrossSearch = false;

  /// Answers `/v1/search` with this status instead of searching.
  int? crossSearchStatus;

  /// Awaited before `/v1/search` answers [q], so a test can hold one
  /// request while another is answered.
  Future<void> Function(String q)? crossSearchHold;

  /// The queries `/v1/search` was asked, in order.
  final List<String> crossSearches = [];

  /// The vaults `/v1/vaults/{v}/search` was asked about, in order.
  final List<String> vaultSearches = [];

  /// A case-insensitive substring match over title and content, standing in
  /// for FTS5. [q] is already lowercased.
  List<Map<String, dynamic>> _searchVault(String vaultId, String q) => [
    for (final note
        in (byVault[vaultId] ?? const <String, ServerNote>{}).values)
      if (!note.path.startsWith('_storm/') &&
          ((note.meta['title'] as String).toLowerCase().contains(q) ||
              note.content.toLowerCase().contains(q)))
        {
          'id': note.id,
          'path': note.path,
          'title': note.meta['title'],
          'snippet': _marked(note.content, q),
        },
  ];

  /// FTS5's `<<match>>` markers around the first match, as the server's
  /// snippet draws them.
  static String _marked(String content, String q) {
    final at = content.toLowerCase().indexOf(q);
    if (at < 0) return content;
    final end = at + q.length;
    return '${content.substring(0, at)}<<${content.substring(at, end)}>>'
        '${content.substring(end)}';
  }

  /// Folders derived from note paths, union the explicitly created ones —
  /// what the real `tree` handler returns.
  List<String> _foldersOf(String vaultId) {
    final out = <String>{...?folders[vaultId]};
    for (final note in (byVault[vaultId] ?? {}).values) {
      final parts = note.path.split('/');
      for (var i = 1; i < parts.length; i++) {
        out.add(parts.take(i).join('/'));
      }
    }
    final list = out.toList()..sort();
    return list;
  }

  /// How many requests have reached this server.
  ///
  /// For the tests that assert a request was *not* sent — "the engine stayed
  /// silent" cannot be shown by inspecting a response that never came.
  int requests = 0;

  /// Holds `/v1/sync` and note fetches until completed, so a test can act
  /// while a pull is in flight.
  Completer<void>? syncGate;
  Completer<void>? noteGate;

  /// Holds a note `PUT` until completed, so a test can act mid-save.
  Completer<void>? saveGate;

  http.Client get client => MockClient((request) async {
    requests++;
    if (throwNext.isNotEmpty) throw throwNext.removeAt(0);
    if (unreachable) {
      throw http.ClientException('Connection refused');
    }
    if (failWith != null) {
      final code = failWith!;
      return http.Response(
        '{"error":"refused"}',
        code,
        headers: {'content-type': 'application/json'},
      );
    }

    var path = request.url.path;
    Map<String, String> j(Object o) => {'content-type': 'application/json'};

    // ---- server-level routes -------------------------------------------

    if (path == '/v1/vaults' && request.method == 'GET') {
      return http.Response(
        jsonEncode({
          'vaults': [
            for (final v in vaults)
              {
                'id': v.id,
                'name': v.name,
                'dir': v.name.toLowerCase(),
                'note_count': (byVault[v.id] ?? const {}).length,
                'missing': v.missing,
              },
          ],
        }),
        200,
        headers: j(''),
      );
    }

    final trash = RegExp(r'^/v1/vaults/([^/]+)/trash$').firstMatch(path);
    if (trash != null &&
        request.method == 'GET' &&
        trashRetention.containsKey(trash.group(1))) {
      return http.Response(
        jsonEncode({
          'items': trashItems[trash.group(1)] ?? const [],
          'retention_days': trashRetention[trash.group(1)],
        }),
        200,
        headers: j(''),
      );
    }

    if (path == '/v1/vaults' && request.method == 'POST') {
      final body = jsonDecode(request.body) as Map<String, dynamic>;
      final name = body['name'] as String;
      final id = 'v-${vaults.length + 1}';
      addVault(id, name);
      return http.Response(
        jsonEncode({'id': id, 'name': name, 'dir': name.toLowerCase()}),
        200,
        headers: j(''),
      );
    }

    // The MCP switch. Server-side state, so a toggle is visible to the next
    // GET /v1/config exactly as it is against the real server.
    if (path == '/v1/config/mcp' && request.method == 'PUT') {
      final body = jsonDecode(request.body) as Map<String, dynamic>;
      lastMcpBody = body;
      // Absent leaves the MCP switches alone, as the real server does.
      if (body['enabled'] case final bool enabled) {
        mcpEnabled = enabled;
        mcpWritable = mcpEnabled && (body['writable'] as bool? ?? false);
      }
      if (body['agent_writes'] case final bool on) agentWrites = on;
      return http.Response(
        jsonEncode({
          'mcp_enabled': mcpEnabled,
          'mcp_writable': mcpWritable,
          'agent_writes': ?agentWrites,
        }),
        200,
        headers: j(''),
      );
    }

    if (path == '/v1/config/discovery' && request.method == 'PUT') {
      final body = jsonDecode(request.body) as Map<String, dynamic>;
      discoverable = body['enabled'] as bool;
      return http.Response(jsonEncode(_discoveryJson()), 200, headers: j(''));
    }

    if (path == '/v1/config/relays' && request.method == 'PUT') {
      final body = jsonDecode(request.body) as Map<String, dynamic>;
      final next = [for (final r in body['relays'] as List) r as String];
      if (next.any((r) => !r.startsWith('wss://') && !r.startsWith('ws://'))) {
        return http.Response(
          '{"error":"a relay URL must start with wss:// or ws://"}',
          400,
          headers: j(''),
        );
      }
      relays = next;
      return http.Response(jsonEncode({'relays': relays}), 200, headers: j(''));
    }

    if (path == '/v1/auth/password' && request.method == 'POST') {
      final body = jsonDecode(request.body) as Map<String, dynamic>;
      if (body['current_password'] != password) {
        return http.Response('wrong password', 401);
      }
      password = body['new_password'] as String;
      return http.Response('', 204);
    }
    if (path == '/v1/auth/devices' && request.method == 'GET') {
      return http.Response(jsonEncode(devices), 200, headers: j(''));
    }
    if (path.startsWith('/v1/auth/devices/') && request.method == 'DELETE') {
      final id = path.substring('/v1/auth/devices/'.length);
      for (final d in devices) {
        if (d['id'] == id) d['revoked'] = '2026-10-08T00:00:00Z';
      }
      return http.Response('', 204);
    }

    if (path == '/v1/keys' && request.method == 'GET') {
      return http.Response(jsonEncode(keys), 200, headers: j(''));
    }
    if (path == '/v1/keys' && request.method == 'POST') {
      final body = jsonDecode(request.body) as Map<String, dynamic>;
      final key = {
        'id': 'key_${keys.length + 1}',
        'user_id': 'u1',
        'name': body['name'],
        'created': '2026-10-08T00:00:00Z',
      };
      keys.add(key);
      return http.Response(
        jsonEncode({...key, 'secret': 'stk_theonlycopy'}),
        200,
        headers: j(''),
      );
    }
    if (path.startsWith('/v1/keys/') && request.method == 'DELETE') {
      final id = path.substring('/v1/keys/'.length);
      for (final k in keys) {
        if (k['id'] == id) k['revoked'] = '2026-10-08T00:00:00Z';
      }
      return http.Response('', 204);
    }

    if (path == '/v1/config') {
      if (request.method == 'PUT') {
        final body = jsonDecode(request.body) as Map<String, dynamic>;
        vaultRoot = body['vault_root'] as String;
        return http.Response(
          jsonEncode({'vault_root': vaultRoot}),
          200,
          headers: j(''),
        );
      }
      return http.Response(
        jsonEncode({
          'vault_root': vaultRoot,
          'state_dir': '/srv/storm/state',
          'vault_count': vaults.length,
          if (!omitMcpField) 'mcp_enabled': mcpEnabled,
          if (!omitMcpField) 'mcp_writable': mcpWritable,
          'relays': relays,
          'agent_writes': ?agentWrites,
          'version': ?version,
          if (discoverable != null) 'discovery': _discoveryJson(),
        }),
        200,
        headers: j(''),
      );
    }

    if (path == '/v1/recents') {
      final out = <Map<String, dynamic>>[];
      for (final v in vaults) {
        for (final entry in (opened[v.id] ?? const {}).entries) {
          final note = (byVault[v.id] ?? const {})[entry.key];
          if (note == null) continue;
          out.add({
            'vault_id': v.id,
            'vault_name': v.name,
            'note_id': note.id,
            'path': note.path,
            'title': note.meta['title'],
            'modified': note.meta['modified'],
            'opened_at': entry.value,
          });
        }
      }
      out.sort(
        (a, b) =>
            (b['opened_at'] as String).compareTo(a['opened_at'] as String),
      );
      return http.Response(jsonEncode({'recents': out}), 200, headers: j(''));
    }

    if (path == '/v1/search') {
      if (noCrossSearch) {
        return http.Response('{"error":"not found"}', 404, headers: j(''));
      }
      final asked = request.url.queryParameters['q'] ?? '';
      crossSearches.add(asked);
      await crossSearchHold?.call(asked);
      if (crossSearchStatus case final status?) {
        return http.Response('{"error":"failed"}', status, headers: j(''));
      }
      final q = asked.trim().toLowerCase();
      final hits = <Map<String, dynamic>>[];
      final skipped = <Map<String, dynamic>>[];
      if (q.isNotEmpty) {
        for (final v in vaults) {
          if (v.missing) continue;
          if (searchSkipped.contains(v.id)) {
            skipped.add({
              'vault_id': v.id,
              'vault_name': v.name,
              'reason': 'busy',
            });
            continue;
          }
          for (final hit in _searchVault(v.id, q)) {
            final title = (hit['title'] as String).toLowerCase();
            hits.add({
              'vault_id': v.id,
              'vault_name': v.name,
              ...hit,
              'tier': title == q
                  ? 0
                  : title.startsWith(q)
                  ? 1
                  : title.contains(q)
                  ? 2
                  : 3,
            });
          }
        }
        hits.sort((a, b) => (a['tier'] as int).compareTo(b['tier'] as int));
      }
      final limit = int.tryParse(request.url.queryParameters['limit'] ?? '');
      return http.Response(
        jsonEncode({
          'hits': hits.take((limit ?? 50).clamp(1, 200)).toList(),
          'skipped': skipped,
        }),
        200,
        headers: j(''),
      );
    }

    // ---- vault-scoped routes -------------------------------------------
    //
    // The prefix is stripped once here so the note handlers below stay the
    // shape they had before vaults existed.
    var vaultId = primaryVault;
    if (path.startsWith('/v1/vaults/')) {
      final rest = path.substring('/v1/vaults/'.length);
      final slash = rest.indexOf('/');
      if (slash < 0) {
        // DELETE or PATCH on the vault itself.
        final id = rest;
        if (request.method == 'DELETE') {
          vaults.removeWhere((v) => v.id == id);
          byVault.remove(id);
          return http.Response('{"removed":"$id"}', 200, headers: j(''));
        }
        if (request.method == 'PATCH') {
          final body = jsonDecode(request.body) as Map<String, dynamic>;
          final at = vaults.indexWhere((v) => v.id == id);
          if (at >= 0) {
            vaults[at] = ServerVault(id: id, name: body['name'] as String);
          }
          return http.Response('{"id":"$id"}', 200, headers: j(''));
        }
      }
      vaultId = rest.substring(0, slash < 0 ? rest.length : slash);
      path = '/v1${rest.substring(slash < 0 ? rest.length : slash)}';

      if (!byVault.containsKey(vaultId)) {
        return http.Response('{"error":"no such vault"}', 404, headers: j(''));
      }
    }
    final vaultNotes = byVault[vaultId]!;

    if (path == '/v1/tags' && request.method == 'GET') {
      return http.Response(
        jsonEncode({
          'tags': [
            for (final e in (tags[vaultId] ?? const <String, int>{}).entries)
              {'tag': e.key, 'count': e.value},
          ],
        }),
        200,
        headers: j(''),
      );
    }

    if (path == '/v1/folders' && request.method == 'POST') {
      final body = jsonDecode(request.body) as Map<String, dynamic>;
      (folders[vaultId] ??= {}).add(body['path'] as String);
      return http.Response('{"path":"ok"}', 200, headers: j(''));
    }

    if (path == '/v1/folders/rename') {
      final body = jsonDecode(request.body) as Map<String, dynamic>;
      final from = body['from'] as String;
      final to = body['to'] as String;
      final set = folders[vaultId] ??= {};
      folders[vaultId] = {
        for (final f in set)
          if (f == from)
            to
          else if (f.startsWith('$from/'))
            '$to${f.substring(from.length)}'
          else
            f,
      };
      var moved = 0;
      for (final note in vaultNotes.values.toList()) {
        if (!note.path.startsWith('$from/')) continue;
        vaultNotes[note.id] = note.copyWith(
          path: '$to${note.path.substring(from.length)}',
          version: note.version + 1,
        );
        pushChange(note.id, 'moved', note.version + 1, vaultId: vaultId);
        moved++;
      }
      return http.Response('{"moved":$moved}', 200, headers: j(''));
    }

    if (path.startsWith('/v1/folders/') && request.method == 'DELETE') {
      final folder = Uri.decodeFull(path.substring('/v1/folders/'.length));
      final held = vaultNotes.values.where(
        (n) => n.path.startsWith('$folder/'),
      );
      // Decision 150: refused unless the person confirmed, then trashed.
      if (held.isNotEmpty && request.url.queryParameters['trash'] != 'true') {
        return http.Response(
          '{"error":"${held.length} note(s) inside"}',
          409,
          headers: j(''),
        );
      }
      for (final id in held.map((n) => n.id).toList()) {
        vaultNotes.remove(id);
      }
      folders[vaultId]?.remove(folder);
      return http.Response('{"deleted":"ok"}', 200, headers: j(''));
    }

    if (path.endsWith('/opened') && request.method == 'POST') {
      final id = path.substring(
        '/v1/notes/'.length,
        path.length - '/opened'.length,
      );
      if (!vaultNotes.containsKey(id)) {
        return http.Response('{"error":"no such note"}', 404, headers: j(''));
      }
      markOpened(vaultId, id, '2026-08-07T12:00:00Z');
      return http.Response('{"ok":true}', 200, headers: j(''));
    }

    if (path == '/v1/sync') {
      await syncGate?.future;
      final since = int.parse(request.url.queryParameters['since'] ?? '0');
      return http.Response(
        jsonEncode({
          'changes': changeLog
              .where(
                (c) => (c['seq'] as int) > since && c['vault_id'] == vaultId,
              )
              .toList(),
          'seq': seq,
        }),
        200,
        headers: j(''),
      );
    }

    if (path == '/v1/search') {
      vaultSearches.add(vaultId);
      final q = (request.url.queryParameters['q'] ?? '').trim().toLowerCase();
      return http.Response(
        jsonEncode({'hits': q.isEmpty ? const [] : _searchVault(vaultId, q)}),
        200,
        headers: j(''),
      );
    }

    if (path == '/v1/agent-writes') {
      return http.Response(
        jsonEncode({
          for (final e in (agentWriteOf[vaultId] ?? const {}).entries)
            e.key: {
              'version': e.value['version'],
              'at': e.value['at'],
              'session_id': e.value['session_id'],
            },
        }),
        200,
        headers: j(''),
      );
    }

    if (path == '/v1/tree') {
      treeRequests++;
      return http.Response(
        jsonEncode({
          'notes': vaultNotes.values.map((n) => n.meta).toList(),
          'folders': _foldersOf(vaultId),
          'seq': seq,
        }),
        200,
        headers: j(''),
      );
    }

    if (request.method == 'POST' && path == '/v1/notes') {
      final body = jsonDecode(request.body) as Map<String, dynamic>;
      final newPath = body['path'] as String;
      if (vaultNotes.values.any((n) => n.path == newPath)) {
        // 409, as the server answers since decision 128a.
        return http.Response(
          '{"error":"a note already exists"}',
          409,
          headers: j(''),
        );
      }
      final newId = 'gen-${vaultNotes.length + 1}';
      // The real server stamps identity into the file on create.
      final content =
          '---\nid: $newId\n---\n\n${body['content'] as String? ?? ''}';
      final created = ServerNote(
        id: newId,
        path: newPath,
        content: content,
        version: 1,
      );
      vaultNotes[newId] = created;
      pushChange(newId, 'created', 1, vaultId: vaultId);
      return http.Response(
        jsonEncode({
          'note': created.meta,
          'content': created.content,
          'seq': seq,
          'merged': false,
          'conflict': false,
        }),
        200,
        headers: j(''),
      );
    }

    var id = path.replaceFirst('/v1/notes/', '');

    if (id.endsWith('/move')) {
      id = id.substring(0, id.length - '/move'.length);
      final note = vaultNotes[id];
      if (note == null) {
        return http.Response('{"error":"no such note"}', 404, headers: j(''));
      }
      final body = jsonDecode(request.body) as Map<String, dynamic>;
      // A move keeps the id: it is a metadata change, not delete + create.
      final moved = note.copyWith(
        path: body['new_path'] as String,
        version: note.version + 1,
      );
      vaultNotes[id] = moved;
      pushChange(id, 'moved', moved.version, vaultId: vaultId);
      return http.Response(
        jsonEncode({
          'note': moved.meta,
          'content': moved.content,
          'seq': seq,
          'merged': false,
          'conflict': false,
        }),
        200,
        headers: j(''),
      );
    }

    if (request.method == 'GET' && path.startsWith('/v1/notes/')) {
      await noteGate?.future;
      final note = vaultNotes[id];
      if (note == null) {
        return http.Response('{"error":"no such note"}', 404, headers: j(''));
      }
      return http.Response(
        jsonEncode({
          ...note.meta,
          'content': note.content,
          'agent_write': ?agentWriteOf[vaultId]?[id],
        }),
        200,
        headers: j(''),
      );
    }

    if (request.method == 'PUT') {
      await saveGate?.future;
      final note = vaultNotes[id];
      if (note == null) {
        return http.Response('{"error":"no such note"}', 404, headers: j(''));
      }
      final body = jsonDecode(request.body) as Map<String, dynamic>;
      lastBaseVersion = body['base_version'] as int;

      final content =
          conflictInstead ?? mergeInstead ?? body['content'] as String;
      final updated = note.copyWith(
        content: content,
        version: note.version + 1,
      );
      vaultNotes[id] = updated;
      pushChange(id, 'updated', updated.version, vaultId: vaultId);

      return http.Response(
        jsonEncode({
          'note': updated.meta,
          'content': content,
          'seq': seq,
          'merged': mergeInstead != null || conflictInstead != null,
          'conflict': conflictInstead != null,
        }),
        200,
        headers: j(''),
      );
    }

    if (request.method == 'DELETE') {
      vaultNotes.remove(id);
      pushChange(id, 'deleted', 0, vaultId: vaultId);
      return http.Response('{"seq":0}', 200, headers: j(''));
    }

    return http.Response('{"error":"unexpected"}', 500, headers: j(''));
  });

  Map<String, dynamic> _discoveryJson() => {
    'discoverable': discoverable ?? true,
    'forced_off': discoveryForcedOff,
    'active': (discoverable ?? true) && !discoveryForcedOff,
  };
}
