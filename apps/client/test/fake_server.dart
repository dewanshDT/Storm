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
  });

  final String id;
  final String path;
  final String content;
  final int version;

  ServerNote copyWith({String? content, int? version, String? path}) =>
      ServerNote(
        id: id,
        path: path ?? this.path,
        content: content ?? this.content,
        version: version ?? this.version,
      );

  Map<String, dynamic> get meta => {
    'id': id,
    'path': path,
    'title': id,
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

  /// Server-recorded opens, per vault: note id -> ISO timestamp.
  final Map<String, Map<String, String>> opened = {};

  String vaultRoot = '/srv/storm/vaults';

  /// Whether the server would serve `/mcp`. Off by default, as the real one is.
  bool mcpEnabled = false;

  /// Whether MCP may change the vault. Off unless explicitly turned on, and
  /// disarmed with the endpoint, exactly as the real server does it.
  bool mcpWritable = false;

  /// Answers `GET /v1/config` without `mcp_enabled`, the way a server built
  /// before the switch existed does.
  bool omitMcpField = false;

  /// Notes in the primary vault. The shorthand most tests use.
  Map<String, ServerNote> get notes => byVault[primaryVault]!;
  final List<Map<String, dynamic>> changeLog = [];
  int seq = 0;

  /// Simulates a dead server: a socket error, not an HTTP status.
  bool unreachable = false;

  /// Simulates a reachable server that refuses.
  int? failWith;

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

  http.Client get client => MockClient((request) async {
    requests++;
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
      mcpEnabled = body['enabled'] as bool;
      mcpWritable = mcpEnabled && (body['writable'] as bool? ?? false);
      return http.Response(
        jsonEncode({'mcp_enabled': mcpEnabled, 'mcp_writable': mcpWritable}),
        200,
        headers: j(''),
      );
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
      if (held.isNotEmpty) {
        return http.Response(
          '{"error":"${held.length} note(s) inside"}',
          409,
          headers: j(''),
        );
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
        return http.Response(
          '{"error":"a note already exists"}',
          400,
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
      final note = vaultNotes[id];
      if (note == null) {
        return http.Response('{"error":"no such note"}', 404, headers: j(''));
      }
      return http.Response(
        jsonEncode({...note.meta, 'content': note.content}),
        200,
        headers: j(''),
      );
    }

    if (request.method == 'PUT') {
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
}
