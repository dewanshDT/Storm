/// An `http.Client` whose `send` runs over an SRP trunk (§5.2).
///
/// This is D1 from the *Storm Relay Dart Client* design, and R13 restated in
/// Dart: the relay introduces no application protocol, so the client must not
/// grow one either. `StormApi` already takes an injectable `http.Client`, so
/// wiring a relayed candidate is swapping one argument and no call sites
/// change. `BaseClient` also gives `StreamedResponse`, which is what carries
/// the SSE change feed for free — the same Response a LAN client reads.
library;

import 'dart:async';
import 'dart:typed_data';

import 'package:http/http.dart' as http;

import 'srp_trunk.dart';

/// Errors that surface a tunnelled request as other than an HTTP exchange.
///
/// Deliberately separate from `http.ClientException`. Storm is offline-first
/// with an outbox and every write carries `base_version`, so a retry either
/// succeeds or conflicts. But a *transport* death (trunk lost) is not the same
/// as the server refusing — callers that need to tell them apart (the
/// offline-vs-relay-unavailable split) read this type, not a string on a
/// generic exception.
class SrpTransportException implements Exception {
  const SrpTransportException(this.code, [this.message]);
  final String code;
  final String? message;
  @override
  String toString() =>
      'SrpTransportException($code${message == null ? '' : ': $message'})';
}

/// Routes every request over one [SrpTrunk].
class SrpHttpClient extends http.BaseClient {
  SrpHttpClient(
    this.trunk, {
    this.ttfb = const Duration(seconds: 30),
    this.idle = const Duration(seconds: 120),
  });

  final SrpTrunk trunk;

  /// `T_ttfb` (§8.7 rule 4): from the end of body being written to the
  /// response head. Measured from the write, so a long upload is not a
  /// silent server.
  final Duration ttfb;

  /// `T_idle`: silence inside a response body. The change feed and the
  /// terminal (`text/event-stream`) are exempt.
  final Duration idle;

  @override
  Future<http.StreamedResponse> send(http.BaseRequest request) async {
    final SrpStream stream;
    try {
      stream = await trunk.openStream();
    } on SrpOpenRefused catch (e) {
      throw SrpTransportException(e.code, e.message);
    }

    // Subscribe to the response before the request head goes out. `stream`
    // delivers events synchronously, so a response that starts the instant the
    // head does would be missed by a listener attached later — the race the
    // broadcast/sync design exists to prevent.
    final pipe = _ResponsePipe(trunk, stream, idle);
    stream.events.listen(pipe.onEvent, onDone: pipe.onDone);
    try {
      final uri = request.url;
      final path = uri.path + (uri.hasQuery ? '?${uri.query}' : '');
      // The relay overwrites `relay_peer_ip` server-ward and *refuses* a client
      // that sends one (§5.2), so it must never leave this client.
      final headers = Map<String, String>.from(request.headers)
        ..remove('relay_peer_ip');
      trunk.sendRequestHead(
        stream,
        method: request.method,
        path: path,
        headers: headers,
      );

      // An empty 0x01 frame means end of body (§5.2), so an empty chunk from
      // `finalize` must not go out as one — and the real end must, always.
      final body = request.finalize();
      await for (final chunk in body) {
        if (chunk.isEmpty) continue;
        await trunk.sendBody(stream, chunk);
      }
      await trunk.endBody(stream);

      // The response is the head, which may have already arrived. If the trunk
      // finished this stream first (a refused open, trunk loss), surface that
      // as a transport error, not as an HTTP response.
      final ttfbWatch = QuietWatch(trunk, ttfb, () {
        pipe.failSilent();
      });
      final http.StreamedResponse? response;
      try {
        response = await pipe.response;
      } finally {
        ttfbWatch.cancel();
      }
      if (response == null) {
        throw const SrpTransportException('no_response_head');
      }
      return response;
    } catch (_) {
      trunk.closeStream(stream);
      rethrow;
    }
  }
}

/// Collects the response pieces for one [SrpStream] into a `StreamedResponse`.
///
/// The listener is attached before the request head is sent, so response events
/// that arrive during the request's own body upload are captured rather than
/// dropped. The body surface handed to the caller is a *new* single-subscription
/// controller: the trunk's broadcast `events` is shared, but the response body
/// belongs to exactly one consumer.
class _ResponsePipe {
  _ResponsePipe(this.trunk, this.stream, this.idle) {
    // A reader that stops listening (a closed terminal, a disposed feed) ends
    // the stream on the trunk too, or the origin keeps streaming to nobody.
    _body = StreamController<Uint8List>(
      onCancel: () {
        if (_done) return;
        _done = true;
        trunk.closeStream(stream);
      },
    );
  }

  final SrpTrunk trunk;
  final SrpStream stream;
  final Duration idle;
  QuietWatch? _idleTimer;
  bool _exempt = false;
  late final StreamController<Uint8List> _body;
  final _head = Completer<http.StreamedResponse?>();
  bool _headSeen = false;
  bool _done = false;

  Future<http.StreamedResponse?> get response => _head.future;

  void onEvent(SrpStreamEvent event) {
    if (_done) return;
    switch (event) {
      case SrpResponseHead():
        _headSeen = true;
        _exempt = (event.headers['content-type'] ?? '').startsWith(
          'text/event-stream',
        );
        _armIdle();
        if (!_head.isCompleted) {
          _head.complete(
            http.StreamedResponse(
              _body.stream,
              event.status,
              headers: event.headers,
              request: null,
              reasonPhrase: '',
            ),
          );
        }
      case SrpBodyChunk():
        _armIdle();
        _body.add(event.bytes);
      case SrpStreamError():
        // An error *before* the head is a failed open; one *after* would be a
        // mid-body truncation. Both end the response, the first as a thrown
        // transport error and the second as a body error the reader sees.
        _fail(SrpTransportException(event.code, event.message));
    }
  }

  /// The trunk finished this stream — a `CLOSE`, trunk loss, or the response
  /// body completing. A normal end closes the body; a head never seen means
  /// the open was refused or the trunk died before the response started.
  void _armIdle() {
    _idleTimer?.cancel();
    if (_exempt) return;
    _idleTimer = QuietWatch(trunk, idle, failSilent);
  }

  void failSilent() {
    if (_done) return;
    _fail(const SrpTransportException('server_silent'));
    trunk.closeStream(stream);
  }

  void onDone() {
    _idleTimer?.cancel();
    if (_done) return;
    _done = true;
    if (!_headSeen && !_head.isCompleted) {
      _head.completeError(const SrpTransportException('no_response_head'));
      if (!_body.isClosed) _body.close();
      return;
    }
    if (!_body.isClosed) _body.close();
  }

  void _fail(Object error) {
    _idleTimer?.cancel();
    _done = true;
    if (_headSeen) {
      _body.addError(error);
      if (!_body.isClosed) _body.close();
    } else if (!_head.isCompleted) {
      _head.completeError(error);
    }
  }
}

/// Calls [onSilent] once [limit] passes with no stream frame on [trunk]
/// (§8.7 rule 4). A response queued at the relay behind another stream's body
/// is waiting, not silent: the trunk is busy delivering, and on a 2 Mbps link
/// a small response can wait minutes behind a download.
class QuietWatch {
  QuietWatch(this.trunk, this.limit, this.onSilent) {
    _seen = trunk.streamFramesReceived;
    _quiet.start();
    var tick = limit ~/ 4;
    if (tick > const Duration(seconds: 1)) tick = const Duration(seconds: 1);
    if (tick < const Duration(milliseconds: 1)) {
      tick = const Duration(milliseconds: 1);
    }
    _timer = Timer.periodic(tick, (_) => _check());
  }

  final SrpTrunk trunk;
  final Duration limit;
  final void Function() onSilent;
  final Stopwatch _quiet = Stopwatch();
  late int _seen;
  late final Timer _timer;

  void _check() {
    final n = trunk.streamFramesReceived;
    if (n != _seen) {
      _seen = n;
      _quiet.reset();
      return;
    }
    if (_quiet.elapsed >= limit) {
      cancel();
      onSilent();
    }
  }

  void cancel() => _timer.cancel();
}
