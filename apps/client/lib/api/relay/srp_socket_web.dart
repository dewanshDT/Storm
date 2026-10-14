import 'dart:async';

import 'package:web_socket_channel/html.dart';

import 'srp_socket.dart';

SrpSocket connectSrpSocket(Uri url) =>
    _WebSrpSocket(HtmlWebSocketChannel.connect(url));

/// A browser socket reports how much it has not yet sent; a write waits
/// until that is small again.
class _WebSrpSocket implements SrpSocket {
  _WebSrpSocket(this._channel);

  static const _buffered = 256 * 1024;
  static const _poll = Duration(milliseconds: 20);
  static const _open = 1;

  final HtmlWebSocketChannel _channel;

  @override
  Stream<dynamic> get stream => _channel.stream;

  @override
  Future<void> get ready => _channel.ready;

  @override
  Future<void> write(Object frame) async {
    _channel.sink.add(frame);
    while ((_channel.bufferedAmount ?? 0) > _buffered &&
        _channel.innerWebSocket.readyState == _open) {
      await Future<void>.delayed(_poll);
    }
  }

  @override
  void close() => unawaited(_channel.sink.close());
}
