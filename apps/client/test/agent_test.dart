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
      expect(sessionSub(failed), 'storm · Claude Code · failed');
    });

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
          'network: host policy',
        ),
        findsOneWidget,
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
          matching: find.textContaining('storm-runtime enroll'),
        ),
        findsOneWidget,
      );
      await tester.tap(find.text('Done'));
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('enrollment-string')), findsNothing);
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
