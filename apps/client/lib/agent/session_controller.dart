import 'dart:async';
import 'dart:typed_data';

import 'package:flutter/foundation.dart';
import 'package:flutter/widgets.dart' show AppLifecycleState, WidgetsBinding;

import '../api/models.dart';
import '../ui/states.dart';
import 'agent_api.dart';
import 'agent_models.dart';
import 'image_paste.dart';
import 'terminal_events.dart';
import 'terminal_stream.dart';
import 'terminal_surface.dart';

/// One open session: its stream, its terminal and its input (decision 77d).
///
/// The stream resumes by offset: on a drop it reconnects from the last byte it
/// rendered, with backoff, never `Last-Event-ID`. Input is coalesced briefly
/// and sent as serial POSTs, at most once and never retried (freeze §11.3).
///
/// A session left for another is kept, hidden, with its terminal (decision
/// 154): its stream closes after [hiddenGrace] and resumes from the kept
/// offset when it is shown again. **A hidden terminal never answers, resizes
/// or focuses**, so it cannot speak for a device the person has left.
class SessionController extends ChangeNotifier {
  SessionController({
    required this.api,
    required this.sessionId,
    this._open,
    bool Function()? isForeground,
    Duration Function()? hiddenGrace,
    this.onGone,
  }) : _isForeground = isForeground ?? _appInForeground,
       _hiddenGrace = hiddenGrace ?? (() => streamGrace);

  /// How long a hidden session's stream stays open on native and relayed
  /// connections, so flipping between two sessions reopens nothing.
  static const streamGrace = Duration(seconds: 30);

  final AgentApi api;
  final String sessionId;

  /// How the stream is opened from an offset, asking for a snapshot or not.
  /// Tests inject one.
  final Stream<TerminalEvent> Function(int offset, bool snapshot)? _open;
  final terminal = StormTerminal();

  AgentSession? session;

  /// Why the stream is not connected, if it is not.
  String? streamError;

  /// Why the last input did not reach the agent.
  String? inputError;
  bool connected = false;

  /// The server no longer has this session (a 404 on open), so the stream
  /// is never reopened.
  bool gone = false;

  /// Told once when [gone] becomes true, so a kept controller is let go.
  final VoidCallback? onGone;

  int _offset = 0;
  bool _disposed = false;

  /// The oldest byte the server still held when it sent the last snapshot;
  /// null until one arrives.
  int? snapshotFloor;

  bool _visible = true;
  bool get visible => _visible;

  /// The app is hidden or paused: no stream is open, the visible one's
  /// included, until it comes back.
  bool _background = false;

  final Duration Function() _hiddenGrace;
  Timer? _grace;

  /// A subscription is open (or opening).
  bool _streaming = false;

  /// The next output, gap or snapshot starts a view: the first after
  /// opening, or after a kept terminal was resumed. Such an open asks for a
  /// snapshot (AM110), which the server sends only from 0 or below its floor,
  /// so a kept terminal it still has every byte for resumes exactly.
  bool _viewStart = true;

  /// When the stream was last closed by a hide or a background, so a list
  /// read from before then is not taken as news about the kept offset.
  DateTime? _pausedAt;

  /// Whether this stream has caught up: before it, output is replay, and the
  /// terminal's replies to queries in it are dropped (decision 132, fix b).
  bool _live = false;

  /// An older server sends no `live` event; a stream quiet this long has
  /// caught up all the same, and one this old has too even if an agent
  /// never stops drawing.
  static const liveAfterQuiet = Duration(milliseconds: 500);
  static const liveAfterAtMost = Duration(seconds: 5);
  Timer? _quiet;
  Timer? _quietCap;

  /// Only a device in use answers the agent's queries, so two open devices
  /// never both do.
  final bool Function() _isForeground;

  static bool _appInForeground() {
    final state = WidgetsBinding.instance.lifecycleState;
    return state == null || state == AppLifecycleState.resumed;
  }

  StreamSubscription<TerminalEvent>? _sub;
  Timer? _retry;
  Duration _backoff = const Duration(seconds: 1);

  final _pending = BytesBuilder(copy: false);
  Timer? _flush;
  bool _sending = false;

  Timer? _resizeTimer;
  bool _focusPending = false;

  /// The size the host's PTY has, as last reported or last set from here.
  (int, int)? _hostSize;
  bool _claimedOnOpen = false;

  /// How long keystrokes gather before they are sent as one POST.
  static const coalesce = Duration(milliseconds: 15);

  /// An image paste in flight, or why the last one failed (D15 AM45).
  final imagePaste = ValueNotifier<ImagePasteState?>(null);
  int _pasteSeq = 0;
  Timer? _pasteShow;
  Timer? _pasteClear;

  /// Pasting takes this long before the chip says so.
  static const pasteQuiet = Duration(milliseconds: 300);

  void start() {
    terminal.onInput = _queueInput;
    terminal.onReply = _queueReply;
    terminal.onImagePaste = () async {
      final image = await readClipboardImage();
      if (image == null) return false;
      unawaited(pasteImages([image]));
      return true;
    };
    terminal.onKeyboardImage = (bytes) async {
      final image = await normalizeImage(bytes);
      if (image != null) unawaited(pasteImages([image]));
    };
    terminal.onResize = (_, _) => _queueResize();
    _connect();
  }

  void _connect() {
    if (_disposed || gone || _background) return;
    _retry?.cancel();
    _retry = null;
    final open =
        _open ??
        (o, snapshot) => openTerminalStream(api, sessionId, o, snapshot);
    _live = false;
    _armQuiet();
    _quietCap?.cancel();
    _quietCap = Timer(liveAfterAtMost, () => _live = true);
    _streaming = true;
    _sub = open(_offset, _viewStart).listen(
      _onEvent,
      onError: (Object e) {
        _streaming = false;
        if (e is StormApiException && e.isNotFound) {
          gone = true;
          connected = false;
          streamError = null;
          _notify();
          onGone?.call();
          return;
        }
        _setStream(connected: false, error: describeFailure(e));
        _scheduleReconnect();
      },
      onDone: () {
        _streaming = false;
        _setStream(connected: false, error: streamError);
        // The server closes the stream once the session has ended; anything
        // else is a drop to recover from.
        if (session?.ended != true) _scheduleReconnect();
      },
      cancelOnError: true,
    );
  }

  void _setStream({required bool connected, required String? error}) {
    if (this.connected == connected && streamError == error) return;
    this.connected = connected;
    streamError = error;
    _notify();
  }

  void _scheduleReconnect() {
    if (_disposed || gone || _background) return;
    _retry?.cancel();
    _retry = Timer(_backoff, _connect);
    _backoff = _backoff * 2 > const Duration(seconds: 30)
        ? const Duration(seconds: 30)
        : _backoff * 2;
  }

  /// Only a status rebuilds the screen around the terminal. Output, gap,
  /// snapshot and live change nothing it shows, and the terminal repaints
  /// itself.
  void _onEvent(TerminalEvent event) {
    switch (event) {
      case StatusEvent(:final session):
        this.session = session;
        _hostSize = (session.cols, session.rows);
        // Opening a session is using it. Another device may have sized the
        // PTY since, and its agent would draw for that height on this screen.
        if (!_claimedOnOpen && _visible) {
          _claimedOnOpen = true;
          _claimIfSizeDiffers();
        }
        connected = true;
        streamError = null;
        _backoff = const Duration(seconds: 1);
        _notify();
        // An end below what was rendered is another stream: the host
        // restarted and numbers from 0 again.
        final end = session.outputEnd;
        if (end != null && end < _offset) _restartFromZero();
      case LiveEvent():
        _quiet?.cancel();
        _quietCap?.cancel();
        _live = true;
        _viewStart = false;
      case OutputEvent(:final end, :final bytes):
        if (!_live) _armQuiet();
        _viewStart = false;
        // A resumed stream never repeats bytes, but a reconnect racing a
        // replay could; the offset is what decides.
        final start = end - bytes.length;
        if (end <= _offset) break;
        final fresh = start < _offset ? bytes.sublist(_offset - start) : bytes;
        terminal.write(fresh);
        _offset = end;
      case SnapshotEvent(:final offset, :final floor, :final complete):
        // Always before `live`: whatever the terminal answers while drawing
        // it is dropped, as for replay.
        _live = false;
        _armQuiet();
        _viewStart = false;
        terminal.reset();
        if (!complete) terminal.note('Earlier output is no longer retained.');
        terminal.write(_afterReset(event.data));
        _offset = offset;
        snapshotFloor = floor;
      case GapEvent(:final to):
        // At the start of a view — a kept terminal shown again included —
        // clear and render from the oldest retained byte (freeze §14), or
        // new bytes paint over a screen missing what drew it. Mid-view: say
        // so, then carry on.
        if (_offset == 0 || _viewStart) {
          terminal.reset();
        }
        _viewStart = false;
        terminal.note('Earlier output is no longer retained.');
        _offset = to;
    }
  }

  /// The snapshot's own leading `ESC c` would wipe the note above it; the
  /// terminal was reset already.
  static Uint8List _afterReset(Uint8List data) =>
      data.length >= 2 && data[0] == 0x1b && data[1] == 0x63
      ? Uint8List.sublistView(data, 2)
      : data;

  void _restartFromZero() {
    _closeStream();
    terminal.reset();
    _offset = 0;
    _viewStart = true;
    _connect();
  }

  void _closeStream() {
    _retry?.cancel();
    _retry = null;
    _quiet?.cancel();
    _quietCap?.cancel();
    _live = false;
    unawaited(_sub?.cancel());
    _sub = null;
    _streaming = false;
  }

  /// Nothing shows this session any more. Its stream stays open for the
  /// grace (none on web-direct, where every open `EventSource` holds one of
  /// the browser's six sockets), then closes, keeping the offset.
  void hide() {
    if (!_visible || _disposed) return;
    _visible = false;
    _resizeTimer?.cancel();
    _focusPending = false;
    _grace?.cancel();
    final grace = _hiddenGrace();
    if (grace <= Duration.zero) {
      _pause();
    } else {
      _grace = Timer(grace, _pause);
    }
  }

  /// Shown again: the stream resumes from the kept offset if it closed, and
  /// the PTY is claimed if another device sized it meanwhile.
  void show() {
    if (_visible || _disposed) return;
    _visible = true;
    _grace?.cancel();
    _grace = null;
    _claimedOnOpen = false;
    _resume();
  }

  void _resume() {
    if (_streaming || (_retry?.isActive ?? false)) {
      if (_hostSize != null) {
        _claimedOnOpen = true;
        _claimIfSizeDiffers();
      }
      return;
    }
    // The server closed an ended session's stream after the last byte.
    if (session?.ended ?? false) return;
    _viewStart = true;
    _connect();
  }

  void _pause() {
    _grace?.cancel();
    _grace = null;
    _closeStream();
    connected = false;
    _pausedAt = DateTime.now();
  }

  /// The app is hidden or paused: every stream closes.
  void background() {
    if (_disposed) return;
    _background = true;
    _pause();
  }

  /// The app is back: only a visible session reconnects; a hidden one waits
  /// to be shown.
  void foreground() {
    if (!_background || _disposed) return;
    _background = false;
    if (_visible) {
      _claimedOnOpen = false;
      _resume();
    }
  }

  /// A list read at [askedAt] says the server's output for this session ends
  /// at [end]. Below the kept offset, the host has renumbered its stream, so
  /// the next open starts again from 0. Only a closed stream is judged: an
  /// open one is ahead of any list.
  void checkOutputEnd(int? end, DateTime askedAt) {
    final pausedAt = _pausedAt;
    if (end == null || _streaming || pausedAt == null) return;
    if (!askedAt.isAfter(pausedAt) || end >= _offset) return;
    terminal.reset();
    _offset = 0;
    _viewStart = true;
  }

  /// Roughly what the kept terminal holds, for the keep-alive budget.
  int get estimatedBytes => terminal.estimatedBytes;

  void _armQuiet() {
    _quiet?.cancel();
    _quiet = Timer(liveAfterQuiet, () => _live = true);
  }

  /// Visible, in front and live (decision 154, Q3): a hidden one inside its
  /// grace would otherwise answer for a session in use elsewhere, the double
  /// answer 132 fix b removed.
  void _queueReply(Uint8List bytes) {
    if (!_visible || !_live || !_isForeground()) return;
    if (session?.ended ?? false) return;
    _pending.add(bytes);
    _flush ??= Timer(coalesce, _send);
  }

  void _queueInput(Uint8List bytes) {
    if (!_visible || (session?.ended ?? false)) return;
    // Typing here is using this device, so the PTY follows it (freeze §11.4).
    // Only the device being typed on claims, so two open ones never fight.
    _claimIfSizeDiffers();
    _pending.add(bytes);
    _flush ??= Timer(coalesce, _send);
  }

  Future<void> _send() async {
    _flush = null;
    if (_sending || _pending.isEmpty) return;
    final bytes = _pending.takeBytes();
    _sending = true;
    try {
      await api.input(sessionId, bytes);
      if (inputError != null) {
        inputError = null;
        _notify();
      }
    } catch (e) {
      inputError = e is StormApiException && e.statusCode == 503
          ? 'The host is offline. That input was not sent.'
          : 'That input was not sent: ${describeFailure(e)}';
      _notify();
    } finally {
      _sending = false;
      if (_pending.isNotEmpty) _flush ??= Timer(Duration.zero, _send);
    }
  }

  /// This client is now the one the person is using: the PTY follows it
  /// (freeze §11.4).
  void focus() {
    if (!_visible) return;
    _focusPending = true;
    _queueResize();
  }

  /// Sends this terminal's size when the host's PTY has another one: a
  /// resize from another device, or one of ours that was lost. Either left
  /// the agent drawing for a shorter screen, with blank rows under it.
  void _claimIfSizeDiffers() {
    final cols = terminal.cols, rows = terminal.rows;
    if (cols <= 0 || rows <= 0 || _hostSize == null) return;
    if (_hostSize != (cols, rows) && _resizeTimer?.isActive != true) focus();
  }

  /// A hidden terminal re-fits locally and tells the host nothing: the PTY
  /// follows the device in use (freeze §11.4), and [show] claims it back.
  void _queueResize() {
    _resizeTimer?.cancel();
    if (!_visible) return;
    _resizeTimer = Timer(const Duration(milliseconds: 150), () async {
      final cols = terminal.cols, rows = terminal.rows;
      if (!_visible || cols <= 0 || rows <= 0) return;
      if (session?.ended ?? true) return;
      final focus = _focusPending;
      _focusPending = false;
      try {
        await api.resize(sessionId, cols, rows, focus: focus);
        _hostSize = (cols, rows);
      } catch (_) {
        // A lost resize is redone by the next one; nothing to tell anyone.
      }
    });
  }

  Future<void> end() => api.end(sessionId);

  void _notify() {
    if (!_disposed) notifyListeners();
  }

  /// Stages each image on the host, then pastes its path into the terminal,
  /// as a local terminal does for a dropped file: the agent attaches it.
  /// Nothing is typed unless every image arrived.
  Future<void> pasteImages(List<PastedImage> images) async {
    if (images.isEmpty) return;
    final seq = ++_pasteSeq;
    _pasteClear?.cancel();
    _pasteShow?.cancel();
    _pasteShow = Timer(pasteQuiet, () {
      if (seq == _pasteSeq && !_disposed) {
        imagePaste.value = ImagePasteState.pasting(images.length);
      }
    });
    try {
      final paths = <String>[];
      for (final image in images) {
        paths.add(await api.stageImage(sessionId, image.bytes, image.mime));
      }
      if (seq != _pasteSeq || _disposed) return;
      for (final path in paths) {
        terminal.paste(path);
      }
      imagePaste.value = null;
    } catch (e) {
      if (seq != _pasteSeq || _disposed) return;
      final why = e is StormApiException
          ? e.message
          : 'the image did not arrive';
      imagePaste.value = ImagePasteState.failed('Image not pasted: $why');
      _pasteClear = Timer(const Duration(seconds: 6), dismissPaste);
    } finally {
      _pasteShow?.cancel();
    }
  }

  /// Cancels a paste in flight (its path is never typed), or clears a
  /// failure.
  void dismissPaste() {
    _pasteSeq++;
    _pasteClear?.cancel();
    if (!_disposed) imagePaste.value = null;
  }

  @override
  void dispose() {
    _disposed = true;
    _pasteShow?.cancel();
    _pasteClear?.cancel();
    imagePaste.dispose();
    _sub?.cancel();
    _retry?.cancel();
    _grace?.cancel();
    _flush?.cancel();
    _resizeTimer?.cancel();
    _quiet?.cancel();
    _quietCap?.cancel();
    terminal.dispose();
    super.dispose();
  }
}

@immutable
class ImagePasteState {
  const ImagePasteState.pasting(this.count) : error = null;
  const ImagePasteState.failed(String this.error) : count = 0;

  final int count;
  final String? error;
}
