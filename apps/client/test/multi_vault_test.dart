import 'package:drift/drift.dart' show Value;
import 'package:storm/api/storm_api.dart';
import 'package:storm/sync/sync_engine.dart';
import 'package:drift/native.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:storm/cache/cache_db.dart';
import 'package:storm/router.dart';
import 'package:storm/state/app_state.dart';

import 'fake_server.dart';
import 'shell_harness.dart';

/// Multiple vaults.
///
/// The hazards worth guarding are all the same shape: something that used to
/// be global — a note id, a sync cursor, a cached row — now has to be scoped,
/// and a miss shows one vault's data under another's name rather than
/// erroring.
void main() {
  group('the cache is scoped to a vault', () {
    late CacheDb cache;

    setUp(() => cache = CacheDb(NativeDatabase.memory()));
    tearDown(() => cache.close());

    test('two vaults can hold the same note path without colliding', () async {
      // The collision one shared index could not have represented: most
      // vaults have a `Daily/2026-08-07.md`.
      for (final vault in ['v-a', 'v-b']) {
        await cache.putNote(
          CachedNotesCompanion.insert(
            vaultId: Value(vault),
            id: 'note-in-$vault',
            path: 'Daily/2026-08-07.md',
            version: 1,
            content: 'from $vault',
            cachedAt: DateTime.now(),
          ),
        );
      }

      final a = await cache.allNotes('v-a');
      final b = await cache.allNotes('v-b');
      expect(a, hasLength(1));
      expect(b, hasLength(1));
      expect(a.single.content, 'from v-a');
      expect(b.single.content, 'from v-b');
    });

    test('the sync cursor is per vault', () async {
      // One shared key would have two vaults overwriting each other's
      // position, which surfaces as randomly missed changes, not an error.
      await cache.setLastSeq('v-a', 42);
      await cache.setLastSeq('v-b', 7);

      expect(await cache.lastSeq('v-a'), 42);
      expect(await cache.lastSeq('v-b'), 7);
      expect(await cache.lastSeq('never-seen'), 0);
    });

    test('one vault cannot evict another vault-s notes', () async {
      for (var i = 0; i < 5; i++) {
        await cache.putNote(
          CachedNotesCompanion.insert(
            vaultId: const Value('v-b'),
            id: 'b$i',
            path: 'B$i.md',
            version: 1,
            content: 'x',
            cachedAt: DateTime.now(),
          ),
        );
      }
      await cache.evict('v-a', keep: 0);
      expect(await cache.allNotes('v-b'), hasLength(5));
    });

    test('a queued edit is keyed by vault as well as note', () async {
      await cache.enqueue(
        vaultId: 'v-a',
        noteId: 'shared-id',
        baseVersion: 1,
        content: 'from a',
      );
      await cache.enqueue(
        vaultId: 'v-b',
        noteId: 'shared-id',
        baseVersion: 1,
        content: 'from b',
      );

      expect((await cache.outboxFor('v-a', 'shared-id'))?.content, 'from a');
      expect((await cache.outboxFor('v-b', 'shared-id'))?.content, 'from b');
      expect(await cache.pendingCount('v-a'), 1);
    });
  });

  group('the legacy migration', () {
    test('adopts pre-multi-vault rows when there is one vault', () async {
      final cache = CacheDb(NativeDatabase.memory());
      addTearDown(cache.close);

      // What a v1 cache looked like: no vault on anything.
      await cache.putNote(
        CachedNotesCompanion.insert(
          vaultId: const Value(kLegacyVault),
          id: 'n1',
          path: 'A.md',
          version: 2,
          content: 'held',
          cachedAt: DateTime.now(),
        ),
      );
      await cache.enqueue(
        vaultId: kLegacyVault,
        noteId: 'n1',
        baseVersion: 2,
        content: 'an edit that exists nowhere else',
      );

      final moved = await cache.adoptLegacyRows('v-real');
      expect(moved, 2);

      expect(await cache.allNotes('v-real'), hasLength(1));
      final queued = await cache.outboxFor('v-real', 'n1');
      expect(
        queued?.content,
        'an edit that exists nowhere else',
        reason: 'discarding a queued edit to simplify a migration is data loss',
      );
      expect(
        queued?.baseVersion,
        2,
        reason: 'the base the user branched from must survive',
      );
    });

    test('leaves rows alone rather than guessing a vault', () async {
      final cache = CacheDb(NativeDatabase.memory());
      addTearDown(cache.close);

      await cache.enqueue(
        vaultId: kLegacyVault,
        noteId: 'n1',
        baseVersion: 1,
        content: 'unattributable',
      );

      // Adopting into the sentinel is a no-op, so a caller that cannot tell
      // which vault the rows belong to cannot silently misfile them.
      expect(await cache.adoptLegacyRows(kLegacyVault), 0);
      expect(await cache.legacyRowCount(), 1);
    });
  });

  group('switching vaults', () {
    testWidgets('never renders the previous vault-s notes', (tester) async {
      // The frame `VaultGate` exists to prevent: the route says one vault
      // while the providers still hold the other's tree.
      final c = shellContainer();
      final server = serverOf(c);
      final second = server.addVault('v-second', 'Second');
      second['s0'] = ServerNote(
        id: 's0',
        path: 'OnlyInSecond.md',
        content: '# other\n',
        version: 1,
      );

      await pumpShell(tester, c);
      await openVault(tester, c);
      expect(find.text('Welcome'), findsOneWidget);

      c.read(routerProvider).go(Routes.browse('v-second'));
      await tester.pumpAndSettle();

      expect(find.text('OnlyInSecond'), findsOneWidget);
      expect(
        find.text('Welcome'),
        findsNothing,
        reason: 'the first vault-s notes must not survive the switch',
      );
      expect(tester.takeException(), isNull);

      await disposeShell(tester, c);
    });

    testWidgets('shows nothing rather than stale notes on the first frame', (
      tester,
    ) async {
      // This is the frame `VaultGate` exists for, and `pumpAndSettle` hides
      // it. Navigating rebuilds the route synchronously, but the active vault
      // is only adopted after the frame — so for one frame the route says one
      // vault while the providers still hold the other's tree.
      //
      // Deleting the gate's guard makes this test fail; the settled version
      // above keeps passing, which is why both exist.
      final c = shellContainer();
      final server = serverOf(c);
      final second = server.addVault('v-second', 'Second');
      second['s0'] = ServerNote(
        id: 's0',
        path: 'OnlyInSecond.md',
        content: '# other\n',
        version: 1,
      );

      await pumpShell(tester, c);
      await openVault(tester, c);
      expect(find.text('Welcome'), findsOneWidget, reason: 'precondition');

      c.read(routerProvider).go(Routes.browse('v-second'));
      await tester.pump();

      expect(
        find.text('Welcome'),
        findsNothing,
        reason:
            'the previous vault-s notes must never appear under the new '
            'vault-s route, not even for one frame',
      );

      await tester.pumpAndSettle();
      await disposeShell(tester, c);
    });

    testWidgets('rebuilds the engine, so its socket does not leak', (
      tester,
    ) async {
      final c = shellContainer();
      serverOf(c).addVault('v-second', 'Second');
      await pumpShell(tester, c);
      await openVault(tester, c);

      final first = c.read(syncEngineProvider);
      expect(first.vaultId, FakeServer.primaryVault);

      c.read(routerProvider).go(Routes.browse('v-second'));
      await tester.pumpAndSettle();

      final second = c.read(syncEngineProvider);
      expect(second.vaultId, 'v-second');
      expect(
        identical(first, second),
        isFalse,
        reason: 'a reused engine would keep the old vault-s cursor and socket',
      );

      await disposeShell(tester, c);
    });
  });

  group('the dashboard', () {
    testWidgets('shows a card per vault and lays out at phone width', (
      tester,
    ) async {
      final c = shellContainer();
      serverOf(c).addVault('v-second', 'Work');
      await pumpShell(tester, c);

      expect(find.text('Primary'), findsOneWidget);
      expect(find.text('Work'), findsOneWidget);
      expect(find.text('Recently opened'), findsOneWidget);
      // The check that caught the AppBar quietly dropping the attach button.
      expect(tester.takeException(), isNull);

      await disposeShell(tester, c);
    });

    testWidgets('recents name the vault each note came from', (tester) async {
      final c = shellContainer();
      final server = serverOf(c);
      final second = server.addVault('v-second', 'Work');
      second['s0'] = ServerNote(
        id: 's0',
        path: 'Standup.md',
        content: '# s\n',
        version: 1,
      );
      server.markOpened(FakeServer.primaryVault, 'n0', '2026-08-07T09:00:00Z');
      server.markOpened('v-second', 's0', '2026-08-07T11:00:00Z');

      await pumpShell(tester, c);

      // Newest first, and each carries the vault — which is what tells two
      // identically-named daily notes apart.
      expect(find.text('Work'), findsWidgets);
      expect(find.text('s0'), findsOneWidget);

      await disposeShell(tester, c);
    });

    testWidgets('a vault whose directory is gone is shown, not hidden', (
      tester,
    ) async {
      final c = shellContainer();
      serverOf(
        c,
      ).vaults.add(ServerVault(id: 'v-gone', name: 'Archive', missing: true));
      serverOf(c).byVault['v-gone'] = {};
      await pumpShell(tester, c);

      expect(find.text('Archive'), findsOneWidget);
      expect(
        find.text('Directory not found'),
        findsOneWidget,
        reason:
            'a vault that vanished from the list looks like one that '
            'never existed',
      );

      await disposeShell(tester, c);
    });
  });

  offlineConflation();

  group('folders', () {
    testWidgets('a new folder appears in the browser', (tester) async {
      final c = shellContainer();
      await pumpShell(tester, c);
      await openVault(tester, c);

      expect(find.text('Archive'), findsNothing);

      await tester.tap(find.byIcon(Icons.more_horiz));
      await tester.pumpAndSettle();
      await tester.tap(find.byTooltip('New folder'));
      await tester.pumpAndSettle();

      await tester.enterText(find.byType(TextField).last, 'Archive');
      await tester.tap(find.text('OK'));
      await tester.pumpAndSettle();

      expect(serverOf(c).folders[FakeServer.primaryVault], contains('Archive'));
      expect(find.text('Archive'), findsOneWidget);
      expect(tester.takeException(), isNull);

      await disposeShell(tester, c);
    });

    testWidgets('a folder name is not given a .md extension', (tester) async {
      // The note dialog appends `.md`; reusing it unchanged would have made
      // every new folder `Archive.md`.
      final c = shellContainer();
      await pumpShell(tester, c);
      await openVault(tester, c);

      await tester.tap(find.byIcon(Icons.more_horiz));
      await tester.pumpAndSettle();
      await tester.tap(find.byTooltip('New folder'));
      await tester.pumpAndSettle();
      await tester.enterText(find.byType(TextField).last, 'Archive');
      await tester.tap(find.text('OK'));
      await tester.pumpAndSettle();

      expect(
        serverOf(c).folders[FakeServer.primaryVault],
        isNot(contains('Archive.md')),
      );

      await disposeShell(tester, c);
    });

    testWidgets('renaming a folder rewrites the notes under it', (
      tester,
    ) async {
      final c = shellContainer();
      await pumpShell(tester, c);
      await openVault(tester, c);

      await tester.longPress(find.text('Projects'));
      await tester.pumpAndSettle();
      await tester.tap(find.textContaining('Rename'));
      await tester.pumpAndSettle();

      await tester.enterText(find.byType(TextField).last, 'Work');
      await tester.tap(find.text('OK'));
      await tester.pumpAndSettle();

      final paths = serverOf(c).notes.values.map((n) => n.path).toList();
      expect(paths, contains('Work/Ideas.md'));
      expect(paths, contains('Work/Storm/Design.md'));
      expect(paths.every((p) => !p.startsWith('Projects/')), isTrue);

      await disposeShell(tester, c);
    });

    testWidgets('deleting a folder with notes is refused, not obeyed', (
      tester,
    ) async {
      final c = shellContainer();
      await pumpShell(tester, c);
      await openVault(tester, c);

      await tester.longPress(find.text('Projects'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Delete folder'));
      await tester.pumpAndSettle();

      // Nothing is taken with it — the server refuses and says why.
      final paths = serverOf(c).notes.values.map((n) => n.path);
      expect(paths, contains('Projects/Ideas.md'));
      expect(find.byType(SnackBar), findsOneWidget);

      await disposeShell(tester, c);
    });
  });
}

/// A failing *local* cache must never be reported as a failing *server*.
///
/// This is the shape that put "Cannot reach the server" in front of a note the
/// server had already created, and then made every later read fail as offline.
class _BrokenCache extends CacheDb {
  _BrokenCache() : super(NativeDatabase.memory());

  @override
  Future<void> putNote(CachedNotesCompanion note) async =>
      throw Exception('simulated cache failure');
}

void offlineConflation() {
  test('creating a note succeeds even when the cache write fails', () async {
    final cache = _BrokenCache();
    addTearDown(cache.close);
    final server = FakeServer();
    final engine = SyncEngine(
      api: StormApi(baseUrl: 'http://test', token: 't', client: server.client),
      cache: cache,
      vaultId: FakeServer.primaryVault,
    );
    addTearDown(engine.dispose);

    final created = await engine.create(path: 'New.md');

    expect(
      created.error,
      isNull,
      reason:
          'the server created it; a local cache problem is not an error '
          'the user can act on',
    );
    expect(created.meta, isNotNull);
    expect(
      engine.isOnline,
      isTrue,
      reason:
          'a cache failure must not mark the client offline — everything '
          'after it then fails as offline too',
    );
  });

  test('a note still opens when it cannot be cached', () async {
    final cache = _BrokenCache();
    addTearDown(cache.close);
    final server = FakeServer();
    server.notes['n1'] = ServerNote(
      id: 'n1',
      path: 'A.md',
      content: '# A',
      version: 1,
    );
    final engine = SyncEngine(
      api: StormApi(baseUrl: 'http://test', token: 't', client: server.client),
      cache: cache,
      vaultId: FakeServer.primaryVault,
    );
    addTearDown(engine.dispose);

    final opened = await engine.openNote('n1');
    expect(
      opened?.content,
      '# A',
      reason:
          'returning null here renders as "not available offline yet" for '
          'a note the server just handed us',
    );
    expect(engine.isOnline, isTrue);
  });

  test('sync advances its cursor even when caching fails', () async {
    // The stall behind "sync got slow": a cache failure left `lastSeq` where
    // it was, so every sync re-pulled the same page and failed the same way.
    final cache = _BrokenCache();
    addTearDown(cache.close);
    final server = FakeServer();
    server.notes['n1'] = ServerNote(
      id: 'n1',
      path: 'A.md',
      content: '# A',
      version: 1,
    );
    server.pushChange('n1', 'updated', 2);

    final engine = SyncEngine(
      api: StormApi(baseUrl: 'http://test', token: 't', client: server.client),
      cache: cache,
      vaultId: FakeServer.primaryVault,
    );
    addTearDown(engine.dispose);

    await engine.sync();

    expect(await cache.lastSeq(FakeServer.primaryVault), server.seq);
    expect(engine.isOnline, isTrue);
  });
}
