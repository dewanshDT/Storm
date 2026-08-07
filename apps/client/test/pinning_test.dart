import 'package:drift/native.dart';
import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:storm/api/storm_api.dart';
import 'package:storm/cache/cache_db.dart';
import 'package:storm/state/app_state.dart';
import 'package:storm/sync/sync_engine.dart';

import 'package:storm/router.dart';

import 'fake_server.dart';
import 'shell_harness.dart';

/// Keeping notes available offline, and reaching the actions that do it.
void main() {
  late CacheDb cache;
  late FakeServer server;
  late SyncEngine engine;

  setUp(() {
    cache = CacheDb(NativeDatabase.memory());
    server = FakeServer();
    server.notes['n1'] = ServerNote(
      id: 'n1',
      path: 'Keep.md',
      content: '# Keep\n',
      version: 1,
    );
    engine = SyncEngine(
      api: StormApi(baseUrl: 'http://test', token: 't', client: server.client),
      cache: cache,
      vaultId: FakeServer.primaryVault,
    );
  });

  tearDown(() async {
    engine.dispose();
    await cache.close();
  });

  group('pinning', () {
    test('a pinned note is fetched immediately', () async {
      // Pinning something only seen in the tree has to actually bring it
      // down, or "available offline" is a lie.
      expect(await cache.note(FakeServer.primaryVault, 'n1'), isNull);

      await engine.setPinned('n1', true);

      final cached = await cache.note(FakeServer.primaryVault, 'n1');
      expect(cached, isNotNull);
      expect(cached!.pinned, isTrue);
      expect(cached.content, contains('# Keep'));
    });

    test('pinned ids are reported', () async {
      await engine.setPinned('n1', true);
      expect(await engine.pinnedIds(), {'n1'});

      await engine.setPinned('n1', false);
      expect(await engine.pinnedIds(), isEmpty);
    });

    test('eviction spares pinned notes', () async {
      // The whole point: a pinned note outlives the LRU window.
      for (var i = 0; i < 12; i++) {
        server.notes['x$i'] = ServerNote(
          id: 'x$i',
          path: 'x$i.md',
          content: 'body $i\n',
          version: 1,
        );
        await engine.openNote('x$i');
      }
      await engine.setPinned('x0', true);

      final evicted = await cache.evict(FakeServer.primaryVault, keep: 3);
      expect(evicted, greaterThan(0));
      expect(
        await cache.note(FakeServer.primaryVault, 'x0'),
        isNotNull,
        reason: 'pinned',
      );
    });

    test('pinning reaches the change stream so the UI refreshes', () async {
      final seen = <Set<String>>[];
      final sub = engine.changes.listen(seen.add);

      await engine.setPinned('n1', true);
      await Future.delayed(Duration.zero);

      expect(seen.any((s) => s.contains('n1')), isTrue);
      await sub.cancel();
    });
  });

  group('the note actions stay reachable', () {
    /// Opens n0 and returns with its actions menu on screen.
    Future<void> openMenu(WidgetTester tester, ProviderContainer c) async {
      c.read(routerProvider).go(Routes.note(FakeServer.primaryVault, 'n0'));
      await tester.pumpAndSettle();
      await tester.tap(find.byTooltip('Note actions'));
      await tester.pumpAndSettle();
    }

    for (final (name, size) in [
      ('a phone', const Size(411, 900)),
      ('a wide screen', const Size(1400, 900)),
    ]) {
      testWidgets('all four are in the menu on $name', (tester) async {
        // One menu at every width, deliberately. The old shell split these
        // into loose app-bar icons above a breakpoint, and an overflowing
        // AppBar drops what doesn't fit *silently* — which is how the attach
        // button looked like it had gone missing.
        final c = shellContainer();
        await pumpShell(tester, c, size: size);
        await openMenu(tester, c);

        expect(tester.takeException(), isNull, reason: 'no layout overflow');
        expect(find.text('Keep offline'), findsOneWidget);
        expect(find.text('Attach a file'), findsOneWidget);
        expect(find.text('Rename or move'), findsOneWidget);
        expect(find.text('Delete'), findsOneWidget);

        await disposeShell(tester, c);
      });
    }

    testWidgets('pinning from the menu reaches the engine', (tester) async {
      final c = shellContainer();
      await pumpShell(tester, c);
      await openMenu(tester, c);

      await tester.tap(find.text('Keep offline'));
      await tester.pumpAndSettle();

      expect(await c.read(syncEngineProvider).pinnedIds(), contains('n0'));
      // And the label flips, so the menu says what tapping it again will do.
      await tester.tap(find.byTooltip('Note actions'));
      await tester.pumpAndSettle();
      expect(find.text('Stop keeping offline'), findsOneWidget);

      await disposeShell(tester, c);
    });

    testWidgets('outside a note there are no note actions', (tester) async {
      final c = shellContainer();
      await pumpShell(tester, c);

      expect(find.byTooltip('Note actions'), findsNothing);
      await disposeShell(tester, c);
    });
  });
}
