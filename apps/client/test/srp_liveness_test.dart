// The Dart trunk's two failure signals (decision 97): a relay that answers
// for an absent server is told apart from a relay that is gone, and a relay
// that falls silent is dropped at the heartbeat deadline instead of hanging
// every request until its own timeout.
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:storm/api/auth_models.dart';
import 'package:storm/api/relay/srp_codec.dart';
import 'package:storm/api/relay/srp_http_client.dart';
import 'package:storm/api/relay/srp_trunk.dart';
import 'package:storm/api/server_verifier.dart';
import 'package:storm/api/storm_connection.dart';
import 'package:storm/state/health.dart';
import 'package:storm/ui/widgets.dart';

/// A relay that answers on protocol events only, never on a clock — except
/// [helloDelay], which stands in for §5's `HELLO` hold.
class _Relay {
  _Relay({this.refuseHello, this.helloDelay = Duration.zero, this.pong = true});

  /// Answer `HELLO` with this trunk-level `ERROR` code instead of `READY`.
  final String? refuseHello;
  final Duration helloDelay;

  /// Whether `PING` gets a `PONG`. Off is a relay gone silent: the socket is
  /// open and nothing comes back, which is a half-open TCP connection.
  final bool pong;

  HttpServer? _server;
  final List<WebSocket> _sockets = [];
  int hellos = 0;
  int pings = 0;

  String get origin => 'ws://127.0.0.1:${_server!.port}';
  String get connectUrl => '$origin/connect/srv_test';

  Future<void> start() async {
    _server = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
    _server!.listen((req) async {
      final socket = await WebSocketTransformer.upgrade(req);
      _sockets.add(socket);
      socket.listen((frame) {
        if (frame is String) _onControl(socket, SrpControl.decode(frame));
      }, onError: (_) {});
    });
  }

  Future<void> _onControl(WebSocket socket, SrpControl frame) async {
    switch (frame.type) {
      case 'HELLO':
        hellos++;
        await Future<void>.delayed(helloDelay);
        final refusal = refuseHello;
        if (refusal != null) {
          socket.add(SrpControl.encode('ERROR', {'code': refusal}));
          await socket.close();
          return;
        }
        socket.add(SrpControl.encode('READY', {'client_trunk_id': 'ctk_t'}));
      case 'PING':
        pings++;
        if (pong) socket.add(SrpControl.encode('PONG'));
      case 'OPEN_STREAM':
        socket.add(
          SrpControl.encode('STREAM_READY', {
            'attempt_id': frame.body['attempt_id'],
            'stream_id': 1,
          }),
        );
      // HTTP_REQUEST_HEAD and body frames get nothing: the request is in
      // flight when the relay goes quiet.
    }
  }

  Future<void> close() async {
    for (final s in _sockets) {
      await s.close();
    }
    await _server?.close(force: true);
  }
}

void main() {
  group('server offline vs relay unavailable', () {
    test('a relay saying server_unreachable keeps the code', () async {
      final relay = _Relay(refuseHello: 'server_unreachable');
      await relay.start();
      final trunk = SrpTrunk(url: Uri.parse(relay.connectUrl), serverId: 's');
      final lost = <String>[];
      final unreachable = <String>[];
      trunk.onTrunkLost.listen(lost.add);
      trunk.onServerUnreachable.listen(unreachable.add);
      try {
        await expectLater(
          trunk.connect(),
          throwsA(
            isA<SrpHandshakeFailed>().having(
              (e) => e.code,
              'code',
              'server_unreachable',
            ),
          ),
        );
        await pumpEventQueue();
        expect(lost, ['relay_error']);
        expect(unreachable, ['server_unreachable']);
        expect(trunk.lossReason, 'server_unreachable');
      } finally {
        trunk.dispose();
        await relay.close();
      }
    });

    test(
      'a refusal that is not about the server is not server offline',
      () async {
        final relay = _Relay(refuseHello: 'rate_limited');
        await relay.start();
        final trunk = SrpTrunk(url: Uri.parse(relay.connectUrl), serverId: 's');
        final unreachable = <String>[];
        trunk.onServerUnreachable.listen(unreachable.add);
        try {
          final error = await trunk.connect().then<Object?>(
            (_) => null,
            onError: (Object e) => e,
          );
          expect((error! as SrpHandshakeFailed).code, 'rate_limited');
          await pumpEventQueue();
          expect(unreachable, isEmpty);
          expect(trunk.lossReason, 'rate_limited');
        } finally {
          trunk.dispose();
          await relay.close();
        }
      },
    );

    test('the handshake outlasts the relay\'s 5 s HELLO hold', () {
      final trunk = SrpTrunk(url: Uri.parse('ws://x/connect/s'), serverId: 's');
      expect(trunk.handshakeTimeout, greaterThan(const Duration(seconds: 5)));
    });

    StormConnection connectionVia(_Relay relay) => StormConnection(
      // Nothing listens on port 9: the direct candidate is refused at once.
      localAddress: 'http://127.0.0.1:9',
      token: 't',
      serverId: 'srv_test',
      relays: [RelayAdvert(url: relay.origin, publicAddress: relay.connectUrl)],
      publicRelayDelay: const Duration(milliseconds: 200),
    );

    test('the connection reports serverOffline, not offline', () async {
      final relay = _Relay(refuseHello: 'server_unreachable');
      await relay.start();
      final c = connectionVia(relay);
      try {
        expect(await c.connect(), ServerProof.unreachable);
        if (c.status != ConnectionStatus.serverOffline) {
          await c.statusChanges
              .firstWhere((s) => s == ConnectionStatus.serverOffline)
              .timeout(const Duration(seconds: 2));
        }
        expect(c.status, ConnectionStatus.serverOffline);
      } finally {
        c.reset();
        await relay.close();
      }
    });

    test(
      'a verdict landing after the race gave up still reaches the status',
      () async {
        // The relay holds HELLO longer than the race waits, as a real one does
        // for 5 s, so the race has already said offline when the answer comes.
        final relay = _Relay(
          refuseHello: 'server_unreachable',
          helloDelay: const Duration(milliseconds: 600),
        );
        await relay.start();
        final c = connectionVia(relay);
        try {
          final changes = <ConnectionStatus>[];
          c.statusChanges.listen(changes.add);
          expect(await c.connect(), ServerProof.unreachable);
          expect(c.status, ConnectionStatus.offline);
          await c.statusChanges
              .firstWhere((s) => s == ConnectionStatus.serverOffline)
              .timeout(const Duration(seconds: 3));
          expect(changes, [
            ConnectionStatus.connecting,
            ConnectionStatus.offline,
            ConnectionStatus.serverOffline,
          ]);
        } finally {
          c.reset();
          await relay.close();
        }
      },
    );

    test('reset forgets the verdict', () async {
      final relay = _Relay(refuseHello: 'server_unreachable');
      await relay.start();
      final c = connectionVia(relay);
      try {
        await c.connect();
        await pumpEventQueue();
        c.reset();
        expect(c.status, ConnectionStatus.idle);
      } finally {
        await relay.close();
      }
      // The relay is gone now; nothing answers, so this is plain offline.
      expect(await c.connect(), ServerProof.unreachable);
      await Future<void>.delayed(const Duration(milliseconds: 300));
      expect(c.status, ConnectionStatus.offline);
      c.reset();
    });

    test('the dot and its line say which one it is', () {
      final down = dotStatusFor(
        online: false,
        syncing: false,
        pending: 0,
        connection: ConnectionStatus.serverOffline,
      );
      expect(down, DotStatus.serverOffline);
      expect(
        dotStatusFor(
          online: false,
          syncing: false,
          pending: 0,
          connection: ConnectionStatus.offline,
        ),
        DotStatus.offline,
      );
      expect(syncLine(down, 0, null), contains('relay reachable'));
      expect(syncLine(down, 2, null), contains('2 edits queued'));
      expect(syncLine(DotStatus.offline, 0, null), isNot(contains('relay')));
      // Identity still outranks both.
      expect(
        dotStatusFor(
          online: false,
          syncing: false,
          pending: 0,
          identityFailed: true,
          connection: ConnectionStatus.serverOffline,
        ),
        DotStatus.untrusted,
      );
    });
  });

  group('a relay gone silent', () {
    test(
      'is dropped at the deadline and fails the request in flight',
      () async {
        final relay = _Relay(pong: false);
        await relay.start();
        final trunk = SrpTrunk(
          url: Uri.parse(relay.connectUrl),
          serverId: 's',
          heartbeat: const Duration(milliseconds: 50),
        );
        final lost = <String>[];
        trunk.onTrunkLost.listen(lost.add);
        try {
          final error = await SrpHttpClient(trunk)
              .send(http.Request('GET', Uri.parse('http://relay.invalid/v1/x')))
              .then<Object?>((_) => null, onError: (Object e) => e)
              .timeout(const Duration(seconds: 3));
          expect(error, isA<SrpTransportException>());
          await pumpEventQueue();
          expect(lost, ['relay_silent']);
          expect(trunk.isConnected, isFalse);
          expect(relay.pings, greaterThan(0));
        } finally {
          trunk.dispose();
          await relay.close();
        }
      },
    );

    test('a relay that answers heartbeats is kept past the deadline', () async {
      final relay = _Relay();
      await relay.start();
      final trunk = SrpTrunk(
        url: Uri.parse(relay.connectUrl),
        serverId: 's',
        heartbeat: const Duration(milliseconds: 50),
      );
      try {
        await trunk.connect();
        await Future<void>.delayed(const Duration(milliseconds: 500));
        expect(trunk.isConnected, isTrue);
        expect(relay.pings, greaterThan(3));
      } finally {
        trunk.dispose();
        await relay.close();
      }
    });

    test(
      'a handshake that times out closes the trunk, not a second socket',
      () async {
        // HELLO is never answered within the (shortened) handshake timeout.
        final relay = _Relay(helloDelay: const Duration(seconds: 2));
        await relay.start();
        final trunk = SrpTrunk(
          url: Uri.parse(relay.connectUrl),
          serverId: 's',
          handshakeTimeout: const Duration(milliseconds: 100),
        );
        try {
          await expectLater(
            trunk.connect(),
            throwsA(isA<SrpHandshakeFailed>()),
          );
          expect(trunk.lossReason, 'handshake_timeout');
          await expectLater(trunk.connect(), throwsA(isA<SrpTrunkClosed>()));
          expect(relay.hellos, 1);
        } finally {
          trunk.dispose();
          await relay.close();
        }
      },
    );
  });
}
