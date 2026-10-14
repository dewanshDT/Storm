import 'dart:async';
import 'dart:collection';
import 'dart:convert';
import 'dart:io';
import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;

import 'package:storm/api/relay/srp_http_client.dart';
import 'package:storm/api/relay/srp_trunk.dart';
import 'package:storm/api/storm_api.dart';

/// SRP §5.2 "Control before bulk" (decision 116b) through a real storm-relay
/// and a real storm-server, each behind a slow link: a long upload from the
/// client and a long download from the server, at once, with small requests
/// alongside. Before 116b the client's trunk died `relay_silent` behind its
/// own upload, the server dropped its trunk behind its own download, and a
/// small request's `STREAM_ACK` waited behind the download until the relay
/// answered `server_timeout`.
///
/// Self-contained: it starts its own server and relay, on ports from
/// `STORM_THROTTLE_PORT` (never 8484–8487), and puts an in-process link with
/// limited bandwidth and latency in front of the relay on both sides.
void main() {
  const serverBin = String.fromEnvironment('STORM_SERVER_BIN');
  const relayBin = String.fromEnvironment('STORM_RELAY_BIN');
  const bootstrap = String.fromEnvironment('STORM_BOOTSTRAP');
  const basePort = int.fromEnvironment('STORM_THROTTLE_PORT', defaultValue: 0);
  const mib = int.fromEnvironment('STORM_THROTTLE_MIB', defaultValue: 20);
  const kbps = int.fromEnvironment('STORM_THROTTLE_KBPS', defaultValue: 2000);
  const delayMs = int.fromEnvironment(
    'STORM_THROTTLE_DELAY_MS',
    defaultValue: 150,
  );
  // `up` or `down` runs one transfer alone.
  const only = String.fromEnvironment('STORM_THROTTLE_ONLY');

  final processes = <Process>[];
  final links = <_Link>[];
  Directory? dir;
  late StormApi direct;
  late String vaultId;
  late String serverId;
  late String token;
  late String serverLog;

  setUpAll(() async {
    if (serverBin.isEmpty ||
        relayBin.isEmpty ||
        bootstrap.isEmpty ||
        basePort == 0) {
      fail(
        'Run through `make test-live`: it passes STORM_SERVER_BIN, '
        'STORM_RELAY_BIN, STORM_BOOTSTRAP and STORM_THROTTLE_PORT.',
      );
    }
    final relayPort = basePort;
    final serverLinkPort = basePort + 1;
    final clientLinkPort = basePort + 2;
    final serverPort = basePort + 3;
    dir = await Directory.systemTemp.createTemp('storm-throttle-');
    final vaults = Directory('${dir!.path}/vaults/primary')
      ..createSync(recursive: true);
    File('${vaults.path}/Seed.md').writeAsStringSync('# Seed\n');
    serverLog = '${dir!.path}/server.log';

    Future<Process> start(String bin, List<String> args, String log) async {
      final p = await Process.start(bin, args);
      // Written through at once: the bootstrap nonce is read from this file.
      final file = File(log);
      void append(List<int> d) =>
          file.writeAsBytesSync(d, mode: FileMode.append);
      p.stdout.listen(append);
      p.stderr.listen(append);
      processes.add(p);
      return p;
    }

    await start(relayBin, [
      '--bind',
      '127.0.0.1:$relayPort',
    ], '${dir!.path}/relay.log');
    // Each side's uplink is the slow one, as on a phone and a home line;
    // its downlink is ten times faster, so the relay itself never backs up.
    final up = kbps * 1000;
    final down = up * 10;
    const delay = Duration(milliseconds: delayMs);
    links
      ..add(await _Link.start(serverLinkPort, relayPort, up, down, delay))
      ..add(await _Link.start(clientLinkPort, relayPort, up, down, delay));
    await start(serverBin, [
      'serve',
      '--vault-root',
      '${dir!.path}/vaults',
      '--state',
      '${dir!.path}/state',
      '--port',
      '$serverPort',
    ], serverLog);

    final origin = 'http://127.0.0.1:$serverPort';
    for (var i = 0; ; i++) {
      try {
        final r = await http.get(Uri.parse('$origin/v1/health'));
        if (r.statusCode == 200) break;
      } on Object {
        // Not listening yet.
      }
      if (i == 60) fail('the server did not start');
      await Future<void>.delayed(const Duration(milliseconds: 500));
    }
    final claimed = await Process.run(
      'python3',
      [bootstrap, origin, serverLog],
      environment: {'STORM_SESSION': '', 'STORM_DEVICE': ''},
    );
    if (claimed.exitCode != 0) fail('bootstrap: ${claimed.stderr}');
    final session = RegExp(
      r"STORM_SESSION='?([^'\n]+)'?",
    ).firstMatch(claimed.stdout as String)!.group(1)!;
    token = session.replaceFirst('Bearer ', '');
    direct = StormApi(baseUrl: origin, token: token);
    vaultId = (await direct.vaults()).first.id;

    final saved = await http.put(
      Uri.parse('$origin/v1/config/relays'),
      headers: {'Authorization': session, 'Content-Type': 'application/json'},
      body: jsonEncode({
        'relays': ['ws://127.0.0.1:$serverLinkPort'],
      }),
    );
    expect(saved.statusCode, 200, reason: saved.body);
    for (var i = 0; ; i++) {
      final info =
          jsonDecode((await http.get(Uri.parse('$origin/v1/server'))).body)
              as Map<String, dynamic>;
      serverId = info['server_id'] as String;
      if ((info['relays'] as List? ?? const []).isNotEmpty) break;
      if (i == 100) fail('the server never registered with the relay');
      await Future<void>.delayed(const Duration(milliseconds: 200));
    }
  });

  tearDownAll(() async {
    for (final p in processes) {
      p.kill();
    }
    for (final l in links) {
      await l.close();
    }
    await dir?.delete(recursive: true);
  });

  test('a long upload and a long download over a slow link both finish, and a '
      'small request alongside never gets server_timeout', () async {
    final size = mib << 20;
    final bytes = Uint8List(size);
    for (var i = 0; i < size; i++) {
      bytes[i] = (i * 31 + 7) & 0xff;
    }
    await direct.uploadAttachment(vaultId, 'throttle/down.bin', bytes);

    final trunk = SrpTrunk(
      url: Uri.parse('ws://127.0.0.1:${basePort + 2}/connect/$serverId'),
      serverId: serverId,
    );
    final lost = <String>[];
    trunk.onTrunkLost.listen(lost.add);
    final relayed = StormApi(
      baseUrl: 'http://127.0.0.1:${basePort + 3}',
      token: token,
      client: SrpHttpClient(trunk),
    );
    try {
      await relayed.vaults();
      final clock = Stopwatch()..start();
      final outcome = <String, String>{};
      Future<void> timed(String name, Future<Object?> Function() run) async {
        try {
          final got = await run();
          final n = got is List<int> ? got.length : size;
          outcome[name] = '${clock.elapsed.inSeconds}s, $n bytes';
        } on Object catch (e) {
          outcome[name] = 'failed at ${clock.elapsed.inSeconds}s: $e';
        }
      }

      final transfers = Future.wait([
        if (only != 'down')
          timed(
            'upload',
            () => relayed.uploadAttachment(vaultId, 'throttle/up.bin', bytes),
          ),
        if (only != 'up')
          timed(
            'download',
            () => relayed.attachment(vaultId, 'throttle/down.bin'),
          ),
      ]);

      final small = <int>[];
      final failures = <String>[];
      var done = false;
      unawaited(transfers.whenComplete(() => done = true));
      while (!done) {
        final t = Stopwatch()..start();
        try {
          await relayed.vaults().timeout(const Duration(minutes: 5));
          small.add(t.elapsed.inMilliseconds);
        } on Object catch (e) {
          failures.add('$e');
        }
        await Future.any([
          transfers,
          Future<void>.delayed(const Duration(seconds: 5)),
        ]);
      }

      final dropped = File(
        serverLog,
      ).readAsLinesSync().where((l) => l.contains('relay trunk closed')).length;
      // ignore: avoid_print
      print(
        'uplinks ${kbps}kbps, ${delayMs}ms one way, $mib MiB: $outcome; '
        'small requests $small ms; failures $failures; '
        'client trunk lost $lost; server trunk drops $dropped',
      );

      for (final entry in outcome.entries) {
        expect(entry.value, endsWith('$size bytes'), reason: entry.key);
      }
      if (only != 'down') {
        final uploaded = await direct.attachment(vaultId, 'throttle/up.bin');
        expect(uploaded.length, size);
      }
      expect(failures, isEmpty, reason: 'a small request failed alongside');
      expect(small, isNotEmpty);
      expect(lost, isEmpty, reason: 'the client trunk died');
      expect(dropped, 0, reason: 'the server dropped its trunk');
    } finally {
      trunk.dispose();
    }
  }, timeout: const Timeout(Duration(minutes: 15)));
}

/// A TCP link of limited bandwidth and fixed latency in each direction, with
/// a small queue, so a sender that outruns it is held back by TCP. "Up" is
/// from whoever dials it toward the relay.
class _Link {
  _Link._(this._server);

  final ServerSocket _server;
  final _sockets = <Socket>[];

  static Future<_Link> start(
    int port,
    int targetPort,
    int up,
    int down,
    Duration delay,
  ) async {
    final server = await ServerSocket.bind(InternetAddress.loopbackIPv4, port);
    final link = _Link._(server);
    server.listen((inbound) async {
      final Socket outbound;
      try {
        outbound = await Socket.connect(
          InternetAddress.loopbackIPv4,
          targetPort,
        );
      } on Object {
        inbound.destroy();
        return;
      }
      link._sockets.addAll([inbound, outbound]);
      _smallReceiveBuffer(inbound);
      _smallReceiveBuffer(outbound);
      _pipe(inbound, outbound, up, delay);
      _pipe(outbound, inbound, down, delay);
    });
    return link;
  }

  /// The link's queue is [_pipe]'s, so the kernel must not hide a bigger one
  /// in an autotuned receive buffer: megabytes of it is seconds of
  /// bufferbloat no real path has. `SO_RCVBUF`, Linux numbering.
  static void _smallReceiveBuffer(Socket socket) {
    if (!Platform.isLinux) return;
    socket.setRawOption(RawSocketOption.fromInt(1, 8, 64 * 1024));
  }

  static void _pipe(Socket from, Socket to, int bitsPerSecond, Duration delay) {
    const queueLimit = 64 * 1024;
    final pending = Queue<Uint8List>();
    var queued = 0;
    var free = DateTime.now();
    late StreamSubscription<Uint8List> sub;
    sub = from.listen(
      (data) {
        final now = DateTime.now();
        final start = free.isAfter(now) ? free : now;
        free = start.add(
          Duration(microseconds: data.length * 8 * 1000000 ~/ bitsPerSecond),
        );
        final release = free.add(delay);
        pending.add(data);
        queued += data.length;
        if (queued > queueLimit) sub.pause();
        Timer(release.difference(DateTime.now()), () {
          final chunk = pending.removeFirst();
          queued -= chunk.length;
          try {
            to.add(chunk);
          } on Object {
            // The other side is gone; its own listener tears this down.
          }
          if (sub.isPaused && queued <= queueLimit) sub.resume();
        });
      },
      onDone: () {
        Timer(free.add(delay).difference(DateTime.now()), () {
          to.destroy();
        });
      },
      onError: (_) {
        to.destroy();
        from.destroy();
      },
      cancelOnError: true,
    );
  }

  Future<void> close() async {
    for (final s in _sockets) {
      s.destroy();
    }
    await _server.close();
  }
}
