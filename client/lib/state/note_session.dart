import 'dart:async';

import 'package:flutter/foundation.dart';

import '../api/models.dart';
import '../api/storm_api.dart';

/// How long typing must pause before a save fires.
const kSaveDebounce = Duration(milliseconds: 800);

/// What happened to the last save, for the UI to report.
enum SaveState { idle, dirty, saving, saved, failed }

/// Owns one open note and its save lifecycle.
///
/// Deliberately a plain [ChangeNotifier] rather than a Riverpod notifier: the
/// interesting behaviour here is the save/merge protocol, and keeping it free
/// of the DI framework means it can be unit-tested directly.
///
/// The protocol it implements, from the server README:
///
///  * every save carries the `base_version` this client last saw;
///  * when the response comes back `merged` or `conflict`, the server
///    reconciled against a version we never had, so we **must** adopt its text
///    — otherwise the next save races a version we never saw.
class NoteSession extends ChangeNotifier {
  NoteSession(this._api);

  StormApi _api;
  set api(StormApi value) => _api = value;

  Note? _note;
  Note? get note => _note;

  /// The version the current buffer is based on. Advances on every save.
  int _baseVersion = 0;
  int get baseVersion => _baseVersion;

  String _buffer = '';
  String get buffer => _buffer;

  SaveState _saveState = SaveState.idle;
  SaveState get saveState => _saveState;

  String? _error;
  String? get error => _error;

  /// Set when the server merged or conflicted, so the UI can say so.
  String? _notice;
  String? get notice => _notice;
  bool get hasConflict => _hasConflict;
  bool _hasConflict = false;

  /// Set when the server's reconciled text replaced the buffer, so the editor
  /// knows to reload its controller rather than keep the user's own text.
  int _revision = 0;
  int get revision => _revision;

  Timer? _debounce;
  bool _closed = false;

  bool get isOpen => _note != null;
  bool get isDirty => _saveState == SaveState.dirty || _saveState == SaveState.saving;

  /// Loads a note, discarding any unsaved buffer for the previous one.
  Future<void> open(String id) async {
    _debounce?.cancel();
    _error = null;
    _notice = null;
    _hasConflict = false;
    _saveState = SaveState.idle;
    notifyListeners();

    try {
      final fetched = await _api.note(id);
      if (_closed) return;
      _adopt(fetched.content, fetched.meta.version, note: fetched);
    } on StormApiException catch (e) {
      _error = e.message;
      _saveState = SaveState.failed;
      notifyListeners();
    }
  }

  void close() {
    _debounce?.cancel();
    _note = null;
    _buffer = '';
    _baseVersion = 0;
    _saveState = SaveState.idle;
    _error = null;
    _notice = null;
    _hasConflict = false;
    notifyListeners();
  }

  /// Records a local edit and schedules a save.
  void edit(String text) {
    if (_note == null || text == _buffer) return;
    _buffer = text;
    _saveState = SaveState.dirty;
    _error = null;
    notifyListeners();

    _debounce?.cancel();
    _debounce = Timer(kSaveDebounce, () => save());
  }

  /// Saves now, cancelling any pending debounce.
  ///
  /// Safe to call when there is nothing to save.
  Future<void> save() async {
    _debounce?.cancel();
    final current = _note;
    if (current == null || _saveState == SaveState.saving) return;
    if (_saveState != SaveState.dirty) return;

    _saveState = SaveState.saving;
    _error = null;
    notifyListeners();

    // Captured before the await: the user may keep typing while it's in
    // flight, and we must not claim to have saved text we never sent.
    final sent = _buffer;
    final sentBase = _baseVersion;

    try {
      final result = await _api.saveNote(
        id: current.meta.id,
        baseVersion: sentBase,
        content: sent,
      );
      if (_closed) return;

      _baseVersion = result.meta.version;
      _note = Note(meta: result.meta, content: result.content);

      if (result.conflict) {
        _hasConflict = true;
        _notice =
            'Another device edited this note at the same time. Both versions '
            'are kept below — delete the markers once resolved.';
        _adopt(result.content, result.meta.version);
      } else if (result.merged) {
        _hasConflict = false;
        _notice = "Merged with another device's changes.";
        _adopt(result.content, result.meta.version);
      } else if (_buffer != sent) {
        // The user typed while the save was in flight, so the buffer is
        // already ahead of what the server has. Keep it dirty and let the
        // next save carry it.
        _saveState = SaveState.dirty;
        _debounce = Timer(kSaveDebounce, () => save());
        notifyListeners();
        return;
      } else {
        _saveState = SaveState.saved;
        notifyListeners();
        return;
      }
    } on StormApiException catch (e) {
      if (_closed) return;
      _error = e.message;
      // Stay dirty: the edit is still only in this buffer, and dropping back
      // to `failed` alone would let it be silently lost.
      _saveState = SaveState.dirty;
      notifyListeners();
    }
  }

  /// Replaces the buffer with authoritative server text.
  void _adopt(String content, int version, {Note? note}) {
    _buffer = content;
    _baseVersion = version;
    if (note != null) _note = note;
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
