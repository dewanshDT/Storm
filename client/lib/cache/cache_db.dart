import 'package:drift/drift.dart';
import 'package:drift_flutter/drift_flutter.dart';

part 'cache_db.g.dart';

/// Notes held locally so reads work offline and open instantly.
///
/// This is a *cache*, not a second copy of record. The server is authoritative;
/// anything here can be discarded and refetched. The one exception is
/// [Outbox], which holds edits that exist nowhere else yet.
class CachedNotes extends Table {
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
  Set<Column> get primaryKey => {id};
}

/// Edits made while offline, waiting to be replayed against the server.
///
/// At most one row per note: repeated edits coalesce into the latest content
/// while keeping the *original* `baseVersion`. That matters — the base is the
/// version the user actually started editing from, and replacing it with a
/// newer one would tell the server there is nothing to merge.
class Outbox extends Table {
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
  Set<Column> get primaryKey => {noteId};
}

/// Scalar client state — currently just the change-log position.
class Meta extends Table {
  TextColumn get key => text()();
  TextColumn get value => text()();

  @override
  Set<Column> get primaryKey => {key};
}

@DriftDatabase(tables: [CachedNotes, Outbox, Meta])
class CacheDb extends _$CacheDb {
  CacheDb([QueryExecutor? executor])
      : super(executor ?? driftDatabase(name: 'storm_cache'));

  @override
  int get schemaVersion => 1;

  // ---- notes ---------------------------------------------------------

  Future<CachedNote?> note(String id) =>
      (select(cachedNotes)..where((n) => n.id.equals(id)))
          .getSingleOrNull();

  Future<List<CachedNote>> allNotes() => select(cachedNotes).get();

  Future<void> putNote(CachedNotesCompanion note) =>
      into(cachedNotes).insertOnConflictUpdate(note);

  Future<void> removeNote(String id) =>
      (delete(cachedNotes)..where((n) => n.id.equals(id))).go();

  Future<void> setPinned(String id, bool pinned) =>
      (update(cachedNotes)..where((n) => n.id.equals(id)))
          .write(CachedNotesCompanion(pinned: Value(pinned)));

  /// Drops the least recently used unpinned notes beyond [keep].
  ///
  /// Notes with queued edits are never evicted — their content is the only
  /// copy of that edit until the outbox drains.
  Future<int> evict({int keep = 200}) async {
    final queued = await select(outbox).get();
    final protected = queued.map((o) => o.noteId).toSet();

    final candidates = await (select(cachedNotes)
          ..where((n) => n.pinned.equals(false))
          ..orderBy([(n) => OrderingTerm.desc(n.cachedAt)]))
        .get();

    final victims = candidates
        .where((n) => !protected.contains(n.id))
        .skip(keep)
        .map((n) => n.id)
        .toList();
    if (victims.isEmpty) return 0;

    await (delete(cachedNotes)..where((n) => n.id.isIn(victims))).go();
    return victims.length;
  }

  // ---- outbox --------------------------------------------------------

  /// Queues an edit, coalescing with any existing entry for the same note.
  Future<void> enqueue({
    required String noteId,
    required int baseVersion,
    required String content,
    String op = 'update',
    String? newPath,
  }) async {
    final existing = await outboxFor(noteId);

    // A queued move outranks a later edit. There is one row per note, so a
    // plain `update` would overwrite the op and the rename would be dropped;
    // keeping `move` lets the drain replay both — rename first, then content.
    final effectiveOp =
        (existing?.op == 'move' && op == 'update') ? 'move' : op;

    await into(outbox).insertOnConflictUpdate(
      OutboxCompanion.insert(
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

  Future<OutboxData?> outboxFor(String noteId) =>
      (select(outbox)..where((o) => o.noteId.equals(noteId)))
          .getSingleOrNull();

  Future<List<OutboxData>> pending() => (select(outbox)
        ..orderBy([(o) => OrderingTerm.asc(o.queuedAt)]))
      .get();

  Future<int> pendingCount() async => (await pending()).length;

  Future<void> dequeue(String noteId) =>
      (delete(outbox)..where((o) => o.noteId.equals(noteId))).go();

  // ---- meta ----------------------------------------------------------

  static const _kLastSeq = 'lastSeq';

  Future<int> lastSeq() async {
    final row = await (select(meta)..where((m) => m.key.equals(_kLastSeq)))
        .getSingleOrNull();
    return int.tryParse(row?.value ?? '') ?? 0;
  }

  Future<void> setLastSeq(int seq) => into(meta).insertOnConflictUpdate(
        MetaCompanion.insert(key: _kLastSeq, value: '$seq'),
      );

  /// Wipes cached content but keeps queued edits, for switching servers.
  Future<void> clearCache() async {
    await delete(cachedNotes).go();
    await (delete(meta)..where((m) => m.key.equals(_kLastSeq))).go();
  }
}
