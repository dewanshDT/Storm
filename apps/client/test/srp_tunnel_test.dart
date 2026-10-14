/// The Dart SRP client (`SrpTrunk` + `SrpHttpClient`) driven against a fake
/// relay that speaks the client half of SRP v1.
///
/// This is where the client's framing and stream lifecycle are pinned to
/// `docs/srp-v1.md` without needing `storm-relay` and the origin running — the
/// end-to-end case is `make test-live`'s job. Setup/teardown is a real
/// `HttpServer`, so these are live-socket tests, not fakes of the socket.
library;

import 'dart:async';
import 'dart:convert';
import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:storm/api/relay/srp_codec.dart';
import 'package:storm/api/relay/srp_http_client.dart';
import 'package:storm/api/relay/srp_trunk.dart';

import 'fake_relay.dart';

Future<void> _withRelay(
  Future<void> Function(FakeRelay relay) body, {
  bool addDefaultHandler = true,
}) async {
  final relay = FakeRelay(
    handler: addDefaultHandler
        ? (m, p, h, b) async => FakeResponse(200, {
            'content-type': 'text/plain',
          }, utf8.encode('ok:$p'))
        : null,
  );
  await relay.start();
  try {
    await body(relay);
  } finally {
    await relay.close();
  }
}

SrpTrunk _trunk(FakeRelay relay) {
  return SrpTrunk(url: relay.url, serverId: 'srv_test');
}

void main() {
  test('the trunk opens with HELLO and READY', () async {
    await _withRelay((relay) async {
      final trunk = _trunk(relay);
      await trunk.connect();
      expect(trunk.isConnected, isTrue);

      // The first thing we sent was a HELLO carrying our server_id.
      final first = await relay.sent.first;
      expect(first['type'], 'HELLO');
      expect(first['server_id'], 'srv_test');
      trunk.dispose();
    });
  });

  test('every relay PING is answered, before READY and after', () async {
    await _withRelay((relay) async {
      relay.pings = 3;
      var pongs = 0;
      final six = Completer<void>();
      relay.sent.listen((m) {
        if (m['type'] == 'PONG' && ++pongs == 6) six.complete();
      });
      final trunk = _trunk(relay);
      await trunk.connect();
      await six.future.timeout(const Duration(seconds: 5));
      await Future<void>.delayed(const Duration(milliseconds: 30));
      expect(pongs, 6);
      trunk.dispose();
    });
  });

  test('a GET request/response round-trips through the tunnel', () async {
    await _withRelay((relay) async {
      final trunk = _trunk(relay);
      final client = SrpHttpClient(trunk);
      await trunk.connect();

      final response = await client.get(
        Uri.parse('http://relay.invalid/v1/notes'),
        headers: {'authorization': 'Bearer x'},
      );

      expect(response.statusCode, 200);
      expect(utf8.decode(response.bodyBytes), 'ok:/v1/notes');
      trunk.dispose();
    });
  });

  test('the credential rides in the HTTP head, never on the trunk', () async {
    await _withRelay((relay) async {
      final trunk = _trunk(relay);
      final client = SrpHttpClient(trunk);
      final hello = Completer<Map<String, dynamic>>();
      Map<String, dynamic>? head;
      relay.sent.listen((m) {
        if (m['type'] == 'HELLO' && !hello.isCompleted) hello.complete(m);
        if (m['type'] == 'HTTP_REQUEST_HEAD') head = m;
      });
      await trunk.connect();
      await client.get(
        Uri.parse('http://x/v1/vaults'),
        headers: {'authorization': 'Bearer secret'},
      );
      await Future<void>.delayed(const Duration(milliseconds: 30));

      expect(head, isNotNull, reason: 'the client never sent an HTTP head');
      // R12 restated: the credential is inside the head, not a HELLO field.
      final helloFrame = await hello.future;
      expect(helloFrame.containsKey('authorization'), isFalse);
      expect(head!['headers'], containsPair('authorization', 'Bearer secret'));
      trunk.dispose();
    });
  });

  test('two concurrent streams multiplex without cross-talk', () async {
    final relay = FakeRelay(
      handler: (m, path, h, b) async {
        // Echo the path back so the test can tell which stream answered.
        return FakeResponse(200, {}, utf8.encode(path));
      },
    );
    await relay.start();
    final trunk = SrpTrunk(url: relay.url, serverId: 'srv_test');
    final client = SrpHttpClient(trunk);
    await trunk.connect();
    try {
      final a = client.get(Uri.parse('http://x/alpha'));
      final b = client.get(Uri.parse('http://x/beta'));
      final results = await Future.wait([a, b]);
      final bodies = results.map((r) => r.body).toSet();
      expect(bodies, {'/alpha', '/beta'});
    } finally {
      trunk.dispose();
      await relay.close();
    }
  });

  test('a reader that cancels a streamed body closes its stream on the '
      'trunk', () async {
    final body = StreamController<List<int>>();
    final relay = FakeRelay(
      handler: (m, p, h, b) async => FakeResponse.streamed(200, {
        'content-type': 'text/event-stream',
      }, body.stream),
    );
    await relay.start();
    final trunk = SrpTrunk(url: relay.url, serverId: 'srv_test');
    final client = SrpHttpClient(trunk);
    await trunk.connect();
    try {
      final response = await client.send(
        http.Request('GET', Uri.parse('http://x/stream')),
      );
      final first = Completer<List<int>>();
      final sub = response.stream.listen(first.complete);
      body.add(utf8.encode('event: change\n\n'));
      expect(utf8.decode(await first.future), 'event: change\n\n');

      // A closed terminal or a disposed feed: the origin must stop streaming.
      await sub.cancel();
      for (var i = 0; i < 100 && relay.closedByClient.isEmpty; i++) {
        await Future<void>.delayed(const Duration(milliseconds: 10));
      }
      expect(relay.closedByClient, isNotEmpty);
    } finally {
      trunk.dispose();
      await body.close();
      await relay.close();
    }
  });

  test('a request body travels as 0x01 chunks and is reassembled', () async {
    final relay = FakeRelay(
      handler: (m, p, h, body) async {
        return FakeResponse(201, {}, body);
      },
    );
    await relay.start();
    final trunk = SrpTrunk(url: relay.url, serverId: 'srv_test');
    final client = SrpHttpClient(trunk);
    await trunk.connect();
    try {
      final payload = List<int>.generate(100, (i) => i % 256);
      final response = await client.post(
        Uri.parse('http://x/v1/notes'),
        headers: {'content-type': 'application/octet-stream'},
        body: payload,
      );
      expect(response.statusCode, 201);
      final echoed = response.bodyBytes;
      expect(echoed, payload);
    } finally {
      trunk.dispose();
      await relay.close();
    }
  });

  test(
    'every request ends its body with one empty 0x01 frame (§5.2)',
    () async {
      await _withRelay((relay) async {
        final trunk = _trunk(relay);
        final client = SrpHttpClient(trunk);
        try {
          await client.get(Uri.parse('http://x/v1/vaults'));
          await client.post(Uri.parse('http://x/v1/notes'), body: '{"a":1}');

          final ends = relay.requestBodyFrames.where((f) => f.$2 == 0).toList();
          expect(ends.length, 2, reason: 'one end of body per request');
          // The GET sends nothing but its end of body.
          final getStream = ends.first.$1;
          expect(relay.requestBodyFrames.where((f) => f.$1 == getStream), [
            (getStream, 0),
          ]);
          // The POST's end comes after its body.
          final postFrames = relay.requestBodyFrames
              .where((f) => f.$1 == ends.last.$1)
              .toList();
          expect(postFrames.last.$2, 0);
          expect(postFrames.first.$2, greaterThan(0));
        } finally {
          trunk.dispose();
        }
      });
    },
  );

  test('an empty chunk mid-body is skipped, not sent as end of body', () async {
    final relay = FakeRelay(
      handler: (m, p, h, body) async => FakeResponse(200, {}, body),
    );
    await relay.start();
    final trunk = SrpTrunk(url: relay.url, serverId: 'srv_test');
    final client = SrpHttpClient(trunk);
    try {
      final request = http.StreamedRequest('POST', Uri.parse('http://x/v1/n'));
      request.sink
        ..add(utf8.encode('abc'))
        ..add(const <int>[])
        ..add(utf8.encode('def'))
        ..close();
      final response = await http.Response.fromStream(
        await client.send(request),
      );
      expect(response.body, 'abcdef');
      expect(relay.requestBodyFrames.where((f) => f.$2 == 0).length, 1);
    } finally {
      trunk.dispose();
      await relay.close();
    }
  });

  test(
    'a body over 16 MiB goes out in bounded frames, whole and in order',
    () async {
      const size = 20 * 1024 * 1024;
      final sent = Uint8List(size);
      for (var i = 0; i < size; i++) {
        sent[i] = (i * 31 + (i >> 16)) & 0xff;
      }
      List<int>? received;
      final relay = FakeRelay(
        handler: (m, p, h, body) async {
          received = body;
          return FakeResponse(200, {}, utf8.encode('${body.length}'));
        },
      );
      await relay.start();
      final trunk = SrpTrunk(url: relay.url, serverId: 'srv_test');
      final client = SrpHttpClient(trunk);
      try {
        final response = await client.post(
          Uri.parse('http://x/v1/attachments'),
          body: sent,
        );
        expect(response.body, '$size');
        expect(received, sent, reason: 'whole and in order');

        final frames = relay.requestBodyFrames;
        expect(frames.last.$2, 0, reason: 'then exactly one end of body');
        expect(frames.where((f) => f.$2 == 0).length, 1);
        final chunks = frames.sublist(0, frames.length - 1);
        expect(chunks.length, size ~/ maxBodyFramePayload);
        expect(chunks.every((f) => f.$2 == maxBodyFramePayload), isTrue);
      } finally {
        trunk.dispose();
        await relay.close();
      }
    },
  );

  test(
    'a refused open surfaces as a transport error, not an HTTP response',
    () async {
      // No handler scripted: the fake refuses every open with ERROR{trunk_lost}.
      final relay = FakeRelay(handler: null);
      await relay.start();
      final trunk = SrpTrunk(url: relay.url, serverId: 'srv_test');
      final client = SrpHttpClient(trunk);
      await trunk.connect();
      try {
        await expectLater(
          client.get(Uri.parse('http://x/v1/notes')),
          throwsA(isA<SrpTransportException>()),
        );
      } finally {
        trunk.dispose();
        await relay.close();
      }
    },
  );

  test('a protocol error on the wire drops the whole trunk', () async {
    final relay = FakeRelay(handler: null);
    await relay.start();
    final trunk = SrpTrunk(url: relay.url, serverId: 'srv_test');
    await trunk.connect();
    final lost = <String>[];
    trunk.onTrunkLost.listen(lost.add);

    // Send a malformed frame that the trunk must treat as fatal (§3).
    relay.sendRaw(utf8.encode('not json'));

    await Future<void>.delayed(const Duration(milliseconds: 50));
    expect(trunk.isConnected, isFalse);
    expect(lost, isNotEmpty);
    trunk.dispose();
    await relay.close();
  });

  test('a refused open fails only its own request, and the next request on '
      'the trunk works', () async {
    await _withRelay((relay) async {
      final trunk = _trunk(relay);
      final client = SrpHttpClient(trunk);
      final lost = <String>[];
      trunk.onTrunkLost.listen(lost.add);
      await trunk.connect();

      relay.refuseOpens = 1;
      await expectLater(
        client.get(Uri.parse('http://relay.invalid/v1/notes/a')),
        throwsA(
          isA<SrpTransportException>().having(
            (e) => e.code,
            'code',
            'rate_limited',
          ),
        ),
      );
      expect(trunk.isConnected, isTrue);

      final response = await client.get(
        Uri.parse('http://relay.invalid/v1/notes/b'),
      );
      expect(response.statusCode, 200);
      expect(utf8.decode(response.bodyBytes), 'ok:/v1/notes/b');
      expect(trunk.isConnected, isTrue);
      expect(lost, isEmpty);
      trunk.dispose();
    });
  });

  test('a refusal that names no attempt still drops the trunk, as from a '
      'relay that predates decision 106', () async {
    await _withRelay((relay) async {
      final trunk = _trunk(relay);
      final client = SrpHttpClient(trunk);
      await trunk.connect();

      relay
        ..refuseOpens = 1
        ..refusalNamesAttempt = false;
      await expectLater(
        client.get(Uri.parse('http://relay.invalid/v1/notes/a')),
        throwsA(isA<SrpStreamAborted>()),
      );
      expect(trunk.isConnected, isFalse);
      expect(trunk.lossReason, 'rate_limited');
      trunk.dispose();
    });
  });

  test('binary framing round-trips per the spec (big-endian id)', () {
    final frame = encodeBodyFrame(bodyResponse, 0x01020304, [1, 2, 3]);
    expect(frame.sublist(0, 5), [bodyResponse, 1, 2, 3, 4]);
    final decoded = decodeBodyFrame(frame)!;
    expect(decoded.kind, bodyResponse);
    expect(decoded.streamId, 0x01020304);
    expect(decoded.payload, [1, 2, 3]);
    expect(decodeBodyFrame([bodyRequest, 0, 0]), isNull);
  });

  test('control messages carry the hard-pinned v:1 envelope', () {
    final good = SrpControl.decode('{"v":1,"type":"PING"}');
    expect(good.type, 'PING');
    expect(
      () => SrpControl.decode('{"v":2,"type":"PING"}'),
      throwsA(isA<SrpProtocolError>()),
    );
    expect(
      () => SrpControl.decode('{"type":"PING"}'),
      throwsA(isA<SrpProtocolError>()),
    );
    expect(
      () => SrpControl.decode('{"v":1}'),
      throwsA(isA<SrpProtocolError>()),
    );
  });
}
