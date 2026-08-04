import 'dart:async';

import 'package:drift/drift.dart';
import 'package:flutter/foundation.dart';
import 'package:web_socket_channel/web_socket_channel.dart';

import '../api/models.dart';
import '../api/storm_api.dart';
import '../cache/cache_db.dart';

/// What happened to a save.
enum SaveStatus {
  /// Written to the server, which had nothing newer.
  saved,

  /// The server reconciled against a version we never saw. Adopt [content].
  merged,

  /// Same, but the edits overlapped, so [content] carries conflict markers.
  conflicted,

  /// The server was unreachable; the edit is in the outbox and will replay.
  queued,

  /// The server rejected it for a reason retrying won't fix.
  failed,
}

class SaveOutcome {
  const SaveOutcome(this.status, {this.content, this.version, this.error});

  final SaveStatus status;
  final String? content;
  final int? version;
  final String? error;

  /// True when the caller must replace its buffer with [content].
  bool get rewritesBuffer =>
      status == SaveStatus.merged || status == SaveStatus.conflicted;
}

/// Owns the local cache, the outbox, and the connection to the server.
///
/// Everything above this layer — [NoteSession], the UI — reads and writes
/// through it and never touches [StormApi] directly, so offline behaviour is
/// implemented once rather than at every call site.
///
/// Online/offline is *inferred from whether requests actually succeed* rather
/// than read from a connectivity plugin. A device can be on wifi with the
/// homelab unreachable — VPN down, server restarting, wrong subnet — and only
/// a real request distinguishes that from working.
class SyncEngine extends ChangeNotifier {
  SyncEngine({required this.api, required this.cache});

  final StormApi api;
  final CacheDb cache;

  bool _online = true;
  bool get isOnline => _online;

  int _pending = 0;
  int get pendingCount => _pending;

  bool _syncing = false;
  bool get isSyncing => _syncing;

  DateTime? _lastSyncedAt;
  DateTime? get lastSyncedAt => _lastSyncedAt;

  /// Note ids touched by the most recent pull, so open editors can react.
  final _changes = StreamController<Set<String>>.broadcast();
  Stream<Set<String>> get changes => _changes.stream;

  WebSocketChannel? _ws;
  StreamSubscription? _wsSub;
  Timer? _reconnect;
  Timer? _pullDebounce;
  int _backoffSeconds = 1;
  bool _disposed = false;

  // ---- lifecycle -----------------------------------------------------

  Future<void> start() async {
    _pending = await cache.pendingCount();
    notifyListeners();
    await sync();
    _openSocket();
  }

  @override
  void dispose() {
    _disposed = true;
    _pullDebounce?.cancel();
    _reconnect?.cancel();
    _wsSub?.cancel();
    _ws?.sink.close();
    _changes.close();
    super.dispose();
  }

  void _setOnline(bool value) {
    if (_online == value) return;
    _online = value;
    notifyListeners();
    // Coming back online is the moment queued edits must go out.
    if (value) unawaited(sync());
  }

  // ---- reads ---------------------------------------------------------

  /// Returns a note, preferring the cache so the editor paints immediately.
  ///
  /// When online it still refetches in the background; the caller is told via
  /// [changes] if the server's copy differed.
  Future<CachedNote?> openNote(String id) async {
    final cached = await cache.note(id);

    if (!_online) return cached;

    try {
      final fresh = await api.note(id);
      await _store(fresh);
      _setOnline(true);
      return cache.note(id);
    } on StormApiException catch (e) {
      // A definite answer from the server: it is reachable, the note is gone.
      if (e.isNotFound) {
        await cache.removeNote(id);
        return null;
      }
      rethrow;
    } catch (_) {
      _setOnline(false);
      return cached;
    }
  }

  /// The vault tree — from the server when reachable, else from the cache.
  ///
  /// The offline tree only lists notes that happen to be cached, which is
  /// honest: those are the ones that can actually be opened.
  Future<List<NoteMeta>> tree() async {
    if (_online) {
      try {
        final vault = await api.tree();
        _setOnline(true);
        return vault.notes;
      } catch (_) {
        _setOnline(false);
      }
    }
    final cached = await cache.allNotes();
    return cached
        .map(
          (n) => NoteMeta(
            id: n.id,
            path: n.path,
            title: n.title,
            version: n.version,
            modified: n.modified,
            size: n.content.length,
          ),
        )
        .toList()
      ..sort((a, b) => a.path.compareTo(b.path));
  }

  // ---- writes --------------------------------------------------------

  /// Saves a note, falling back to the outbox when the server is unreachable.
  ///
  /// The cache is updated either way, so the editor and any later read see the
  /// user's own text rather than a stale copy.
  Future<SaveOutcome> save({
    required String id,
    required int baseVersion,
    required String content,
  }) async {
    if (!_online) return _queue(id, baseVersion, content);

    try {
      final result = await api.saveNote(
        id: id,
        baseVersion: baseVersion,
        content: content,
      );
      _setOnline(true);
      await _storeWrite(id, result);
      await cache.dequeue(id);
      await _refreshPending();

      if (result.conflict) {
        return SaveOutcome(
          SaveStatus.conflicted,
          content: result.content,
          version: result.meta.version,
        );
      }
      if (result.merged) {
        return SaveOutcome(
          SaveStatus.merged,
          content: result.content,
          version: result.meta.version,
        );
      }
      return SaveOutcome(SaveStatus.saved, version: result.meta.version);
    } on StormApiException catch (e) {
      // The server answered and refused. Retrying identically won't help, so
      // don't queue it — surface it instead.
      return SaveOutcome(SaveStatus.failed, error: e.message);
    } catch (_) {
      _setOnline(false);
      return _queue(id, baseVersion, content);
    }
  }

  Future<SaveOutcome> _queue(String id, int baseVersion, String content) async {
    await cache.enqueue(noteId: id, baseVersion: baseVersion, content: content);
    // Keep the cache showing the user's own text while it waits.
    final existing = await cache.note(id);
    if (existing != null) {
      await cache.putNote(
        CachedNotesCompanion(
          id: Value(id),
          path: Value(existing.path),
          title: Value(existing.title),
          version: Value(existing.version),
          content: Value(content),
          modified: Value(existing.modified),
          cachedAt: Value(DateTime.now()),
          pinned: Value(existing.pinned),
        ),
      );
    }
    await _refreshPending();
    return const SaveOutcome(SaveStatus.queued);
  }

  /// Renames or moves a note, queueing it when the server is unreachable.
  ///
  /// The note keeps its id, so a queued move and a concurrent edit from
  /// another device both survive — that is what tracking by UUID buys.
  Future<SaveOutcome> move({
    required String id,
    required String newPath,
  }) async {
    if (_online) {
      try {
        final result = await api.moveNote(id: id, newPath: newPath);
        _setOnline(true);
        await _storeWrite(id, result);
        return SaveOutcome(SaveStatus.saved, version: result.meta.version);
      } on StormApiException catch (e) {
        return SaveOutcome(SaveStatus.failed, error: e.message);
      } catch (_) {
        _setOnline(false);
      }
    }

    final cached = await cache.note(id);
    await cache.enqueue(
      noteId: id,
      baseVersion: cached?.version ?? 0,
      content: cached?.content ?? '',
      op: 'move',
      newPath: newPath,
    );
    // Show the new path immediately; the server confirms it on replay.
    if (cached != null) {
      await cache.putNote(
        CachedNotesCompanion(
          id: Value(id),
          path: Value(newPath),
          title: Value(cached.title),
          version: Value(cached.version),
          content: Value(cached.content),
          modified: Value(cached.modified),
          cachedAt: Value(DateTime.now()),
          pinned: Value(cached.pinned),
        ),
      );
    }
    await _refreshPending();
    return const SaveOutcome(SaveStatus.queued);
  }

  Future<void> setPinned(String id, bool pinned) async {
    await cache.setPinned(id, pinned);
    if (pinned && _online) {
      try {
        await _store(await api.note(id));
      } catch (_) {
        _setOnline(false);
      }
    }
    notifyListeners();
  }

  // ---- syncing -------------------------------------------------------

  /// Drains the outbox, then pulls changes. Safe to call at any time.
  Future<void> sync() async {
    if (_syncing || _disposed) return;
    _syncing = true;
    notifyListeners();
    try {
      await _drainOutbox();
      await _pull();
      _lastSyncedAt = DateTime.now();
    } finally {
      _syncing = false;
      if (!_disposed) notifyListeners();
    }
  }

  /// Replays queued edits oldest-first.
  ///
  /// Each carries the `baseVersion` it was originally edited from, so the
  /// server merges against that rather than clobbering whatever landed while
  /// this device was away.
  Future<void> _drainOutbox() async {
    final queued = await cache.pending();
    if (queued.isEmpty) return;

    final touched = <String>{};
    for (final item in queued) {
      try {
        if (item.op == 'delete') {
          await api.deleteNote(item.noteId);
          await cache.removeNote(item.noteId);
        } else if (item.op == 'move') {
          // Move first, then push the content it was queued with. Both are
          // keyed by id, so neither depends on the path being current.
          final moved = await api.moveNote(
            id: item.noteId,
            newPath: item.newPath!,
          );
          await _storeWrite(item.noteId, moved);
          if (item.content.isNotEmpty && item.content != moved.content) {
            final saved = await api.saveNote(
              id: item.noteId,
              baseVersion: item.baseVersion,
              content: item.content,
            );
            await _storeWrite(item.noteId, saved);
          }
        } else {
          final result = await api.saveNote(
            id: item.noteId,
            baseVersion: item.baseVersion,
            content: item.content,
          );
          await _storeWrite(item.noteId, result);
        }
        await cache.dequeue(item.noteId);
        touched.add(item.noteId);
        _setOnline(true);
      } on StormApiException catch (e) {
        // The note was deleted elsewhere, or the server refused outright.
        // Retrying forever would wedge the queue behind one bad entry.
        if (e.isNotFound) {
          await cache.dequeue(item.noteId);
          await cache.removeNote(item.noteId);
          touched.add(item.noteId);
        }
        // Any other refusal: leave it queued for a human to notice.
      } catch (_) {
        // Still unreachable — stop, keep the rest queued, try again later.
        _setOnline(false);
        break;
      }
    }

    await _refreshPending();
    if (touched.isNotEmpty) _changes.add(touched);
  }

  /// How many changes to request per page.
  static const _pageSize = 500;

  /// Applies everything that changed on the server since our last position.
  ///
  /// Pages until caught up. The server caps a response at [_pageSize], so a
  /// client that has been away — or is meeting a large vault for the first
  /// time — has more waiting than one response can carry. Advancing straight
  /// to the server's latest `seq` after a single page would skip every change
  /// past the first one, which is silent data loss.
  Future<void> _pull() async {
    var since = await cache.lastSeq();
    final touched = <String>{};

    // Bounded so a server that keeps changing can't spin here forever; the
    // next sync picks up whatever is left.
    for (var page = 0; page < 100; page++) {
      final SyncBatch batch;
      try {
        batch = await api.sync(since: since, limit: _pageSize);
        _setOnline(true);
      } catch (_) {
        _setOnline(false);
        if (touched.isNotEmpty) _changes.add(touched);
        return;
      }

      final applied = await _applyChanges(batch.changes, touched);
      if (!applied) {
        // A request failed mid-page. Everything up to here is committed;
        // leave `since` where it is so the rest is retried.
        if (touched.isNotEmpty) _changes.add(touched);
        return;
      }

      if (batch.changes.length < _pageSize) {
        // Caught up: safe to adopt the server's position.
        await cache.setLastSeq(batch.seq);
        break;
      }

      // A full page means there is more. Resume from the last one applied.
      since = batch.changes.last.seq;
      await cache.setLastSeq(since);
    }

    await cache.evict();
    if (touched.isNotEmpty) _changes.add(touched);
  }

  /// Applies one page. Returns false if the server became unreachable.
  Future<bool> _applyChanges(List<Change> changes, Set<String> touched) async {
    for (final change in changes) {
      if (change.kind == 'deleted') {
        await cache.removeNote(change.noteId);
        touched.add(change.noteId);
        continue;
      }

      // Only pull content for notes we actually hold. The tree comes from
      // /v1/tree, so there is no need to hydrate the whole vault.
      final held = await cache.note(change.noteId);
      if (held == null) continue;

      // Don't overwrite a note whose local edit hasn't been sent yet.
      if (await cache.outboxFor(change.noteId) != null) continue;

      try {
        await _store(await api.note(change.noteId));
        touched.add(change.noteId);
      } on StormApiException catch (e) {
        if (e.isNotFound) {
          await cache.removeNote(change.noteId);
          touched.add(change.noteId);
        }
      } catch (_) {
        _setOnline(false);
        return false; // leave lastSeq alone so this page is retried
      }
    }
    return true;
  }

  // ---- websocket -----------------------------------------------------

  /// Opens the change stream so other devices' edits arrive promptly.
  ///
  /// The socket only signals *that* something changed; the authoritative list
  /// still comes from `GET /v1/sync?since=`. That way a dropped or missed
  /// frame costs nothing — the next pull catches up regardless.
  void _openSocket() {
    if (_disposed) return;
    _wsSub?.cancel();
    _ws?.sink.close();

    final url = api.baseUrl.replaceFirst(RegExp(r'^http'), 'ws');
    final uri = Uri.parse(
      '$url/v1/stream?token=${Uri.encodeComponent(api.token)}',
    );

    try {
      final channel = WebSocketChannel.connect(uri);
      _ws = channel;
      _wsSub = channel.stream.listen(
        (_) {
          _setOnline(true);
          _backoffSeconds = 1;
          // Coalesce bursts: a rename or a multi-note edit fires several
          // frames, and one pull covers them all.
          _pullDebounce?.cancel();
          _pullDebounce = Timer(const Duration(milliseconds: 300), sync);
        },
        onError: (_) => _scheduleReconnect(),
        onDone: _scheduleReconnect,
        cancelOnError: true,
      );
    } catch (_) {
      _scheduleReconnect();
    }
  }

  void _scheduleReconnect() {
    if (_disposed) return;
    _setOnline(false);
    _reconnect?.cancel();
    _reconnect = Timer(Duration(seconds: _backoffSeconds), () {
      // Capped exponential backoff: a server that is down for an hour
      // shouldn't be probed every second.
      _backoffSeconds = (_backoffSeconds * 2).clamp(1, 60);
      _openSocket();
      unawaited(sync());
    });
  }

  // ---- helpers -------------------------------------------------------

  Future<void> _store(Note note) => cache.putNote(
    CachedNotesCompanion.insert(
      id: note.meta.id,
      path: note.meta.path,
      title: Value(note.meta.title),
      version: note.meta.version,
      content: note.content,
      modified: Value(note.meta.modified),
      cachedAt: DateTime.now(),
    ),
  );

  Future<void> _storeWrite(String id, WriteResult result) async {
    final existing = await cache.note(id);
    await cache.putNote(
      CachedNotesCompanion.insert(
        id: id,
        path: result.meta.path,
        title: Value(result.meta.title),
        version: result.meta.version,
        content: result.content,
        modified: Value(result.meta.modified),
        cachedAt: DateTime.now(),
        pinned: Value(existing?.pinned ?? false),
      ),
    );
  }

  Future<void> _refreshPending() async {
    _pending = await cache.pendingCount();
    if (!_disposed) notifyListeners();
  }
}
