import 'package:drift/drift.dart';
import 'package:flutter/foundation.dart' show debugPrint;
import 'package:drift_flutter/drift_flutter.dart';

part 'cache_db.g.dart';

/// Vault id stamped on rows written before multi-vault existed.
///
/// A sentinel rather than an empty string so it is obvious in a dump what
/// these rows are, and so `adoptLegacyRows` can find them exactly.
const kLegacyVault = 'legacy';

/// Notes held locally so reads work offline and open instantly.
///
/// This is a *cache*, not a second copy of record. The server is authoritative;
/// anything here can be discarded and refetched. The one exception is
/// [Outbox], which holds edits that exist nowhere else yet.
class CachedNotes extends Table {
  /// Which vault this note belongs to.
  ///
  /// Part of the primary key: note ids are unique per vault, not per server,
  /// and two vaults routinely hold the same *path*.
  TextColumn get vaultId => text().withDefault(const Constant(kLegacyVault))();
  TextColumn get id => text()();
  TextColumn get path => text()();
  TextColumn get title => text().withDefault(const Constant(''))();

  /// The server version this content came from. Sent as `base_version` on the
  /// next save, which is what lets the server merge rather than clobber.
  IntColumn get version => integer()();
  TextColumn get content => text()();
  TextColumn get modified => text().withDefault(const Constant(''))();
  DateTimeColumn get cachedAt => dateTime()();

  /// User asked to keep this available offline, so eviction must skip it.
  BoolColumn get pinned => boolean().withDefault(const Constant(false))();

  @override
  Set<Column> get primaryKey => {vaultId, id};
}

/// Edits made while offline, waiting to be replayed against the server.
///
/// At most one row per note: repeated edits coalesce into the latest content
/// while keeping the *original* `baseVersion`. That matters — the base is the
/// version the user actually started editing from, and replacing it with a
/// newer one would tell the server there is nothing to merge.
class Outbox extends Table {
  TextColumn get vaultId => text().withDefault(const Constant(kLegacyVault))();
  TextColumn get noteId => text()();
  IntColumn get baseVersion => integer()();
  TextColumn get content => text()();

  /// `update`, `move` or `delete`. Creates are sent immediately and never
  /// queued, because the client cannot invent a server-assigned id.
  TextColumn get op => text().withDefault(const Constant('update'))();

  /// Destination for a queued `move`. Null for every other op.
  TextColumn get newPath => text().nullable()();
  DateTimeColumn get queuedAt => dateTime()();

  @override
  Set<Column> get primaryKey => {vaultId, noteId};
}

/// Notes opened recently, mirroring the server's cross-vault list.
///
/// Held locally so the dashboard renders offline and so an open shows up
/// immediately rather than after the next round trip.
class Recents extends Table {
  TextColumn get vaultId => text()();
  TextColumn get noteId => text()();
  TextColumn get vaultName => text().withDefault(const Constant(''))();
  TextColumn get path => text().withDefault(const Constant(''))();
  TextColumn get title => text().withDefault(const Constant(''))();
  DateTimeColumn get openedAt => dateTime()();

  @override
  Set<Column> get primaryKey => {vaultId, noteId};
}

/// Scalar client state — the per-vault change-log positions.
class Meta extends Table {
  TextColumn get key => text()();
  TextColumn get value => text()();

  @override
  Set<Column> get primaryKey => {key};
}

@DriftDatabase(tables: [CachedNotes, Outbox, Meta, Recents])
class CacheDb extends _$CacheDb {
  CacheDb([QueryExecutor? executor]) : super(executor ?? _open());

  /// Opens the platform's database.
  ///
  /// The `web` options are **not** optional: `driftDatabase()` throws on web
  /// when they are missing, so omitting them compiles fine and then crashes
  /// the moment the browser first touches the cache. Both assets are checked
  /// into `web/` and served by storm-server.
  static QueryExecutor _open() => driftDatabase(
    name: 'storm_cache',
    web: DriftWebOptions(
      sqlite3Wasm: Uri.parse('sqlite3.wasm'),
      driftWorker: Uri.parse('drift_worker.js'),
      onResult: (result) {
        // Which backend the browser allowed. OPFS needs cross-origin
        // isolation (the COOP/COEP headers storm-server sets); without it
        // drift silently drops to IndexedDB, which is slower and loses
        // cross-tab safety on Chrome for Android. Worth being able to see.
        if (result.missingFeatures.isNotEmpty) {
          debugPrint(
            'storm: cache using ${result.chosenImplementation} — '
            'missing browser features: ${result.missingFeatures}',
          );
        }
      },
    ),
  );

  @override
  int get schemaVersion => 2;

  /// v1 held one vault's notes with no way to say which.
  ///
  /// Existing rows are stamped [kLegacyVault] and left alone rather than
  /// discarded: [Outbox] rows are edits that exist nowhere else, and throwing
  /// them away to simplify a migration is data loss. [adoptLegacyRows] moves
  /// them to the real vault once the server says which one it is.
  @override
  MigrationStrategy get migration => MigrationStrategy(
    onCreate: (m) => m.createAll(),
    onUpgrade: (m, from, to) async {
      if (from < 2) {
        await m.addColumn(cachedNotes, cachedNotes.vaultId);
        await m.addColumn(outbox, outbox.vaultId);
        await m.createTable(recents);
      }
    },
  );

  /// Re-stamps pre-multi-vault rows once the vault they belong to is known.
  ///
  /// Only ever called with a single vault: with more than one there is no way
  /// to tell which the old rows came from, and guessing would attach one
  /// vault's queued edits to another.
  Future<int> adoptLegacyRows(String vaultId) async {
    if (vaultId == kLegacyVault) return 0;
    final notes =
        await (update(cachedNotes)
              ..where((n) => n.vaultId.equals(kLegacyVault)))
            .write(CachedNotesCompanion(vaultId: Value(vaultId)));
    final queued =
        await (update(outbox)..where((o) => o.vaultId.equals(kLegacyVault)))
            .write(OutboxCompanion(vaultId: Value(vaultId)));

    final seq = await (select(
      meta,
    )..where((m) => m.key.equals('lastSeq'))).getSingleOrNull();
    if (seq != null) {
      await into(meta).insertOnConflictUpdate(
        MetaCompanion.insert(key: _seqKey(vaultId), value: seq.value),
      );
      await (delete(meta)..where((m) => m.key.equals('lastSeq'))).go();
    }
    return notes + queued;
  }

  /// Rows still waiting to be attributed to a vault.
  Future<int> legacyRowCount() async {
    final rows = await (select(
      outbox,
    )..where((o) => o.vaultId.equals(kLegacyVault))).get();
    return rows.length;
  }

  // ---- notes ---------------------------------------------------------

  Future<CachedNote?> note(String vaultId, String id) =>
      (select(cachedNotes)
            ..where((n) => n.vaultId.equals(vaultId) & n.id.equals(id)))
          .getSingleOrNull();

  Future<List<CachedNote>> allNotes(String vaultId) =>
      (select(cachedNotes)..where((n) => n.vaultId.equals(vaultId))).get();

  Future<void> putNote(CachedNotesCompanion note) =>
      into(cachedNotes).insertOnConflictUpdate(note);

  Future<void> removeNote(String vaultId, String id) => (delete(
    cachedNotes,
  )..where((n) => n.vaultId.equals(vaultId) & n.id.equals(id))).go();

  /// Ids the user asked to keep available offline, in one vault.
  Future<Set<String>> pinnedIds(String vaultId) async {
    final rows = await (select(
      cachedNotes,
    )..where((n) => n.vaultId.equals(vaultId) & n.pinned.equals(true))).get();
    return rows.map((n) => n.id).toSet();
  }

  Future<void> setPinned(String vaultId, String id, bool pinned) =>
      (update(cachedNotes)
            ..where((n) => n.vaultId.equals(vaultId) & n.id.equals(id)))
          .write(CachedNotesCompanion(pinned: Value(pinned)));

  /// Drops the least recently used unpinned notes beyond [keep].
  ///
  /// Notes with queued edits are never evicted — their content is the only
  /// copy of that edit until the outbox drains.
  Future<int> evict(String vaultId, {int keep = 200}) async {
    final queued = await select(outbox).get();
    // Keyed by (vault, note) so one vault's queued edit cannot protect — or
    // fail to protect — a same-id row in another.
    final protected = queued.map((o) => '\${o.vaultId}/\${o.noteId}').toSet();

    final candidates =
        await (select(cachedNotes)
              ..where((n) => n.vaultId.equals(vaultId) & n.pinned.equals(false))
              ..orderBy([(n) => OrderingTerm.desc(n.cachedAt)]))
            .get();

    final victims = candidates
        .where((n) => !protected.contains('\${n.vaultId}/\${n.id}'))
        .skip(keep)
        .map((n) => n.id)
        .toList();
    if (victims.isEmpty) return 0;

    await (delete(
      cachedNotes,
    )..where((n) => n.vaultId.equals(vaultId) & n.id.isIn(victims))).go();
    return victims.length;
  }

  // ---- outbox --------------------------------------------------------

  /// Queues an edit, coalescing with any existing entry for the same note.
  Future<void> enqueue({
    required String vaultId,
    required String noteId,
    required int baseVersion,
    required String content,
    String op = 'update',
    String? newPath,
  }) async {
    final existing = await outboxFor(vaultId, noteId);

    // A queued move outranks a later edit. There is one row per note, so a
    // plain `update` would overwrite the op and the rename would be dropped;
    // keeping `move` lets the drain replay both — rename first, then content.
    final effectiveOp = (existing?.op == 'move' && op == 'update')
        ? 'move'
        : op;

    await into(outbox).insertOnConflictUpdate(
      OutboxCompanion.insert(
        vaultId: Value(vaultId),
        noteId: noteId,
        // Keep the original base: it is the version the user branched from.
        baseVersion: existing?.baseVersion ?? baseVersion,
        content: content,
        op: Value(effectiveOp),
        newPath: Value(newPath ?? existing?.newPath),
        queuedAt: DateTime.now(),
      ),
    );
  }

  Future<OutboxData?> outboxFor(String vaultId, String noteId) =>
      (select(outbox)
            ..where((o) => o.vaultId.equals(vaultId) & o.noteId.equals(noteId)))
          .getSingleOrNull();

  Future<List<OutboxData>> pending(String vaultId) =>
      (select(outbox)
            ..where((o) => o.vaultId.equals(vaultId))
            ..orderBy([(o) => OrderingTerm.asc(o.queuedAt)]))
          .get();

  Future<int> pendingCount(String vaultId) async =>
      (await pending(vaultId)).length;

  Future<void> dequeue(String vaultId, String noteId) => (delete(
    outbox,
  )..where((o) => o.vaultId.equals(vaultId) & o.noteId.equals(noteId))).go();

  // ---- recents -------------------------------------------------------

  /// Replaces the mirrored recents list with the server's.
  Future<void> replaceRecents(List<RecentsCompanion> rows) async {
    await batch((b) {
      b.deleteAll(recents);
      b.insertAll(recents, rows);
    });
  }

  /// Records an open locally, so it shows immediately and survives being
  /// offline. The server's copy wins on the next refresh.
  Future<void> noteOpened(RecentsCompanion row) =>
      into(recents).insertOnConflictUpdate(row);

  Future<List<Recent>> recentNotes({int limit = 20}) =>
      (select(recents)
            ..orderBy([(r) => OrderingTerm.desc(r.openedAt)])
            ..limit(limit))
          .get();

  Future<void> forgetRecent(String vaultId, String noteId) => (delete(
    recents,
  )..where((r) => r.vaultId.equals(vaultId) & r.noteId.equals(noteId))).go();

  // ---- meta ----------------------------------------------------------

  /// The sync cursor is **per vault**.
  ///
  /// One shared key would have two vaults overwriting each other's position,
  /// which surfaces as randomly missed changes rather than as an error.
  static String _seqKey(String vaultId) => 'lastSeq:$vaultId';

  Future<int> lastSeq(String vaultId) async {
    final row = await (select(
      meta,
    )..where((m) => m.key.equals(_seqKey(vaultId)))).getSingleOrNull();
    return int.tryParse(row?.value ?? '') ?? 0;
  }

  Future<void> setLastSeq(String vaultId, int seq) =>
      into(meta).insertOnConflictUpdate(
        MetaCompanion.insert(key: _seqKey(vaultId), value: '$seq'),
      );
}
