import 'dart:async';
import 'dart:typed_data';

import 'package:flutter/foundation.dart';

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
class SessionController extends ChangeNotifier {
  SessionController({required this.api, required this.sessionId, this._open});

  final AgentApi api;
  final String sessionId;

  /// How the stream is opened from an offset. Tests inject one.
  final Stream<TerminalEvent> Function(int offset)? _open;
  final terminal = StormTerminal();

  AgentSession? session;

  /// Why the stream is not connected, if it is not.
  String? streamError;

  /// Why the last input did not reach the agent.
  String? inputError;
  bool connected = false;

  int _offset = 0;
  bool _disposed = false;
  StreamSubscription<TerminalEvent>? _sub;
  Timer? _retry;
  Duration _backoff = const Duration(seconds: 1);

  final _pending = BytesBuilder(copy: false);
  Timer? _flush;
  bool _sending = false;

  Timer? _resizeTimer;
  bool _focusPending = false;

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
    terminal.onImagePaste = () async {
      final image = await readClipboardImage();
      if (image == null) return false;
      unawaited(pasteImages([image]));
      return true;
    };
    terminal.onResize = (_, _) => _queueResize();
    _connect();
  }

  void _connect() {
    if (_disposed) return;
    final open = _open ?? (o) => openTerminalStream(api, sessionId, o);
    _sub = open(_offset).listen(
      _onEvent,
      onError: (Object e) {
        connected = false;
        streamError = describeFailure(e);
        _notify();
        _scheduleReconnect();
      },
      onDone: () {
        connected = false;
        _notify();
        // The server closes the stream once the session has ended; anything
        // else is a drop to recover from.
        if (session?.ended != true) _scheduleReconnect();
      },
      cancelOnError: true,
    );
  }

  void _scheduleReconnect() {
    if (_disposed) return;
    _retry?.cancel();
    _retry = Timer(_backoff, _connect);
    _backoff = _backoff * 2 > const Duration(seconds: 30)
        ? const Duration(seconds: 30)
        : _backoff * 2;
  }

  void _onEvent(TerminalEvent event) {
    switch (event) {
      case StatusEvent(:final session):
        this.session = session;
        connected = true;
        streamError = null;
        _backoff = const Duration(seconds: 1);
      case OutputEvent(:final end, :final bytes):
        // A resumed stream never repeats bytes, but a reconnect racing a
        // replay could; the offset is what decides.
        final start = end - bytes.length;
        if (end <= _offset) break;
        final fresh = start < _offset ? bytes.sublist(_offset - start) : bytes;
        terminal.write(fresh);
        _offset = end;
      case GapEvent(:final to):
        // At the start of a view: clear and render from the oldest retained
        // byte (freeze §14). Mid-view: say so, then carry on.
        if (_offset == 0) {
          terminal.reset();
        }
        terminal.note('Earlier output is no longer retained.');
        _offset = to;
    }
    _notify();
  }

  void _queueInput(Uint8List bytes) {
    if (session?.ended ?? false) return;
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
    _focusPending = true;
    _queueResize();
  }

  void _queueResize() {
    _resizeTimer?.cancel();
    _resizeTimer = Timer(const Duration(milliseconds: 150), () async {
      final cols = terminal.cols, rows = terminal.rows;
      if (cols <= 0 || rows <= 0 || (session?.ended ?? true)) return;
      final focus = _focusPending;
      _focusPending = false;
      try {
        await api.resize(sessionId, cols, rows, focus: focus);
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
    _flush?.cancel();
    _resizeTimer?.cancel();
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
