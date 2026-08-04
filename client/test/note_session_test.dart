import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';

import 'package:storm/api/storm_api.dart';
import 'package:storm/state/note_session.dart';

/// Tests for the save/merge protocol.
///
/// This is where the client can silently lose an edit, so the cases that
/// matter are the ones where the server's answer disagrees with what the
/// client thought it was doing.
void main() {
  /// A fake server that records requests and replies from a script.
  late List<Map<String, dynamic>> sent;
  late Map<String, dynamic> Function(Map<String, dynamic> body) onPut;
  late Map<String, dynamic> noteBody;
  late StormApi api;

  Map<String, dynamic> meta({
    String id = 'n1',
    String path = 'A.md',
    int version = 1,
  }) =>
      {
        'id': id,
        'path': path,
        'title': 'A',
        'version': version,
        'content_hash': 'h',
        'created': '2026-08-05T10:00:00Z',
        'modified': '2026-08-05T10:00:00Z',
        'size': 10,
      };

  setUp(() {
    sent = [];
    noteBody = {...meta(), 'content': 'original\n'};
    onPut = (body) => {
          'note': meta(version: 2),
          'content': body['content'],
          'seq': 5,
          'merged': false,
          'conflict': false,
        };

    final client = MockClient((request) async {
      if (request.method == 'GET' && request.url.path.startsWith('/v1/notes/')) {
        return http.Response(jsonEncode(noteBody), 200,
            headers: {'content-type': 'application/json'});
      }
      if (request.method == 'PUT') {
        final body = jsonDecode(request.body) as Map<String, dynamic>;
        sent.add(body);
        return http.Response(jsonEncode(onPut(body)), 200,
            headers: {'content-type': 'application/json'});
      }
      return http.Response('{"error":"unexpected"}', 500);
    });

    api = StormApi(baseUrl: 'http://test', token: 't', client: client);
  });

  Future<NoteSession> opened() async {
    final s = NoteSession(api);
    await s.open('n1');
    return s;
  }

  test('opening loads content and the base version', () async {
    final s = await opened();
    expect(s.isOpen, isTrue);
    expect(s.buffer, 'original\n');
    expect(s.baseVersion, 1);
    expect(s.saveState, SaveState.saved);
  });

  test('an edit marks the session dirty without saving immediately', () async {
    final s = await opened();
    s.edit('edited\n');
    expect(s.saveState, SaveState.dirty);
    expect(sent, isEmpty, reason: 'saves must be debounced, not immediate');
  });

  test('save sends the base version the buffer was edited from', () async {
    final s = await opened();
    s.edit('edited\n');
    await s.save();

    expect(sent.single['base_version'], 1);
    expect(sent.single['content'], 'edited\n');
    expect(s.baseVersion, 2, reason: 'must advance to the returned version');
    expect(s.saveState, SaveState.saved);
  });

  test('saving with nothing dirty is a no-op', () async {
    final s = await opened();
    await s.save();
    expect(sent, isEmpty);
  });

  test('a merged response replaces the buffer with the server text', () async {
    // The server reconciled against a version we never saw. Keeping our own
    // text would make the next save race a version we do not have.
    onPut = (body) => {
          'note': meta(version: 7),
          'content': 'merged by server\n',
          'seq': 9,
          'merged': true,
          'conflict': false,
        };

    final s = await opened();
    final revisionBefore = s.revision;
    s.edit('my local edit\n');
    await s.save();

    expect(s.buffer, 'merged by server\n');
    expect(s.baseVersion, 7);
    expect(s.revision, greaterThan(revisionBefore),
        reason: 'the editor must be told to reload');
    expect(s.notice, isNotNull);
    expect(s.hasConflict, isFalse);
  });

  test('a conflict response adopts the marked text and flags it', () async {
    onPut = (body) => {
          'note': meta(version: 8),
          'content': '<<<<<<< ours\nmine\n=======\ntheirs\n>>>>>>> theirs\n',
          'seq': 10,
          'merged': true,
          'conflict': true,
        };

    final s = await opened();
    s.edit('mine\n');
    await s.save();

    expect(s.hasConflict, isTrue);
    expect(s.buffer, contains('<<<<<<<'));
    expect(s.buffer, contains('mine'));
    expect(s.buffer, contains('theirs'));
    expect(s.notice, contains('Another device'));
    expect(s.baseVersion, 8);
  });

  test('typing during an in-flight save keeps the session dirty', () async {
    // Otherwise the newer keystrokes would be marked "saved" and then lost if
    // the app closed before another edit triggered a save.
    final s = await opened();
    s.edit('first\n');

    final saving = s.save();
    s.edit('second\n'); // lands while the PUT is in flight
    await saving;

    expect(s.saveState, SaveState.dirty,
        reason: 'the buffer is ahead of the server, so it is not saved');
    expect(s.buffer, 'second\n');
  });

  test('a failed save stays dirty so the edit is not lost', () async {
    final client = MockClient((request) async {
      if (request.method == 'GET') {
        return http.Response(jsonEncode(noteBody), 200,
            headers: {'content-type': 'application/json'});
      }
      return http.Response('{"error":"server exploded"}', 500,
          headers: {'content-type': 'application/json'});
    });
    final s = NoteSession(
        StormApi(baseUrl: 'http://test', token: 't', client: client));
    await s.open('n1');

    s.edit('precious edit\n');
    await s.save();

    expect(s.saveState, SaveState.dirty,
        reason: 'a failed save must not look saved');
    expect(s.error, contains('server exploded'));
    expect(s.buffer, 'precious edit\n', reason: 'the edit must survive');
  });

  test('open surfaces a server error rather than showing a blank note', () async {
    final client = MockClient((request) async =>
        http.Response('{"error":"no such note"}', 404,
            headers: {'content-type': 'application/json'}));
    final s = NoteSession(
        StormApi(baseUrl: 'http://test', token: 't', client: client));
    await s.open('missing');

    expect(s.isOpen, isFalse);
    expect(s.error, contains('no such note'));
  });

  test('close clears state so a stale save cannot fire', () async {
    final s = await opened();
    s.edit('edited\n');
    s.close();

    expect(s.isOpen, isFalse);
    expect(s.buffer, isEmpty);
    await s.save();
    expect(sent, isEmpty);
  });

  test('editing with identical text does not dirty the session', () async {
    final s = await opened();
    s.edit('original\n');
    expect(s.saveState, SaveState.saved);
  });
}
