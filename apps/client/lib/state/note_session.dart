import 'dart:async';

import 'package:flutter/foundation.dart';

import '../api/models.dart';
import '../editor/frontmatter.dart' as fm;
import '../editor/frontmatter_edit.dart' as fme;
import '../sync/sync_engine.dart';
import 'replay_edit.dart';

/// How long typing must pause before a save fires.
const kSaveDebounce = Duration(milliseconds: 800);

/// What happened to the last save, for the UI to report.
enum SaveState { idle, dirty, saving, saved, queued, failed }

/// Owns one open note and its save lifecycle.
///
/// Deliberately a plain [ChangeNotifier] rather than a Riverpod notifier: the
/// interesting behaviour is the save protocol, and keeping it free of the DI
/// framework means it can be unit-tested directly.
///
/// All I/O goes through [SyncEngine], never [StormApi], so offline behaviour is
/// implemented once rather than duplicated here.
///
/// The protocol it implements:
///
///  * every save carries the `base_version` the buffer was edited from;
///  * a `merged` or `conflict` answer means the server reconciled against a
///    version this client never saw, so the server's text **replaces** the
///    buffer — keeping our own would make the next save race a version we
///    never had;
///  * …except for what was typed while that save was in flight, which is
///    replayed onto the server's text, or kept on the old base when it cannot
///    be placed, so the server merges it (decision 128b);
///  * an edit is never marked saved until the server has the exact bytes.
class NoteSession extends ChangeNotifier {
  NoteSession(this._engine);

  SyncEngine _engine;
  set engine(SyncEngine value) => _engine = value;

  String? _noteId;
  String? get noteId => _noteId;

  NoteMeta? _meta;
  NoteMeta? get meta => _meta;

  /// The version the current buffer is based on. Advances on every save.
  int _baseVersion = 0;
  int get baseVersion => _baseVersion;

  /// The whole file, frontmatter included. This is what gets saved, and what
  /// the merge protocol reasons about.
  String _buffer = '';
  String get buffer => _buffer;

  /// The frontmatter block, verbatim, or `''`. Shown as a properties panel
  /// rather than as raw text in the editor.
  String get frontmatter => fm.split(_buffer).frontmatter;

  /// What the editor actually shows and edits.
  ///
  /// Keeping the frontmatter out of the text field is the only safe way to
  /// hide it: the controller's buffer has to match what it renders character
  /// for character, so anything hidden must not be in the buffer at all.
  String get body => fm.split(_buffer).body;

  List<fm.NoteProperty> get properties => fm.readProperties(frontmatter);
  List<String> get tags => fm.readTags(frontmatter);

  SaveState _saveState = SaveState.idle;
  SaveState get saveState => _saveState;

  String? _error;
  String? get error => _error;

  String? _notice;
  String? get notice => _notice;

  bool _hasConflict = false;
  bool get hasConflict => _hasConflict;

  /// Bumped whenever the buffer is replaced from outside the editor, so the
  /// widget knows to reload its controller rather than keep the user's text.
  int _revision = 0;
  int get revision => _revision;

  Timer? _debounce;
  bool _closed = false;

  /// The save in flight, so [flush] can wait for it.
  Future<void>? _inFlight;

  bool get isOpen => _noteId != null;
  bool get isDirty =>
      _saveState == SaveState.dirty || _saveState == SaveState.saving;

  /// Loads a note, discarding any unsaved buffer for the previous one.
  Future<void> open(String id) async {
    final opening = ++_opening;
    _debounce?.cancel();
    _error = null;
    _failedId = null;
    _notice = null;
    _hasConflict = false;
    _saveState = SaveState.idle;
    _noteId = id;
    notifyListeners();

    try {
      final result = await _engine.open(id);
      // Closed, or another note was opened while this one loaded.
      if (_closed || opening != _opening) return;
      final cached = result.note;
      if (cached == null) {
        _noteId = null;
        _error = switch (result.outcome) {
          NoteOpenOutcome.gone => 'That note no longer exists.',
          NoteOpenOutcome.offline => "That note isn't available offline yet.",
          _ => _couldNotLoad(result.error!),
        };
        if (result.outcome != NoteOpenOutcome.gone) _failedId = id;
        _saveState = SaveState.failed;
        notifyListeners();
        return;
      }
      _meta = NoteMeta(
        id: cached.id,
        path: cached.path,
        title: cached.title,
        version: cached.version,
        modified: cached.modified,
        size: cached.content.length,
      );
      _adopt(cached.content, cached.version);
    } catch (e) {
      if (_closed || opening != _opening) return;
      _noteId = null;
      _error = _couldNotLoad(e);
      _failedId = id;
      _saveState = SaveState.failed;
      notifyListeners();
    }
  }

  static String _couldNotLoad(Object e) => e is StormApiException
      ? "Couldn't load this note: ${e.message}"
      : "Couldn't load this note. The server didn't answer; try again.";

  /// Bumped by every [open] and [close], so a slow open that finishes after
  /// another note was opened is dropped.
  int _opening = 0;

  /// The note that could not be loaded but may still exist, for [retry].
  String? _failedId;
  bool get canRetry => _failedId != null;

  Future<void> retry() async {
    final id = _failedId;
    if (id != null) await open(id);
  }

  void close() {
    _opening++;
    _failedId = null;
    _debounce?.cancel();
    _noteId = null;
    _meta = null;
    _buffer = '';
    _baseVersion = 0;
    _saveState = SaveState.idle;
    _error = null;
    _notice = null;
    _hasConflict = false;
    notifyListeners();
  }

  /// Records an edit to the **body** and schedules a save.
  ///
  /// The frontmatter is carried through untouched — the editor never shows it,
  /// so it must never be able to lose it.
  void editBody(String newBody) {
    if (_noteId == null) return;
    edit(fm.split(_buffer).withBody(newBody));
  }

  /// Records an edit to the **whole file** made by the properties panel.
  ///
  /// The mirror image of [editBody]: the panel splices frontmatter lines and
  /// hands back the complete file, body carried through untouched.
  ///
  /// Goes through [edit], so it gets the debounce, the base version, the merge
  /// protocol and the offline outbox for free — and, crucially, **does not
  /// bump [revision]**. Only `_adopt` does that, and the body editor reacts to
  /// it by replacing its entire text value; a property keystroke that bumped
  /// it would reset the caret in the note body on every character.
  void editProperties(String wholeFile) {
    if (_noteId == null) return;
    edit(wholeFile);
  }

  /// Records a local edit to the whole file and schedules a save.
  void edit(String text) {
    if (_noteId == null || text == _buffer) return;
    _buffer = text;
    _saveState = SaveState.dirty;
    _error = null;
    notifyListeners();

    _debounce?.cancel();
    _debounce = Timer(kSaveDebounce, () => save());
  }

  /// Saves now, cancelling any pending debounce.
  Future<void> save() {
    _debounce?.cancel();
    if (_saveState == SaveState.saving) return _inFlight ?? Future.value();
    final saving = _save();
    _inFlight = saving;
    return saving.whenComplete(() {
      if (identical(_inFlight, saving)) _inFlight = null;
    });
  }

  Future<void> _save() async {
    final id = _noteId;
    if (id == null || _saveState != SaveState.dirty) return;

    _saveState = SaveState.saving;
    _error = null;
    notifyListeners();

    // Captured before the await: the user may keep typing while it's in
    // flight, and we must not claim to have saved text we never sent.
    final sent = _buffer;
    final sentBase = _baseVersion;

    final outcome = await _engine.save(
      id: id,
      baseVersion: sentBase,
      content: sent,
    );
    if (_closed || _noteId != id) return;

    switch (outcome.status) {
      case SaveStatus.conflicted:
        _hasConflict = true;
        _notice =
            'Another device edited this note at the same time. Both '
            'versions are kept below — delete the markers once resolved.';
        _adoptKeepingTyping(sent, outcome);
        return;

      case SaveStatus.merged:
        _hasConflict = false;
        _notice = "Merged with another device's changes.";
        _adoptKeepingTyping(sent, outcome);
        return;

      case SaveStatus.queued:
        // Offline. The edit is durable in the outbox, but the server does not
        // have it yet, so the base version deliberately does not advance.
        if (_buffer != sent) {
          // Typed while it was being queued: the outbox holds only what was
          // sent, so the rest must go through another save.
          _saveState = SaveState.dirty;
          _debounce = Timer(kSaveDebounce, () => save());
        } else {
          _saveState = SaveState.queued;
        }
        notifyListeners();
        return;

      case SaveStatus.failed:
        _error = outcome.error;
        // Stay dirty: the edit is still only in this buffer, and anything
        // else invites losing it.
        _saveState = SaveState.dirty;
        notifyListeners();
        return;

      case SaveStatus.saved:
        _baseVersion = outcome.version!;
        if (_buffer != sent) {
          // Typed while the save was in flight, so the buffer is already
          // ahead of the server. Keep it dirty for the next save.
          _saveState = SaveState.dirty;
          _debounce = Timer(kSaveDebounce, () => save());
        } else {
          _saveState = SaveState.saved;
        }
        notifyListeners();
        return;
    }
  }

  /// Waits for any save in flight, then saves what is still unsaved.
  ///
  /// Returns null when the server has the buffer or the outbox holds it.
  /// A save the server refused leaves the edit only in this buffer, so it is
  /// queued anyway before the caller moves on: the outbox keeps retrying
  /// a refusal rather than dropping it, and the error is returned to show.
  Future<String?> flush() async {
    await _inFlight;
    if (_saveState == SaveState.dirty) await save();
    final id = _noteId;
    if (id == null || _saveState != SaveState.dirty) return null;
    final error = _error ?? 'The server did not take the change.';
    await _engine.keepUnsent(
      id: id,
      baseVersion: _baseVersion,
      content: _buffer,
    );
    _saveState = SaveState.queued;
    notifyListeners();
    return error;
  }

  /// Moves the session to the note its edit was kept as, after the note it
  /// had open was deleted elsewhere (decision 128c).
  Future<void> followRecovered(RecoveredNote recovered) async {
    await open(recovered.newId);
    if (_closed || _noteId != recovered.newId) return;
    _notice =
        'This note was deleted on another device. Your edit was kept as '
        '${recovered.path}.';
    notifyListeners();
  }

  /// Called when the sync engine sees this note change on the server.
  ///
  /// A local edit always wins the race — replacing the buffer under someone's
  /// cursor would destroy work. The incoming change is left for the next save
  /// to merge against.
  Future<void> onRemoteChange() async {
    final id = _noteId;
    if (id == null || isDirty || _saveState == SaveState.queued) return;

    final cached = await _engine.cache.note(_engine.vaultId, id);
    if (_closed || _noteId != id) return;

    if (cached == null) {
      _notice = 'This note was deleted on another device.';
      _noteId = null;
      _buffer = '';
      _revision++;
      notifyListeners();
      return;
    }
    // The server owns `modified:` and rewrites it on every save, so our own
    // write comes back differing from the buffer by exactly that one line.
    // Comparing raw text called that "another device" and put a banner over
    // the note on every keystroke's save. The server blanks the same field
    // before it merges, for the same reason — see `VOLATILE` in `index.rs`.
    if (_sameIgnoringStamp(cached.content, _buffer)) {
      // Still adopt it: the version moved, and saving against a stale base
      // would make the next write a conflict the user never caused.
      _adopt(cached.content, cached.version);
      return;
    }

    _notice = 'Updated from another device.';
    _adopt(cached.content, cached.version);
  }

  /// Whether two versions differ only by the server's own timestamp.
  static bool _sameIgnoringStamp(String a, String b) {
    if (a == b) return true;
    const blank = '@@storm-modified@@';
    // Raw, so neither side is quoted differently from the other.
    return fme.setScalar(a, 'modified', blank, raw: true) ==
        fme.setScalar(b, 'modified', blank, raw: true);
  }

  /// Adopts a `merged` or `conflicted` answer without losing what was typed
  /// while it was in flight (decision 128b).
  void _adoptKeepingTyping(String sent, SaveOutcome outcome) {
    final server = outcome.content!;
    final version = outcome.version!;
    if (_buffer == sent) {
      _adopt(server, version);
      return;
    }
    final replayed = replayEdit(sent: sent, edited: _buffer, server: server);
    if (replayed != null) {
      _buffer = replayed;
      _baseVersion = version;
      _revision++;
    }
    // Otherwise keep the buffer on the base it was edited from: the next save
    // sends that base, and the server's diff3 places the typing.
    _saveState = SaveState.dirty;
    _debounce = Timer(kSaveDebounce, () => save());
    notifyListeners();
  }

  /// Replaces the buffer with authoritative text.
  void _adopt(String content, int version) {
    _buffer = content;
    _baseVersion = version;
    _saveState = SaveState.saved;
    _revision++;
    notifyListeners();
  }

  void dismissNotice() {
    _notice = null;
    notifyListeners();
  }

  @override
  void dispose() {
    _closed = true;
    _debounce?.cancel();
    super.dispose();
  }
}
