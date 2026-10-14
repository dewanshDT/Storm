import 'dart:async';
import 'dart:io';

import 'srp_socket.dart';

SrpSocket connectSrpSocket(Uri url) => _IoSrpSocket(url);

/// dart:io's `WebSocket.add` buffers without limit, but `addStream` pauses
/// its source while the TCP socket is full. One frame per `addStream`
/// therefore completes only once the socket has taken it.
class _IoSrpSocket implements SrpSocket {
  _IoSrpSocket(Uri url) : _socket = WebSocket.connect(url.toString()) {
    _socket.then<void>(
      (ws) {
        ws.listen(
          _in.add,
          onError: _in.addError,
          onDone: _in.close,
          cancelOnError: true,
        );
      },
      onError: (Object e, StackTrace s) {
        _in.addError(e, s);
        _in.close();
      },
    );
  }

  final Future<WebSocket> _socket;
  final _in = StreamController<dynamic>();
  Future<void> _last = Future.value();

  @override
  Stream<dynamic> get stream => _in.stream;

  @override
  Future<void> get ready => _socket.then((_) {});

  @override
  Future<void> write(Object frame) => _last = _last.then((_) async {
    final ws = await _socket;
    await ws.addStream(Stream.value(frame));
  });

  @override
  void close() {
    // After the write in progress: closing a bound sink throws.
    _last = _last
        .then((_) async => (await _socket).close())
        .catchError((Object _) => null);
  }
}
