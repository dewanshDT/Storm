import 'package:drift/native.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:storm/api/storm_api.dart';
import 'package:storm/cache/cache_db.dart';
import 'package:storm/state/note_session.dart';
import 'package:storm/sync/sync_engine.dart';

import 'fake_server.dart';

/// The save protocol.
///
/// This is where an edit can silently vanish, so the cases that matter are the
/// ones where the server's answer disagrees with what the client thought it
/// was doing, or where the user keeps typing through a save.
void main() {
  late CacheDb cache;
  late FakeServer server;
  late SyncEngine engine;
  late NoteSession session;

  setUp(() {
    cache = CacheDb(NativeDatabase.memory());
    server = FakeServer();
    engine = SyncEngine(
      api: StormApi(baseUrl: 'http://test', token: 't', client: server.client),
      cache: cache,
      vaultId: FakeServer.primaryVault,
    );
    session = NoteSession(engine);
  });

  tearDown(() async {
    session.dispose();
    engine.dispose();
    await cache.close();
  });

  Future<void> open({int version = 1, String content = 'original\n'}) async {
    server.notes['n1'] = ServerNote(
      id: 'n1',
      path: 'A.md',
      content: content,
      version: version,
    );
    await session.open('n1');
  }

  test('opening loads content and the base version', () async {
    await open(version: 4);
    expect(session.isOpen, isTrue);
    expect(session.buffer, 'original\n');
    expect(session.baseVersion, 4);
    expect(session.saveState, SaveState.saved);
  });

  test(
    'opening a missing note reports it rather than showing a blank page',
    () async {
      await session.open('nope');
      expect(session.isOpen, isFalse);
      expect(session.error, isNotNull);
    },
  );

  test('an edit marks the session dirty without saving immediately', () async {
    await open();
    session.edit('edited\n');
    expect(session.saveState, SaveState.dirty);
    expect(
      server.notes['n1']!.content,
      'original\n',
      reason: 'saves must be debounced, not immediate',
    );
  });

  test('save sends the base version the buffer was edited from', () async {
    await open(version: 3);
    session.edit('edited\n');
    await session.save();

    expect(server.lastBaseVersion, 3);
    expect(server.notes['n1']!.content, 'edited\n');
    expect(session.baseVersion, 4, reason: 'must advance to the new version');
    expect(session.saveState, SaveState.saved);
  });

  test('saving with nothing dirty is a no-op', () async {
    await open();
    await session.save();
    expect(server.lastBaseVersion, isNull);
  });

  test('editing with identical text does not dirty the session', () async {
    await open();
    session.edit('original\n');
    expect(session.saveState, SaveState.saved);
  });

  test('a merged response replaces the buffer with the server text', () async {
    // The server reconciled against a version we never saw. Keeping our own
    // text would make the next save race a version we do not have.
    await open();
    final revisionBefore = session.revision;
    server.mergeInstead = 'merged by server\n';

    session.edit('my local edit\n');
    await session.save();

    expect(session.buffer, 'merged by server\n');
    expect(
      session.revision,
      greaterThan(revisionBefore),
      reason: 'the editor must be told to reload',
    );
    expect(session.notice, isNotNull);
    expect(session.hasConflict, isFalse);
  });

  test('a conflict adopts the marked text and flags it', () async {
    await open();
    server.conflictInstead =
        '<<<<<<< ours\nmine\n=======\ntheirs\n>>>>>>> theirs\n';

    session.edit('mine\n');
    await session.save();

    expect(session.hasConflict, isTrue);
    expect(session.buffer, contains('<<<<<<<'));
    expect(session.buffer, contains('mine'));
    expect(session.buffer, contains('theirs'));
    expect(session.notice, contains('Another device'));
  });

  test('typing during an in-flight save keeps the session dirty', () async {
    // Otherwise the newer keystrokes would be marked saved and then lost if
    // the app closed before another edit triggered a save.
    await open();
    session.edit('first\n');

    final saving = session.save();
    session.edit('second\n'); // lands while the PUT is in flight
    await saving;

    expect(
      session.saveState,
      SaveState.dirty,
      reason: 'the buffer is ahead of the server, so it is not saved',
    );
    expect(session.buffer, 'second\n');
  });

  test('a rejected save stays dirty so the edit is not lost', () async {
    await open();
    server.failWith = 500;

    session.edit('precious edit\n');
    await session.save();

    expect(
      session.saveState,
      SaveState.dirty,
      reason: 'a failed save must not look saved',
    );
    expect(session.error, isNotNull);
    expect(session.buffer, 'precious edit\n', reason: 'the edit must survive');
  });

  test(
    'an offline save is queued and the base version does not advance',
    () async {
      // The server has not accepted it, so advancing the base would tell a
      // later save there is nothing to merge against.
      await open(version: 2);
      server.unreachable = true;

      session.edit('offline edit\n');
      await session.save();

      expect(session.saveState, SaveState.queued);
      expect(session.baseVersion, 2);
      expect(await cache.pendingCount(FakeServer.primaryVault), 1);
      expect(session.buffer, 'offline edit\n');
    },
  );

  test('close clears state so a stale save cannot fire', () async {
    await open();
    session.edit('edited\n');
    session.close();

    expect(session.isOpen, isFalse);
    expect(session.buffer, isEmpty);
    await session.save();
    expect(server.notes['n1']!.content, 'original\n');
  });

  test(
    'switching notes does not save the new note over the old buffer',
    () async {
      await open();
      session.edit('edits to A\n');

      server.notes['n2'] = ServerNote(
        id: 'n2',
        path: 'B.md',
        content: 'note B\n',
        version: 1,
      );
      await session.open('n2');
      await session.save();

      expect(session.buffer, 'note B\n');
      expect(
        server.notes['n2']!.content,
        'note B\n',
        reason: "A's buffer must never be written to B",
      );
    },
  );
}
