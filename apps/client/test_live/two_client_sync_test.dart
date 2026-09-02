import 'dart:io';

import 'package:drift/native.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:storm/api/models.dart';
import 'package:storm/api/storm_connection.dart';
import 'package:storm/api/storm_api.dart';
import 'package:storm/cache/cache_db.dart';
import 'package:storm/state/note_session.dart';
import 'package:storm/sync/sync_engine.dart';

import 'live_auth.dart';

/// Scenarios 1, 2 and 9 of the sync matrix, with two independent clients
/// against one real server.
///
/// The unit tests can't reach these: `MockClient` has no WebSocket, and a
/// single fake server can't show one device observing another's write. Here
/// each client gets its own cache and its own connection, exactly as two
/// devices would.
///
///   cargo run -- --vault-root /tmp/vaults --state /tmp/s
///   flutter test test_live/
///
/// The vault under test is whichever one the server reports first, so this
/// runs against whatever the harness seeded.
void main() {
  const baseUrl = 'http://127.0.0.1:8484';
  // Obtained by pairing and signing in — there is no shared token.
  late String token;

  /// Resolved once, from the running server.
  late String vaultId;

  late _Client a;
  late _Client b;

  setUpAll(() async {
    try {
      final socket = await Socket.connect(
        '127.0.0.1',
        8484,
        timeout: const Duration(seconds: 2),
      );
      socket.destroy();
    } catch (_) {
      fail('No server on 127.0.0.1:8484 — start storm-server first.');
    }

    // **The harness claims the server once and passes the session in.** The
    // bootstrap pairing nonce is single-use, so these suites cannot each pair
    // themselves — the first would consume it and the rest would fail
    // authenticating rather than failing at what they test.
    const inherited = String.fromEnvironment('STORM_SESSION');
    if (inherited.isNotEmpty) {
      token = inherited.replaceFirst('Bearer ', '');
    } else {
      const logPath = String.fromEnvironment(
        'STORM_LIVE_LOG',
        defaultValue: '../../.dev/live-server.log',
      );
      token = (await signIn(baseUrl: baseUrl, logPath: logPath)).accessToken;
    }

    // Every note-shaped route is vault-scoped; ask the server which vault
    // rather than assuming a name.
    final probe = StormApi(baseUrl: baseUrl, token: token);
    final vaults = await probe.vaults();
    probe.dispose();
    if (vaults.isEmpty) {
      fail('The server has no vaults — the harness should have seeded one.');
    }
    vaultId = vaults.first.id;
  });

  setUp(() {
    a = _Client(baseUrl, token, vaultId);
    b = _Client(baseUrl, token, vaultId);
  });

  tearDown(() async {
    await a.close();
    await b.close();
  });

  String scratch(String name) =>
      'TwoClient/${DateTime.now().microsecondsSinceEpoch}-$name.md';

  /// Polls until [check] passes, so tests don't depend on socket timing.
  Future<bool> eventually(
    Future<bool> Function() check, {
    Duration timeout = const Duration(seconds: 10),
  }) async {
    final deadline = DateTime.now().add(timeout);
    while (DateTime.now().isBefore(deadline)) {
      if (await check()) return true;
      await Future.delayed(const Duration(milliseconds: 200));
    }
    return false;
  }

  test('scenario 1: an edit on A reaches B on next open', () async {
    final created = await a.api.createNote(
      vaultId: vaultId,
      path: scratch('s1'),
      content: '# One\n',
    );
    final id = created.meta.id;

    await a.engine.openNote(id);
    await a.engine.save(
      id: id,
      baseVersion: created.meta.version,
      content: '# One\n\nFrom A.\n',
    );

    final onB = await b.engine.openNote(id);
    expect(onB!.content, contains('From A.'));

    await a.api.deleteNote(vaultId, id);
  });

  test('scenario 2: B is pushed A\'s change over the WebSocket', () async {
    final created = await a.api.createNote(
      vaultId: vaultId,
      path: scratch('s2'),
      content: '# Two\n',
    );
    final id = created.meta.id;

    // B holds the note and is listening.
    await b.engine.openNote(id);
    await b.engine.start();

    final saved = await a.engine.save(
      id: id,
      baseVersion: created.meta.version,
      content: '# Two\n\nPushed from A.\n',
    );
    expect(saved.status, SaveStatus.saved);

    final arrived = await eventually(() async {
      final cached = await b.cache.note(vaultId, id);
      return cached != null && cached.content.contains('Pushed from A.');
    });
    expect(
      arrived,
      isTrue,
      reason: "B should have been pushed A's change without polling",
    );

    await a.api.deleteNote(vaultId, id);
  });

  test(
    'scenario 9: a delete on A is noticed by B, with no resurrection',
    () async {
      final created = await a.api.createNote(
        vaultId: vaultId,
        path: scratch('s9'),
        content: '# Nine\n',
      );
      final id = created.meta.id;

      final session = NoteSession(b.engine);
      await session.open(id);
      expect(session.isOpen, isTrue);

      await a.api.deleteNote(vaultId, id);
      await b.engine.sync();
      await session.onRemoteChange();

      expect(session.isOpen, isFalse);
      expect(session.notice, contains('deleted'));
      expect(await b.cache.note(vaultId, id), isNull);

      // And B must not recreate it by saving a stale buffer.
      await session.save();
      await expectLater(
        a.api.note(vaultId, id),
        throwsA(
          isA<StormApiException>().having(
            (e) => e.isNotFound,
            'isNotFound',
            isTrue,
          ),
        ),
      );
      session.dispose();
    },
  );

  test(
    'offline edit on B replays and merges with A\'s concurrent edit',
    () async {
      // The end-to-end version of the whole design: two devices, one offline,
      // non-overlapping edits, reconciled by the server on reconnect.
      final body = '# Merge\n\nAlpha.\n\nBeta.\n\nGamma.\n\nDelta.\n\nOmega.\n';
      final created = await a.api.createNote(
        vaultId: vaultId,
        path: scratch('merge'),
        content: body,
      );
      final id = created.meta.id;

      final onB = await b.engine.openNote(id);
      final sharedBase = onB!.version;

      // B goes offline and edits the bottom.
      b.engine.api.dispose(); // kills B's HTTP client -> requests now fail
      final queued = await b.engine.save(
        id: id,
        baseVersion: sharedBase,
        content: onB.content.replaceAll(
          'Omega.',
          'Omega, edited on B offline.',
        ),
      );
      expect(queued.status, SaveStatus.queued);

      // Meanwhile A edits the top and lands first.
      await a.engine.openNote(id);
      await a.engine.save(
        id: id,
        baseVersion: sharedBase,
        content: onB.content.replaceAll('Alpha.', 'Alpha, edited on A.'),
      );

      // B reconnects with a fresh client and drains.
      b.reconnect(baseUrl, token);
      await b.engine.sync();

      final merged = await a.api.note(vaultId, id);
      expect(merged.content, contains('edited on A'));
      expect(merged.content, contains('edited on B offline'));
      expect(
        merged.content,
        isNot(contains('<<<<<<<')),
        reason: 'non-overlapping edits should merge cleanly',
      );
      expect(await b.cache.pendingCount(vaultId), 0);

      await a.api.deleteNote(vaultId, id);
    },
  );
}

/// One simulated device: its own cache, engine and connection.
class _Client {
  _Client(String baseUrl, String token, this.vaultId)
    : cache = CacheDb(NativeDatabase.memory()) {
    engine = SyncEngine(
      connection: StormConnection.direct(
        api: StormApi(baseUrl: baseUrl, token: token),
      ),
      cache: cache,
      vaultId: vaultId,
    );
  }

  final String vaultId;
  final CacheDb cache;
  late SyncEngine engine;

  StormApi get api => engine.api;

  /// Replaces the engine's transport, as reconnecting would.
  void reconnect(String baseUrl, String token) {
    final replacement = SyncEngine(
      connection: StormConnection.direct(
        api: StormApi(baseUrl: baseUrl, token: token),
      ),
      cache: cache,
      vaultId: vaultId,
    );
    engine.dispose();
    engine = replacement;
  }

  Future<void> close() async {
    engine.dispose();
    await cache.close();
  }
}
