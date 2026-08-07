import 'package:drift/native.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:storm/api/storm_api.dart';
import 'package:storm/cache/cache_db.dart';
import 'package:storm/state/app_state.dart';
import 'package:storm/state/note_session.dart';

import 'fake_server.dart';
import 'shell_harness.dart';

/// Provider lifetimes.
///
/// The trap this guards: watching a `ChangeNotifierProvider` rebuilds the
/// watcher on **every** `notifyListeners()`, not just when the object is
/// replaced. `SyncEngine` notifies on every status tick — going online,
/// draining the outbox, starting and finishing a sync — so anything that
/// `watch`es it gets torn down and rebuilt constantly.
///
/// When `NoteSession` was built that way, opening a note set the id, the
/// engine ticked, the session was recreated empty, and the editor fell back to
/// "Select a note to start editing" while the tree still showed the note as
/// selected.
/// One test's container and the server behind it.
///
/// Deliberately *not* shared `late` fields. These tests leave async work in
/// flight — a sync that completes after the test body returns — and with a
/// shared fixture that work lands on the next test's server, which showed up
/// as request counts from one test failing another's assertion.
typedef Fixture = ({ProviderContainer container, FakeServer server});

void main() {
  Future<Fixture> makeFixture() async {
    final cache = CacheDb(NativeDatabase.memory());
    final server = FakeServer();
    server.notes['n1'] = ServerNote(
      id: 'n1',
      path: 'A.md',
      content: '# A\n\nbody\n',
      version: 1,
    );

    final container = ProviderContainer(
      overrides: [
        cacheProvider.overrideWithValue(cache),
        apiProvider.overrideWithValue(
          StormApi(baseUrl: 'http://test', token: 't', client: server.client),
        ),
        // The engine takes its vault from settings, so these have to say
        // which one is open or every cache key would be blank.
        settingsProvider.overrideWith(() => FakeSettings(true)),
      ],
    );
    // Settings load asynchronously, and the engine takes its vault from them.
    // Reading before they resolve would build an engine with no vault.
    await container.read(settingsProvider.future);
    return (container: container, server: server);
  }

  test('the note session survives sync-engine status ticks', () async {
    final (:container, :server) = await makeFixture();
    addTearDown(container.dispose);

    final session = container.read(noteSessionProvider);
    await session.open('n1');
    expect(session.isOpen, isTrue, reason: 'precondition');

    // Exactly what happens in the app a moment after opening a note: the
    // engine reports status.
    final engine = container.read(syncEngineProvider);
    for (var i = 0; i < 5; i++) {
      await engine.sync();
    }

    final after = container.read(noteSessionProvider);
    expect(
      identical(after, session),
      isTrue,
      reason: 'the session must not be rebuilt by a status notification',
    );
    expect(
      after.isOpen,
      isTrue,
      reason: 'the editor would otherwise fall back to its empty state',
    );
    expect(after.buffer, contains('body'));
  });

  test('opening a note leaves it open and readable', () async {
    final (:container, :server) = await makeFixture();
    addTearDown(container.dispose);

    final session = container.read(noteSessionProvider);
    await session.open('n1');

    expect(session.isOpen, isTrue);
    expect(session.error, isNull);
    expect(session.saveState, SaveState.saved);
    expect(session.meta?.path, 'A.md');
  });

  test('the engine itself is not rebuilt by its own notifications', () async {
    final (:container, :server) = await makeFixture();
    addTearDown(container.dispose);

    final engine = container.read(syncEngineProvider);
    await engine.sync();
    await engine.sync();

    expect(
      identical(container.read(syncEngineProvider), engine),
      isTrue,
      reason: 'a rebuilt engine would drop the cache and the outbox',
    );
  });

  test('the tree does not refetch on every status tick', () async {
    final (:container, :server) = await makeFixture();
    addTearDown(container.dispose);

    await container.read(treeProvider.future);
    final before = server.treeRequests;

    final engine = container.read(syncEngineProvider);
    for (var i = 0; i < 5; i++) {
      await engine.sync();
    }
    await Future.delayed(Duration.zero);

    // Some refetching is expected when data actually changes; what must not
    // happen is a request per status notification.
    expect(
      server.treeRequests - before,
      lessThan(3),
      reason: 'status ticks should not invalidate the tree',
    );
  });
}
