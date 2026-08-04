import 'dart:io';

import 'package:drift/native.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:storm/api/models.dart';
import 'package:storm/api/storm_api.dart';
import 'package:storm/cache/cache_db.dart';
import 'package:storm/state/note_session.dart';
import 'package:storm/sync/sync_engine.dart';

/// Client-against-real-server integration test.
///
/// Unit tests use a mocked HTTP client, which proves the client behaves
/// correctly against the responses we *think* the server sends. This proves it
/// against the ones it actually sends — the gap where a field-name or
/// semantics mismatch would otherwise hide until runtime.
///
/// Lives outside `test/` so a plain `flutter test` doesn't need a server.
/// Run it explicitly, with a server up:
///
///   cargo run -- --vault /tmp/v --state /tmp/s --token testtoken --port 8484
///   flutter test test_live/
void main() {
  const baseUrl = 'http://127.0.0.1:8484';
  const token = 'testtoken';
  late StormApi api;

  setUpAll(() async {
    try {
      final socket = await Socket.connect('127.0.0.1', 8484,
          timeout: const Duration(seconds: 2));
      socket.destroy();
    } catch (_) {
      fail('No server on 127.0.0.1:8484 — start storm-server first.');
    }
  });

  late CacheDb cache;
  late SyncEngine engine;

  setUp(() {
    api = StormApi(baseUrl: baseUrl, token: token);
    cache = CacheDb(NativeDatabase.memory());
    engine = SyncEngine(api: api, cache: cache);
  });

  tearDown(() async {
    engine.dispose();
    await cache.close();
    api.dispose();
  });

  /// Unique per run so repeated runs don't collide.
  String scratch(String name) =>
      'ClientTest/${DateTime.now().microsecondsSinceEpoch}-$name.md';

  test('rejects a bad token the way the client expects', () async {
    final bad = StormApi(baseUrl: baseUrl, token: 'wrong');
    await expectLater(
      bad.tree(),
      throwsA(isA<StormApiException>()
          .having((e) => e.isUnauthorized, 'isUnauthorized', isTrue)),
    );
    bad.dispose();
  });

  test('parses the vault tree the server actually returns', () async {
    final tree = await api.tree();
    expect(tree.seq, greaterThanOrEqualTo(0));
    for (final note in tree.notes) {
      expect(note.id, isNotEmpty);
      expect(note.path, endsWith('.md'));
      expect(note.version, greaterThan(0));
    }
  });

  test('create, read, edit, save round-trips through the real server', () async {
    final path = scratch('roundtrip');
    final created = await api.createNote(path: path, content: '# Round trip\n');
    expect(created.meta.path, path);

    // The server stamps identity into the file; the client must see it.
    final fetched = await api.note(created.meta.id);
    expect(fetched.content, contains('id:'));
    expect(fetched.content, contains('# Round trip'));

    final session = NoteSession(engine);
    await session.open(created.meta.id);
    expect(session.baseVersion, fetched.meta.version);

    session.edit('${session.buffer}\nAdded by the client.\n');
    await session.save();

    expect(session.saveState, SaveState.saved);
    expect(session.error, isNull);

    final reread = await api.note(created.meta.id);
    expect(reread.content, contains('Added by the client.'));
    expect(reread.meta.version, greaterThan(fetched.meta.version));

    await api.deleteNote(created.meta.id);
  });

  test('a stale save is merged by the server and adopted by the client',
      () async {
    final path = scratch('merge');
    final created = await api.createNote(
      path: path,
      content: '# Merge\n\nAlpha.\n\nBeta.\n\nGamma.\n\nDelta.\n\nEpsilon.\n',
    );

    final session = NoteSession(engine);
    await session.open(created.meta.id);
    final staleBase = session.baseVersion;
    final original = session.buffer;

    // Another device writes first, editing a far-away region.
    await api.saveNote(
      id: created.meta.id,
      baseVersion: staleBase,
      content: original.replaceAll('Alpha.', 'Alpha from desktop.'),
    );

    // Our session is now stale and edits a different region.
    session.edit(original.replaceAll('Epsilon.', 'Epsilon from phone.'));
    await session.save();

    expect(session.error, isNull);
    expect(session.hasConflict, isFalse,
        reason: 'non-overlapping edits should merge cleanly');
    expect(session.buffer, contains('Alpha from desktop.'));
    expect(session.buffer, contains('Epsilon from phone.'));
    expect(session.baseVersion, greaterThan(staleBase),
        reason: 'the client must adopt the version the server returned');

    await api.deleteNote(created.meta.id);
  });

  test('an overlapping save conflicts and the client shows both sides',
      () async {
    final path = scratch('conflict');
    final created =
        await api.createNote(path: path, content: '# Conflict\n\nShared.\n');

    final session = NoteSession(engine);
    await session.open(created.meta.id);
    final staleBase = session.baseVersion;
    final original = session.buffer;

    await api.saveNote(
      id: created.meta.id,
      baseVersion: staleBase,
      content: original.replaceAll('Shared.', 'Desktop wins.'),
    );

    session.edit(original.replaceAll('Shared.', 'Phone wins.'));
    await session.save();

    expect(session.hasConflict, isTrue);
    expect(session.buffer, contains('<<<<<<<'));
    expect(session.buffer, contains('Desktop wins.'));
    expect(session.buffer, contains('Phone wins.'));
    // Orientation: the client's own edit is the `ours` side.
    expect(
      session.buffer.indexOf('Phone wins.'),
      lessThan(session.buffer.indexOf('=======')),
    );

    await api.deleteNote(created.meta.id);
  });

  test('move preserves note identity', () async {
    final from = scratch('move-from');
    final created = await api.createNote(path: from, content: '# Move me\n');
    final to = scratch('move-to');

    final moved = await api.moveNote(id: created.meta.id, newPath: to);
    expect(moved.meta.id, created.meta.id);
    expect(moved.meta.path, to);

    final fetched = await api.note(created.meta.id);
    expect(fetched.meta.path, to);

    await api.deleteNote(created.meta.id);
  });

  test('search returns hits the client can render', () async {
    final path = scratch('search');
    final marker = 'zqxjkvw${DateTime.now().microsecondsSinceEpoch}';
    final created =
        await api.createNote(path: path, content: '# Search\n\n$marker\n');

    final hits = await api.search(marker);
    expect(hits, hasLength(1));
    expect(hits.single.id, created.meta.id);
    expect(hits.single.snippet, contains('<<'),
        reason: 'the client renders <<..>> as highlight spans');

    await api.deleteNote(created.meta.id);
  });

  test('search tolerates punctuation without erroring', () async {
    // FTS5 would treat these as syntax; the server sanitizes them.
    for (final query in ['foo-bar', 'NEAR(a b)', 'a "quoted" b', '((']) {
      await expectLater(api.search(query), completes, reason: query);
    }
  });

  test('backlinks resolve through the real index', () async {
    final targetPath = scratch('backlink-target');
    final target = await api.createNote(
      path: targetPath,
      content: '# BacklinkTarget\n',
    );
    final sourcePath = scratch('backlink-source');
    final source = await api.createNote(
      path: sourcePath,
      content: '# Source\n\nSee [[BacklinkTarget]].\n',
    );

    final back = await api.backlinks(target.meta.id);
    expect(back.map((n) => n.id), contains(source.meta.id));

    await api.deleteNote(source.meta.id);
    await api.deleteNote(target.meta.id);
  });

  test('deleting a note makes it 404 afterwards', () async {
    final created =
        await api.createNote(path: scratch('delete'), content: '# Bye\n');
    await api.deleteNote(created.meta.id);

    await expectLater(
      api.note(created.meta.id),
      throwsA(isA<StormApiException>()
          .having((e) => e.isNotFound, 'isNotFound', isTrue)),
    );
  });

  test('creating over an existing path is refused', () async {
    final path = scratch('dupe');
    final created = await api.createNote(path: path, content: '# One\n');
    await expectLater(
      api.createNote(path: path, content: '# Two\n'),
      throwsA(isA<StormApiException>()),
    );
    await api.deleteNote(created.meta.id);
  });

  test('unicode content survives the round trip', () async {
    // Guards the utf8.decode in StormApi: reading r.body instead would
    // mangle this into latin-1 mojibake.
    const content = '# 日本語\n\nEmoji 🎉 and accents: café, naïve.\n';
    final created =
        await api.createNote(path: scratch('unicode'), content: content);

    final fetched = await api.note(created.meta.id);
    expect(fetched.content, contains('日本語'));
    expect(fetched.content, contains('🎉'));
    expect(fetched.content, contains('café'));

    await api.deleteNote(created.meta.id);
  });
}
