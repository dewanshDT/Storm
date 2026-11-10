import 'dart:async';
import 'dart:convert';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'package:storm/agent/agent_api.dart';
import 'package:storm/agent/agent_models.dart';
import 'package:storm/agent/agent_state.dart';
import 'package:storm/agent/agent_widgets.dart';
import 'package:storm/agent/launcher.dart';
import 'package:storm/agent/hosts_screen.dart';
import 'package:storm/agent/session_controller.dart';
import 'package:storm/agent/terminal_events.dart';
import 'package:storm/agent/terminal_surface.dart';
import 'package:storm/ui/theme.dart';

/// Agent Runtime V1, client side (decision 77d).
void main() {
  Map<String, dynamic> session({
    String id = 'ags_1',
    String status = 'running',
    String? endReason,
    int? exitCode,
  }) => {
    'id': id,
    'host_id': 'hst_1',
    'workspace': 'storm',
    'provider': 'claude-code',
    'status': status,
    'end_reason': endReason,
    'exit_code': exitCode,
    'signal': null,
    'cols': 80,
    'rows': 24,
    'created_at': '2026-10-03T12:00:00Z',
    'provider_fallback': null,
  };

  group('the stream is parsed exactly', () {
    test('events split across chunks arrive whole', () {
      final p = SseParser();
      final status = 'event: status\ndata: ${jsonEncode(session())}\n\n';
      final output =
          'event: output\nid: 5\ndata: ${base64.encode(utf8.encode('hello'))}\n\n';
      final all = ': keepalive\n\n$status$output';
      final events = <TerminalEvent>[];
      for (var i = 0; i < all.length; i += 7) {
        events.addAll(p.feed(all.substring(i, (i + 7).clamp(0, all.length))));
      }
      expect(events, hasLength(2));
      expect((events[0] as StatusEvent).session.status, 'running');
      final out = events[1] as OutputEvent;
      expect((out.end, utf8.decode(out.bytes)), (5, 'hello'));
    });

    test('a gap names the missing range, and CRLF is accepted', () {
      final e = SseParser().feed(
        'event: gap\r\ndata: {"from":0,"to":900}\r\n\r\n',
      );
      final gap = e.single as GapEvent;
      expect((gap.from, gap.to), (0, 900));
    });
  });

  group('a session is called what the server named it (Q12)', () {
    test('the name, context and write vault come from the record', () {
      final s = AgentSession.fromJson({
        ...session(),
        'name': 'gateway-spec',
        'context': {'vault_id': 'v1', 'note_id': 'n1', 'title': 'Gateway spec'},
        'write_vault_id': 'v1',
        'wrote_count': 2,
      });
      expect(s.name, 'gateway-spec');
      expect(s.context!.title, 'Gateway spec');
      expect((s.writeVaultId, s.wroteCount), ('v1', 2));
    });

    test('a session from before names is called by its workspace', () {
      expect(AgentSession.fromJson(session()).name, 'storm');
    });
  });

  group('a session says what happened in plain words', () {
    test('the handoff status words, and why it ended beside them', () {
      AgentSession s(String st, {String? r, int? x}) =>
          AgentSession.fromJson(session(status: st, endReason: r, exitCode: x));
      expect(s('starting').statusLabel, 'Starting');
      expect(s('running').statusLabel, 'Running');
      expect(s('unknown').statusLabel, 'Unknown');
      expect(s('completed', x: 0).statusLabel, 'Completed');
      expect(s('completed', x: 0).endDetail, isNull);
      expect(s('completed', x: 2).endDetail, 'exit 2');
      expect(s('stopped').statusLabel, 'Stopped');
      expect(s('failed', r: 'host_restart').statusLabel, 'Failed');
      expect(s('failed', r: 'host_restart').endDetail, 'host restarted');
      expect(s('failed', r: 'host_revoked').endDetail, 'host revoked');
      expect(
        s('failed', r: 'credential_unavailable').endDetail,
        'no credential this host may use',
      );
      expect(s('failed', r: 'lost').endDetail, 'lost');
      expect(providerLabel('claude-code'), 'Claude Code');
    });

    test('the ended line says when and for how long', () {
      final done = AgentSession.fromJson({
        ...session(status: 'completed', exitCode: 0),
        'started_at': '2026-10-03T12:00:00Z',
        'ended_at': '2026-10-03T12:38:00Z',
      });
      expect(
        endedLine(done),
        matches(RegExp(r'^Completed \d\d:\d\d · ran 38 min$')),
      );
      final failed = AgentSession.fromJson(
        session(status: 'failed', endReason: 'host_restart'),
      );
      expect(endedLine(failed), 'Failed · host restarted');
      expect(sessionSub(failed, 'build-vm'), 'build-vm · storm · failed');
    });

    test(
      'the agent\'s title names the session; the launch name is the fallback',
      () {
        final plain = AgentSession.fromJson({...session(), 'name': 'storm-1'});
        expect(plain.displayName, 'storm-1');
        final named = AgentSession.fromJson({
          ...session(),
          'name': 'storm-1',
          'title': 'Fix the login',
          'activity': 'working',
        });
        expect(named.displayName, 'Fix the login');
        expect(named.working, isTrue);
        expect(
          sessionSpoken(named, 'build-vm'),
          'Fix the login, Claude Code on build-vm, working',
        );
        // The stream's bare record brings the newest title and activity.
        final live = AgentSession.fromJson({
          ...session(),
          'title': 'Fix the login redirect',
          'activity': 'idle',
        });
        final merged = plain.withLive(live);
        expect(merged.displayName, 'Fix the login redirect');
        expect(merged.name, 'storm-1');
        expect(merged.working, isFalse);
        expect(
          sessionSpoken(merged, 'build-vm'),
          'Fix the login redirect, Claude Code on build-vm, running',
        );
        // A record without a title keeps the one already known.
        expect(named.withLive(plain).displayName, 'Fix the login');
        // A child is called what its parent named it.
        final child = AgentSession.fromJson({
          ...session(),
          'name': 'outside-test',
          'title': 'Storm session task loop',
          'parent_session': 'ags_parent',
          'root_session': 'ags_parent',
          'depth': 1,
        });
        expect(child.displayName, 'outside-test');
        expect(child.withLive(live).displayName, 'outside-test');
      },
    );

    test('a kit script in Wrote is a file under its path, not a note', () {
      final w = SessionWrite.fromJson({
        'vault_id': 'v-kit',
        'note_id': null,
        'title': 'tool.sh',
        'path': 'scripts/tool.sh',
        'kind': 'script_created',
        'version': null,
        'at': '2026-10-03T12:00:00Z',
      });
      expect(w.note, isNull);
      expect((w.isScript, w.created), (true, true));
      expect(noteTitleOf(w.title, w.path), 'tool.sh');
      expect(noteCrumb('kit', w.path), 'kit / scripts');
    });
  });

  group('the terminal surface', () {
    test('a multibyte character split across chunks renders whole', () {
      final t = StormTerminal();
      final bytes = utf8.encode('café ✓');
      // Split inside the é and inside the ✓.
      t.write(Uint8List.fromList(bytes.sublist(0, 4)));
      t.write(Uint8List.fromList(bytes.sublist(4, 7)));
      t.write(Uint8List.fromList(bytes.sublist(7)));
      expect(t.textForTest, contains('café ✓'));
      expect(t.textForTest, isNot(contains('�')));
    });

    test('sticky Ctrl turns the next letter into its control code', () {
      final t = StormTerminal();
      final sent = <List<int>>[];
      t.onInput = (b) => sent.add(b);
      t.stickyCtrl = true;
      t.typeForTest('c');
      t.typeForTest('c');
      expect(sent, [
        [0x03],
        [0x63],
      ]);
      expect(t.stickyCtrl, isFalse, reason: 'one keystroke, then released');
    });

    group('sticky Shift', () {
      List<String> run(StormTerminal t, void Function() act) {
        final sent = <String>[];
        t.onInput = (b) => sent.add(latin1.decode(b));
        t.stickyShift = true;
        act();
        return sent;
      }

      test('Tab becomes Shift+Tab, once', () {
        final t = StormTerminal();
        expect(run(t, t.tab), ['\x1b[Z']);
        expect(t.stickyShift, isFalse);
        t.tab();
        expect(t.stickyShift, isFalse);
      });

      test('the keyboard Enter becomes a new line, not a submit', () {
        final t = StormTerminal();
        expect(run(t, () => t.typeForTest('\r')), ['\n']);
        expect(t.stickyShift, isFalse);
      });

      test('under kitty, the Enter is the exact Shift+Enter', () {
        final t = StormTerminal();
        t.write(Uint8List.fromList(utf8.encode('\x1b[>5u')));
        expect(run(t, () => t.typeForTest('\r')), ['\x1b[13;2u']);
      });

      test('a typed letter comes out upper case', () {
        final t = StormTerminal();
        expect(run(t, () => t.typeForTest('a')), ['A']);
      });

      test('arrows carry it, as xterm encodes them', () {
        final t = StormTerminal();
        expect(run(t, t.up), ['\x1b[1;2A']);
        expect(run(t, t.left), ['\x1b[1;2D']);
        final sent = <String>[];
        t.onInput = (b) => sent.add(latin1.decode(b));
        t.stickyCtrl = true;
        t.stickyShift = true;
        t.right();
        expect(sent, ['\x1b[1;6C'], reason: 'Ctrl+Shift together');
      });

      test('armed modifiers are announced, so the row can light up', () {
        final t = StormTerminal();
        var changes = 0;
        t.shiftArmed.addListener(() => changes++);
        t.stickyShift = true;
        t.typeForTest('x');
        expect(changes, 2, reason: 'armed, then released by the keystroke');
      });
    });
  });

  group('colour-scheme reports (DEC 2031)', () {
    Future<List<String>> reports(
      WidgetTester tester,
      Future<void> Function(Future<void> Function(ThemeData) pump) act,
    ) async {
      final t = StormTerminal();
      final sent = <String>[];
      t.onInput = (b) => sent.add(latin1.decode(b));
      Future<void> pump(ThemeData theme) => tester.pumpWidget(
        MaterialApp(
          theme: theme,
          home: Scaffold(body: StormTerminalView(terminal: t)),
        ),
      );
      await pump(StormTheme.dark());
      t.write(Uint8List.fromList(utf8.encode('\x1b[?2031h')));
      sent.clear();
      await act(pump);
      return sent;
    }

    testWidgets('a rebuild tells the agent nothing', (tester) async {
      // A late report lands after the agent restored echo on exit, and the
      // PTY prints it into the scrollback as ^[[?997;1n.
      expect(
        await reports(tester, (pump) async {
          await pump(StormTheme.dark());
          await pump(StormTheme.dark());
        }),
        isEmpty,
      );
    });

    testWidgets('a real theme change is still reported', (tester) async {
      expect(
        await reports(tester, (pump) async {
          await pump(StormTheme.light());
          await tester.pumpAndSettle();
        }),
        isNotEmpty,
      );
    });
  });

  group('chords the legacy encoding cannot express', () {
    // Real key events through the real view, so the hook, its gate and
    // xterm2's own encoding are all on the path.
    Future<List<String>> press(
      WidgetTester tester,
      List<LogicalKeyboardKey> modifiers,
      LogicalKeyboardKey key, {
      String? agentOutput,
    }) async {
      final t = StormTerminal();
      final sent = <String>[];
      t.onInput = (b) => sent.add(latin1.decode(b));
      if (agentOutput != null) {
        t.write(Uint8List.fromList(utf8.encode(agentOutput)));
      }
      await tester.pumpWidget(
        MaterialApp(
          home: Scaffold(body: StormTerminalView(terminal: t, autofocus: true)),
        ),
      );
      await tester.pump();
      for (final m in modifiers) {
        await tester.sendKeyDownEvent(m);
      }
      await tester.sendKeyEvent(key);
      for (final m in modifiers.reversed) {
        await tester.sendKeyUpEvent(m);
      }
      await tester.pump();
      return sent;
    }

    const shift = LogicalKeyboardKey.shiftLeft;
    const ctrl = LogicalKeyboardKey.controlLeft;
    const meta = LogicalKeyboardKey.metaLeft;
    const enter = LogicalKeyboardKey.enter;
    const backspace = LogicalKeyboardKey.backspace;
    // What Claude Code and OpenCode send when they start (measured).
    const kittyOn = '\x1b[>5u';

    testWidgets('Shift+Enter is a new line, not keypad Enter', (tester) async {
      expect(await press(tester, [shift], enter), ['\n']);
    });

    testWidgets('Ctrl+Backspace deletes a word', (tester) async {
      expect(await press(tester, [ctrl], backspace), ['\x17']);
    });

    testWidgets('Cmd+Backspace clears the line on macOS', (tester) async {
      debugDefaultTargetPlatformOverride = TargetPlatform.macOS;
      try {
        expect(await press(tester, [meta], backspace), ['\x15']);
      } finally {
        debugDefaultTargetPlatformOverride = null;
      }
    });

    testWidgets('plain Enter and Backspace are untouched', (tester) async {
      expect(await press(tester, [], enter), ['\r']);
      expect(await press(tester, [], backspace), ['\x7f']);
    });

    testWidgets('Super+Backspace off macOS is not an editing chord', (
      tester,
    ) async {
      debugDefaultTargetPlatformOverride = TargetPlatform.linux;
      try {
        expect(await press(tester, [meta], backspace), isNot(contains('\x15')));
      } finally {
        debugDefaultTargetPlatformOverride = null;
      }
    });

    testWidgets('once the agent asks for kitty, xterm2 encodes exactly', (
      tester,
    ) async {
      // xterm2 also reports the modifier keys themselves under kitty; only
      // the chord's own encoding, and the fallback's absence, are this
      // surface's business.
      final enterSent = await press(
        tester,
        [shift],
        enter,
        agentOutput: kittyOn,
      );
      expect(enterSent, contains('\x1b[13;2u'));
      expect(enterSent, isNot(contains('\n')));
      final backSent = await press(
        tester,
        [ctrl],
        backspace,
        agentOutput: kittyOn,
      );
      expect(backSent, contains('\x1b[127;5u'));
      expect(backSent, isNot(contains('\x17')));
    });

    testWidgets('popping kitty brings the fallback back', (tester) async {
      expect(
        await press(tester, [shift], enter, agentOutput: '$kittyOn\x1b[<u'),
        ['\n'],
      );
    });
  });

  group('the session controller', () {
    AgentApi api(MockClient client) =>
        AgentApi(baseUrl: 'http://s', token: 't', client: client);

    test(
      'it resumes from the last offset and never re-renders a byte',
      () async {
        final opened = <int>[];
        late StreamController<TerminalEvent> stream;
        final c = SessionController(
          api: api(MockClient((_) async => http.Response('', 204))),
          sessionId: 'ags_1',
          open: (offset) {
            opened.add(offset);
            stream = StreamController<TerminalEvent>();
            return stream.stream;
          },
        )..start();
        stream.add(StatusEvent(AgentSession.fromJson(session())));
        stream.add(OutputEvent(3, Uint8List.fromList(utf8.encode('abc'))));
        await pumpEventQueue();
        await stream.close(); // a drop, not an ending
        await Future<void>.delayed(const Duration(milliseconds: 1100));
        expect(opened, [0, 3], reason: 'reconnect from the rendered offset');
        // A replayed overlap is trimmed by offset.
        stream.add(OutputEvent(5, Uint8List.fromList(utf8.encode('cde'))));
        await pumpEventQueue();
        expect(c.terminal.textForTest.trim(), 'abcde');
        c.dispose();
      },
    );

    test('a gap at the start clears and says so', () async {
      final stream = StreamController<TerminalEvent>();
      final c = SessionController(
        api: api(MockClient((_) async => http.Response('', 204))),
        sessionId: 'ags_1',
        open: (_) => stream.stream,
      )..start();
      stream.add(GapEvent(0, 4096));
      stream.add(OutputEvent(4100, Uint8List.fromList(utf8.encode('tail'))));
      await pumpEventQueue();
      expect(c.terminal.textForTest, contains('no longer retained'));
      expect(c.terminal.textForTest, contains('tail'));
      c.dispose();
    });

    group('the terminal answers only live queries (decision 132, fix b)', () {
      ({
        SessionController c,
        StreamController<TerminalEvent> stream,
        List<String> sent,
      })
      open({bool foreground = true}) {
        final sent = <String>[];
        final stream = StreamController<TerminalEvent>();
        final c = SessionController(
          api: api(
            MockClient((req) async {
              if (req.url.path.endsWith('/terminal/input')) {
                sent.add(latin1.decode(req.bodyBytes));
              }
              return http.Response('', 204);
            }),
          ),
          sessionId: 'ags_1',
          open: (_) => stream.stream,
          isForeground: () => foreground,
        )..start();
        stream.add(StatusEvent(AgentSession.fromJson(session())));
        return (c: c, stream: stream, sent: sent);
      }

      // Device attributes: xterm2 answers `CSI ? … c`.
      final query = Uint8List.fromList(utf8.encode('\x1b[c'));

      test('a query replayed from history is not answered again', () async {
        final o = open();
        o.stream.add(OutputEvent(3, query));
        o.stream.add(LiveEvent());
        await Future<void>.delayed(const Duration(milliseconds: 100));
        expect(o.sent, isEmpty);

        o.stream.add(OutputEvent(6, query));
        await Future<void>.delayed(const Duration(milliseconds: 100));
        expect(o.sent, hasLength(1));
        expect(o.sent.single, startsWith('\x1b[?'));
        o.c.dispose();
      });

      test('a device in the background never answers', () async {
        final o = open(foreground: false);
        o.stream.add(LiveEvent());
        o.stream.add(OutputEvent(3, query));
        await Future<void>.delayed(const Duration(milliseconds: 100));
        expect(o.sent, isEmpty);
        o.c.dispose();
      });

      test('against an older server, a quiet stream counts as live', () async {
        final o = open();
        o.stream.add(OutputEvent(3, query));
        await Future<void>.delayed(
          SessionController.liveAfterQuiet + const Duration(milliseconds: 100),
        );
        expect(o.sent, isEmpty, reason: 'it was replay');
        o.stream.add(OutputEvent(6, query));
        await Future<void>.delayed(const Duration(milliseconds: 100));
        expect(o.sent, hasLength(1));
        o.c.dispose();
      });

      test('a keystroke is still sent at once', () async {
        final o = open();
        o.c.terminal.typeForTest('x');
        await Future<void>.delayed(const Duration(milliseconds: 100));
        expect(o.sent, ['x']);
        o.c.dispose();
      });
    });

    group('the PTY follows the device in use (freeze §11.4)', () {
      Map<String, dynamic> sized(int cols, int rows) => {
        ...session(),
        'cols': cols,
        'rows': rows,
      };

      ({
        SessionController c,
        StreamController<TerminalEvent> stream,
        List<Map<String, dynamic>> resizes,
      })
      open() {
        final resizes = <Map<String, dynamic>>[];
        final stream = StreamController<TerminalEvent>();
        final c = SessionController(
          api: api(
            MockClient((req) async {
              if (req.url.path.endsWith('/terminal/resize')) {
                resizes.add(jsonDecode(req.body) as Map<String, dynamic>);
              }
              return http.Response('', 204);
            }),
          ),
          sessionId: 'ags_1',
          open: (_) => stream.stream,
        )..start();
        return (c: c, stream: stream, resizes: resizes);
      }

      test('opening a session another device sized claims it', () async {
        final o = open();
        final cols = o.c.terminal.cols, rows = o.c.terminal.rows;
        o.stream.add(StatusEvent(AgentSession.fromJson(sized(cols, rows - 4))));
        await Future<void>.delayed(const Duration(milliseconds: 250));
        expect(o.resizes, [
          {'cols': cols, 'rows': rows, 'focus': true},
        ]);
        o.c.dispose();
      });

      test('typing claims it back, and only typing', () async {
        final o = open();
        final cols = o.c.terminal.cols, rows = o.c.terminal.rows;
        o.stream.add(StatusEvent(AgentSession.fromJson(sized(cols, rows))));
        await Future<void>.delayed(const Duration(milliseconds: 250));
        expect(o.resizes, isEmpty, reason: 'it already fits');

        // Another device resizes the PTY: watching is not using.
        o.stream.add(StatusEvent(AgentSession.fromJson(sized(120, rows - 4))));
        await Future<void>.delayed(const Duration(milliseconds: 250));
        expect(o.resizes, isEmpty, reason: 'no fight between open devices');

        o.c.terminal.typeForTest('x');
        o.c.terminal.typeForTest('y');
        await Future<void>.delayed(const Duration(milliseconds: 250));
        expect(o.resizes, [
          {'cols': cols, 'rows': rows, 'focus': true},
        ]);

        o.c.terminal.typeForTest('z');
        await Future<void>.delayed(const Duration(milliseconds: 250));
        expect(o.resizes, hasLength(1), reason: 'claimed once, then it fits');
        o.c.dispose();
      });
    });

    test(
      'input is coalesced, serial, never retried, and a 503 says why',
      () async {
        final bodies = <String>[];
        var status = 204;
        final c = SessionController(
          api: api(
            MockClient((req) async {
              bodies.add(utf8.decode(req.bodyBytes));
              return http.Response('', status);
            }),
          ),
          sessionId: 'ags_1',
          open: (_) => const Stream.empty(),
        )..start();
        c.terminal.typeForTest('l');
        c.terminal.typeForTest('s');
        await Future<void>.delayed(const Duration(milliseconds: 40));
        expect(bodies, ['ls'], reason: 'keystrokes gather into one POST');

        status = 503;
        c.terminal.typeForTest('x');
        await Future<void>.delayed(const Duration(milliseconds: 40));
        expect(bodies, ['ls', 'x'], reason: 'sent once, not retried');
        expect(c.inputError, contains('host is offline'));
        c.dispose();
      },
    );
  });

  group('screens', () {
    final hosts = [
      {
        'id': 'hst_1',
        'name': 'build-vm',
        'status': 'online',
        'egress': 'host',
        'capabilities': {
          'providers': [
            {'id': 'claude-code', 'kind': 'cli', 'available': false},
            {'id': 'shell', 'kind': 'cli', 'available': true},
          ],
          'workspaces': ['storm'],
          'max_sessions': 8,
        },
      },
    ];

    MockClient server({List<Map<String, dynamic>>? sessions}) => MockClient((
      req,
    ) async {
      final path = req.url.path;
      if (path == '/v1/agent/hosts' && req.method == 'GET') {
        return http.Response(jsonEncode(hosts), 200);
      }
      if (path == '/v1/config/agent') {
        return http.Response('{"default_provider":"claude-code"}', 200);
      }
      if (path == '/v1/agent/hosts/enrollments') {
        return http.Response(
          jsonEncode({
            'enrollment': 'storm-enroll:v1:http://s:srv_X:pk:sen_TOKEN.secret',
            'expires': '2026-10-03T12:10:00Z',
          }),
          200,
        );
      }
      if (path == '/v1/agent/sessions') {
        return http.Response(jsonEncode(sessions ?? const []), 200);
      }
      return http.Response('{"error":"nope"}', 404);
    });

    Widget app(Widget child, MockClient client) => ProviderScope(
      overrides: [
        agentApiFactoryProvider.overrideWithValue(
          () => AgentApi(baseUrl: 'http://s', token: 't', client: client),
        ),
      ],
      // Desk width: Settings pages need no router there.
      child: MaterialApp(
        theme: StormTheme.light(),
        home: Builder(
          builder: (context) => MediaQuery(
            data: MediaQuery.of(context).copyWith(size: const Size(1280, 800)),
            child: Scaffold(body: child),
          ),
        ),
      ),
    );

    setUp(() => SharedPreferences.setMockInitialValues({}));

    testWidgets('hosts show their real status, providers and network line', (
      tester,
    ) async {
      await tester.pumpWidget(app(const HostsScreen(), server()));
      await tester.pumpAndSettle();
      expect(find.text('build-vm'), findsOneWidget);
      // Freeze §12.2: the row says the host's network policy applies.
      expect(
        find.text(
          'online now · Claude Code (not installed), Shell · '
          'network: host policy\n$kRunsAsUnknown',
        ),
        findsOneWidget,
      );
    });

    group('a host’s limits (152)', () {
      Map<String, dynamic> limits({
        int children = 3,
        int tree = 6,
        int? sessions,
        int effective = 8,
      }) => {
        'max_children': children,
        'max_tree': tree,
        'max_sessions': sessions,
        'host_max_sessions': 8,
        'effective_max_sessions': effective,
        'depth': 1,
        'hard_max_children': 10,
        'hard_max_tree': 24,
      };

      (MockClient, List<Map<String, dynamic>>) limitsServer() {
        final puts = <Map<String, dynamic>>[];
        var current = limits();
        final client = MockClient((req) async {
          final path = req.url.path;
          if (path == '/v1/agent/hosts' && req.method == 'GET') {
            return http.Response(
              jsonEncode([
                {...hosts.first, 'limits': current},
              ]),
              200,
            );
          }
          if (path == '/v1/agent/hosts/hst_1/limits' && req.method == 'PUT') {
            final body = jsonDecode(req.body) as Map<String, dynamic>;
            puts.add(body);
            if ((body['max_tree'] as int) < (body['max_children'] as int)) {
              return http.Response('{"error":"tree_below_session"}', 400);
            }
            final s = body['max_sessions'] as int?;
            current = limits(
              children: body['max_children'] as int,
              tree: body['max_tree'] as int,
              sessions: s,
              effective: s == null || s > 8 ? 8 : s,
            );
            return http.Response(jsonEncode(current), 200);
          }
          if (path == '/v1/config/agent') {
            return http.Response('{"default_provider":"claude-code"}', 200);
          }
          return http.Response('{"error":"nope"}', 404);
        });
        return (client, puts);
      }

      testWidgets('the row says them; the section shows the defaults and '
          'depth read-only', (tester) async {
        final (client, _) = limitsServer();
        await tester.pumpWidget(app(const HostsScreen(), client));
        await tester.pumpAndSettle();
        expect(
          find.textContaining(
            'Limits: 3 agents per session · 6 per tree · 8 sessions at once',
          ),
          findsOneWidget,
        );
        await tester.tap(find.byKey(const Key('limits-host-hst_1')));
        await tester.pumpAndSettle();
        expect(find.byKey(const Key('host-limits')), findsOneWidget);
        expect(
          tester.widget<Text>(find.byKey(const Key('limit-children'))).data,
          '3',
        );
        expect(
          tester.widget<Text>(find.byKey(const Key('limit-tree'))).data,
          '6',
        );
        expect(find.text('The host’s own (8)'), findsOneWidget);
        expect(
          tester.widget<Text>(find.byKey(const Key('limit-depth'))).data,
          'Depth: 1 level (agents can’t start their own agents yet)',
        );
        expect(
          find.descendant(
            of: find.byKey(const Key('host-limits')),
            matching: find.byType(TextField),
          ),
          findsOneWidget,
          reason: 'depth is not a field',
        );
      });

      testWidgets('saving sends them all and the row follows', (tester) async {
        final (client, puts) = limitsServer();
        await tester.pumpWidget(app(const HostsScreen(), client));
        await tester.pumpAndSettle();
        await tester.tap(find.byKey(const Key('limits-host-hst_1')));
        await tester.pumpAndSettle();
        await tester.tap(find.byKey(const Key('limit-children-less')));
        await tester.tap(find.byKey(const Key('limit-tree-more')));
        await tester.enterText(find.byKey(const Key('limit-sessions')), '4');
        await tester.tap(find.byKey(const Key('save-limits')));
        await tester.pumpAndSettle();
        expect(puts, [
          {'max_children': 2, 'max_tree': 7, 'max_sessions': 4},
        ]);
        expect(find.byKey(const Key('host-limits')), findsNothing);
        expect(
          find.textContaining(
            'Limits: 2 agents per session · 7 per tree · 4 sessions at once',
          ),
          findsOneWidget,
        );
      });

      testWidgets('a refusal is shown and the section stays open', (
        tester,
      ) async {
        final (client, puts) = limitsServer();
        await tester.pumpWidget(app(const HostsScreen(), client));
        await tester.pumpAndSettle();
        await tester.tap(find.byKey(const Key('limits-host-hst_1')));
        await tester.pumpAndSettle();
        for (var i = 0; i < 4; i++) {
          await tester.tap(find.byKey(const Key('limit-children-more')));
          await tester.pump();
        }
        await tester.tap(find.byKey(const Key('save-limits')));
        await tester.pumpAndSettle();
        expect(puts.single['max_children'], 7);
        expect(
          find.text('Agents per tree must be at least agents per session.'),
          findsOneWidget,
        );
        expect(find.byKey(const Key('host-limits')), findsOneWidget);
      });

      test('an override above the host’s own is not in force', () {
        final l = HostLimits.fromJson(limits(sessions: 20));
        expect(l.overrideIneffective, isTrue);
        expect(sessionsLimitText(l), '8 sessions at once');
        expect(
          HostLimits.fromJson(
            limits(sessions: 4, effective: 4),
          ).overrideIneffective,
          isFalse,
        );
        expect(
          AgentHost.fromJson({...hosts.first}).limits,
          isNull,
          reason: 'an older server says none',
        );
      });
    });

    testWidgets('the enroll blurb points at the sheet, not a bare command', (
      tester,
    ) async {
      await tester.pumpWidget(app(const HostsScreen(), server()));
      await tester.pumpAndSettle();
      expect(
        find.textContaining(
          'Enrolling shows a one-time string and the commands to run on the '
          'machine. Paste the string when storm-runtime enroll asks for it.',
          findRichText: true,
        ),
        findsOneWidget,
      );
      expect(
        find.textContaining('on the machine and paste it', findRichText: true),
        findsNothing,
      );
    });

    testWidgets('an enrollment string is shown once, with the command', (
      tester,
    ) async {
      await tester.pumpWidget(app(const HostsScreen(), server()));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const Key('enroll-host')));
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('enrollment-string')), findsOneWidget);
      expect(
        find.descendant(
          of: find.byType(AlertDialog),
          matching: find.text('sudo -u storm-runtime storm-runtime enroll'),
        ),
        findsOneWidget,
      );
      await tester.tap(find.text('Done'));
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('enrollment-string')), findsNothing);
    });

    // B-6: the sheet ends up in screenshots, so the string is masked until
    // asked for; Copy still copies the whole string.
    testWidgets('the enrollment string is masked until revealed', (
      tester,
    ) async {
      const full = 'storm-enroll:v1:http://s:srv_X:pk:sen_TOKEN.secret';
      await tester.pumpWidget(app(const HostsScreen(), server()));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const Key('enroll-host')));
      await tester.pumpAndSettle();
      expect(find.text(full), findsNothing);
      expect(find.text(maskEnrollment(full)), findsOneWidget);
      expect(maskEnrollment(full), isNot(contains('TOKEN')));
      await tester.tap(find.byKey(const Key('reveal-enrollment')));
      await tester.pumpAndSettle();
      expect(find.text(full), findsOneWidget);
    });

    // AM40: the machine being enrolled need not be this client, so the sheet
    // offers both platforms, and each shows only its own commands.
    Finder inDialog(Finder f) =>
        find.descendant(of: find.byType(AlertDialog), matching: f);

    void expectOnly(HostPlatform shown) {
      for (final platform in HostPlatform.values) {
        for (final shape in EnrollShape.values) {
          for (final step in enrollSteps(platform, shape)) {
            expect(
              inDialog(find.text(step.command)),
              platform == shown ? findsWidgets : findsNothing,
              reason: '${step.command} on $shown',
            );
          }
          expect(
            find.text(enrollFootnote(platform, shape)),
            platform == shown ? findsOneWidget : findsNothing,
            reason: 'the $shape footnote for $platform, on $shown',
          );
        }
      }
    }

    testWidgets('the enroll sheet shows one platform at a time and switches', (
      tester,
    ) async {
      await tester.pumpWidget(app(const HostsScreen(), server()));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const Key('enroll-host')));
      await tester.pumpAndSettle();
      // Not a Mac (the test platform is Android): Linux first.
      expectOnly(HostPlatform.linux);
      expect(inDialog(find.textContaining('brew')), findsNothing);
      expect(inDialog(find.textContaining('_stormruntime')), findsNothing);

      await tester.tap(inDialog(find.text('macOS')));
      await tester.pumpAndSettle();
      expectOnly(HostPlatform.macos);
      expect(inDialog(find.textContaining('systemctl')), findsNothing);
      expect(inDialog(find.textContaining('apt install')), findsNothing);
      expect(
        inDialog(find.textContaining('sudo -u storm-runtime')),
        findsNothing,
      );
      // The string is the same whichever platform is shown.
      expect(find.byKey(const Key('enrollment-string')), findsOneWidget);

      await tester.tap(inDialog(find.text('Windows')));
      await tester.pumpAndSettle();
      expectOnly(HostPlatform.windows);
      expect(inDialog(find.textContaining('sudo')), findsNothing);
      expect(inDialog(find.textContaining('brew')), findsNothing);

      await tester.tap(inDialog(find.text('Linux')));
      await tester.pumpAndSettle();
      expectOnly(HostPlatform.linux);
    });

    testWidgets('on a Mac the enroll sheet opens on macOS', (tester) async {
      debugDefaultTargetPlatformOverride = TargetPlatform.macOS;
      try {
        await tester.pumpWidget(app(const HostsScreen(), server()));
        await tester.pumpAndSettle();
        await tester.tap(find.byKey(const Key('enroll-host')));
        await tester.pumpAndSettle();
        expectOnly(HostPlatform.macos);
      } finally {
        debugDefaultTargetPlatformOverride = null;
      }
    });

    testWidgets('on Windows the enroll sheet opens on Windows', (tester) async {
      debugDefaultTargetPlatformOverride = TargetPlatform.windows;
      try {
        await tester.pumpWidget(app(const HostsScreen(), server()));
        await tester.pumpAndSettle();
        await tester.tap(find.byKey(const Key('enroll-host')));
        await tester.pumpAndSettle();
        expectOnly(HostPlatform.windows);
      } finally {
        debugDefaultTargetPlatformOverride = null;
      }
    });

    // Decision 151 E5: a new host runs as you, so the sheet lists those
    // commands first and the service's second.
    testWidgets('the enroll sheet lists as you first, the service second', (
      tester,
    ) async {
      expect(EnrollShape.values, [EnrollShape.you, EnrollShape.service]);
      await tester.pumpWidget(app(const HostsScreen(), server()));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const Key('enroll-host')));
      await tester.pumpAndSettle();
      for (final platform in ['Linux', 'macOS', 'Windows']) {
        await tester.tap(inDialog(find.text(platform)));
        await tester.pumpAndSettle();
        double top(String key) => tester.getTopLeft(find.byKey(Key(key))).dy;
        expect(find.text('As you'), findsOneWidget);
        expect(find.text('As its own account'), findsOneWidget);
        expect(
          top('enroll-shape-you'),
          lessThan(top('enroll-shape-service')),
          reason: platform,
        );
        expect(
          top('enroll-footnote-you'),
          lessThan(top('enroll-shape-service')),
          reason: '$platform: every as-you line before the service heading',
        );
        expect(
          top('enroll-shape-service'),
          lessThan(top('enroll-service-command-0')),
          reason: platform,
        );
      }
    });

    test('the as-you steps need no sudo and no administrator', () {
      for (final platform in HostPlatform.values) {
        final you = enrollSteps(platform, EnrollShape.you);
        for (final step in you) {
          expect(step.command, isNot(contains('sudo')), reason: step.command);
          expect(step.command, isNot(contains('ProgramFiles')));
          expect(step.command, isNot(contains('_stormruntime')));
          expect(step.command, isNot(contains('apt ')));
        }
        expect(you.map((s) => s.command).join('\n'), contains('--user'));
      }
      expect(
        enrollSteps(HostPlatform.linux, EnrollShape.you).first.command,
        'curl -fsSL https://dewanshdt.github.io/Storm/install.sh | '
        'sh -s -- --yes --runtime --user',
      );
      expect(
        enrollSteps(HostPlatform.macos, EnrollShape.you).last.command,
        r'"$HOME/Library/Application Support/StormRuntime/bin/storm-runtime" enroll',
      );
      expect(
        enrollSteps(HostPlatform.windows, EnrollShape.you).last.command,
        r'& "$env:LOCALAPPDATA\StormRuntime\bin\storm-runtime.exe" enroll',
      );
      expect(
        enrollFootnote(HostPlatform.windows, EnrollShape.you),
        contains('not as administrator'),
      );
      expect(
        enrollFootnote(HostPlatform.macos, EnrollShape.you),
        contains('LaunchAgent'),
      );
      for (final (platform, root) in [
        (HostPlatform.linux, '~/.local/state/storm-runtime/workspaces'),
        (
          HostPlatform.macos,
          '~/Library/Application Support/StormRuntime/state/workspaces',
        ),
        (HostPlatform.windows, r'%LOCALAPPDATA%\StormRuntime\state\workspaces'),
      ]) {
        expect(enrollFootnote(platform, EnrollShape.you), contains(root));
      }
    });

    test('each platform\'s steps are its own, with one enroll step', () {
      final linux = [
        for (final shape in EnrollShape.values)
          ...enrollSteps(HostPlatform.linux, shape),
      ];
      final mac = [
        for (final shape in EnrollShape.values)
          ...enrollSteps(HostPlatform.macos, shape),
      ];
      final windows = [
        for (final shape in EnrollShape.values)
          ...enrollSteps(HostPlatform.windows, shape),
      ];
      for (final step in linux) {
        expect(step.command, isNot(contains('brew')));
        expect(step.command, isNot(contains('_stormruntime')));
      }
      for (final step in mac) {
        expect(step.command, isNot(contains('systemctl')));
        expect(step.command, isNot(contains('apt ')));
        expect(step.command, isNot(contains('-u storm-runtime')));
      }
      // AM56: PowerShell, never a POSIX command.
      for (final step in windows) {
        expect(step.command, isNot(contains('sudo')));
        expect(step.command, isNot(contains('brew')));
        expect(step.command, isNot(contains('systemctl')));
        expect(step.command, isNot(contains('/Library/')));
      }
      for (final platform in HostPlatform.values) {
        for (final shape in EnrollShape.values) {
          final steps = enrollSteps(platform, shape);
          final enroll = steps.where((s) => s.pastesString).toList();
          expect(enroll, hasLength(1), reason: '$platform $shape');
          expect(enroll.single.command, contains('storm-runtime'));
          expect(enroll.single.command, endsWith(' enroll'));
          // The enrollment string is a secret: never an argument (freeze §5.3).
          expect(enroll.single.command, isNot(contains('storm-enroll:')));
        }
      }
      final macService = enrollSteps(HostPlatform.macos, EnrollShape.service);
      expect(
        macService.last.command,
        'cd / && sudo -u _stormruntime /Library/StormRuntime/bin/storm-runtime enroll',
      );
      expect(
        enrollFootnote(HostPlatform.macos, EnrollShape.service),
        contains('dev.storm.runtime'),
      );
      // `install` puts the binary in Program Files; enroll runs from there.
      final winService = enrollSteps(HostPlatform.windows, EnrollShape.service);
      expect(
        winService.last.command,
        r'& "$env:ProgramFiles\StormRuntime\storm-runtime.exe" enroll',
      );
      expect(winService[1].command, endsWith(' install'));
      expect(
        enrollFootnote(HostPlatform.windows, EnrollShape.service),
        contains(r'NT SERVICE\StormRuntime'),
      );
    });

    // The sessions list at both widths is in agents_navigation_test.dart,
    // on the real router: since decision 78 the wide list is the sidebar's.

    testWidgets('the launcher announces the network and refuses uninstalled', (
      tester,
    ) async {
      await tester.pumpWidget(
        app(
          Scaffold(
            body: NewSessionLauncher(
              hosts: hosts.map((h) => AgentHost.fromJson(h)).toList(),
            ),
          ),
          server(),
        ),
      );
      await tester.pumpAndSettle();
      expect(
        find.textContaining('Network: inherits build-vm’s policy'),
        findsOneWidget,
      );
      // The default is not installed here, so the one that is is chosen.
      final agent = find.byKey(const Key('launcher-agent'));
      expect(
        find.descendant(of: agent, matching: find.text('Shell')),
        findsOneWidget,
      );
      await tester.tap(agent);
      await tester.pumpAndSettle();
      await tester.tap(find.text('Claude Code (not installed)'));
      await tester.pumpAndSettle();
      expect(
        find.descendant(of: agent, matching: find.text('Shell')),
        findsOneWidget,
        reason: 'not installed: never chosen',
      );
    });
  });
}
