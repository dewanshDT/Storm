/// The socket under an SRP trunk, written one frame at a time.
///
/// SRP §5.2 "Control before bulk" (decision 116b) needs to know when the
/// socket can take another frame: a `PING` handed to a sink that buffers
/// without limit still waits behind the whole upload queued before it.
/// `WebSocketChannel`'s sink says nothing about that, so the trunk writes
/// through this instead.
library;

import 'dart:async';

import 'package:web_socket_channel/web_socket_channel.dart';

import 'srp_socket_io.dart'
    if (dart.library.js_interop) 'srp_socket_web.dart'
    as platform;

abstract class SrpSocket {
  /// Opens a socket to [url] whose [write] waits for the network.
  factory SrpSocket.connect(Uri url) => platform.connectSrpSocket(url);

  /// A plain channel. [write] only hands the frame over, so it gives no
  /// priority over what the channel buffers. For tests.
  factory SrpSocket.channel(WebSocketChannel channel) = _ChannelSocket;

  Stream<dynamic> get stream;

  /// Completes once the handshake is done; fails if it cannot be.
  Future<void> get ready;

  /// Writes [frame]. Completes when the socket can take another frame.
  /// Calls must not overlap.
  Future<void> write(Object frame);

  /// Closes the socket once any write in progress has finished.
  void close();
}

class _ChannelSocket implements SrpSocket {
  _ChannelSocket(this._channel);

  final WebSocketChannel _channel;

  @override
  Stream<dynamic> get stream => _channel.stream;

  @override
  Future<void> get ready => _channel.ready;

  @override
  Future<void> write(Object frame) async => _channel.sink.add(frame);

  @override
  void close() => unawaited(_channel.sink.close());
}
