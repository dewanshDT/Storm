import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';

import 'package:storm/agent/agent_api.dart';
import 'package:storm/agent/terminal_events.dart';
import 'package:storm/agent/terminal_stream.dart';

/// The Dart client against a real server and a real host (decision 77d).
///
/// Needs a server with an enrolled, online host offering the `fake` provider
/// and a workspace named `storm`. Set `STORM_AGENT_BASE` and
/// `STORM_AGENT_TOKEN` (an owner's bearer token) to run it; otherwise it is
/// skipped. `apps/server/tests/agent_e2e.py` proves the server and host; this
/// proves the client speaks the same wire.
void main() {
  final base = Platform.environment['STORM_AGENT_BASE'];
  final token = Platform.environment['STORM_AGENT_TOKEN'];
  final skip = base == null || token == null
      ? 'set STORM_AGENT_BASE and STORM_AGENT_TOKEN'
      : false;

  test('launch, stream, input, resize and end through the real wire', () async {
    final api = AgentApi(baseUrl: base!, token: token!);
    expect(await api.canUseAgents(), isTrue);
    final host = (await api.hosts()).firstWhere((h) => h.online);
    final s = await api.launch(
      hostId: host.id,
      workspace: 'storm',
      provider: 'fake',
      cols: 80,
      rows: 24,
    );

    final text = StringBuffer();
    var last = 0;
    final statuses = <String>[];
    final done = Completer<void>();
    final sub = openTerminalStream(api, s.id, 0).listen((e) {
      switch (e) {
        case StatusEvent(:final session):
          statuses.add(session.status);
        case OutputEvent(:final end, :final bytes):
          text.write(utf8.decode(bytes));
          last = end;
        case GapEvent():
          break;
      }
    }, onDone: () => done.complete());

    Future<void> until(bool Function() ok) async {
      for (var i = 0; i < 100 && !ok(); i++) {
        await Future<void>.delayed(const Duration(milliseconds: 100));
      }
      expect(ok(), isTrue, reason: 'got: $text / $statuses');
    }

    await until(() => text.toString().contains('fake session ${s.id}'));
    await api.input(s.id, Uint8List.fromList(utf8.encode('from dart\r')));
    await until(() => text.toString().contains('from dart'));
    await api.resize(s.id, 100, 30, focus: true);
    await until(() => text.toString().contains('[resize 100x30]'));

    // Resume from an offset: exactly what followed, nothing earlier.
    final resumed = StringBuffer();
    final sub2 = openTerminalStream(api, s.id, last).listen((e) {
      if (e is OutputEvent) resumed.write(utf8.decode(e.bytes));
    });
    await api.input(s.id, Uint8List.fromList(utf8.encode('after')));
    await until(() => resumed.toString() == 'after');
    await sub2.cancel();

    await api.end(s.id);
    await done.future.timeout(const Duration(seconds: 10));
    expect(statuses.last, 'stopped');
    await sub.cancel();
    await api.dismiss(s.id);
    api.dispose();
  }, skip: skip);
}
