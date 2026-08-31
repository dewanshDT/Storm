/// The client half of an SRP trunk (§5): one WebSocket carrying every HTTP
/// exchange a client has with one server, multiplexed by `stream_id`.
///
/// This is D2 from the *Storm Relay Dart Client* design: one trunk, not one
/// per request. One trunk means one `HELLO` against the relay's per-IP rate
/// limit, one failure domain, and reconnect logic in exactly one place. It
/// mirrors what `apps/relay/src/connect.rs` serves and what the origin's
/// tunnel client does in `apps/server/src/relay/`.
///
/// The client presents **no credential to the relay** (R12). The user's
/// credential rides inside each tunnelled request and is checked by the origin
/// exactly as on the LAN. All this class does is move frames.
library;

import 'dart:async';
import 'dart:collection';
import 'dart:math';
import 'dart:typed_data';

import 'package:web_socket_channel/web_socket_channel.dart';

import 'srp_codec.dart';

/// One logical HTTP exchange over a trunk, tied to a relay-assigned
/// `stream_id`.
///
/// The trunk feeds decoded response pieces into a stream, and the `SrpHttpClient`
/// reads status/headers/body out of it. Nothing here knows what the request
/// was — it is transport (D1, R13).
class SrpStream {
  SrpStream._(this.attemptId) : _ready = Completer<int>() {
    _events = StreamController<SrpStreamEvent>.broadcast(sync: true);
  }

  /// Echoed by the relay only so this stream can be correlated with the
  /// `stream_id` it assigns (§5.1). Dropped once the stream is established.
  final String attemptId;

  final Completer<int> _ready;

  /// The relay-assigned id, once `STREAM_READY` arrives.
  int? _streamId;

  /// A broadcast pipe of decoded response pieces. Broadcast and synchronous so
  /// the Http client can subscribe before the request head even goes out and
  /// not miss the head that answers it — the response may start the instant the
  /// request does.
  late final StreamController<SrpStreamEvent> _events;

  /// The request head MAYBE sent before ready; body chunks MUST follow ready.
  bool _final = false;

  /// Fires with the relay-assigned `stream_id` once `STREAM_READY` arrives.
  Future<int> get ready => _ready.future;

  /// Whether `STREAM_READY` has arrived and the origin has ACKed the open.
  bool get isReady => _streamId != null;

  /// Where decoded response events arrive.
  Stream<SrpStreamEvent> get events => _events.stream;

  /// The relay-assigned id, valid only after [ready].
  int get streamId => _streamId!;

  /// Called by [SrpTrunk] when `STREAM_READY` arrives.
  void signalReady(int streamId) {
    if (_streamId != null) return; // double-open is the trunk's problem
    _streamId = streamId;
    _ready.complete(streamId);
  }

  /// Called by [SrpTrunk] to deliver a decoded response event.
  ///
  /// Safe to call before [ready] resolves — the events pipe is independent of
  /// the ack, because a fast response can arrive while the open is still being
  /// correlated.
  void push(SrpStreamEvent event) {
    if (_final) return;
    _events.add(event);
  }

  /// Called by [SrpTrunk] when the stream is finished — a `CLOSE` for it, a
  /// stream-scoped `ERROR`, or trunk loss.
  void finish() {
    if (_final) return;
    _final = true;
    if (!_ready.isCompleted) _ready.completeError(const SrpStreamAborted());
    if (!_events.isClosed) _events.close();
  }

  bool get isFinal => _final;
}

/// One decoded response event the trunk hands a stream.
sealed class SrpStreamEvent {
  const SrpStreamEvent();
}

/// `HTTP_RESPONSE_HEAD` (§5.2): status and headers known.
class SrpResponseHead extends SrpStreamEvent {
  const SrpResponseHead(this.status, this.headers);
  final int status;

  /// Header map as it crossed the wire.
  final Map<String, String> headers;
}

/// One `0x02` response-body chunk.
class SrpBodyChunk extends SrpStreamEvent {
  const SrpBodyChunk(this.bytes);
  final Uint8List bytes;
}

/// A stream-scoped error or a `CLOSE` for this stream.
class SrpStreamError extends SrpStreamEvent {
  const SrpStreamError(this.code, [this.message]);
  final String code;
  final String? message;
}

/// One relay at a time. Create a fresh trunk for each relayed connection; a
/// dropped trunk re-races from scratch rather than migrating (§2).
class SrpTrunk {
  SrpTrunk({
    required this.url,
    required this.serverId,
    this.connector = WebSocketChannel.connect,
    this.heartbeat = const Duration(seconds: 15),
    this.handshakeTimeout = const Duration(seconds: 5),
    this.openTimeout = const Duration(seconds: 5),
    this.maxQueuedFrames = 256,
  });

  /// The full `wss://<relay>/connect/<server_id>` URL to dial.
  final Uri url;
  final String serverId;

  /// Test seam: the real default opens a socket to [url].
  final WebSocketChannel Function(Uri) connector;

  /// `PING`/`PONG` cadence (§4.2).
  final Duration heartbeat;
  final Duration handshakeTimeout;
  final Duration openTimeout;

  /// Bounded outbound queue, for the same reason the origin's tunnel client
  /// bounds its own: a relay that stops reading must apply backpressure, not
  /// let response tasks buffer without limit.
  final int maxQueuedFrames;

  final Map<int, SrpStream> _streams = {};
  final HashMap<String, SrpStream> _pendingOpens = HashMap();

  WebSocketChannel? _ws;
  StreamSubscription<dynamic>? _sub;
  Timer? _heartbeatTimer;
  bool _closed = false;

  final Completer<void> _connected = Completer<void>();

  final _onTrunkLost = StreamController<String>.broadcast();
  Stream<String> get onTrunkLost => _onTrunkLost.stream;

  bool get isConnected => _ws != null && !_closed && _connected.isCompleted;

  /// Dial and complete the `HELLO`/`READY` handshake (§5).
  ///
  /// The trunk is not usable until this future resolves. A relay that answers
  /// nothing is `SrpHandshakeFailed` — the caller decides what offline means.
  Future<void> connect() async {
    if (_closed) return Future.error(const SrpTrunkClosed());
    if (isConnected) return _connected.future;
    await _dial();
    return _connected.future.timeout(
      handshakeTimeout,
      onTimeout: () => throw const SrpHandshakeFailed(),
    );
  }

  Future<void> _dial() async {
    try {
      final ws = connector(url);
      _ws = ws;
      // `connect` reports a failed handshake through `ready`, not by throwing.
      // With nothing listening, an unreachable host becomes an unhandled async
      // error in the zone. This observes the rejection; liveness is driven by
      // the stream's onError/onDone below.
      unawaited(ws.ready.catchError((Object _) => _fail('socket_error')));
      _sub = ws.stream.listen(
        _onFrame,
        onError: (Object _) => _fail('socket_error'),
        onDone: () => _fail('socket_closed'),
        cancelOnError: true,
      );
      _send(SrpControl.encode('HELLO', {'server_id': serverId}));
    } catch (_) {
      _fail('socket_error');
    }
  }

  void _onFrame(dynamic raw) {
    if (raw is String) {
      _onControl(raw);
    } else if (raw is List<int>) {
      _onBody(raw);
    } else {
      _fail('protocol_error');
    }
  }

  void _onControl(String text) {
    SrpControl frame;
    try {
      frame = SrpControl.decode(text);
    } on SrpProtocolError {
      _fail('protocol_error');
      return;
    }
    switch (frame.type) {
      case 'READY':
        if (!_connected.isCompleted) _connected.complete();
        _startHeartbeat();
      case 'STREAM_READY':
        _onStreamReady(frame);
      case 'HTTP_RESPONSE_HEAD':
        _onResponseHead(frame);
      case 'PING':
        _send(SrpControl.encode('PONG'));
      case 'PONG':
        break; // Liveness is implied by hearing anything.
      case 'CLOSE':
        _onClose(frame);
      case 'ERROR':
        _onError(frame);
      default:
        // An unknown type at v:1 is a relay that has grown a message this
        // build does not know. Ignored rather than fatal, matching the origin
        // side: taking a working tunnel down over a message addressed to a
        // later slice would make every relay upgrade an outage.
        break;
    }
  }

  void _onStreamReady(SrpControl frame) {
    final attemptId = frame.body['attempt_id'];
    final streamId = frame.body['stream_id'];
    if (attemptId is! String || streamId is! int) {
      _fail('protocol_error');
      return;
    }
    final stream = _pendingOpens.remove(attemptId);
    if (stream == null) return; // We never asked, or already aborted.
    stream.signalReady(streamId);
    _streams[streamId] = stream;
  }

  void _onResponseHead(SrpControl frame) {
    final streamId = frame.body['stream_id'];
    if (streamId is! int) return;
    final status = frame.body['status'];
    if (status is! int) {
      _fail('protocol_error');
      return;
    }
    final headers = _parseHeaders(frame.body['headers']);
    final stream = _streams[streamId];
    if (stream == null) return;
    stream.push(SrpResponseHead(status, headers));
  }

  Map<String, String> _parseHeaders(Object? raw) {
    final out = <String, String>{};
    if (raw is Map) {
      for (final e in raw.entries) {
        if (e.key is String && e.value is String) {
          out[e.key] = e.value as String;
        }
      }
    } else if (raw is List) {
      for (final e in raw) {
        if (e is List && e.length == 2 && e[0] is String && e[1] is String) {
          out[e[0] as String] = e[1] as String;
        }
      }
    }
    return out;
  }

  void _onBody(List<int> raw) {
    final frame = decodeBodyFrame(raw);
    if (frame == null || frame.kind != bodyResponse) {
      _fail('protocol_error');
      return;
    }
    final stream = _streams[frame.streamId];
    if (stream == null) return;
    stream.push(SrpBodyChunk(frame.payload));
  }

  void _onClose(SrpControl frame) {
    final streamId = frame.body['stream_id'];
    if (streamId is! int) {
      // `CLOSE {}` — the whole trunk (§5.4).
      _fail('closed');
      return;
    }
    final stream = _streams.remove(streamId);
    if (stream != null) {
      // `CLOSE { stream_id }` is the end of a response (§5.4) as well as a
      // teardown. The stream just completes — whether that is a finished body
      // or a refused open is for the consumer to tell from whether it saw a
      // head. No error event: a normal response is not an error.
      stream.finish();
    }
  }

  void _onError(SrpControl frame) {
    final streamId = frame.body['stream_id'];
    final code = (frame.body['code'] as String?) ?? 'unknown';
    final message = frame.body['message'] as String?;
    if (streamId is! int) {
      // A trunk-scoped error is fatal.
      _fail('relay_error');
      return;
    }
    final stream = _streams.remove(streamId);
    if (stream != null) {
      stream.push(SrpStreamError(code, message));
      stream.finish();
    }
  }

  /// Opens one stream (sends `OPEN_STREAM`, waits for `STREAM_READY`).
  ///
  /// A refusal (`ERROR{rate_limited}`) surfaces by timing out [openTimeout]
  /// and completing the returned stream's `finish()` — §5.1 refuses immediately
  /// rather than queueing, so a caller that discovers the in-flight cap this
  /// way has already turned a queueing problem into an error.
  Future<SrpStream> openStream() async {
    // The trunk only comes alive on first use: `StormConnection` hands a
    // wrapped client to the app before any request is queued, so dialling here
    // — not at construction — is what keeps an idle session socket-free.
    if (!isConnected) {
      await connect();
    }
    final attemptId = _newAttemptId();
    final stream = SrpStream._(attemptId);
    _pendingOpens[attemptId] = stream;
    Timer(openTimeout, () {
      if (_pendingOpens.remove(attemptId) == stream) {
        stream.finish();
      }
    });
    _send(SrpControl.encode('OPEN_STREAM', {'attempt_id': attemptId}));
    await stream.ready;
    return stream;
  }

  /// Sends `HTTP_REQUEST_HEAD` for [stream].
  void sendRequestHead(
    SrpStream stream, {
    required String method,
    required String path,
    required Map<String, String> headers,
  }) {
    assert(stream.isReady, 'request head must follow STREAM_READY (§5.2)');
    _send(
      SrpControl.encode('HTTP_REQUEST_HEAD', {
        'stream_id': stream.streamId,
        'method': method,
        'path': path,
        'headers': headers,
      }),
    );
  }

  /// Sends one `0x01` request-body chunk for [stream].
  void sendBody(SrpStream stream, List<int> bytes) {
    _send(encodeBodyFrame(bodyRequest, stream.streamId, bytes));
  }

  /// Sends `CLOSE { stream_id }` and forgets the stream (§5.4).
  void closeStream(SrpStream stream) {
    if (stream.isReady) {
      try {
        _send(SrpControl.encode('CLOSE', {'stream_id': stream.streamId}));
      } on SrpTrunkClosed {
        // The trunk is already gone; every stream died with it.
      }
    }
    _streams.remove(stream.streamId);
    _pendingOpens.remove(stream.attemptId);
    stream.finish();
  }

  void _send(Object frame) {
    final ws = _ws;
    if (ws == null || _closed) {
      throw const SrpTrunkClosed();
    }
    ws.sink.add(frame);
  }

  void _startHeartbeat() {
    _heartbeatTimer?.cancel();
    _heartbeatTimer = Timer.periodic(heartbeat, (_) {
      if (!isConnected) return;
      try {
        _send(SrpControl.encode('PING'));
      } on SrpTrunkClosed {
        _fail('closed');
      }
    });
  }

  /// Terminal. Closes every stream, stops the timers, and tells the owner.
  void _fail(String reason) {
    if (_closed) return;
    _closed = true;
    _heartbeatTimer?.cancel();
    _sub?.cancel();
    _ws?.sink.close();
    for (final s in _streams.values) {
      s.push(SrpStreamError('trunk_lost', reason));
    }
    final all = [..._streams.values, ..._pendingOpens.values];
    _streams.clear();
    _pendingOpens.clear();
    for (final s in all) {
      s.finish();
    }
    if (!_connected.isCompleted) _connected.completeError(SrpHandshakeFailed());
    if (!_onTrunkLost.isClosed) _onTrunkLost.add(reason);
  }

  String _newAttemptId() {
    // `attempt_id` only needs to be plausible and unique per trunk run, and
    // bounded (§5.1). A non-cryptographic RNG is fine — unlike a challenge
    // nonce, which is why `server_verifier` uses `Random.secure()`.
    final rng = Random();
    const alpha =
        'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789';
    return String.fromCharCodes(
      List.generate(16, (_) => alpha.codeUnitAt(rng.nextInt(alpha.length))),
    );
  }

  /// Releases the socket and all streams. The owner re-races from scratch.
  void dispose() {
    _fail('closed');
    if (!_onTrunkLost.isClosed) _onTrunkLost.close();
  }
}

/// The trunk or its streams are gone.
class SrpTrunkClosed implements Exception {
  const SrpTrunkClosed();
}

/// The `HELLO`/`READY` handshake never completed within [SrpTrunk.handshakeTimeout].
class SrpHandshakeFailed implements Exception {
  const SrpHandshakeFailed();
}

/// The task that asked for a stream is gone before the open completed.
class SrpStreamAborted implements Exception {
  const SrpStreamAborted();
}
