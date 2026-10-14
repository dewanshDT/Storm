// SRP §5.2 "Control before bulk" (decision 116b): the trunk's PING goes
// ahead of an upload already queued, request bodies wait for the socket
// instead of piling up in front of it, and the trunk is never judged silent
// while its own PING cannot be written.
//
// The relay here speaks WebSocket over a raw socket, because a dart:io
// WebSocket keeps reading TCP into memory while its stream is paused, and
// the point is a relay whose socket really stops draining.
import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'dart:typed_data';

import 'package:cryptography/cryptography.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:storm/api/relay/srp_codec.dart';
import 'package:storm/api/relay/srp_trunk.dart';

class _StallingRelay {
  ServerSocket? _server;
  Socket? _socket;
  StreamSubscription<Uint8List>? _sub;
  final _buf = BytesBuilder(copy: false);
  bool _upgraded = false;
  bool _stalled = false;
  final stalled = Completer<void>();
  final after = <Object>[];
  final _changed = StreamController<void>.broadcast();

  String get url => 'ws://127.0.0.1:${_server!.port}/connect/srv_test';

  Future<void> start() async {
    _server = await ServerSocket.bind(InternetAddress.loopbackIPv4, 0);
    _server!.listen((socket) {
      _socket = socket;
      _sub = socket.listen((data) {
        _buf.add(data);
        unawaited(_drain());
      }, onError: (_) {});
    });
  }

  bool _draining = false;

  Future<void> _drain() async {
    if (_draining) return;
    _draining = true;
    try {
      if (!_upgraded) {
        final text = latin1.decode(_buf.toBytes());
        final end = text.indexOf('\r\n\r\n');
        if (end < 0) return;
        final rest = _buf.takeBytes().sublist(end + 4);
        _buf.add(rest);
        final key = RegExp(
          r'Sec-WebSocket-Key: *(.*)\r\n',
          caseSensitive: false,
        ).firstMatch(text)!.group(1)!.trim();
        final hash = await Sha1().hash(
          utf8.encode('${key}258EAFA5-E914-47DA-95CA-C5AB0DC85B11'),
        );
        _socket!.add(
          latin1.encode(
            'HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\n'
            'Connection: Upgrade\r\nSec-WebSocket-Accept: '
            '${base64.encode(hash.bytes)}\r\n\r\n',
          ),
        );
        _upgraded = true;
      }
      while (!_sub!.isPaused) {
        final frame = _nextFrame();
        if (frame == null) return;
        _onFrame(frame);
      }
    } finally {
      _draining = false;
    }
  }

  /// One client frame (masked, unfragmented), or null for not enough bytes.
  Object? _nextFrame() {
    final bytes = _buf.toBytes();
    if (bytes.length < 2) return null;
    final opcode = bytes[0] & 0x0f;
    var len = bytes[1] & 0x7f;
    var at = 2;
    if (len == 126) {
      if (bytes.length < 4) return null;
      len = (bytes[2] << 8) | bytes[3];
      at = 4;
    } else if (len == 127) {
      if (bytes.length < 10) return null;
      len = ByteData.sublistView(bytes, 2, 10).getUint64(0);
      at = 10;
    }
    if (bytes.length < at + 4 + len) return null;
    final mask = bytes.sublist(at, at + 4);
    final payload = Uint8List(len);
    for (var i = 0; i < len; i++) {
      payload[i] = bytes[at + 4 + i] ^ mask[i % 4];
    }
    final rest = _buf.takeBytes().sublist(at + 4 + len);
    _buf.add(rest);
    return opcode == 1 ? utf8.decode(payload) : payload;
  }

  void _sendText(String text) {
    final payload = utf8.encode(text);
    assert(payload.length < 126);
    _socket!.add([0x81, payload.length, ...payload]);
  }

  void _onFrame(Object frame) {
    if (_stalled) {
      after.add(frame);
      if (frame is String && SrpControl.decode(frame).type == 'PING') {
        _sendText(SrpControl.encode('PONG'));
      }
      _changed.add(null);
      return;
    }
    if (frame is String) {
      final control = SrpControl.decode(frame);
      switch (control.type) {
        case 'HELLO':
          _sendText(SrpControl.encode('READY', {'client_trunk_id': 'ctk_t'}));
        case 'OPEN_STREAM':
          _sendText(
            SrpControl.encode('STREAM_READY', {
              'attempt_id': control.body['attempt_id'],
              'stream_id': 1,
            }),
          );
      }
      return;
    }
    _stalled = true;
    _sub!.pause();
    stalled.complete();
  }

  void resume() {
    _sub!.resume();
    unawaited(_drain());
  }

  /// Body frames read before the first [type] after the stall.
  Future<int> bodiesBefore(String type) async {
    while (true) {
      var bodies = 0;
      for (final frame in after) {
        if (frame is String) {
          if (SrpControl.decode(frame).type == type) return bodies;
        } else {
          bodies++;
        }
      }
      await _changed.stream.first;
    }
  }

  Future<void> close() async {
    await _sub?.cancel();
    _socket?.destroy();
    await _server?.close();
  }
}

void main() {
  test('a PING overtakes a queued upload, and a stalled trunk is not silent '
      'while its own PING is unwritten', () async {
    final relay = _StallingRelay();
    await relay.start();
    final trunk = SrpTrunk(
      url: Uri.parse(relay.url),
      serverId: 'srv_test',
      heartbeat: const Duration(milliseconds: 200),
    );
    final lost = <String>[];
    trunk.onTrunkLost.listen(lost.add);
    try {
      final stream = await trunk.openStream();
      trunk.sendRequestHead(
        stream,
        method: 'PUT',
        path: '/v1/attachments/x',
        headers: const {},
      );
      final upload = Future<void>.sync(
        () => trunk.sendBody(stream, Uint8List(20 << 20)),
      );
      unawaited(upload.catchError((Object _) {}));

      await relay.stalled.future.timeout(const Duration(seconds: 10));
      // A second request opened behind the upload.
      unawaited(trunk.openStream().then<void>((_) {}, onError: (_) {}));
      await Future<void>.delayed(const Duration(seconds: 2));
      expect(lost, isEmpty, reason: 'judged silent behind its own upload');

      relay.resume();
      for (final type in ['PING', 'OPEN_STREAM']) {
        final bodies = await relay
            .bodiesBefore(type)
            .timeout(const Duration(seconds: 20));
        expect(
          bodies,
          lessThan(40),
          reason: 'the $type waited behind $bodies of the 80 body frames',
        );
      }
      expect(lost, isEmpty);
    } finally {
      trunk.dispose();
      await relay.close();
    }
  });

  test('a request body waits for room instead of queueing whole', () async {
    final relay = _StallingRelay();
    await relay.start();
    final trunk = SrpTrunk(
      url: Uri.parse(relay.url),
      serverId: 'srv_test',
      maxQueuedFrames: 8,
    );
    try {
      final stream = await trunk.openStream();
      trunk.sendRequestHead(
        stream,
        method: 'PUT',
        path: '/v1/attachments/x',
        headers: const {},
      );
      var queued = false;
      final upload = Future<void>.sync(
        () => trunk.sendBody(stream, Uint8List(64 << 20)),
      ).then((_) => queued = true);
      unawaited(upload.catchError((Object _) => false));

      await relay.stalled.future.timeout(const Duration(seconds: 10));
      await Future<void>.delayed(const Duration(seconds: 1));
      expect(queued, isFalse, reason: 'a 64 MiB body was queued whole');

      relay.resume();
      await upload.timeout(const Duration(seconds: 30));
      expect(queued, isTrue);
    } finally {
      trunk.dispose();
      await relay.close();
    }
  });
}
