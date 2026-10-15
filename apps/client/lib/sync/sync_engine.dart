import 'dart:async';
import 'dart:convert';
import 'dart:math';

import 'package:drift/drift.dart';
import 'package:flutter/foundation.dart';
import 'package:http/http.dart' as http;
import 'package:web_socket_channel/web_socket_channel.dart';

import '../api/connection_log.dart';
import '../api/models.dart';
import '../api/net.dart';
import '../api/relay/srp_http_client.dart';
import '../api/server_verifier.dart';
import '../api/storm_connection.dart';
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
  SyncEngine({
    required this.connection,
    required this.cache,
    required this.vaultId,
    this.outboxRetryMin = const Duration(seconds: 5),
    this.outboxRetryMax = const Duration(seconds: 60),
    this.feedRetryMin = const Duration(seconds: 1),
    this.feedRetryMax = const Duration(seconds: 60),
  }) {
    // The connection's state is followed, never inferred: whoever re-raced,
    // entering `Connected` is when the outbox drains and the feed re-opens
    // (§8.9 rule 3), and `Unproven` is what stops every request.
    _statusSub = connection.statusChanges.listen((status) {
      switch (status) {
        case ConnectionStatus.connected:
          _setImpostor(false);
          if (_started) _onConnected();
        case ConnectionStatus.unproven:
          _setImpostor(true);
        case _:
          break;
      }
      notifyListeners();
    });
  }

  /// The outbox retry while writes wait on a connected transport (§8.8):
  /// from [outboxRetryMin], doubling to [outboxRetryMax], ±20 %.
  final Duration outboxRetryMin;
  final Duration outboxRetryMax;

  /// The change feed's own re-open backoff (§8.4): the feed ending never
  /// touches the connection.
  final Duration feedRetryMin;
  final Duration feedRetryMax;

  StreamSubscription<ConnectionStatus>? _statusSub;

  /// How the server is reached, and the only object that knows.
  ///
  /// The engine only asks it to *ensure connected* and follows its state;
  /// it never resets it. Racing, re-proving, backoff and deciding that a
  /// transport is lost all belong to the connection (§8.1, decision 116h).
  final StormConnection connection;

  /// The API client for whatever transport currently wins.
  ///
  /// A getter, so every `api.x()` below follows a re-race without knowing one
  /// happened. That is the seam doing its job: nothing here distinguishes a
  /// direct dial from a relayed one.
  StormApi get api => connection.sessionApi;

  final CacheDb cache;

  /// The vault this engine serves.
  ///
  /// One engine per active vault: switching vaults rebuilds `apiProvider`,
  /// which disposes this engine and closes its socket. Every cache key and
  /// every request carries it, so two vaults can never share a sync cursor or
  /// see each other's notes.
  final String vaultId;

  bool _online = true;

  /// Whether requests are getting through: the last one did, and the
  /// connection is not racing, unreachable or unproven (§8.6). A sync
  /// engine's own idea of being synced is never enough.
  bool get isOnline =>
      _online &&
      (connection.status == ConnectionStatus.connected ||
          connection.status == ConnectionStatus.idle);

  /// Set when the server answered a challenge and could not prove it holds the
  /// key we pinned at pairing.
  ///
  /// **Deliberately not folded into [isOnline].** That flag means "requests to
  /// this server succeed", is inferred from real request outcomes, and a
  /// dropped socket explicitly does not clear it. "Reachable, but not who it
  /// claims to be" is a third state: the traffic is arriving somewhere, and
  /// that somewhere is the problem. Collapsing the two would report an
  /// interception as a wifi hiccup.
  bool _impostor = false;
  bool get serverIdentityFailed => _impostor;

  /// Whether a request may go out at all.
  ///
  /// An unverified server gets nothing — not a note body, not a save. The
  /// point of the check is to avoid handing plaintext to whatever is
  /// answering, so "block sync" has to mean every request, not just the pull.
  /// Offline is re-checked, never latched (§8.9 rule 1): only a connection
  /// waiting out its backoff skips the attempt.
  bool get _mayReachServer => !_impostor && !connection.inBackoff;

  int _pending = 0;
  int get pendingCount => _pending;

  bool _syncing = false;
  bool get isSyncing => _syncing;

  /// When the last sync completed, for the vault bubble to report.
  ///
  /// Advanced only on a sync that actually reached the server — saying "synced
  /// just now" after a failed attempt would be a lie in the one place the user
  /// looks to find out.
  DateTime? _lastSyncedAt;
  DateTime? get lastSyncedAt => _lastSyncedAt;

  /// Note ids touched by the most recent pull, so open editors can react.
  final _changes = StreamController<Set<String>>.broadcast();
  Stream<Set<String>> get changes => _changes.stream;

  WebSocketChannel? _ws;
  StreamSubscription? _wsSub;
  StreamSubscription? _sseSub;
  Timer? _pullDebounce;
  bool _disposed = false;
  bool _started = false;
  final _random = Random();

  _Feed _feed = _Feed.closed;
  int _feedRun = 0;
  Timer? _feedRetry;
  DateTime? _feedRetryDue;
  Duration? _feedBackoff;

  Timer? _outboxTimer;
  DateTime? _outboxDue;
  Duration? _outboxBackoff;
  bool _drainHitStreamError = false;

  /// The active connection's tier (direct, self-hosted relay, public relay).
  /// Used by the UI to show direct-vs-relayed.
  CandidateTier get connectionTier => connection.active.tier;

  /// Whether the active connection is relayed (vs direct LAN).
  bool get isRelayed => connection.isRelayed;

  /// The connection status from the seam (idle, connecting, connected,
  /// unproven, offline, serverOffline).
  ConnectionStatus get connectionStatus => connection.status;

  // ---- lifecycle -----------------------------------------------------

  Future<void> start() async {
    _pending = await cache.pendingCount(vaultId);
    if (_disposed) return;
    notifyListeners();
    _started = true;
    // The feed wants a transport for as long as the engine lives: that is
    // the demand under which the connection's backoff re-races (§8.3).
    connection.addDemand(this);
    final proof = await connection.connect();
    if (_disposed) return;
    switch (proof) {
      case ServerProof.verified:
        // A server that proves itself clears a previous failure.
        _setImpostor(false);
        _onConnected();
      case ServerProof.impostor:
        _setImpostor(true);
      case ServerProof.unreachable:
        // The connection retries on its own backoff; entering `Connected`
        // brings the engine back.
        _setOnline(false);
    }
  }

  /// A proven transport: drain, pull, and make sure the feed is open.
  void _onConnected() {
    if (_disposed || _impostor) return;
    unawaited(sync());
    if (_feed == _Feed.closed) {
      _cancelFeedRetry();
      unawaited(_openSocket());
    }
  }

  /// What a refused write says. Same sentence the pairing screen uses when
  /// the challenge fails there, because it is the same event: something is
  /// answering for the server and cannot prove it is the server.
  static const _impostorMessage =
      'Server failed the cryptographic challenge. Connection may be intercepted.';

  void _setImpostor(bool value) {
    if (_impostor == value) return;
    _impostor = value;
    notifyListeners();
  }

  @override
  void dispose() {
    _disposed = true;
    connection.removeDemand(this);
    _statusSub?.cancel();
    _pullDebounce?.cancel();
    _feedRetry?.cancel();
    _outboxTimer?.cancel();
    _wsSub?.cancel();
    _ws?.sink.close();
    _sseSub?.cancel();
    _changes.close();
    super.dispose();
  }

  void _setOnline(bool value) {
    if (_disposed || _online == value) return;
    _online = value;
    notifyListeners();
    // Coming back online is the moment queued edits must go out.
    if (value && _pending > 0) unawaited(sync());
  }

  /// A request failed with something other than an HTTP answer. A
  /// stream-scoped error is that request's alone (§8.4): it changes no
  /// status, and a queued write is retried by the outbox timer. Anything
  /// else says requests are not getting through; whether the transport is
  /// lost is the connection's call, made where the request was sent.
  void _requestFailed(Object e) {
    if (_streamScoped(e)) return;
    _setOnline(false);
  }

  static bool _streamScoped(Object e) =>
      e is SrpTransportException &&
      e.code != 'trunk_lost' &&
      e.code != 'trunk_superseded';

  // ---- reads ---------------------------------------------------------

  /// Returns a note, preferring the cache so the editor paints immediately.
  ///
  /// When online it still refetches in the background; the caller is told via
  /// [changes] if the server's copy differed.
  Future<CachedNote?> openNote(String id) async {
    final cached = await cache.note(vaultId, id);

    if (!_mayReachServer) return cached;

    try {
      final fresh = await api.note(vaultId, id);
      _setOnline(true);
      // Outside the network judgement: see `_cache`.
      await _cache(() => _store(fresh), 'caching $id');
      return await cache.note(vaultId, id) ?? _asCached(fresh);
    } on StormApiException catch (e) {
      // A definite answer from the server: it is reachable, the note is gone.
      if (e.isNotFound) {
        await cache.removeNote(vaultId, id);
        return null;
      }
      rethrow;
    } catch (e) {
      _requestFailed(e);
      return cached;
    }
  }

  /// Folders from the last successful tree fetch.
  ///
  /// Held here rather than fetched separately so an *empty* folder — one that
  /// exists only because someone created it — can be shown without a second
  /// request. Folders derived from note paths are recomputed by the browser
  /// anyway; these are the ones it could not know about.
  List<String> _folders = const [];
  List<String> get folders => _folders;

  /// The vault tree — from the server when reachable, else from the cache.
  ///
  /// The offline tree only lists notes that happen to be cached, which is
  /// honest: those are the ones that can actually be opened.
  Future<List<NoteMeta>> tree() async {
    if (_mayReachServer) {
      try {
        final vault = await api.tree(vaultId);
        _folders = vault.folders;
        _setOnline(true);
        return vault.notes;
      } catch (e) {
        _requestFailed(e);
      }
    }
    final cached = await cache.allNotes(vaultId);
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
    if (!_mayReachServer) return _queue(id, baseVersion, content);

    try {
      final result = await api.saveNote(
        vaultId: vaultId,
        id: id,
        baseVersion: baseVersion,
        content: content,
      );
      _setOnline(true);
      await _storeWrite(id, result);
      await cache.dequeue(vaultId, id);
      await _refreshPending();
      // After every successful write, whatever else waits goes too (§8.8).
      if (_pending > 0) unawaited(sync());

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
    } catch (e) {
      _requestFailed(e);
      final queued = await _queue(id, baseVersion, content);
      if (_streamScoped(e)) {
        _scheduleOutboxRetry('a save failed: ${logError(e)}');
      }
      return queued;
    }
  }

  Future<SaveOutcome> _queue(String id, int baseVersion, String content) async {
    await cache.enqueue(
      vaultId: vaultId,
      noteId: id,
      baseVersion: baseVersion,
      content: content,
    );
    // Keep the cache showing the user's own text while it waits.
    final existing = await cache.note(vaultId, id);
    if (existing != null) {
      await cache.putNote(
        CachedNotesCompanion(
          vaultId: Value(vaultId),
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

  /// Creates a note and caches it, so it can be opened immediately.
  ///
  /// Never queued: the client cannot invent a server-assigned id, so this
  /// requires a reachable server and says so plainly when there isn't one.
  Future<({NoteMeta? meta, String? error})> create({
    required String path,
    String content = '',
  }) async {
    // An unproven server must not be handed note content, and unlike a
    // save there is no outbox to fall back on — creating offline is
    // already refused, so this refuses the same way.
    if (_impostor) return (meta: null, error: _impostorMessage);
    final WriteResult result;
    try {
      result = await api.createNote(
        vaultId: vaultId,
        path: path,
        content: content,
      );
      _setOnline(true);
    } on StormApiException catch (e) {
      return (meta: null, error: e.message);
    } catch (e) {
      // Never resent: a create whose head went out may have happened (§8.5).
      _requestFailed(e);
      return (
        meta: null,
        error: 'Cannot reach the server — new notes need a connection.',
      );
    }

    // Caching happens *outside* the block above on purpose. It is a local
    // concern, and folding it in made a broken cache schema report itself as
    // "cannot reach the server" and mark the client offline — after which
    // everything else failed as offline too, while the note sat on the server
    // perfectly fine.
    await _cache(() => _storeWrite(result.meta.id, result), 'caching $path');
    return (meta: result.meta, error: null);
  }

  /// A server note shaped like a cached row, for when the cache write failed.
  ///
  /// Returning `null` there would render as "not available offline yet" for a
  /// note the server just handed us.
  CachedNote _asCached(Note note) => CachedNote(
    vaultId: vaultId,
    id: note.meta.id,
    path: note.meta.path,
    title: note.meta.title,
    version: note.meta.version,
    content: note.content,
    modified: note.meta.modified,
    cachedAt: DateTime.now(),
    pinned: false,
  );

  /// Runs a cache write, logging rather than throwing.
  ///
  /// The server is the copy of record; a local cache failure must never be
  /// mistaken for a network one, and must never mark the client offline.
  Future<void> _cache(Future<void> Function() write, String what) async {
    try {
      await write();
    } catch (e) {
      debugPrint('storm: $what failed locally (the server has it): $e');
    }
  }

  /// Renames or moves a note, queueing it when the server is unreachable.
  ///
  /// The note keeps its id, so a queued move and a concurrent edit from
  /// another device both survive — that is what tracking by UUID buys.
  Future<SaveOutcome> move({
    required String id,
    required String newPath,
  }) async {
    if (_mayReachServer) {
      try {
        final result = await api.moveNote(
          vaultId: vaultId,
          id: id,
          newPath: newPath,
        );
        _setOnline(true);
        await _storeWrite(id, result);
        return SaveOutcome(SaveStatus.saved, version: result.meta.version);
      } on StormApiException catch (e) {
        return SaveOutcome(SaveStatus.failed, error: e.message);
      } catch (e) {
        _requestFailed(e);
      }
    }

    final cached = await cache.note(vaultId, id);
    await cache.enqueue(
      vaultId: vaultId,
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
          vaultId: Value(vaultId),
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

  /// Uploads an attachment and returns the vault-relative path to link to.
  ///
  /// Never queued offline: attachments are large and the outbox is meant for
  /// small text diffs, so this needs a reachable server and says so.
  Future<({String? path, String? error})> attach({
    required String fileName,
    required List<int> bytes,
  }) async {
    if (_impostor) return (path: null, error: _impostorMessage);
    final safe = _safeAttachmentName(fileName);
    final path = 'attachments/$safe';
    try {
      await api.uploadAttachment(vaultId, path, bytes);
      _setOnline(true);
      return (path: path, error: null);
    } on StormApiException catch (e) {
      return (path: null, error: e.message);
    } catch (e) {
      _requestFailed(e);
      return (
        path: null,
        error: 'Cannot reach the server — attachments need a connection.',
      );
    }
  }

  /// Makes a filename safe for a vault path, and unique enough not to clobber
  /// an existing attachment with the same name.
  String _safeAttachmentName(String fileName) {
    final cleaned = fileName
        .split('/')
        .last
        .replaceAll(RegExp(r'[^A-Za-z0-9._-]'), '-')
        .replaceAll(RegExp(r'-+'), '-');
    final base = cleaned.startsWith('.') ? 'file$cleaned' : cleaned;
    final stamp = DateTime.now().millisecondsSinceEpoch.toRadixString(36);
    final dot = base.lastIndexOf('.');
    return dot <= 0
        ? '$base-$stamp'
        : '${base.substring(0, dot)}-$stamp${base.substring(dot)}';
  }

  /// Keeps a note available offline, or stops doing so.
  ///
  /// Pinning fetches the note immediately: the point is that it survives
  /// eviction *and* is actually present, which it wouldn't be if the user
  /// pinned something they had only seen in the tree.
  Future<void> setPinned(String id, bool pinned) async {
    // Fetch first: `setPinned` is an UPDATE, so pinning a note that has never
    // been opened would otherwise touch no row at all.
    if (pinned && _mayReachServer) {
      try {
        await _store(await api.note(vaultId, id));
      } catch (e) {
        _requestFailed(e);
      }
    }
    await cache.setPinned(vaultId, id, pinned);
    _changes.add({id});
    notifyListeners();
  }

  Future<Set<String>> pinnedIds() => cache.pinnedIds(vaultId);

  // ---- syncing -------------------------------------------------------

  /// Drains the outbox, then pulls changes. Safe to call at any time.
  Future<void> sync() async {
    // An unproven server gets no requests at all — not the outbox, not the
    // pull. Offline is deliberately *not* checked here: a sync attempt is how
    // the client discovers it is back.
    if (_syncing || _disposed || _impostor) return;
    _syncing = true;
    notifyListeners();
    try {
      await _drainOutbox();
      if (_disposed) return;
      if (_drainHitStreamError) _scheduleOutboxRetry('a queued write failed');
      await _pull();
      if (_online && !_disposed) _lastSyncedAt = DateTime.now();
    } catch (_) {
      // Sign-out or a server/vault switch closes the cache under an in-flight
      // sync; a query already sent to it can still fail.
      if (!_disposed) rethrow;
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
    final queued = await cache.pending(vaultId);
    _drainHitStreamError = false;
    if (queued.isEmpty) {
      _outboxBackoff = null;
      return;
    }

    final touched = <String>{};
    for (final item in queued) {
      if (_disposed) return;
      try {
        if (item.op == 'delete') {
          await api.deleteNote(vaultId, item.noteId);
          await cache.removeNote(vaultId, item.noteId);
        } else if (item.op == 'move') {
          // Move first, then push the content it was queued with. Both are
          // keyed by id, so neither depends on the path being current.
          final moved = await api.moveNote(
            vaultId: vaultId,
            id: item.noteId,
            newPath: item.newPath!,
          );
          await _storeWrite(item.noteId, moved);
          if (item.content.isNotEmpty && item.content != moved.content) {
            final saved = await api.saveNote(
              vaultId: vaultId,
              id: item.noteId,
              baseVersion: item.baseVersion,
              content: item.content,
            );
            await _storeWrite(item.noteId, saved);
          }
        } else {
          final result = await api.saveNote(
            vaultId: vaultId,
            id: item.noteId,
            baseVersion: item.baseVersion,
            content: item.content,
          );
          await _storeWrite(item.noteId, result);
        }
        await cache.dequeue(vaultId, item.noteId);
        touched.add(item.noteId);
        _setOnline(true);
      } on StormApiException catch (e) {
        // The note was deleted elsewhere, or the server refused outright.
        // Retrying forever would wedge the queue behind one bad entry.
        if (e.isNotFound) {
          await cache.dequeue(vaultId, item.noteId);
          await cache.removeNote(vaultId, item.noteId);
          touched.add(item.noteId);
        }
        // Any other refusal: leave it queued for a human to notice.
      } catch (e) {
        // Still unreachable, or this one stream failed: stop, keep the rest
        // queued, try again later.
        _drainHitStreamError = _streamScoped(e);
        _requestFailed(e);
        break;
      }
    }

    await _refreshPending();
    if (_pending == 0) _outboxBackoff = null;
    _emit(touched);
  }

  /// The outbox timer (§8.8): one pending, the earliest wins, and it fires
  /// only into a connected transport, since entering `Connected` drains
  /// anyway.
  void _scheduleOutboxRetry(String why) {
    if (_disposed) return;
    final base = _outboxBackoff ?? outboxRetryMin;
    final next = base * 2;
    _outboxBackoff = next > outboxRetryMax ? outboxRetryMax : next;
    final delay = _jitter(base);
    final due = DateTime.now().add(delay);
    final pending = _outboxDue;
    if (_outboxTimer?.isActive == true &&
        pending != null &&
        !pending.isAfter(due)) {
      return;
    }
    _outboxTimer?.cancel();
    _outboxDue = due;
    ConnectionLog.instance.add(
      'outbox retry in ${delay.inMilliseconds} ms: $why',
    );
    _outboxTimer = Timer(delay, () {
      _outboxDue = null;
      if (_disposed || _pending == 0) return;
      if (connection.phase == ConnectionPhase.racing || connection.inBackoff) {
        return;
      }
      unawaited(sync());
    });
  }

  Duration _jitter(Duration d) => Duration(
    milliseconds: (d.inMilliseconds * (0.8 + _random.nextDouble() * 0.4))
        .round(),
  );

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
    var since = await cache.lastSeq(vaultId);
    final touched = <String>{};

    // Bounded so a server that keeps changing can't spin here forever; the
    // next sync picks up whatever is left.
    for (var page = 0; page < 100; page++) {
      if (_disposed) return;
      final SyncBatch batch;
      try {
        batch = await api.sync(vaultId, since: since, limit: _pageSize);
        _setOnline(true);
      } catch (e) {
        _requestFailed(e);
        _emit(touched);
        return;
      }

      final applied = await _applyChanges(batch.changes, touched);
      if (_disposed) return;
      if (!applied) {
        // A request failed mid-page. Everything up to here is committed;
        // leave `since` where it is so the rest is retried.
        _emit(touched);
        return;
      }

      if (batch.changes.length < _pageSize) {
        // Caught up: safe to adopt the server's position.
        await cache.setLastSeq(vaultId, batch.seq);
        break;
      }

      // A full page means there is more. Resume from the last one applied.
      since = batch.changes.last.seq;
      await cache.setLastSeq(vaultId, since);
    }

    if (_disposed) return;
    await cache.evict(vaultId);
    _emit(touched);
  }

  void _emit(Set<String> touched) {
    if (touched.isNotEmpty && !_disposed) _changes.add(touched);
  }

  /// Applies one page. Returns false if the server became unreachable.
  Future<bool> _applyChanges(List<Change> changes, Set<String> touched) async {
    for (final change in changes) {
      if (_disposed) return false;
      if (change.kind == 'deleted') {
        await cache.removeNote(vaultId, change.noteId);
        touched.add(change.noteId);
        continue;
      }

      // Only pull content for notes we actually hold. The tree comes from
      // /v1/tree, so there is no need to hydrate the whole vault.
      final held = await cache.note(vaultId, change.noteId);
      if (held == null) continue;

      // Don't overwrite a note whose local edit hasn't been sent yet.
      if (await cache.outboxFor(vaultId, change.noteId) != null) continue;
      if (_disposed) return false;

      final Note fresh;
      try {
        fresh = await api.note(vaultId, change.noteId);
      } on StormApiException catch (e) {
        if (_disposed) return false;
        if (e.isNotFound) {
          await cache.removeNote(vaultId, change.noteId);
          touched.add(change.noteId);
        }
        continue;
      } catch (e) {
        _requestFailed(e);
        return false; // leave lastSeq alone so this page is retried
      }
      if (_disposed) return false;

      // Storing is local, and failing to store must not stall the cursor.
      // Conflating the two meant a bad cache schema pinned `lastSeq` forever:
      // every sync re-pulled the same page, failed the same way, and flipped
      // the client offline — which read as "sync got slow".
      await _cache(() => _store(fresh), 'caching ${change.noteId}');
      touched.add(change.noteId);
    }
    return true;
  }

  // ---- change feed -----------------------------------------------------

  /// Opens the change feed so other devices' edits arrive promptly.
  ///
  /// Uses WebSocket on direct connections, SSE on relayed connections (the
  /// relay tunnels HTTP, not WebSocket upgrades). The feed only signals
  /// *that* something changed; the authoritative list still comes from
  /// `GET /v1/sync?since=`. A dropped or missed frame costs nothing — the
  /// next pull catches up regardless.
  Future<void> _openSocket() async {
    if (_disposed || _impostor || _feed != _Feed.closed) return;
    _feed = _Feed.opening;
    final run = ++_feedRun;
    _wsSub?.cancel();
    _ws?.sink.close();
    _sseSub?.cancel();

    // A handshake carries no headers, so the credential has to ride in the
    // URL — and a URL is logged by proxies and kept in history. So it is a
    // **ticket**: one use, sixty seconds, minted over a normal authenticated
    // request that does carry a header. The session token itself never
    // appears in a URL.
    final String ticket;
    try {
      ticket = await api.wsTicket();
    } catch (e) {
      _feedEnded(run, 'no feed ticket: ${logError(e)}');
      return;
    }
    if (_disposed) return;

    if (connection.isRelayed) {
      await _openSseStream(run, ticket);
    } else {
      await _openWebSocket(run, ticket);
    }
  }

  /// Opens a WebSocket change feed (direct LAN connection).
  Future<void> _openWebSocket(int run, String ticket) async {
    final uri = connection.streamUri(ticket);

    try {
      final channel = connectFeed(uri);
      _ws = channel;

      _wsSub = channel.stream.listen(
        (_) {
          _feedHeard();
          // Coalesce bursts: a rename or a multi-note edit fires several
          // frames, and one pull covers them all.
          _pullDebounce?.cancel();
          _pullDebounce = Timer(const Duration(milliseconds: 300), sync);
        },
        onError: (Object e) => _feedEnded(run, 'feed error: ${logError(e)}'),
        onDone: () => _feedEnded(run, 'feed closed'),
        cancelOnError: true,
      );

      // The server subscribes before answering the upgrade, so from here no
      // change is missed. A failed handshake also errors the stream.
      await channel.ready;
      if (run == _feedRun && _feed == _Feed.opening) _feed = _Feed.open;
    } catch (e) {
      _feedEnded(run, 'feed socket failed: ${logError(e)}');
    }
  }

  /// Opens an SSE change feed (relayed connection).
  Future<void> _openSseStream(int run, String ticket) async {
    final uri = connection.sseUri(ticket);
    final client = connection.transportClient;

    try {
      final request = http.Request('GET', uri);
      request.headers.addAll(connection.authHeaders);
      request.headers['Accept'] = 'text/event-stream';

      final streamed = await client.send(request);
      if (_disposed) {
        unawaited(streamed.stream.listen(null).cancel());
        return;
      }

      if (streamed.statusCode != 200) {
        _feedEnded(run, 'feed answered ${streamed.statusCode}');
        return;
      }

      if (run == _feedRun && _feed == _Feed.opening) _feed = _Feed.open;
      _sseSub = streamed.stream
          .transform(utf8.decoder)
          .transform(const LineSplitter())
          .listen(
            _handleSseLine,
            onError: (Object e) =>
                _feedEnded(run, 'feed error: ${logError(e)}'),
            onDone: () => _feedEnded(run, 'feed closed'),
            cancelOnError: true,
          );
    } catch (e) {
      _feedEnded(run, 'feed request failed: ${logError(e)}');
    }
  }

  /// Handles one line from the SSE stream.
  void _handleSseLine(String line) {
    // SSE format: lines of "field: value", blank line ends an event.
    // We only care about "event: change" and "event: resync".
    if (line.startsWith('event:')) {
      final event = line.substring(6).trim();
      if (event == 'change' || event == 'resync') {
        _feedHeard();
        _pullDebounce?.cancel();
        _pullDebounce = Timer(const Duration(milliseconds: 300), sync);
      }
    }
  }

  void _feedHeard() {
    _setOnline(true);
    _feedBackoff = null;
  }

  /// Simulates a change frame arriving on the feed, for tests.
  @visibleForTesting
  void debugSimulateChangeFrame() => _handleSseLine('event: change');

  @visibleForTesting
  bool get isDisposed => _disposed;

  /// Simulates the change feed closing, for tests.
  @visibleForTesting
  void debugSimulateSocketDrop() => _feedEnded(_feedRun, 'simulated drop');

  /// The feed ended. It re-opens on its own backoff, over whatever transport
  /// is connected then. It never resets the connection or cancels a race:
  /// on a phone, the 113 build's feed timer did both (§8.3, §8.4).
  void _feedEnded(int run, String why) {
    if (_disposed || run != _feedRun || _feed == _Feed.closed) return;
    _feed = _Feed.closed;
    _scheduleFeedRetry(why);
  }

  /// At most one feed retry pending; the earliest wins.
  void _scheduleFeedRetry(String why) {
    final base = _feedBackoff ?? feedRetryMin;
    final next = base * 2;
    _feedBackoff = next > feedRetryMax ? feedRetryMax : next;
    final delay = _jitter(base);
    final due = DateTime.now().add(delay);
    final pending = _feedRetryDue;
    if (_feedRetry?.isActive == true &&
        pending != null &&
        !pending.isAfter(due)) {
      ConnectionLog.instance.add('feed retry kept: an earlier one is pending');
      return;
    }
    _feedRetry?.cancel();
    _feedRetryDue = due;
    ConnectionLog.instance.add(
      'feed re-open in ${delay.inMilliseconds} ms: $why',
    );
    _feedRetry = Timer(delay, _feedRetryFired);
  }

  void _cancelFeedRetry() {
    _feedRetry?.cancel();
    _feedRetry = null;
    _feedRetryDue = null;
  }

  /// Ensures connected, then re-opens the feed. Joining a race in flight,
  /// or finding the connection waiting out a backoff, is all it does there:
  /// entering `Connected` re-opens the feed.
  Future<void> _feedRetryFired() async {
    _feedRetry = null;
    _feedRetryDue = null;
    if (_disposed || _feed != _Feed.closed) return;
    ConnectionLog.instance.add(
      'feed retry: ensure connected (${connection.phase.name})',
    );
    final proof = await connection.connect();
    if (_disposed || proof != ServerProof.verified) return;
    await _openSocket();
  }

  // ---- helpers -------------------------------------------------------

  /// Caches a note fetched from the server.
  ///
  /// Carries the existing `pinned` flag forward. Without that, every refresh
  /// of a pinned note silently unpinned it — the insert would fall back to the
  /// column default and the note would quietly become evictable again.
  Future<void> _store(Note note) async {
    final existing = await cache.note(vaultId, note.meta.id);
    await cache.putNote(
      CachedNotesCompanion.insert(
        vaultId: Value(vaultId),
        id: note.meta.id,
        path: note.meta.path,
        title: Value(note.meta.title),
        version: note.meta.version,
        content: note.content,
        modified: Value(note.meta.modified),
        cachedAt: DateTime.now(),
        pinned: Value(existing?.pinned ?? false),
      ),
    );
  }

  Future<void> _storeWrite(String id, WriteResult result) async {
    final existing = await cache.note(vaultId, id);
    await cache.putNote(
      CachedNotesCompanion.insert(
        vaultId: Value(vaultId),
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
    if (_disposed) return;
    _pending = await cache.pendingCount(vaultId);
    if (!_disposed) notifyListeners();
  }
}

enum _Feed { closed, opening, open }
