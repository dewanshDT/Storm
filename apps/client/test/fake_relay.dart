/// A fake SRP relay for the tunnel client's unit tests.
///
/// A real end-to-end test would need both `storm-relay` (Rust) and the origin
/// server running. That is what `make test-live` is for. Here, the fake is a
/// single WebSocket that speaks the client half of SRP v1 (§5) and answers a
/// tunnelled HTTP exchange by itself — enough to exercise [SrpTrunk] and
/// [SrpHttpClient] deterministically, including the refused-open and trunk-loss
/// paths that are the dangerous ones.
///
/// It is deliberately a *fake relay+origin in one*: the client sees exactly the
/// frames `apps/relay/src/connect.rs` would emit. How the request would reach an
/// origin is not this helper's concern.
library;

import 'dart:async';
import 'dart:io';

import 'package:storm/api/relay/srp_codec.dart';

/// A scripted response the fake relay serves for a request.
class FakeResponse {
  FakeResponse(this.status, this.headers, this.body) : stream = null;

  /// A response whose body goes out chunk by chunk as [stream] yields, the way
  /// a relay forwards an SSE stream, and ends when it does.
  FakeResponse.streamed(
    this.status,
    this.headers,
    Stream<List<int>> this.stream,
  ) : body = const [];

  final int status;
  final Map<String, String> headers;
  final List<int> body;
  final Stream<List<int>>? stream;
}

/// A handler scripted per-test.
typedef FakeHandler =
    Future<FakeResponse> Function(
      String method,
      String path,
      Map<String, String> headers,
      List<int> body,
    );

/// Stands up a local WebSocket relay on an ephemeral port and exposes what it
/// has been asked to do.
class FakeRelay {
  FakeRelay({this.onHandshake, this.handler, this.helloError});

  /// When set, `HELLO` is refused with a trunk-scoped `ERROR` of this code —
  /// `server_unreachable` is what a relay with no trunk for the id answers.
  final String? helloError;

  /// Fired when the client trunk sends a control message (for the test to
  /// assert on what it saw). Each frame is decoded; the first is `HELLO`.
  final _ws = StreamController<Map<String, dynamic>>();
  Stream<Map<String, dynamic>> get sent => _ws.stream;

  HttpServer? _server;
  WebSocket? _socket;

  /// Assertion hook: called with the client's `HELLO` body.
  void Function(Map<String, dynamic> hello)? onHandshake;

  /// Serves a request, or null to make the open fail the way the client
  /// expects (a refused open / trunk loss the test scripts).
  FakeHandler? handler;

  final _streams = <int, _RequestState>{};

  /// How many of the next `OPEN_STREAM`s to refuse with `rate_limited`, as a
  /// relay at its in-flight cap does (§5.1).
  int refuseOpens = 0;

  /// Whether a refusal names its `attempt_id` (decision 106). False is a
  /// relay that predates it: a trunk-scoped `ERROR` naming nothing.
  bool refusalNamesAttempt = true;

  /// `PING`s sent before `READY` and again after it: a relay may `PING` any
  /// trunk to learn how much its peer has read (§4.2, decision 108c).
  int pings = 0;

  /// Every `0x01` frame the client sent, as `(stream_id, payload length)`.
  final requestBodyFrames = <(int, int)>[];

  /// Streams the client ended with its own `CLOSE`.
  final closedByClient = <int>{};
  final _streaming = <int, StreamSubscription<List<int>>>{};
  int _nextStream = 1;

  /// How long every frame the relay sends takes to arrive, in order: a phone
  /// on mobile data, where a probe is several round trips.
  Duration latency = Duration.zero;
  Future<void> _wire = Future.value();

  /// Request paths the relay answers with a stream-scoped `ERROR` of the
  /// given code instead of forwarding, as it does for a `server_timeout`.
  final streamErrors = <String, String>{};

  /// Trunks the client has opened (`HELLO`s seen).
  int hellos = 0;

  /// When set, the relay answers nothing at all, as a socket that went
  /// silent without a FIN does.
  bool mute = false;

  /// How long the relay holds each `STREAM_READY`, as one waiting on a slow
  /// origin's acknowledgement does.
  Duration openDelay = Duration.zero;

  /// The relay's in-flight open cap (§5.1): past it an `OPEN_STREAM` is
  /// refused with `rate_limited` at once. Null is no cap.
  int? openCap;
  int _inFlightOpens = 0;

  /// The most opens the client had outstanding at once.
  int maxInFlightOpens = 0;

  /// Opens refused for the cap.
  int capRefusals = 0;

  /// Sees every request head's path and headers.
  void Function(String path, Map<String, String> headers)? onHeaders;

  int get port => _server!.port;

  /// The `ws://` connect URL the client dials.
  Uri get url => Uri.parse('ws://127.0.0.1:$port/connect/srv_test');

  Future<void> start() async {
    _server = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
    _server!.listen((HttpRequest req) async {
      if (req.uri.path != '/connect/srv_test') {
        req.response.statusCode = HttpStatus.notFound;
        await req.response.close();
        return;
      }
      final socket = await WebSocketTransformer.upgrade(req);
      _socket = socket;
      socket.listen(
        _onFrame,
        onDone: () => _ws.close(),
        onError: (_) => _ws.close(),
      );
    });
  }

  void _onFrame(dynamic frame) {
    if (frame is String) {
      _onControl(frame);
      return;
    }
    if (frame is List<int>) {
      final decoded = decodeBodyFrame(frame);
      if (decoded != null && decoded.kind == bodyRequest) {
        requestBodyFrames.add((decoded.streamId, decoded.payload.length));
        // An empty 0x01 frame ends the body (§5.2): serve then, as the origin
        // does. Any other 0x01 chunk contributes to the body.
        if (decoded.payload.isEmpty) {
          unawaited(_serve(decoded.streamId));
        } else {
          _streams
              .putIfAbsent(decoded.streamId, _RequestState.new)
              .body
              .addAll(decoded.payload);
        }
      }
    }
  }

  void _onControl(String text) {
    if (mute) return;
    final frame = SrpControl.decode(text);
    // [sent] ends with the first trunk; a later trunk is still served.
    if (!_ws.isClosed) {
      _ws.add(Map<String, dynamic>.from(frame.body)..['type'] = frame.type);
    }

    switch (frame.type) {
      case 'HELLO':
        hellos++;
        onHandshake?.call(Map<String, dynamic>.from(frame.body));
        final refusal = helloError;
        if (refusal != null) {
          _send(SrpControl.encode('ERROR', {'code': refusal}));
          return;
        }
        for (var i = 0; i < pings; i++) {
          _send(SrpControl.encode('PING'));
        }
        _send('{"v":1,"type":"READY","client_trunk_id":"ctk_fake"}');
        for (var i = 0; i < pings; i++) {
          _send(SrpControl.encode('PING'));
        }
      case 'PING':
        _send(SrpControl.encode('PONG'));
      case 'OPEN_STREAM':
        final attemptId = frame.body['attempt_id'];
        final cap = openCap;
        if (cap != null && _inFlightOpens >= cap) {
          capRefusals++;
          _send(
            SrpControl.encode('ERROR', {
              'code': 'rate_limited',
              'attempt_id': attemptId,
            }),
          );
          return;
        }
        if (refuseOpens > 0) {
          refuseOpens--;
          _send(
            SrpControl.encode('ERROR', {
              'code': 'rate_limited',
              'message': 'rate limited',
              if (refusalNamesAttempt) 'attempt_id': attemptId,
            }),
          );
          return;
        }
        final streamId = _nextStream++;
        void ready() => _send(
          SrpControl.encode('STREAM_READY', {
            'attempt_id': attemptId,
            'stream_id': streamId,
          }),
        );
        if (openDelay == Duration.zero) {
          ready();
          return;
        }
        _inFlightOpens++;
        if (_inFlightOpens > maxInFlightOpens) {
          maxInFlightOpens = _inFlightOpens;
        }
        Timer(openDelay, () {
          _inFlightOpens--;
          ready();
        });
      case 'HTTP_REQUEST_HEAD':
        final streamId = streamIdOf(frame.body);
        final state = _streams.putIfAbsent(streamId, _RequestState.new);
        state.method = frame.body['method'] as String?;
        state.path = frame.body['path'] as String?;
        state.headers = _headersOf(frame.body['headers']);
        onHeaders?.call(
          (state.path ?? '/').split('?').first,
          state.headers ?? const {},
        );
      case 'CLOSE':
        final streamId = streamIdOf(frame.body);
        closedByClient.add(streamId);
        _streaming.remove(streamId)?.cancel();
      default:
        break;
    }
  }

  int streamIdOf(Map<String, dynamic> frame) => frame['stream_id'] as int? ?? 0;

  Map<String, String> _headersOf(Object? raw) {
    final out = <String, String>{};
    if (raw is Map) {
      for (final e in raw.entries) {
        if (e.key is String && e.value is String) {
          out[e.key as String] = e.value as String;
        }
      }
    }
    return out;
  }

  Future<void> _serve(int streamId) async {
    final state = _streams.remove(streamId);
    if (state == null) return;
    final scripted = streamErrors[(state.path ?? '/').split('?').first];
    if (scripted != null) {
      _send(
        SrpControl.encode('ERROR', {'code': scripted, 'stream_id': streamId}),
      );
      return;
    }
    final handler = this.handler;
    if (handler == null) {
      // Nothing scripted: refuse the open like a closed channel.
      _send(
        SrpControl.encode('ERROR', {
          'code': 'trunk_lost',
          'stream_id': streamId,
        }),
      );
      return;
    }
    final response = await handler(
      state.method ?? 'GET',
      state.path ?? '/',
      state.headers ?? const {},
      List<int>.from(state.body),
    );
    _send(
      SrpControl.encode('HTTP_RESPONSE_HEAD', {
        'stream_id': streamId,
        'status': response.status,
        'headers': response.headers,
      }),
    );
    final streamed = response.stream;
    if (streamed != null) {
      _streaming[streamId] = streamed.listen(
        (chunk) => _send(encodeBodyFrame(bodyResponse, streamId, chunk)),
        onDone: () {
          if (_streaming.remove(streamId) != null) {
            _send(SrpControl.encode('CLOSE', {'stream_id': streamId}));
          }
        },
      );
      return;
    }
    for (final chunk in _chunk(response.body)) {
      _send(encodeBodyFrame(bodyResponse, streamId, chunk));
    }
    _send(SrpControl.encode('CLOSE', {'stream_id': streamId}));
  }

  Iterable<List<int>> _chunk(List<int> body) sync* {
    const size = 3;
    for (var i = 0; i < body.length; i += size) {
      yield body.sublist(i, i + size > body.length ? body.length : i + size);
    }
  }

  void _send(Object text) {
    final socket = _socket;
    if (latency == Duration.zero) {
      _add(socket, text);
      return;
    }
    final due = DateTime.now().add(latency);
    _wire = _wire.then((_) async {
      final wait = due.difference(DateTime.now());
      if (wait > Duration.zero) await Future<void>.delayed(wait);
      _add(socket, text);
    });
  }

  /// A frame for a socket the client already closed goes nowhere, as on a
  /// real relay.
  void _add(WebSocket? socket, Object frame) {
    try {
      socket?.add(frame);
    } on StateError {
      // Closed.
    }
  }

  /// Whether the last trunk's socket has closed.
  bool get socketClosed => _socket?.closeCode != null;

  /// Drops the client's trunk without a `CLOSE`, as a carrier does.
  Future<void> dropTrunk() async => _socket?.close();

  void sendRaw(List<int> bytes) {
    _socket?.add(bytes);
  }

  Future<void> close() async {
    await _socket?.close();
    await _server?.close(force: true);
  }
}

/// In-flight request state for one stream. The fake keeps this per-stream so
/// concurrent opens can't overwrite each other's head/body.
class _RequestState {
  final List<int> body = <int>[];
  String? method;
  String? path;
  Map<String, String>? headers;
}
