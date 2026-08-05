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

/// Minimal stand-in for storm-server, with a switch for connectivity.
class FakeServer {
  final Map<String, ServerNote> notes = {};
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

  void pushChange(String noteId, String kind, int version) {
    seq++;
    changeLog.add({
      'seq': seq,
      'note_id': noteId,
      'kind': kind,
      'version': version,
      'at': '2026-08-05T10:00:00Z',
    });
  }

  http.Client get client => MockClient((request) async {
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

    final path = request.url.path;
    Map<String, String> j(Object o) => {'content-type': 'application/json'};

    if (path == '/v1/sync') {
      final since = int.parse(request.url.queryParameters['since'] ?? '0');
      return http.Response(
        jsonEncode({
          'changes': changeLog.where((c) => (c['seq'] as int) > since).toList(),
          'seq': seq,
        }),
        200,
        headers: j(''),
      );
    }

    if (path == '/v1/tree') {
      return http.Response(
        jsonEncode({
          'notes': notes.values.map((n) => n.meta).toList(),
          'folders': <String>[],
          'seq': seq,
        }),
        200,
        headers: j(''),
      );
    }

    var id = path.replaceFirst('/v1/notes/', '');

    if (id.endsWith('/move')) {
      id = id.substring(0, id.length - '/move'.length);
      final note = notes[id];
      if (note == null) {
        return http.Response('{"error":"no such note"}', 404, headers: j(''));
      }
      final body = jsonDecode(request.body) as Map<String, dynamic>;
      // A move keeps the id: it is a metadata change, not delete + create.
      final moved = note.copyWith(
        path: body['new_path'] as String,
        version: note.version + 1,
      );
      notes[id] = moved;
      pushChange(id, 'moved', moved.version);
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
      final note = notes[id];
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
      final note = notes[id];
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
      notes[id] = updated;
      pushChange(id, 'updated', updated.version);

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
      notes.remove(id);
      pushChange(id, 'deleted', 0);
      return http.Response('{"seq":0}', 200, headers: j(''));
    }

    return http.Response('{"error":"unexpected"}', 500, headers: j(''));
  });
}
