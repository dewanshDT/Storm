import 'package:drift/native.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_riverpod/legacy.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:storm/api/storm_api.dart';
import 'package:storm/api/storm_connection.dart';
import 'package:storm/cache/cache_db.dart';
import 'package:storm/state/app_state.dart';
import 'package:storm/sync/sync_engine.dart';

import 'fake_server.dart';

/// The open note follows the vault: after a switch, a note of the new vault
/// is asked of the new vault, not of the one first opened (found on staging
/// 2026-10-26: every note of the second vault said "That note no longer
/// exists").
void main() {
  late CacheDb cache;
  late FakeServer server;
  final vault = StateProvider<String>((ref) => FakeServer.primaryVault);

  ProviderContainer container() {
    cache = CacheDb(NativeDatabase.memory());
    server = FakeServer();
    server.notes['a1'] = ServerNote(
      id: 'a1',
      path: 'A.md',
      content: '# A\n',
      version: 1,
    );
    server.addVault('v-second', 'Second')['b1'] = ServerNote(
      id: 'b1',
      path: 'B.md',
      content: '# B\n',
      version: 1,
    );
    final api = StormApi(
      baseUrl: 'http://test',
      token: 't',
      client: server.client,
    );
    return ProviderContainer(
      overrides: [
        cacheProvider.overrideWithValue(cache),
        apiProvider.overrideWithValue(api),
        activeVaultProvider.overrideWith((ref) => ref.watch(vault)),
        // Never `start()`ed, so it opens no WebSocket.
        syncEngineProvider.overrideWith(
          (ref) => SyncEngine(
            connection: StormConnection.direct(api: api),
            cache: cache,
            vaultId: ref.watch(activeVaultProvider),
          ),
        ),
      ],
    );
  }

  tearDown(() => cache.close());

  test('a note of the vault switched to opens from that vault', () async {
    final c = container();
    addTearDown(c.dispose);

    await c.read(noteSessionProvider).open('a1');
    expect(c.read(noteSessionProvider).body, contains('# A'));

    c.read(vault.notifier).state = 'v-second';
    final session = c.read(noteSessionProvider);
    await session.open('b1');

    expect(session.error, isNull);
    expect(session.noteId, 'b1');
    expect(session.body, contains('# B'));
    expect(server.byVault['v-second']!.containsKey('b1'), isTrue);
  });

  test('typing not yet saved when the vault switches is kept in its own '
      "vault's outbox", () async {
    final c = container();
    addTearDown(c.dispose);

    final first = c.read(noteSessionProvider);
    await first.open('a1');
    first.edit('# A\n\nUnsaved line.\n');

    c.read(vault.notifier).state = 'v-second';
    c.read(noteSessionProvider);
    await pumpEventQueue();

    final kept = await cache.outboxFor(FakeServer.primaryVault, 'a1');
    expect(kept, isNotNull);
    expect(kept!.content, contains('Unsaved line.'));
    expect(await cache.outboxFor('v-second', 'a1'), isNull);
  });
}
