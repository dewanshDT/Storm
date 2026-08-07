import 'dart:async';

import 'package:flutter/foundation.dart';

import '../api/models.dart';
import '../editor/frontmatter.dart' as fm;
import '../sync/sync_engine.dart';

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

  bool get isOpen => _noteId != null;
  bool get isDirty =>
      _saveState == SaveState.dirty || _saveState == SaveState.saving;

  /// Loads a note, discarding any unsaved buffer for the previous one.
  Future<void> open(String id) async {
    _debounce?.cancel();
    _error = null;
    _notice = null;
    _hasConflict = false;
    _saveState = SaveState.idle;
    _noteId = id;
    notifyListeners();

    try {
      final cached = await _engine.openNote(id);
      if (_closed) return;
      if (cached == null) {
        _noteId = null;
        // Only the server can say a note is gone. Offline it simply isn't
        // here yet, which is a different thing and needs different wording.
        _error = _engine.isOnline
            ? 'That note no longer exists.'
            : "That note isn't available offline yet.";
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
      if (_closed) return;
      _error = '$e';
      _saveState = SaveState.failed;
      notifyListeners();
    }
  }

  void close() {
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
  Future<void> save() async {
    _debounce?.cancel();
    final id = _noteId;
    if (id == null || _saveState == SaveState.saving) return;
    if (_saveState != SaveState.dirty) return;

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
        _adopt(outcome.content!, outcome.version!);
        return;

      case SaveStatus.merged:
        _hasConflict = false;
        _notice = "Merged with another device's changes.";
        _adopt(outcome.content!, outcome.version!);
        return;

      case SaveStatus.queued:
        // Offline. The edit is durable in the outbox, but the server does not
        // have it yet, so the base version deliberately does not advance.
        _saveState = SaveState.queued;
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
    if (cached.content == _buffer) return;

    _notice = 'Updated from another device.';
    _adopt(cached.content, cached.version);
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
