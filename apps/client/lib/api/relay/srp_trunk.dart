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

import '../connection_log.dart';
import 'srp_codec.dart';
import 'srp_socket.dart';

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

  /// Whether `STREAM_READY` has arrived. It does not say the origin has
  /// acknowledged the open: the relay sends it first (§5.1).
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

  /// Called by [SrpTrunk] when the relay refuses this open (§5.1).
  void refuse(String code, String? message) {
    if (!_ready.isCompleted) {
      _ready.completeError(SrpOpenRefused(code, message));
    }
    finish();
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

/// Codes that mean the relay answered and the *server* behind it did not
/// (§6). What tells "server offline" apart from "relay unavailable".
const srpServerSideCodes = {
  'server_unreachable',
  'server_timeout',
  'trunk_lost',
};

/// [url] with `pubkey=<key>` set (§4.3), replacing any it carried. base64url
/// needs no percent-encoding.
Uri keyedConnectUrl(Uri url, String key) {
  if (key.isEmpty) return url;
  return url.replace(queryParameters: {...url.queryParameters, 'pubkey': key});
}

/// One relay at a time. Create a fresh trunk for each relayed connection; a
/// dropped trunk re-races from scratch rather than migrating (§2).
class SrpTrunk {
  SrpTrunk({
    required Uri url,
    required this.serverId,
    this.serverKey = '',
    this.connector,
    this.heartbeat = const Duration(seconds: 15),
    Duration? silenceLimit,
    this.handshakeTimeout = const Duration(seconds: 8),
    this.dialTimeout = const Duration(seconds: 15),
    this.openTimeout = const Duration(seconds: 15),
    this.maxPendingOpens = 19,
    this.maxQueuedFrames = 256,
    ConnectionLog? log,
  }) : url = keyedConnectUrl(url, serverKey),
       silenceLimit = silenceLimit ?? heartbeat * 3,
       _log = log ?? ConnectionLog.instance;

  final ConnectionLog _log;
  final Stopwatch _age = Stopwatch();

  /// The full `wss://<relay>/connect/<server_id>` URL to dial, with
  /// `?pubkey=` when [serverKey] is known (§4.3).
  final Uri url;
  final String serverId;

  /// The key pinned at pairing, base64url unpadded. Named in `HELLO` and on
  /// the URL so an open relay routes to that registration only (§5). Empty
  /// only before pairing has pinned one.
  final String serverKey;

  /// Test seam. Without one the trunk opens an [SrpSocket], whose writes
  /// wait for the network; a channel's sink only buffers.
  final WebSocketChannel Function(Uri)? connector;

  /// `PING`/`PONG` cadence (§4.2).
  final Duration heartbeat;

  /// How long the relay may say nothing before the trunk is dead: three missed
  /// heartbeats, the allowance the relay gives a server (§4.2). The relay
  /// answers every `PING`, so silence this long is a half-open socket — a
  /// network change or a sleep — that would otherwise hang every request
  /// until its own timeout (decision 97).
  final Duration silenceLimit;

  /// From the WebSocket opening to `READY` (`T_handshake`, §8.8): longer
  /// than the relay's 5 s `HELLO` hold, so a relay with no live server trunk
  /// gets to say `server_unreachable` first. The dial before it is bounded
  /// by [dialTimeout]; a slow TLS dial on LTE must not eat the hold.
  final Duration handshakeTimeout;
  final Duration dialTimeout;

  /// From `OPEN_STREAM` being *written* to `STREAM_READY` (§5.1): never while
  /// the frame is still queued behind an upload.
  final Duration openTimeout;

  /// `cap − 1` (§5.1): the relay refuses past 20 in flight, so a burst of
  /// image loads waits here instead of drawing `rate_limited`.
  final int maxPendingOpens;

  /// How many frames may wait for the socket before a request body waits for
  /// room (§5.2). Control frames never wait and are never refused.
  final int maxQueuedFrames;

  final Map<int, SrpStream> _streams = {};
  final HashMap<String, SrpStream> _pendingOpens = HashMap();

  SrpSocket? _ws;
  StreamSubscription<dynamic>? _sub;

  /// Every JSON control message, in order, written before any queued body
  /// frame (§5.2 "Control before bulk", decision 116b). A stream's head
  /// still precedes its body: the body is queued after it.
  final Queue<_Outbound> _control = Queue();

  /// Request-body frames per stream, taken in turn so a small request's
  /// end of body is not queued behind a whole upload.
  final LinkedHashMap<int, Queue<_Outbound>> _bodies = LinkedHashMap();
  int _bodyFrames = 0;
  Completer<void>? _room;
  bool _writing = false;
  bool _pingUnwritten = false;
  Timer? _heartbeatTimer;
  bool _closed = false;

  /// The code of a trunk-scoped `ERROR` that ended the trunk, if one did.
  String? _relayErrorCode;

  final Completer<void> _connected = Completer<void>();

  final _onTrunkLost = StreamController<String>.broadcast();

  /// Why the trunk died: `relay_error` for a trunk-scoped `ERROR` (its code
  /// is [lossReason]), or one of this side's (`socket_error`,
  /// `socket_closed`, `relay_silent`, `handshake_timeout`, `closed`).
  Stream<String> get onTrunkLost => _onTrunkLost.stream;

  final _onServerUnreachable = StreamController<String>.broadcast();

  /// The relay answered and said the server is not there — at `HELLO`, for a
  /// stream, or for the whole trunk. Carries the [srpServerSideCodes] code.
  Stream<String> get onServerUnreachable => _onServerUnreachable.stream;

  String? _lossReason;

  /// Time since the relay last sent anything, checked at each heartbeat. It
  /// stops while this side's own `PING` waits to be written: a relay cannot
  /// answer what it has not been sent (§5.2).
  final Stopwatch _sinceHeard = Stopwatch();

  /// Frames received for any stream: heads, bodies, `STREAM_READY`, `CLOSE`,
  /// `ERROR`. Not `PING`/`PONG`, which prove only the relay is there.
  int get streamFramesReceived => _streamFrames;
  int _streamFrames = 0;

  /// Why the trunk is dead: the relay's own `ERROR` code when it sent one,
  /// otherwise the [onTrunkLost] reason.
  String? get lossReason => _relayErrorCode ?? _lossReason;

  /// The relay's `ERROR` code when it refused the `HELLO` itself — what tells
  /// "server offline" from "relay unavailable" in a race (§8.6).
  String? get helloRefusal => _helloRefusal;
  String? _helloRefusal;

  /// Whether the relay has sent anything at all. A relay that never answered
  /// says the device's network is in doubt, which is what keeps a refused
  /// direct address from reading as "server offline" (§8.6).
  bool get answered => _answered;
  bool _answered = false;

  final List<Completer<void>> _probes = [];
  int _openSlots = 0;
  final Queue<Completer<void>> _openWaiters = Queue();

  bool get isClosed => _closed;

  bool get isConnected => _ws != null && !_closed && _connected.isCompleted;

  /// Dial and complete the `HELLO`/`READY` handshake (§5).
  ///
  /// The trunk is not usable until this future resolves. A relay that answers
  /// nothing is `SrpHandshakeFailed` — the caller decides what offline means.
  ///
  /// One dial per trunk: a caller arriving mid-handshake joins it. Dialling
  /// again would send a second `HELLO` and orphan the first socket, whose
  /// close then failed the trunk (decision 113).
  Future<void> connect() {
    if (_closed) return Future.error(const SrpTrunkClosed());
    if (isConnected) return _connected.future;
    return _handshake ??= _handshakeOnce();
  }

  Future<void>? _handshake;

  Future<void> _handshakeOnce() async {
    _dial();
    final ws = _ws;
    if (ws != null && !_closed) {
      try {
        await ws.ready.timeout(dialTimeout);
      } on TimeoutException {
        _fail('dial_timeout');
      } catch (_) {
        _fail('socket_error');
      }
    }
    return _connected.future.timeout(
      handshakeTimeout,
      onTimeout: () {
        // Failed rather than left dialling: a caller that retries would
        // otherwise open a second socket beside the first.
        _fail('handshake_timeout');
        throw const SrpHandshakeFailed();
      },
    );
  }

  void _dial() {
    _age.start();
    _log.add('trunk dial ${logHost(url.toString())}');
    try {
      final channel = connector?.call(url);
      final ws = channel == null
          ? SrpSocket.connect(url)
          : SrpSocket.channel(channel);
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
      _send(
        SrpControl.encode('HELLO', {
          'server_id': serverId,
          if (serverKey.isNotEmpty) 'pubkey': serverKey,
        }),
      );
    } catch (_) {
      _fail('socket_error');
    }
  }

  void _onFrame(dynamic raw) {
    _sinceHeard.reset();
    for (final p in _probes) {
      if (!p.isCompleted) p.complete();
    }
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
    _answered = true;
    if (frame.type != 'PING' && frame.type != 'PONG') _streamFrames++;
    switch (frame.type) {
      case 'READY':
        _log.add('trunk ready after ${_age.elapsedMilliseconds} ms');
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
    if (stream == null) {
      // An open we stopped waiting for: release it at the relay and origin.
      _log.add('late STREAM_READY: CLOSE sent');
      _send(SrpControl.encode('CLOSE', {'stream_id': streamId}));
      return;
    }
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
    _streamFrames++;
    final frame = decodeBodyFrame(raw);
    if (frame == null || frame.kind != bodyResponse) {
      _fail('protocol_error');
      return;
    }
    final stream = _streams[frame.streamId];
    // An empty 0x02 has no meaning, so it is ignored (§5.2).
    if (stream == null || frame.payload.isEmpty) return;
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
    final attemptId = frame.body['attempt_id'];
    if (streamId is! int && attemptId is String) {
      // A refused open names its attempt and leaves the trunk up (§5.1).
      _pendingOpens.remove(attemptId)?.refuse(code, message);
      return;
    }
    if (streamId is! int) {
      // A trunk-scoped error is fatal. A relay that predates decision 106
      // refuses an open this way too, and the trunk cannot tell which.
      _relayErrorCode = code;
      if (!_connected.isCompleted) _helloRefusal = code;
      _fail('relay_error');
      return;
    }
    _noteServerSide(code);
    final stream = _streams.remove(streamId);
    if (stream != null) {
      stream.push(SrpStreamError(code, message));
      stream.finish();
    }
  }

  /// Opens one stream (sends `OPEN_STREAM`, waits for `STREAM_READY`).
  ///
  /// A refusal (`ERROR{rate_limited, attempt_id}`) throws [SrpOpenRefused] and
  /// leaves the trunk up. At most [maxPendingOpens] are outstanding; the rest
  /// wait here. An open the relay has not answered [openTimeout] after its
  /// frame was written fails with `open_timeout`, and a `STREAM_READY` that
  /// arrives after that is answered with `CLOSE` (§5.1).
  Future<SrpStream> openStream() async {
    // The trunk only comes alive on first use: `StormConnection` hands a
    // wrapped client to the app before any request is queued, so dialling here
    // — not at construction — is what keeps an idle session socket-free.
    if (!isConnected) {
      await connect();
    }
    await _takeOpenSlot();
    final attemptId = _newAttemptId();
    final stream = SrpStream._(attemptId);
    try {
      if (_closed) throw const SrpTrunkClosed();
      _pendingOpens[attemptId] = stream;
      final written = Completer<void>();
      _send(
        SrpControl.encode('OPEN_STREAM', {'attempt_id': attemptId}),
        written: written,
      );
      unawaited(
        written.future.then((_) {
          if (_closed || !identical(_pendingOpens[attemptId], stream)) return;
          Timer(openTimeout, () {
            if (_pendingOpens.remove(attemptId) == stream) {
              _log.add('open timed out ${openTimeout.inSeconds} s after write');
              stream.refuse('open_timeout', null);
            }
          });
        }),
      );
      await stream.ready;
      return stream;
    } finally {
      _releaseOpenSlot();
    }
  }

  Future<void> _takeOpenSlot() async {
    while (_openSlots >= maxPendingOpens && !_closed) {
      final waiter = Completer<void>();
      _openWaiters.add(waiter);
      await waiter.future;
    }
    _openSlots++;
  }

  void _releaseOpenSlot() {
    _openSlots--;
    if (_openWaiters.isNotEmpty) _openWaiters.removeFirst().complete();
  }

  /// One liveness probe (§8.7 rule 3): a `PING`, and anything heard back
  /// within [timeout].
  Future<bool> probe(Duration timeout) async {
    if (!isConnected) return false;
    final heard = Completer<void>();
    _probes.add(heard);
    try {
      _send(SrpControl.encode('PING'));
      await heard.future.timeout(timeout);
      return !_closed;
    } catch (_) {
      return false;
    } finally {
      _probes.remove(heard);
    }
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

  /// Sends [bytes] as `0x01` request-body frames for [stream], none larger
  /// than [maxBodyFramePayload]. [bytes] must not be empty: an empty chunk is
  /// the end of body (§5.2), sent by [endBody].
  ///
  /// Completes once every frame is queued, waiting for room while the queue
  /// holds [maxQueuedFrames]: an upload is paced by the socket, not buffered
  /// whole in front of the heartbeat.
  Future<void> sendBody(SrpStream stream, List<int> bytes) async {
    assert(bytes.isNotEmpty, 'an empty 0x01 frame ends the body (§5.2)');
    for (var i = 0; i < bytes.length; i += maxBodyFramePayload) {
      while (_bodyFrames >= maxQueuedFrames && !_closed) {
        await (_room ??= Completer<void>()).future;
      }
      if (stream.isFinal) return;
      final end = min(i + maxBodyFramePayload, bytes.length);
      _sendBody(
        stream.streamId,
        encodeBodyFrame(
          bodyRequest,
          stream.streamId,
          bytes is Uint8List
              ? Uint8List.sublistView(bytes, i, end)
              : bytes.sublist(i, end),
        ),
      );
    }
  }

  /// Ends [stream]'s request body with a zero-length `0x01` frame (§5.2).
  /// Every request sends it once, a body-less `GET` included. Completes when
  /// the frame has been written, which is when `T_ttfb` starts (§8.7).
  Future<void> endBody(SrpStream stream) {
    if (stream.isFinal) return Future.value();
    final written = Completer<void>();
    _sendBody(
      stream.streamId,
      encodeBodyFrame(bodyRequest, stream.streamId, const []),
      written: written,
    );
    return written.future;
  }

  /// Sends `CLOSE { stream_id }` and forgets the stream (§5.4).
  void closeStream(SrpStream stream) {
    if (stream.isReady && !_closed) {
      // A CLOSE is an abort and goes ahead of the queue, so the stream's own
      // queued body would follow it to a relay that no longer knows the id.
      final dropped = _bodies.remove(stream.streamId);
      _bodyFrames -= dropped?.length ?? 0;
      for (final o in dropped ?? const <_Outbound>[]) {
        o.markWritten();
      }
      _wakeSenders();
      _send(SrpControl.encode('CLOSE', {'stream_id': stream.streamId}));
    }
    _streams.remove(stream.streamId);
    _pendingOpens.remove(stream.attemptId);
    stream.finish();
  }

  void _send(String frame, {bool ping = false, Completer<void>? written}) {
    if (_ws == null || _closed) {
      throw const SrpTrunkClosed();
    }
    _control.add(_Outbound(frame, ping: ping, written: written));
    _write();
  }

  void _sendBody(int streamId, Object frame, {Completer<void>? written}) {
    if (_ws == null || _closed) {
      throw const SrpTrunkClosed();
    }
    _bodies
        .putIfAbsent(streamId, Queue.new)
        .add(_Outbound(frame, written: written));
    _bodyFrames++;
    _write();
  }

  _Outbound? _nextBody() {
    if (_bodies.isEmpty) return null;
    final streamId = _bodies.keys.first;
    final queue = _bodies.remove(streamId)!;
    final frame = queue.removeFirst();
    if (queue.isNotEmpty) _bodies[streamId] = queue;
    _bodyFrames--;
    _wakeSenders();
    return frame;
  }

  /// The socket's one writer: a frame at a time, each when the socket can
  /// take it, control first.
  Future<void> _write() async {
    if (_writing) return;
    _writing = true;
    _Outbound? next;
    try {
      while (!_closed) {
        final ws = _ws;
        if (ws == null) return;
        if (_control.isNotEmpty) {
          next = _control.removeFirst();
        } else if (_nextBody() case final body?) {
          next = body;
        } else {
          return;
        }
        await ws.write(next.frame);
        next.markWritten();
        if (next.ping) {
          _pingUnwritten = false;
          _sinceHeard.start();
        }
      }
    } catch (_) {
      next?.markWritten();
      _fail('socket_error');
    } finally {
      _writing = false;
    }
  }

  void _wakeSenders() {
    final room = _room;
    _room = null;
    room?.complete();
  }

  void _startHeartbeat() {
    _heartbeatTimer?.cancel();
    _sinceHeard
      ..reset()
      ..start();
    _heartbeatTimer = Timer.periodic(heartbeat, (_) {
      if (!isConnected) return;
      if (_sinceHeard.elapsed >= silenceLimit) {
        _fail('relay_silent');
        return;
      }
      if (_pingUnwritten) return;
      _pingUnwritten = true;
      _sinceHeard.stop();
      _send(SrpControl.encode('PING'), ping: true);
    });
  }

  /// Terminal. Closes every stream, stops the timers, and tells the owner.
  void _fail(String reason) {
    if (_closed) return;
    _closed = true;
    _lossReason = reason;
    _log.add(
      'trunk lost: $reason${_relayErrorCode == null ? '' : ' ($_relayErrorCode)'}'
      ' after ${_age.elapsedMilliseconds} ms, ${_streams.length} streams open',
    );
    _noteServerSide(_relayErrorCode);
    _heartbeatTimer?.cancel();
    _sinceHeard.stop();
    _sub?.cancel();
    _ws?.close();
    for (final o in [..._control, for (final q in _bodies.values) ...q]) {
      o.markWritten();
    }
    _control.clear();
    _bodies.clear();
    _bodyFrames = 0;
    _wakeSenders();
    for (final p in _probes) {
      if (!p.isCompleted) p.completeError(const SrpTrunkClosed());
    }
    while (_openWaiters.isNotEmpty) {
      _openWaiters.removeFirst().complete();
    }
    for (final s in _streams.values) {
      s.push(SrpStreamError('trunk_lost', reason));
    }
    final all = [..._streams.values, ..._pendingOpens.values];
    _streams.clear();
    _pendingOpens.clear();
    for (final s in all) {
      s.finish();
    }
    if (!_connected.isCompleted) {
      _connected.completeError(SrpHandshakeFailed(_relayErrorCode));
    }
    if (!_onTrunkLost.isClosed) _onTrunkLost.add(reason);
  }

  void _noteServerSide(String? code) {
    if (srpServerSideCodes.contains(code) && !_onServerUnreachable.isClosed) {
      _onServerUnreachable.add(code!);
    }
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
    if (!_onServerUnreachable.isClosed) _onServerUnreachable.close();
  }
}

class _Outbound {
  _Outbound(this.frame, {this.ping = false, this.written});
  final Object frame;
  final bool ping;
  final Completer<void>? written;

  void markWritten() {
    final w = written;
    if (w != null && !w.isCompleted) w.complete();
  }
}

/// The trunk or its streams are gone.
class SrpTrunkClosed implements Exception {
  const SrpTrunkClosed();
}

/// The `HELLO`/`READY` handshake never completed within [SrpTrunk.handshakeTimeout].
class SrpHandshakeFailed implements Exception {
  const SrpHandshakeFailed([this.code]);

  /// The relay's `ERROR` code when it refused the `HELLO` — `server_unreachable`
  /// means the relay is up and the server is not connected to it — or null when
  /// the relay never answered.
  final String? code;

  @override
  String toString() => 'SrpHandshakeFailed(${code ?? 'no answer'})';
}

/// The task that asked for a stream is gone before the open completed.
class SrpStreamAborted implements Exception {
  const SrpStreamAborted();
}

/// The relay refused one `OPEN_STREAM` (§5.1) — `rate_limited` today. Only
/// that request failed; the trunk is still up.
class SrpOpenRefused implements Exception {
  const SrpOpenRefused(this.code, [this.message]);
  final String code;
  final String? message;

  @override
  String toString() => 'SrpOpenRefused($code)';
}
