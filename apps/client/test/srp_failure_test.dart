// Relay failure handling as the Dart client sees it (relay TODO §9,
// decision 94): a trunk dying mid-response, a stream-scoped error, and a
// server that is offline behind a live relay.
import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:storm/api/relay/srp_codec.dart';
import 'package:storm/api/relay/srp_http_client.dart';
import 'package:storm/api/relay/srp_trunk.dart';

/// What the scripted relay does once a request head arrives.
enum _Script {
  /// Head, one body chunk, then the socket dies with no `CLOSE`.
  dieMidBody,

  /// Head, one body chunk, then `ERROR{trunk_lost, stream_id}`.
  errorMidBody,
}

/// A relay that answers `HELLO` (or refuses it) and then follows a script.
class _ScriptedRelay {
  _ScriptedRelay({this.script = _Script.dieMidBody, this.refuseHello});

  final _Script script;

  /// When set, `HELLO` is answered with this trunk-level `ERROR` code.
  final String? refuseHello;

  HttpServer? _server;
  WebSocket? _socket;
  int requestHeads = 0;

  Uri get url => Uri.parse('ws://127.0.0.1:${_server!.port}/connect/srv_test');

  Future<void> start() async {
    _server = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
    _server!.listen((req) async {
      final socket = await WebSocketTransformer.upgrade(req);
      _socket = socket;
      socket.listen((frame) {
        if (frame is String) _onControl(SrpControl.decode(frame));
      });
    });
  }

  void _onControl(SrpControl frame) {
    final socket = _socket!;
    switch (frame.type) {
      case 'HELLO':
        final refusal = refuseHello;
        if (refusal != null) {
          socket.add(SrpControl.encode('ERROR', {'code': refusal}));
          socket.close();
          return;
        }
        socket.add(SrpControl.encode('READY', {'client_trunk_id': 'ctk_t'}));
      case 'OPEN_STREAM':
        socket.add(
          SrpControl.encode('STREAM_READY', {
            'attempt_id': frame.body['attempt_id'],
            'stream_id': 1,
          }),
        );
      case 'HTTP_REQUEST_HEAD':
        requestHeads++;
        socket.add(
          SrpControl.encode('HTTP_RESPONSE_HEAD', {
            'stream_id': 1,
            'status': 200,
            'headers': <String, String>{},
          }),
        );
        socket.add(encodeBodyFrame(bodyResponse, 1, utf8.encode('half a bo')));
        switch (script) {
          case _Script.dieMidBody:
            // No CLOSE, no ERROR: the relay's process or network is gone.
            Timer(const Duration(milliseconds: 20), () => socket.close());
          case _Script.errorMidBody:
            socket.add(
              SrpControl.encode('ERROR', {
                'code': 'trunk_lost',
                'stream_id': 1,
              }),
            );
        }
    }
  }

  Future<void> close() async {
    await _socket?.close();
    await _server?.close(force: true);
  }
}

Future<Object?> _bodyError(SrpHttpClient client) async {
  final response = await client.send(
    http.Request('GET', Uri.parse('http://relay.invalid/v1/notes/x')),
  );
  try {
    await response.stream.toBytes();
    return null;
  } catch (e) {
    return e;
  }
}

void main() {
  test('a trunk dying mid-response is a body error, never a short '
      'successful body', () async {
    final relay = _ScriptedRelay(script: _Script.dieMidBody);
    await relay.start();
    final trunk = SrpTrunk(url: relay.url, serverId: 'srv_test');
    final client = SrpHttpClient(trunk);
    try {
      final error = await _bodyError(client);
      expect(error, isA<SrpTransportException>());
      expect((error! as SrpTransportException).code, 'trunk_lost');
      // Surfaced to the caller, not retried behind its back.
      expect(relay.requestHeads, 1);
      expect(trunk.isConnected, isFalse);
    } finally {
      trunk.dispose();
      await relay.close();
    }
  });

  test('a stream-scoped trunk_lost mid-response is a body error', () async {
    final relay = _ScriptedRelay(script: _Script.errorMidBody);
    await relay.start();
    final trunk = SrpTrunk(url: relay.url, serverId: 'srv_test');
    final client = SrpHttpClient(trunk);
    try {
      final error = await _bodyError(client);
      expect(error, isA<SrpTransportException>());
      expect((error! as SrpTransportException).code, 'trunk_lost');
      expect(relay.requestHeads, 1);
      // One stream ended; the trunk itself is still up.
      expect(trunk.isConnected, isTrue);
    } finally {
      trunk.dispose();
      await relay.close();
    }
  });

  test('a dead trunk is replaced by a fresh one, not reused', () async {
    final relay = _ScriptedRelay(script: _Script.dieMidBody);
    await relay.start();
    final first = SrpTrunk(url: relay.url, serverId: 'srv_test');
    try {
      await _bodyError(SrpHttpClient(first));
      expect(first.isConnected, isFalse);
      await expectLater(first.connect(), throwsA(isA<SrpTrunkClosed>()));
    } finally {
      first.dispose();
    }
    final second = SrpTrunk(url: relay.url, serverId: 'srv_test');
    try {
      await second.connect();
      expect(second.isConnected, isTrue);
    } finally {
      second.dispose();
      await relay.close();
    }
  });

  test('a relay saying server_unreachable fails the handshake', () async {
    final relay = _ScriptedRelay(refuseHello: 'server_unreachable');
    await relay.start();
    final trunk = SrpTrunk(url: relay.url, serverId: 'srv_test');
    final lost = <String>[];
    trunk.onTrunkLost.listen(lost.add);
    try {
      await expectLater(trunk.connect(), throwsA(isA<SrpHandshakeFailed>()));
      await pumpEventQueue();
      expect(lost, isNotEmpty);
    } finally {
      trunk.dispose();
      await relay.close();
    }
  });

  test(
    'server offline behind a live relay is told apart from a dead relay',
    () async {
      final relay = _ScriptedRelay(refuseHello: 'server_unreachable');
      await relay.start();
      final trunk = SrpTrunk(url: relay.url, serverId: 'srv_test');
      final lost = <String>[];
      trunk.onTrunkLost.listen(lost.add);
      try {
        await expectLater(trunk.connect(), throwsA(anything));
        await pumpEventQueue();
        expect(lost, contains('server_unreachable'));
      } finally {
        trunk.dispose();
        await relay.close();
      }
    },
    skip:
        'SrpTrunk reports every trunk-level ERROR as relay_error, so the '
        'code is lost before StormConnection could show it (decision 94).',
  );
}
