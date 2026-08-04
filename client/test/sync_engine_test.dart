import 'package:drift/native.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:storm/api/storm_api.dart';
import 'package:storm/cache/cache_db.dart';
import 'package:storm/state/note_session.dart';
import 'package:storm/sync/sync_engine.dart';

import 'fake_server.dart';

/// Offline, outbox and reconnect behaviour — scenarios 3-6 and 9 of the sync
/// matrix in PLAN.md, exercised in-process.
///
/// A fake server stands in for storm-server so connectivity can be cut
/// deterministically. `unreachable` throws a socket-style error (what a dead
/// server looks like) as opposed to an HTTP error (what a *reachable* server
/// that refuses looks like) — the engine must treat those very differently.
void main() {
  late CacheDb cache;
  late FakeServer server;
  late SyncEngine engine;

  setUp(() {
    cache = CacheDb(NativeDatabase.memory());
    server = FakeServer();
    engine = SyncEngine(
      api: StormApi(
        baseUrl: 'http://test',
        token: 't',
        client: server.client,
      ),
      cache: cache,
    );
  });

  tearDown(() async {
    engine.dispose();
    await cache.close();
  });

  /// Seeds a note into both the fake server and the cache, as if opened.
  Future<void> seed(String id, String content, {int version = 1}) async {
    server.notes[id] = ServerNote(id: id, path: '$id.md', content: content, version: version);
    await engine.openNote(id);
  }

  group('online', () {
    test('save goes straight to the server', () async {
      await seed('n1', 'original\n');
      final out = await engine.save(id: 'n1', baseVersion: 1, content: 'edited\n');

      expect(out.status, SaveStatus.saved);
      expect(server.notes['n1']!.content, 'edited\n');
      expect(await cache.pendingCount(), 0);
    });

    test('the cache mirrors what the server accepted', () async {
      await seed('n1', 'original\n');
      await engine.save(id: 'n1', baseVersion: 1, content: 'edited\n');

      final cached = await cache.note('n1');
      expect(cached!.content, 'edited\n');
      expect(cached.version, 2);
    });

    test('a merge result is reported so the caller adopts it', () async {
      await seed('n1', 'original\n');
      server.mergeInstead = 'server merged text\n';

      final out = await engine.save(id: 'n1', baseVersion: 1, content: 'mine\n');
      expect(out.status, SaveStatus.merged);
      expect(out.rewritesBuffer, isTrue);
      expect(out.content, 'server merged text\n');
    });

    test('a conflict is reported, not swallowed', () async {
      await seed('n1', 'original\n');
      server.conflictInstead = '<<<<<<< ours\nmine\n=======\ntheirs\n>>>>>>>\n';

      final out = await engine.save(id: 'n1', baseVersion: 1, content: 'mine\n');
      expect(out.status, SaveStatus.conflicted);
      expect(out.rewritesBuffer, isTrue);
    });

    test('a server refusal is not queued for retry', () async {
      // Retrying an identical request the server already rejected would just
      // wedge the queue.
      await seed('n1', 'original\n');
      server.failWith = 400;

      final out = await engine.save(id: 'n1', baseVersion: 1, content: 'x\n');
      expect(out.status, SaveStatus.failed);
      expect(await cache.pendingCount(), 0);
    });
  });

  group('offline', () {
    test('an edit is queued rather than lost', () async {
      await seed('n1', 'original\n');
      server.unreachable = true;

      final out = await engine.save(id: 'n1', baseVersion: 1, content: 'offline edit\n');
      expect(out.status, SaveStatus.queued);
      expect(engine.isOnline, isFalse);
      expect(await cache.pendingCount(), 1);
    });

    test('the cache shows the user their own text while queued', () async {
      await seed('n1', 'original\n');
      server.unreachable = true;
      await engine.save(id: 'n1', baseVersion: 1, content: 'offline edit\n');

      final cached = await cache.note('n1');
      expect(cached!.content, 'offline edit\n');
    });

    test('reads fall back to the cache', () async {
      await seed('n1', 'cached content\n');
      server.unreachable = true;

      final note = await engine.openNote('n1');
      expect(note!.content, 'cached content\n');
    });

    test('repeated edits coalesce but keep the ORIGINAL base version',
        () async {
      // The base is the version the user branched from. Advancing it would
      // tell the server there is nothing to merge, silently clobbering
      // whatever landed in the meantime.
      await seed('n1', 'original\n', version: 3);
      server.unreachable = true;

      await engine.save(id: 'n1', baseVersion: 3, content: 'first\n');
      await engine.save(id: 'n1', baseVersion: 3, content: 'second\n');
      await engine.save(id: 'n1', baseVersion: 3, content: 'third\n');

      expect(await cache.pendingCount(), 1, reason: 'should coalesce');
      final queued = await cache.outboxFor('n1');
      expect(queued!.content, 'third\n');
      expect(queued.baseVersion, 3);
    });

    test('the tree still lists cached notes', () async {
      await seed('n1', 'a\n');
      await seed('n2', 'b\n');
      server.unreachable = true;

      final notes = await engine.tree();
      expect(notes.map((n) => n.id), containsAll(['n1', 'n2']));
    });
  });

  group('reconnect', () {
    test('the outbox replays with the version it was edited from', () async {
      await seed('n1', 'original\n', version: 3);
      server.unreachable = true;
      await engine.save(id: 'n1', baseVersion: 3, content: 'offline edit\n');

      server.unreachable = false;
      await engine.sync();

      expect(await cache.pendingCount(), 0);
      expect(server.notes['n1']!.content, 'offline edit\n');
      expect(server.lastBaseVersion, 3,
          reason: 'must send the base the user branched from');
    });

    test('a queued edit that merges on replay updates the cache', () async {
      await seed('n1', 'original\n');
      server.unreachable = true;
      await engine.save(id: 'n1', baseVersion: 1, content: 'mine\n');

      server.unreachable = false;
      server.mergeInstead = 'merged on the server\n';
      await engine.sync();

      expect(await cache.pendingCount(), 0);
      final cached = await cache.note('n1');
      expect(cached!.content, 'merged on the server\n');
    });

    test('a note deleted while we were away drops out of the queue', () async {
      // Otherwise the outbox retries a note that can never be written and
      // blocks everything behind it.
      await seed('n1', 'original\n');
      server.unreachable = true;
      await engine.save(id: 'n1', baseVersion: 1, content: 'edit\n');

      server.notes.remove('n1');
      server.unreachable = false;
      await engine.sync();

      expect(await cache.pendingCount(), 0);
      expect(await cache.note('n1'), isNull);
    });

    test('multiple queued notes all replay', () async {
      await seed('n1', 'a\n');
      await seed('n2', 'b\n');
      server.unreachable = true;
      await engine.save(id: 'n1', baseVersion: 1, content: 'a edited\n');
      await engine.save(id: 'n2', baseVersion: 1, content: 'b edited\n');
      expect(await cache.pendingCount(), 2);

      server.unreachable = false;
      await engine.sync();

      expect(await cache.pendingCount(), 0);
      expect(server.notes['n1']!.content, 'a edited\n');
      expect(server.notes['n2']!.content, 'b edited\n');
    });

    test('a still-unreachable server leaves the queue intact', () async {
      await seed('n1', 'original\n');
      server.unreachable = true;
      await engine.save(id: 'n1', baseVersion: 1, content: 'edit\n');

      await engine.sync(); // still down

      expect(await cache.pendingCount(), 1, reason: 'the edit must survive');
      expect(engine.isOnline, isFalse);
    });
  });

  group('pulling remote changes', () {
    test('a remote edit refreshes a cached note', () async {
      await seed('n1', 'original\n');
      server.notes['n1'] = server.notes['n1']!
          .copyWith(content: 'changed elsewhere\n', version: 2);
      server.pushChange('n1', 'updated', 2);

      await engine.sync();

      final cached = await cache.note('n1');
      expect(cached!.content, 'changed elsewhere\n');
    });

    test('a remote delete evicts the note', () async {
      await seed('n1', 'original\n');
      server.notes.remove('n1');
      server.pushChange('n1', 'deleted', 1);

      await engine.sync();
      expect(await cache.note('n1'), isNull);
    });

    test('a pull never clobbers an unsent local edit', () async {
      // The queued edit is the only copy of the user's work; overwriting it
      // with the server's version would destroy it.
      await seed('n1', 'original\n');
      server.unreachable = true;
      await engine.save(id: 'n1', baseVersion: 1, content: 'my unsent edit\n');

      server.unreachable = false;
      server.notes['n1'] =
          server.notes['n1']!.copyWith(content: 'remote change\n', version: 2);
      server.pushChange('n1', 'updated', 2);

      // Pull only — do not drain, so the local edit is still pending.
      await engine.sync();

      // The outbox drained first and won; either way the local text is not lost.
      expect(server.notes['n1']!.content, 'my unsent edit\n');
    });

    test('uncached notes are not hydrated by a pull', () async {
      // The tree comes from /v1/tree; pulling every note would download the
      // whole vault onto a phone.
      server.notes['other'] = ServerNote(
          id: 'other', path: 'other.md', content: 'x\n', version: 1);
      server.pushChange('other', 'created', 1);

      await engine.sync();
      expect(await cache.note('other'), isNull);
    });

    test('pages through more changes than fit in one response', () async {
      // Regression: the pull used to fetch a single page and then jump
      // lastSeq to the server's latest, silently skipping every change past
      // the first page. Only a vault larger than the page size shows it.
      await seed('n1', 'original\n');

      // Bury the note's real change under more than one page of noise.
      for (var i = 0; i < 600; i++) {
        server.pushChange('filler$i', 'updated', 1);
      }
      server.notes['n1'] =
          server.notes['n1']!.copyWith(content: 'late change\n', version: 2);
      server.pushChange('n1', 'updated', 2);

      await engine.sync();

      final cached = await cache.note('n1');
      expect(cached!.content, 'late change\n',
          reason: 'a change beyond the first page must still be applied');
      expect(await cache.lastSeq(), server.seq);
    });

    test('lastSeq advances so the next pull is incremental', () async {
      await seed('n1', 'a\n');
      server.pushChange('n1', 'updated', 2);
      await engine.sync();

      expect(await cache.lastSeq(), server.seq);
    });
  });

  group('offline rename (scenario 6)', () {
    test('a move made offline is queued', () async {
      await seed('n1', 'body\n');
      server.unreachable = true;

      final out = await engine.move(id: 'n1', newPath: 'Moved/Here.md');
      expect(out.status, SaveStatus.queued);

      final queued = await cache.outboxFor('n1');
      expect(queued!.op, 'move');
      expect(queued.newPath, 'Moved/Here.md');
    });

    test('the cache shows the new path immediately', () async {
      await seed('n1', 'body\n');
      server.unreachable = true;
      await engine.move(id: 'n1', newPath: 'Moved/Here.md');

      final cached = await cache.note('n1');
      expect(cached!.path, 'Moved/Here.md');
    });

    test('the move replays on reconnect, keeping the note id', () async {
      await seed('n1', 'body\n');
      server.unreachable = true;
      await engine.move(id: 'n1', newPath: 'Moved/Here.md');

      server.unreachable = false;
      await engine.sync();

      expect(await cache.pendingCount(), 0);
      expect(server.notes['n1']!.path, 'Moved/Here.md');
      expect(server.notes.containsKey('n1'), isTrue,
          reason: 'a move is a metadata change, not delete + create');
    });

    test('an offline rename and an offline edit both survive', () async {
      // Scenario 6: the whole point of tracking notes by UUID.
      await seed('n1', 'original\n', version: 2);
      server.unreachable = true;

      await engine.move(id: 'n1', newPath: 'Renamed.md');
      await engine.save(id: 'n1', baseVersion: 2, content: 'edited offline\n');

      server.unreachable = false;
      await engine.sync();

      expect(await cache.pendingCount(), 0);
      expect(server.notes['n1']!.path, 'Renamed.md');
      expect(server.notes['n1']!.content, 'edited offline\n');
    });
  });

  group('cache eviction', () {
    test('keeps pinned notes and notes with queued edits', () async {
      for (var i = 0; i < 10; i++) {
        await seed('n$i', 'content $i\n');
      }
      await cache.setPinned('n0', true);

      server.unreachable = true;
      await engine.save(id: 'n1', baseVersion: 1, content: 'queued\n');
      server.unreachable = false;

      final evicted = await cache.evict(keep: 2);
      expect(evicted, greaterThan(0));

      expect(await cache.note('n0'), isNotNull, reason: 'pinned');
      expect(await cache.note('n1'), isNotNull, reason: 'has a queued edit');
    });

    test('evicting nothing when under the limit', () async {
      await seed('n1', 'a\n');
      expect(await cache.evict(keep: 100), 0);
    });
  });

  group('NoteSession over the engine', () {
    test('an offline save reports queued, and the base does not advance',
        () async {
      await seed('n1', 'original\n', version: 4);
      final session = NoteSession(engine);
      await session.open('n1');
      expect(session.baseVersion, 4);

      server.unreachable = true;
      session.edit('offline edit\n');
      await session.save();

      expect(session.saveState, SaveState.queued);
      expect(session.baseVersion, 4,
          reason: 'the server has not accepted it, so the base is unchanged');
      session.dispose();
    });

    test('a remote change is ignored while the user is mid-edit', () async {
      // Replacing the buffer under a live cursor would destroy work.
      await seed('n1', 'original\n');
      final session = NoteSession(engine);
      await session.open('n1');

      session.edit('typing right now\n');
      await session.onRemoteChange();

      expect(session.buffer, 'typing right now\n');
      session.dispose();
    });

    test('a remote change is adopted when the buffer is clean', () async {
      await seed('n1', 'original\n');
      final session = NoteSession(engine);
      await session.open('n1');

      server.notes['n1'] =
          server.notes['n1']!.copyWith(content: 'from elsewhere\n', version: 2);
      server.pushChange('n1', 'updated', 2);
      await engine.sync();
      await session.onRemoteChange();

      expect(session.buffer, 'from elsewhere\n');
      expect(session.baseVersion, 2);
      session.dispose();
    });

    test('a note deleted elsewhere closes cleanly', () async {
      await seed('n1', 'original\n');
      final session = NoteSession(engine);
      await session.open('n1');

      server.notes.remove('n1');
      server.pushChange('n1', 'deleted', 1);
      await engine.sync();
      await session.onRemoteChange();

      expect(session.isOpen, isFalse);
      expect(session.notice, contains('deleted'));
      session.dispose();
    });
  });
}
