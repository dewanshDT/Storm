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
  SrpHttpClient(this.trunk);

  final SrpTrunk trunk;

  @override
  Future<http.StreamedResponse> send(http.BaseRequest request) async {
    final stream = await trunk.openStream();

    // Subscribe to the response before the request head goes out. `stream`
    // delivers events synchronously, so a response that starts the instant the
    // head does would be missed by a listener attached later — the race the
    // broadcast/sync design exists to prevent.
    final pipe = _ResponsePipe(stream);
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

      // Body goes out as 0x01 chunks. `finalize` may be empty for GET/HEAD.
      final body = request.finalize();
      await for (final chunk in body) {
        trunk.sendBody(stream, chunk);
      }

      // The response is the head, which may have already arrived. If the trunk
      // finished this stream first (a refused open, trunk loss), surface that
      // as a transport error, not as an HTTP response.
      final response = await pipe.response;
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
  _ResponsePipe(this.stream) {
    _body = StreamController<Uint8List>();
  }

  final SrpStream stream;
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
  void onDone() {
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
    _done = true;
    if (_headSeen) {
      _body.addError(error);
      if (!_body.isClosed) _body.close();
    } else if (!_head.isCompleted) {
      _head.completeError(error);
    }
  }
}
