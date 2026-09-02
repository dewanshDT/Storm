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
  FakeResponse(this.status, this.headers, this.body);
  final int status;
  final Map<String, String> headers;
  final List<int> body;
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
  FakeRelay({this.onHandshake, this.handler});

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
  int _nextStream = 1;

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
      // A 0x01 chunk contributes to the in-flight request body.
      if (decoded != null && decoded.kind == bodyRequest) {
        _streams
            .putIfAbsent(decoded.streamId, _RequestState.new)
            .body
            .addAll(decoded.payload);
      }
    }
  }

  void _onControl(String text) {
    final frame = SrpControl.decode(text);
    _ws.add(Map<String, dynamic>.from(frame.body)..['type'] = frame.type);

    switch (frame.type) {
      case 'HELLO':
        onHandshake?.call(Map<String, dynamic>.from(frame.body));
        _send('{"v":1,"type":"READY","client_trunk_id":"ctk_fake"}');
      case 'OPEN_STREAM':
        final attemptId = frame.body['attempt_id'];
        final streamId = _nextStream++;
        _send(
          SrpControl.encode('STREAM_READY', {
            'attempt_id': attemptId,
            'stream_id': streamId,
          }),
        );
      case 'HTTP_REQUEST_HEAD':
        final streamId = streamIdOf(frame.body);
        final state = _streams.putIfAbsent(streamId, _RequestState.new);
        state.method = frame.body['method'] as String?;
        state.path = frame.body['path'] as String?;
        state.headers = _headersOf(frame.body['headers']);
        // Serve once the client's body chunks have drained. A bounded delay is
        // deterministic for the small payloads these tests send; the head and
        // the 0x01 chunks arrive as separate frames, so the serve must not run
        // on the head frame itself.
        unawaited(
          Future<void>.delayed(const Duration(milliseconds: 20), () {
            _serve(streamId);
          }),
        );
      case 'CLOSE':
      // Client ended a stream; nothing more to do for it.
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
    _socket?.add(text);
  }

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
