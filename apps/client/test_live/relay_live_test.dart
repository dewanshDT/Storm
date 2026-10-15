import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;

import 'package:storm/api/relay/srp_http_client.dart';
import 'package:storm/api/relay/srp_trunk.dart';
import 'package:storm/api/storm_api.dart';

/// The Dart SRP client against a real `storm-relay` and a real storm-server.
///
/// `test/srp_tunnel_test.dart` pins the framing against a fake relay; this is
/// the one place the three implementations meet. It exists because a relayed
/// write hung until timeout (P2, decision 92) while every unit test was
/// green: the fake answered after a fixed delay rather than at end of body.
///
/// `make test-live` starts the relay, points the server at it and passes
/// `STORM_RELAY_ORIGIN` (the server's direct URL) and `STORM_SESSION` as
/// `--dart-define`s.
void main() {
  const origin = String.fromEnvironment('STORM_RELAY_ORIGIN');
  const session = String.fromEnvironment('STORM_SESSION');
  // A hang is the failure under test; it must fail fast, not at the default.
  const deadline = Duration(seconds: 10);

  late String vaultId;
  late String token;
  late String serverId;
  late String serverKey;
  late Uri relayAddress;
  late SrpTrunk trunk;
  late StormApi relayed;
  late StormApi direct;

  /// A client on a trunk of its own, for a test that must not share one: a
  /// trunk the relay closes takes every stream on it down.
  (SrpTrunk, StormApi) freshRelayed() {
    final t = SrpTrunk(
      url: relayAddress,
      serverId: serverId,
      serverKey: serverKey,
    );
    return (
      t,
      StormApi(baseUrl: origin, token: token, client: SrpHttpClient(t)),
    );
  }

  Uint8List pattern(int size) {
    final bytes = Uint8List(size);
    for (var i = 0; i < size; i++) {
      bytes[i] = (i * 31 + 7) & 0xff;
    }
    return bytes;
  }

  setUpAll(() async {
    if (origin.isEmpty || session.isEmpty) {
      fail(
        'Run through `make test-live`: it starts storm-relay and passes '
        'STORM_RELAY_ORIGIN and STORM_SESSION.',
      );
    }
    token = session.replaceFirst('Bearer ', '');
    direct = StormApi(baseUrl: origin, token: token);

    // Registration happens in the background after the relay list is saved,
    // so wait for the server to report the relay rather than racing it.
    Map<String, dynamic>? relay;
    String? id;
    String? key;
    final until = DateTime.now().add(deadline);
    while (relay == null && DateTime.now().isBefore(until)) {
      final info =
          jsonDecode((await http.get(Uri.parse('$origin/v1/server'))).body)
              as Map<String, dynamic>;
      id = info['server_id'] as String;
      key = info['public_key'] as String;
      final relays = (info['relays'] as List? ?? const [])
          .cast<Map<String, dynamic>>();
      if (relays.isNotEmpty) {
        relay = relays.first;
      } else {
        await Future<void>.delayed(const Duration(milliseconds: 200));
      }
    }
    if (relay == null) fail('The server never registered with the relay.');

    serverId = id!;
    serverKey = key!;
    relayAddress = Uri.parse(relay['public_address'] as String);
    trunk = SrpTrunk(
      url: relayAddress,
      serverId: serverId,
      serverKey: serverKey,
    );
    relayed = StormApi(
      baseUrl: origin,
      token: token,
      client: SrpHttpClient(trunk),
    );
    vaultId = (await direct.vaults()).first.id;
  });

  tearDownAll(() {
    trunk.dispose();
    direct.dispose();
  });

  String scratch(String name) =>
      'RelayTest/${DateTime.now().microsecondsSinceEpoch}-$name.md';

  test('a relayed GET answers what the LAN answers', () async {
    final viaRelay = await relayed.vaults().timeout(deadline);
    final viaLan = await direct.vaults();
    expect(viaRelay.map((v) => v.id), viaLan.map((v) => v.id));
  });

  test('a relayed POST with a JSON body completes', () async {
    final path = scratch('create');
    final created = await relayed
        .createNote(vaultId: vaultId, path: path, content: '# Relayed\n')
        .timeout(deadline);
    expect(created.meta.path, path);

    final fetched = await direct.note(vaultId, created.meta.id);
    expect(fetched.content, contains('# Relayed'));
  });

  test('a relayed save (PUT with base_version) completes', () async {
    final created = await direct.createNote(
      vaultId: vaultId,
      path: scratch('save'),
      content: '# Before\n',
    );
    final saved = await relayed
        .saveNote(
          vaultId: vaultId,
          id: created.meta.id,
          baseVersion: created.meta.version,
          content: '# After\n',
        )
        .timeout(deadline);
    expect(saved.content, contains('# After'));
  });

  test('a relayed POST with no body completes', () async {
    final created = await direct.createNote(
      vaultId: vaultId,
      path: scratch('opened'),
      content: '# Opened\n',
    );
    await relayed.markOpened(vaultId, created.meta.id).timeout(deadline);
  });

  // ---- decision 98: what the unit fakes cannot show ------------------------

  test('the change feed reaches a relayed client when the LAN edits', () async {
    final (feedTrunk, feedApi) = freshRelayed();
    addTearDown(feedTrunk.dispose);
    final ticket = await feedApi.wsTicket().timeout(deadline);
    final request = http.Request(
      'GET',
      Uri.parse('$origin/v1/stream?ticket=${Uri.encodeComponent(ticket)}'),
    )..headers['Accept'] = 'text/event-stream';
    final response = await SrpHttpClient(
      feedTrunk,
    ).send(request).timeout(deadline);
    expect(response.statusCode, 200);

    final change = Completer<void>();
    final sub = response.stream
        .transform(utf8.decoder)
        .transform(const LineSplitter())
        .listen((line) {
          if (line.trim() == 'event: change' && !change.isCompleted) {
            change.complete();
          }
        });
    addTearDown(sub.cancel);

    await direct.createNote(
      vaultId: vaultId,
      path: scratch('feed'),
      content: '# Pushed\n',
    );
    await change.future.timeout(deadline);
  });

  test(
    'a multi-megabyte note saves and reads back through the relay',
    () async {
      final body = '# Big\n\n${'lorem ipsum dolor sit amet\n' * 120000}';
      final created = await relayed
          .createNote(vaultId: vaultId, path: scratch('big'), content: body)
          .timeout(deadline);
      final read = await relayed
          .note(vaultId, created.meta.id)
          .timeout(deadline);
      expect(read.content, contains(body.substring(body.length - 1000)));
      expect(read.content.length, greaterThanOrEqualTo(body.length));
    },
  );

  test('a 12 MiB attachment round-trips through the relay', () async {
    final bytes = pattern(12 * 1024 * 1024);
    final path = 'RelayTest/${DateTime.now().microsecondsSinceEpoch}.bin';
    await relayed.uploadAttachment(vaultId, path, bytes).timeout(deadline);
    final back = await relayed.attachment(vaultId, path).timeout(deadline);
    expect(back.length, bytes.length);
    expect(back, bytes);
  });

  test('a download larger than a WebSocket frame arrives whole and leaves the '
      'server registered', () async {
    // One 0x02 frame per handler chunk put a 17 MiB attachment over the
    // 16 MiB frame limit; the relay closed the server's trunk and every
    // relayed client of the server lost it (decision 98).
    final bytes = pattern(20 * 1024 * 1024);
    final path = 'RelayTest/${DateTime.now().microsecondsSinceEpoch}-20m.bin';
    await direct.uploadAttachment(vaultId, path, bytes);

    final (bigTrunk, bigApi) = freshRelayed();
    addTearDown(bigTrunk.dispose);
    final back = await bigApi.attachment(vaultId, path).timeout(deadline);
    expect(back.length, bytes.length);
    expect(back, bytes);

    final (nextTrunk, nextApi) = freshRelayed();
    addTearDown(nextTrunk.dispose);
    await nextApi.vaults().timeout(deadline);
  });

  test('an upload larger than a WebSocket frame crosses the relay', () async {
    // Sent as one 0x01 frame, a 20 MiB body made the relay close the
    // client's trunk (decision 100).
    final (bigTrunk, bigApi) = freshRelayed();
    addTearDown(bigTrunk.dispose);
    final path = scratch('20m-up');
    final bytes = pattern(20 * 1024 * 1024);
    await bigApi.uploadAttachment(vaultId, path, bytes).timeout(deadline);
    expect(await direct.attachment(vaultId, path), bytes);
  });

  test('a bad or repeated head from one client leaves the server trunk up for '
      'the others (116a)', () async {
    // Raw sockets: SrpTrunk never sends either frame. Before 116a each one
    // reached storm-server, which sent ERROR and dropped its trunk, and every
    // relayed client of the server was cut off.
    Future<(WebSocket, StreamIterator<Map<String, dynamic>>)> raw() async {
      final ws = await WebSocket.connect(
        keyedConnectUrl(relayAddress, serverKey).toString(),
      );
      final frames = StreamIterator(
        ws
            .where((m) => m is String)
            .map((m) => jsonDecode(m as String) as Map<String, dynamic>)
            .where((m) => m['type'] != 'PING'),
      );
      ws.add(
        jsonEncode({
          'v': 1,
          'type': 'HELLO',
          'server_id': serverId,
          'pubkey': serverKey,
        }),
      );
      expect(await frames.moveNext().timeout(deadline), isTrue);
      expect(frames.current['type'], 'READY');
      return (ws, frames);
    }

    Future<int> open(
      WebSocket ws,
      StreamIterator<Map<String, dynamic>> frames,
    ) async {
      ws.add(jsonEncode({'v': 1, 'type': 'OPEN_STREAM', 'attempt_id': 'm'}));
      expect(await frames.moveNext().timeout(deadline), isTrue);
      expect(frames.current['type'], 'STREAM_READY');
      return frames.current['stream_id'] as int;
    }

    Map<String, dynamic> head(int id, Object headers) => {
      'v': 1,
      'type': 'HTTP_REQUEST_HEAD',
      'stream_id': id,
      'method': 'GET',
      'path': '/v1/health',
      'headers': headers,
    };

    Future<Map<String, dynamic>> nextError(
      StreamIterator<Map<String, dynamic>> frames,
    ) async {
      while (await frames.moveNext().timeout(deadline)) {
        if (frames.current['type'] == 'ERROR') return frames.current;
      }
      fail('the relay closed without an ERROR');
    }

    await relayed.vaults().timeout(deadline);

    final (bad, badFrames) = await raw();
    addTearDown(bad.close);
    final badId = await open(bad, badFrames);
    bad.add(jsonEncode(head(badId, {'x': 1})));
    expect((await nextError(badFrames))['code'], 'protocol_error');

    final (twice, twiceFrames) = await raw();
    addTearDown(twice.close);
    final twiceId = await open(twice, twiceFrames);
    twice.add(jsonEncode(head(twiceId, <String, String>{})));
    twice.add(jsonEncode(head(twiceId, <String, String>{})));
    final refused = await nextError(twiceFrames);
    expect(refused['code'], 'stream_closed');
    expect(refused['stream_id'], twiceId);

    // The shared trunk is still up: the long-lived client is served, and the
    // server still reports itself registered.
    await Future<void>.delayed(const Duration(milliseconds: 500));
    expect(await relayed.vaults().timeout(deadline), isNotEmpty);
    final info =
        jsonDecode((await http.get(Uri.parse('$origin/v1/server'))).body)
            as Map<String, dynamic>;
    expect(info['relays'], isNotEmpty);
  });

  test('MCP initializes and writes through the real relay', () async {
    final before = await direct.config();
    await direct.setMcpEnabled(true, writable: true);
    addTearDown(
      () =>
          direct.setMcpEnabled(before.mcpEnabled, writable: before.mcpWritable),
    );
    final key = await direct.createMcpKey(name: 'relay-live');
    final client = SrpHttpClient(trunk);
    var id = 0;

    Future<Map<String, dynamic>> rpc(
      String method,
      Map<String, dynamic> params,
    ) async {
      final r = await client
          .post(
            Uri.parse('$origin/mcp'),
            headers: {
              'Authorization': 'Bearer ${key.secret}',
              'Content-Type': 'application/json',
              'Accept': 'application/json, text/event-stream',
            },
            body: jsonEncode({
              'jsonrpc': '2.0',
              'id': ++id,
              'method': method,
              'params': params,
            }),
          )
          .timeout(deadline);
      expect(r.statusCode, 200, reason: r.body);
      return jsonDecode(r.body) as Map<String, dynamic>;
    }

    final init = await rpc('initialize', {
      'protocolVersion': '2025-11-25',
      'capabilities': <String, dynamic>{},
      'clientInfo': {'name': 'relay-live', 'version': '1'},
    });
    expect(init['result']['serverInfo']['name'], 'storm');

    final path = scratch('mcp');
    final written = await rpc('tools/call', {
      'name': 'create_note',
      'arguments': {'vault': vaultId, 'path': path, 'content': '# Via MCP\n'},
    });
    expect(written['result']['isError'], isNot(true), reason: '$written');
    final id0 = written['result']['structuredContent']['note']['id'] as String;
    expect((await direct.note(vaultId, id0)).content, contains('# Via MCP'));
  });
}
