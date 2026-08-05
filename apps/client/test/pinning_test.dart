import 'package:drift/native.dart';
import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:storm/api/storm_api.dart';
import 'package:storm/cache/cache_db.dart';
import 'package:storm/state/app_state.dart';
import 'package:storm/sync/sync_engine.dart';
import 'package:storm/ui/vault_screen.dart';

import 'fake_server.dart';

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
      expect(await cache.note('n1'), isNull);

      await engine.setPinned('n1', true);

      final cached = await cache.note('n1');
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

      final evicted = await cache.evict(keep: 3);
      expect(evicted, greaterThan(0));
      expect(await cache.note('x0'), isNotNull, reason: 'pinned');
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
    // Its own cache and engine: ChangeNotifierProvider disposes whatever it
    // holds, so sharing the outer engine would dispose it twice.
    ProviderContainer container() {
      final localCache = CacheDb(NativeDatabase.memory());
      final localServer = FakeServer();
      localServer.notes['n1'] = ServerNote(
        id: 'n1',
        path: 'Keep.md',
        content: '# Keep\n',
        version: 1,
      );
      final localApi = StormApi(
        baseUrl: 'http://test',
        token: 't',
        client: localServer.client,
      );
      return ProviderContainer(
        overrides: [
          cacheProvider.overrideWithValue(localCache),
          apiProvider.overrideWithValue(localApi),
          syncEngineProvider.overrideWith(
            (ref) => SyncEngine(api: localApi, cache: localCache),
          ),
        ],
      );
    }

    Future<void> pump(
      WidgetTester tester,
      ProviderContainer c,
      Size size,
    ) async {
      tester.view.physicalSize = size;
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);
      await tester.pumpWidget(
        UncontrolledProviderScope(
          container: c,
          child: const MaterialApp(home: VaultScreen()),
        ),
      );
      await tester.pumpAndSettle();
    }

    testWidgets('on a phone they collapse into a menu, without overflowing', (
      tester,
    ) async {
      // An overflowing AppBar silently drops the icons past its edge, which is
      // how the attach button looked like it was missing.
      final c = container();
      await c.read(noteSessionProvider).open('n1');
      await pump(tester, c, const Size(411, 900));

      expect(tester.takeException(), isNull, reason: 'no layout overflow');
      expect(find.byIcon(Icons.more_vert), findsOneWidget);

      await tester.tap(find.byIcon(Icons.more_vert));
      await tester.pumpAndSettle();

      expect(find.text('Keep offline'), findsOneWidget);
      expect(find.text('Attach a file'), findsOneWidget);
      expect(find.text('Rename or move'), findsOneWidget);
      expect(find.text('Delete'), findsOneWidget);

      await tester.pumpWidget(const SizedBox());
      c.dispose();
    });

    testWidgets('on a wide screen they are separate icons', (tester) async {
      final c = container();
      await c.read(noteSessionProvider).open('n1');
      await pump(tester, c, const Size(1400, 900));

      expect(tester.takeException(), isNull);
      expect(find.byIcon(Icons.attach_file), findsOneWidget);
      expect(find.byIcon(Icons.push_pin_outlined), findsOneWidget);
      expect(find.byIcon(Icons.more_vert), findsNothing);

      await tester.pumpWidget(const SizedBox());
      c.dispose();
    });

    testWidgets('with no note open, note actions are absent', (tester) async {
      final c = container();
      await pump(tester, c, const Size(411, 900));

      expect(find.byIcon(Icons.more_vert), findsNothing);
      expect(find.byIcon(Icons.note_add_outlined), findsOneWidget);

      await tester.pumpWidget(const SizedBox());
      c.dispose();
    });
  });
}
