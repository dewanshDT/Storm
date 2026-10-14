import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'dart:math';

import 'package:cryptography/cryptography.dart';

import 'srp_socket.dart';

SrpSocket connectSrpSocket(Uri url) => _IoSrpSocket(url);

/// dart:io's `WebSocket.add` buffers without limit, but `addStream` pauses
/// its source while the TCP socket is full. One frame per `addStream`
/// therefore completes only once the socket has taken it.
///
/// The upgrade is done here rather than by `WebSocket.connect` to reach the
/// socket: on a slow uplink an autotuned send buffer holds seconds of upload,
/// and a `PING` written after it waits that long. `TCP_NOTSENT_LOWAT` bounds
/// only what is unsent, so throughput is unchanged.
class _IoSrpSocket implements SrpSocket {
  _IoSrpSocket(Uri url) : _socket = _connect(url) {
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

  static const _unsentLowat = 128 * 1024;

  final Future<WebSocket> _socket;
  final _in = StreamController<dynamic>();
  Future<void> _last = Future.value();

  static Future<WebSocket> _connect(Uri url) async {
    final key = base64.encode(
      List.generate(16, (_) => Random.secure().nextInt(256)),
    );
    final client = HttpClient();
    try {
      final request = await client.openUrl(
        'GET',
        url.replace(scheme: url.scheme == 'wss' ? 'https' : 'http'),
      );
      request.followRedirects = false;
      request.headers
        ..set(HttpHeaders.connectionHeader, 'Upgrade')
        ..set(HttpHeaders.upgradeHeader, 'websocket')
        ..set('Sec-WebSocket-Key', key)
        ..set('Sec-WebSocket-Version', '13');
      final response = await request.close();
      final accept = base64.encode(
        (await Sha1().hash(
          utf8.encode('${key}258EAFA5-E914-47DA-95CA-C5AB0DC85B11'),
        )).bytes,
      );
      if (response.statusCode != HttpStatus.switchingProtocols ||
          response.headers.value('Sec-WebSocket-Accept') != accept) {
        await response.drain<void>().catchError((Object _) {});
        throw WebSocketException(
          'relay refused the upgrade: ${response.statusCode}',
        );
      }
      final socket = await response.detachSocket();
      socket.setOption(SocketOption.tcpNoDelay, true);
      _boundUnsent(socket);
      return WebSocket.fromUpgradedSocket(socket, serverSide: false);
    } finally {
      client.close();
    }
  }

  static void _boundUnsent(Socket socket) {
    final int option;
    if (Platform.isLinux || Platform.isAndroid) {
      option = 25;
    } else if (Platform.isMacOS || Platform.isIOS) {
      option = 0x201;
    } else {
      return;
    }
    try {
      socket.setRawOption(
        RawSocketOption.fromInt(RawSocketOption.levelTcp, option, _unsentLowat),
      );
    } on Object {
      // A kernel without it keeps its default buffering.
    }
  }

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
