import 'dart:io';

import 'package:drift/drift.dart' show Value;
import 'package:drift/native.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:sqlite3/sqlite3.dart';

import 'package:storm/cache/cache_db.dart';

/// The v1 → v2 upgrade, run against a database that is actually v1.
///
/// The first version of these tests opened `NativeDatabase.memory()`, which
/// runs `onCreate` and produces a *correct* schema — so it never executed the
/// upgrade path at all and passed while the real upgrade was broken on the
/// phone. A migration test that never migrates is not a migration test.
///
/// This builds the old schema by hand, stamps `user_version = 1`, and then
/// opens `CacheDb` over it, which is what a phone upgrading from the previous
/// build does.
void main() {
  late Directory dir;
  late File file;

  setUp(() {
    dir = Directory.systemTemp.createTempSync('storm-cache-migration');
    file = File('${dir.path}/cache.sqlite');

    // Exactly what drift generated for schemaVersion 1: no vault column
    // anywhere, and the note id alone as the primary key.
    final raw = sqlite3.open(file.path);
    raw.execute('''
      CREATE TABLE cached_notes (
        id TEXT NOT NULL,
        path TEXT NOT NULL,
        title TEXT NOT NULL DEFAULT '',
        version INTEGER NOT NULL,
        content TEXT NOT NULL,
        modified TEXT NOT NULL DEFAULT '',
        cached_at INTEGER NOT NULL,
        pinned INTEGER NOT NULL DEFAULT 0,
        PRIMARY KEY (id)
      );
      CREATE TABLE outbox (
        note_id TEXT NOT NULL,
        base_version INTEGER NOT NULL,
        content TEXT NOT NULL,
        op TEXT NOT NULL DEFAULT 'update',
        new_path TEXT NULL,
        queued_at INTEGER NOT NULL,
        PRIMARY KEY (note_id)
      );
      CREATE TABLE meta (
        key TEXT NOT NULL,
        value TEXT NOT NULL,
        PRIMARY KEY (key)
      );
      PRAGMA user_version = 1;
    ''');
    raw.execute(
      "INSERT INTO cached_notes (id, path, title, version, content, modified, "
      "cached_at, pinned) VALUES ('n1', 'Welcome.md', 'Welcome', 11, "
      "'# Welcome', '2026-08-05T10:00:00Z', 1000, 0)",
    );
    raw.execute(
      "INSERT INTO outbox (note_id, base_version, content, op, queued_at) "
      "VALUES ('n1', 11, 'an edit that exists nowhere else', 'update', 1000)",
    );
    raw.execute("INSERT INTO meta (key, value) VALUES ('lastSeq', '110')");
    raw.close();
  });

  tearDown(() => dir.deleteSync(recursive: true));

  CacheDb open() => CacheDb(NativeDatabase(file));

  test('a note can still be cached after the upgrade', () async {
    // The failure the phone hit. `addColumn` cannot change a primary key, so
    // the table kept v1's `PRIMARY KEY(id)` while drift generated
    // `ON CONFLICT(vault_id, id)` — which SQLite rejects outright. Every
    // cache write then threw, and because `SyncEngine.create` wrapped its
    // cache write in the same `try` as the request, a local schema error was
    // reported as "cannot reach the server" and marked the client offline.
    final db = open();
    addTearDown(db.close);

    await db.putNote(
      CachedNotesCompanion.insert(
        vaultId: const Value('v-real'),
        id: 'n2',
        path: 'New.md',
        version: 1,
        content: 'body',
        cachedAt: DateTime.now(),
      ),
    );

    expect(await db.note('v-real', 'n2'), isNotNull);
  });

  test('the same note id can exist in two vaults afterwards', () async {
    final db = open();
    addTearDown(db.close);

    for (final vault in ['v-a', 'v-b']) {
      await db.putNote(
        CachedNotesCompanion.insert(
          vaultId: Value(vault),
          id: 'same-id',
          path: 'Daily/2026-08-07.md',
          version: 1,
          content: 'from $vault',
          cachedAt: DateTime.now(),
        ),
      );
    }

    expect((await db.note('v-a', 'same-id'))?.content, 'from v-a');
    expect((await db.note('v-b', 'same-id'))?.content, 'from v-b');
  });

  test('a queued edit survives the upgrade and can be re-queued', () async {
    final db = open();
    addTearDown(db.close);

    // The row is still there, attributed to the sentinel.
    final carried = await db.outboxFor(kLegacyVault, 'n1');
    expect(
      carried?.content,
      'an edit that exists nowhere else',
      reason: 'an outbox row is the only copy of that edit',
    );
    expect(carried?.baseVersion, 11);

    // And the outbox still takes writes — same conflict-target problem.
    await db.enqueue(
      vaultId: 'v-real',
      noteId: 'n2',
      baseVersion: 3,
      content: 'later',
    );
    expect((await db.outboxFor('v-real', 'n2'))?.content, 'later');
  });

  test('cached notes and their pinned state carry over', () async {
    final db = open();
    addTearDown(db.close);

    final held = await db.note(kLegacyVault, 'n1');
    expect(held, isNotNull, reason: 'the upgrade must not drop cached notes');
    expect(held!.path, 'Welcome.md');
    expect(held.version, 11, reason: 'the base version must survive');
    expect(held.content, '# Welcome');
  });

  test('adopting moves the legacy rows and the sync cursor', () async {
    final db = open();
    addTearDown(db.close);

    expect(
      await db.lastSeq(kLegacyVault),
      0,
      reason: 'v1 stored it under a bare `lastSeq` key, not a scoped one',
    );

    final moved = await db.adoptLegacyRows('v-real');
    expect(moved, 2, reason: 'one cached note and one queued edit');

    expect(await db.allNotes('v-real'), hasLength(1));
    expect((await db.outboxFor('v-real', 'n1'))?.baseVersion, 11);
    expect(
      await db.lastSeq('v-real'),
      110,
      reason: 'losing the cursor would re-pull the whole change log',
    );
    expect(await db.legacyRowCount(), 0);
  });

  test('the recents table exists after the upgrade', () async {
    final db = open();
    addTearDown(db.close);
    expect(await db.recentNotes(), isEmpty);
  });

  group('repairing a device that already ran the broken v2', () {
    late Directory brokenDir;
    late File brokenFile;

    setUp(() {
      // What v2's `addColumn` actually produced: the vault column is there,
      // but the primary key never moved. Writes fail against this, and
      // `user_version = 2` means no further upgrade would ever run — the
      // device is stuck unless a later version repairs it.
      brokenDir = Directory.systemTemp.createTempSync('storm-cache-broken');
      brokenFile = File('${brokenDir.path}/cache.sqlite');

      final raw = sqlite3.open(brokenFile.path);
      raw.execute('''
        CREATE TABLE cached_notes (
          id TEXT NOT NULL,
          path TEXT NOT NULL,
          title TEXT NOT NULL DEFAULT '',
          version INTEGER NOT NULL,
          content TEXT NOT NULL,
          modified TEXT NOT NULL DEFAULT '',
          cached_at INTEGER NOT NULL,
          pinned INTEGER NOT NULL DEFAULT 0,
          vault_id TEXT NOT NULL DEFAULT 'legacy',
          PRIMARY KEY (id)
        );
        CREATE TABLE outbox (
          note_id TEXT NOT NULL,
          base_version INTEGER NOT NULL,
          content TEXT NOT NULL,
          op TEXT NOT NULL DEFAULT 'update',
          new_path TEXT NULL,
          queued_at INTEGER NOT NULL,
          vault_id TEXT NOT NULL DEFAULT 'legacy',
          PRIMARY KEY (note_id)
        );
        CREATE TABLE meta (
          key TEXT NOT NULL, value TEXT NOT NULL, PRIMARY KEY (key)
        );
        CREATE TABLE recents (
          vault_id TEXT NOT NULL,
          note_id TEXT NOT NULL,
          vault_name TEXT NOT NULL DEFAULT '',
          path TEXT NOT NULL DEFAULT '',
          title TEXT NOT NULL DEFAULT '',
          opened_at INTEGER NOT NULL,
          PRIMARY KEY (vault_id, note_id)
        );
        PRAGMA user_version = 2;
      ''');
      // A row that already got a real vault id before the writes started
      // failing — the repair must not stamp it back to the sentinel.
      raw.execute(
        "INSERT INTO cached_notes (id, path, version, content, cached_at, "
        "vault_id) VALUES ('n1', 'Welcome.md', 11, '# Welcome', 1000, "
        "'v-real')",
      );
      raw.execute(
        "INSERT INTO outbox (note_id, base_version, content, queued_at, "
        "vault_id) VALUES ('n1', 11, 'unsent', 1000, 'v-real')",
      );
      raw.close();
    });

    tearDown(() => brokenDir.deleteSync(recursive: true));

    test('writes work again', () async {
      final db = CacheDb(NativeDatabase(brokenFile));
      addTearDown(db.close);

      await db.putNote(
        CachedNotesCompanion.insert(
          vaultId: const Value('v-real'),
          id: 'n2',
          path: 'New.md',
          version: 1,
          content: 'body',
          cachedAt: DateTime.now(),
        ),
      );
      expect(await db.note('v-real', 'n2'), isNotNull);
    });

    test('rows that already had a real vault keep it', () async {
      final db = CacheDb(NativeDatabase(brokenFile));
      addTearDown(db.close);

      expect(await db.note('v-real', 'n1'), isNotNull);
      expect(await db.note(kLegacyVault, 'n1'), isNull);
      expect(
        (await db.outboxFor('v-real', 'n1'))?.content,
        'unsent',
        reason: 'the repair must not lose an unsent edit or misfile it',
      );
    });
  });
}
