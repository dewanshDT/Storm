import 'dart:async';
import 'dart:convert';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_riverpod/legacy.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'package:storm/agent/agent_api.dart';
import 'package:storm/agent/agent_models.dart';
import 'package:storm/agent/agent_state.dart';
import 'package:storm/agent/session_controller.dart';
import 'package:storm/agent/session_keep_alive.dart';
import 'package:storm/agent/terminal_events.dart';
import 'package:storm/agent/terminal_surface.dart';
import 'package:storm/api/models.dart';
import 'package:storm/router.dart';
import 'package:storm/ui/theme.dart';

import 'agent_fakes.dart';
import 'shell_harness.dart';

/// Decision 154, slice 154-2: recently viewed sessions are kept alive in the
/// client, hidden, and resume by offset.
void main() {
  Uint8List bytes(String s) => Uint8List.fromList(utf8.encode(s));

  AgentSession record({
    String id = 'ags_1',
    String status = 'running',
    int cols = 80,
    int rows = 24,
    int? outputEnd,
  }) => AgentSession.fromJson({
    ...agentSession(id, status: status),
    'cols': cols,
    'rows': rows,
    'output_end': ?outputEnd,
  });

  AgentApi api({void Function(http.Request)? onRequest}) => AgentApi(
    baseUrl: 'http://s',
    token: 't',
    client: MockClient((req) async {
      onRequest?.call(req);
      return http.Response('', 204);
    }),
  );

  Future<void> settle([int ms = 10]) =>
      Future<void>.delayed(Duration(milliseconds: ms));

  group('a hidden controller', () {
    ({
      SessionController c,
      List<int> opened,
      List<int> cancelled,
      StreamController<TerminalEvent> Function() current,
    })
    open({
      Duration grace = const Duration(milliseconds: 50),
      void Function(http.Request)? onRequest,
    }) {
      final opened = <int>[], cancelled = <int>[];
      late StreamController<TerminalEvent> current;
      final c = SessionController(
        api: api(onRequest: onRequest),
        sessionId: 'ags_1',
        open: (offset) {
          final i = opened.length;
          opened.add(offset);
          current = StreamController<TerminalEvent>(
            onCancel: () => cancelled.add(i),
          );
          return current.stream;
        },
        hiddenGrace: () => grace,
      )..start();
      return (
        c: c,
        opened: opened,
        cancelled: cancelled,
        current: () => current,
      );
    }

    test('keeps its stream for the grace, then resumes from the kept offset '
        'with its terminal text', () async {
      final o = open();
      o.current()
        ..add(StatusEvent(record()))
        ..add(OutputEvent(5, bytes('hello')))
        ..add(LiveEvent());
      await settle();

      o.c.hide();
      await settle(20);
      expect(o.cancelled, isEmpty, reason: 'inside the grace');
      o.c.show();
      await settle(80);
      expect(o.opened, [0], reason: 'shown inside the grace: nothing reopens');
      expect(o.cancelled, isEmpty);

      o.c.hide();
      await settle(80);
      expect(o.cancelled, [0], reason: 'the grace ran out');
      expect(o.c.terminal.textForTest, contains('hello'));

      o.c.show();
      await settle();
      expect(o.opened, [0, 5], reason: 'resumed by offset, nothing replayed');
      o.current()
        ..add(StatusEvent(record()))
        ..add(OutputEvent(8, bytes(' w!')));
      await settle();
      expect(o.c.terminal.textForTest.trim(), 'hello w!');
      o.c.dispose();
    });

    test('a gap on a resumed stream starts the view again; one mid-view '
        'only says so', () async {
      final o = open(grace: Duration.zero);
      o.current()
        ..add(StatusEvent(record()))
        ..add(OutputEvent(3, bytes('old')))
        ..add(GapEvent(3, 100));
      await settle();
      expect(o.c.terminal.textForTest, contains('old'), reason: 'mid-view');
      expect(o.c.terminal.textForTest, contains('no longer retained'));

      o.c.hide();
      o.c.show();
      await settle();
      expect(o.opened, [0, 100]);
      o.current()
        ..add(StatusEvent(record()))
        ..add(GapEvent(100, 4096))
        ..add(OutputEvent(4099, bytes('new')));
      await settle();
      final text = o.c.terminal.textForTest;
      expect(text, isNot(contains('old')), reason: 'reset before the note');
      expect(text, contains('no longer retained'));
      expect(text, contains('new'));
      o.c.dispose();
    });

    test(
      'a status ending below the kept offset resets and reopens from 0',
      () async {
        final o = open();
        o.current()
          ..add(StatusEvent(record()))
          ..add(OutputEvent(10, bytes('0123456789')));
        await settle();
        o.current().add(StatusEvent(record(outputEnd: 4)));
        await settle();
        expect(o.opened, [0, 0], reason: 'the host renumbered its stream');
        expect(o.cancelled, [0]);
        expect(o.c.terminal.textForTest, isNot(contains('0123456789')));
        o.c.dispose();
      },
    );

    test('a list newer than the pause that ends below the offset reopens '
        'from 0; an older one is not news', () async {
      final o = open(grace: Duration.zero);
      o.current()
        ..add(StatusEvent(record()))
        ..add(OutputEvent(10, bytes('0123456789')));
      await settle();
      final before = DateTime.now();
      await settle();
      o.c.hide();
      o.c.checkOutputEnd(4, before);
      o.c.show();
      await settle();
      expect(o.opened, [0, 10]);

      o.c.hide();
      await settle();
      o.c.checkOutputEnd(4, DateTime.now());
      o.c.show();
      await settle();
      expect(o.opened, [0, 10, 0]);
      expect(o.c.terminal.textForTest, isNot(contains('0123456789')));
      o.c.dispose();
    });

    test('a session gone from the server is opened once and says so', () async {
      var opens = 0, told = 0;
      final c = SessionController(
        api: api(),
        sessionId: 'ags_1',
        open: (_) {
          opens++;
          return Stream.error(StormApiException(404, 'not found'));
        },
        onGone: () => told++,
      )..start();
      await settle();
      c.hide();
      c.show();
      await settle(1200);
      expect((opens, told, c.gone), (1, 1, true));
      c.dispose();
    });

    test('an ended session is kept and never reopened', () async {
      final o = open(grace: Duration.zero);
      o.current()
        ..add(StatusEvent(record(status: 'completed')))
        ..add(OutputEvent(4, bytes('done')));
      await o.current().close();
      await settle();
      o.c.hide();
      o.c.show();
      await settle(1200);
      expect(o.opened, [0]);
      expect(o.c.terminal.textForTest, contains('done'));
      o.c.dispose();
    });

    group('sends nothing for the PTY', () {
      ({
        SessionController c,
        List<Map<String, dynamic>> resizes,
        StreamController<TerminalEvent> Function() current,
      })
      sized() {
        final resizes = <Map<String, dynamic>>[];
        final o = open(
          grace: const Duration(seconds: 5),
          onRequest: (req) {
            if (req.url.path.endsWith('/terminal/resize')) {
              resizes.add(jsonDecode(req.body) as Map<String, dynamic>);
            }
          },
        );
        return (c: o.c, resizes: resizes, current: o.current);
      }

      test(
        'shown again, it claims a PTY another device sized meanwhile',
        () async {
          final o = sized();
          final cols = o.c.terminal.cols, rows = o.c.terminal.rows;
          o.current().add(StatusEvent(record(cols: cols, rows: rows)));
          await settle(250);
          expect(o.resizes, isEmpty, reason: 'it fits');

          o.c.hide();
          o.current().add(StatusEvent(record(cols: 120, rows: rows - 4)));
          await settle(250);
          expect(o.resizes, isEmpty, reason: 'watching from hiding is not use');

          o.c.show();
          await settle(250);
          expect(o.resizes, [
            {'cols': cols, 'rows': rows, 'focus': true},
          ]);
          o.c.dispose();
        },
      );

      test('hidden, a re-fit, a focus and a keystroke reach nothing', () async {
        final sent = <String>[];
        final resizes = <Map<String, dynamic>>[];
        final o = open(
          onRequest: (req) {
            if (req.url.path.endsWith('/terminal/resize')) {
              resizes.add(jsonDecode(req.body) as Map<String, dynamic>);
            } else if (req.url.path.endsWith('/terminal/input')) {
              sent.add(latin1.decode(req.bodyBytes));
            }
          },
        );
        o.current()
          ..add(StatusEvent(record()))
          ..add(LiveEvent());
        await settle();
        o.c.hide();
        o.c.terminal.onResize!(100, 30);
        o.c.focus();
        o.c.terminal.typeForTest('x');
        await settle(250);
        expect(resizes, isEmpty);
        expect(sent, isEmpty);
        o.c.dispose();
      });
    });

    testWidgets('a theme change while hidden is not reported; shown, it is', (
      tester,
    ) async {
      final sent = <String>[];
      final stream = StreamController<TerminalEvent>();
      final c = SessionController(
        api: api(
          onRequest: (req) {
            if (req.url.path.endsWith('/terminal/input')) {
              sent.add(latin1.decode(req.bodyBytes));
            }
          },
        ),
        sessionId: 'ags_1',
        open: (_) => stream.stream,
        isForeground: () => true,
        hiddenGrace: () => const Duration(seconds: 30),
      )..start();
      Future<void> pump(ThemeData theme) async {
        await tester.pumpWidget(
          MaterialApp(
            theme: theme,
            home: Scaffold(body: StormTerminalView(terminal: c.terminal)),
          ),
        );
        // Past the theme's own animation, which never settles beside a
        // blinking cursor.
        for (var i = 0; i < 6; i++) {
          await tester.pump(const Duration(milliseconds: 100));
        }
      }

      await pump(StormTheme.dark());
      stream
        ..add(StatusEvent(record()))
        ..add(OutputEvent(8, bytes('\x1b[?2031h')))
        ..add(LiveEvent());
      await tester.pump(const Duration(milliseconds: 50));
      sent.clear();

      c.hide();
      await pump(StormTheme.light());
      expect(sent, isEmpty, reason: 'a hidden terminal never answers');

      c.show();
      await pump(StormTheme.dark());
      expect(sent, ['\x1b[?997;1n']);

      await tester.pumpWidget(const SizedBox());
      c.dispose();
    });
  });

  group('the keep-alive registry', () {
    final credentials = StateProvider<(String, String)?>(
      (ref) => ('http://s', 't'),
    );

    ({
      ProviderContainer c,
      List<(String, int)> opens,
      List<String> cancelled,
      Map<String, StreamController<TerminalEvent>> streams,
    })
    harness(
      KeepAliveBudget budget, {
      Stream<TerminalEvent> Function(String id, int offset)? custom,
    }) {
      final opens = <(String, int)>[];
      final cancelled = <String>[];
      final streams = <String, StreamController<TerminalEvent>>{};
      final c = ProviderContainer(
        overrides: [
          sessionCredentialsProvider.overrideWith(
            (ref) => ref.watch(credentials),
          ),
          agentApiFactoryProvider.overrideWith((ref) {
            final creds = ref.watch(sessionCredentialsProvider);
            if (creds == null) return null;
            return () => AgentApi(
              baseUrl: creds.$1,
              token: creds.$2,
              client: MockClient((_) async => http.Response('', 204)),
            );
          }),
          terminalStreamFactoryProvider.overrideWithValue(
            custom ??
                (id, offset) {
                  opens.add((id, offset));
                  final s = StreamController<TerminalEvent>(
                    onCancel: () => cancelled.add(id),
                  );
                  streams[id] = s;
                  s.add(StatusEvent(record(id: id)));
                  final text = bytes('$id-text');
                  if (offset == 0) s.add(OutputEvent(text.length, text));
                  s.add(LiveEvent());
                  return s.stream;
                },
          ),
          sessionKeepAliveBudgetProvider.overrideWithValue(budget),
        ],
      );
      addTearDown(c.dispose);
      return (c: c, opens: opens, cancelled: cancelled, streams: streams);
    }

    ProviderSubscription<SessionController?> view(
      ProviderContainer c,
      String id,
    ) => c.listen(sessionControllerProvider(id), (_, _) {});

    bool kept(ProviderContainer c, String id) =>
        c.exists(sessionControllerProvider(id));

    const budget = KeepAliveBudget(
      hidden: 2,
      bytes: 1 << 30,
      ttl: null,
      grace: Duration(milliseconds: 60),
    );

    test('A → B → A: A is kept, its stream closes after the grace, and it '
        'reopens from its offset', () async {
      final h = harness(budget);
      final a = view(h.c, 'a');
      await settle();
      a.close();
      final b = view(h.c, 'b');
      await settle();
      expect(kept(h.c, 'a'), isTrue);
      expect(h.cancelled, isEmpty, reason: 'inside the grace');

      await settle(100);
      expect(h.cancelled, ['a']);
      b.close();
      final again = view(h.c, 'a');
      await settle();
      expect(h.opens, [('a', 0), ('b', 0), ('a', 6)]);
      expect(again.read()!.terminal.textForTest, contains('a-text'));
      again.close();
    });

    test(
      'a covered route hides it: only paused listeners are no viewer',
      () async {
        final h = harness(
          const KeepAliveBudget(
            hidden: 2,
            bytes: 1 << 30,
            ttl: null,
            grace: Duration(milliseconds: 60),
            evictOnMemoryPressure: true,
          ),
        );
        final a = view(h.c, 'a');
        await settle();
        final c = a.read()!;
        a.pause();
        expect(c.visible, isFalse);
        await settle(100);
        expect(h.cancelled, ['a'], reason: 'the grace ran out under the cover');
        expect(kept(h.c, 'a'), isTrue);

        // Let go while covered, it is held again once uncovered.
        h.c.read(sessionKeepAliveProvider).memoryPressure();
        await settle();
        expect(kept(h.c, 'a'), isTrue, reason: 'its listener is only paused');
        a.resume();
        expect(c.visible, isTrue);
        await settle();
        expect(h.opens, [('a', 0), ('a', 6)]);
        a.close();
        await settle();
        expect(kept(h.c, 'a'), isTrue);
      },
    );

    test('one more hidden than the budget lets the oldest go', () async {
      final h = harness(
        const KeepAliveBudget(
          hidden: 2,
          bytes: 1 << 30,
          ttl: null,
          grace: Duration(seconds: 5),
        ),
      );
      for (final id in ['a', 'b', 'c']) {
        final s = view(h.c, id);
        await settle();
        s.close();
      }
      await settle();
      expect(
        [
          for (final id in ['a', 'b', 'c']) kept(h.c, id),
        ],
        [false, true, true],
      );
      expect(h.cancelled, ['a'], reason: 'evicted, not waiting out its grace');
    });

    test('the byte budget lets the oldest go', () async {
      final probe = harness(budget);
      final p = view(probe.c, 'p');
      await settle();
      final each = p.read()!.estimatedBytes;
      p.close();
      expect(each, greaterThan(0));

      final h = harness(
        KeepAliveBudget(
          hidden: 5,
          bytes: each * 3 ~/ 2,
          ttl: null,
          grace: Duration.zero,
        ),
      );
      for (final id in ['a', 'b']) {
        final s = view(h.c, id);
        await settle();
        s.close();
      }
      await settle();
      expect((kept(h.c, 'a'), kept(h.c, 'b')), (false, true));
    });

    test('a terminal is estimated at its widest line capacity', () {
      expect(
        [
          for (final w in [40, 64, 80, 128, 200, 256, 300])
            StormTerminal.lineCapacity(w),
        ],
        [64, 64, 128, 128, 256, 256, 320],
      );
    });

    test('a logout or a new token lets every kept session go', () async {
      final h = harness(budget);
      for (final id in ['a', 'b']) {
        final s = view(h.c, id);
        await settle();
        s.close();
      }
      // Nothing on screen: the hidden ones' own subscriptions are paused.
      h.c.read(credentials.notifier).state = ('http://s', 't2');
      await settle();
      expect((kept(h.c, 'a'), kept(h.c, 'b')), (false, false));

      final visible = view(h.c, 'v');
      await settle();
      final before = visible.read();
      h.c.read(credentials.notifier).state = ('http://s', 't3');
      await settle();
      expect(visible.read(), isNot(same(before)), reason: 'rebuilt, new token');
      expect(h.opens.last, ('v', 0));

      h.c.read(credentials.notifier).state = null;
      await settle();
      expect(visible.read(), isNull);
      expect(h.cancelled.toSet(), {'a', 'b', 'v'});
      visible.close();
    });

    test('an ended session is kept and not reopened', () async {
      final opens = <int>[];
      final h = harness(
        budget,
        custom: (id, offset) {
          opens.add(offset);
          return Stream.fromIterable([
            StatusEvent(record(id: id, status: 'completed')),
            OutputEvent(4, bytes('done')),
          ]);
        },
      );
      final s = view(h.c, 'a');
      await settle();
      s.close();
      await settle(100);
      final again = view(h.c, 'a');
      await settle();
      expect(opens, [0]);
      expect(again.read()!.terminal.textForTest, contains('done'));
      again.close();
    });

    test(
      'a session gone from the server, or from two lists, is let go',
      () async {
        final h = harness(
          budget,
          custom: (id, offset) => id == 'gone'
              ? Stream.error(StormApiException(404, 'not found'))
              : StreamController<TerminalEvent>().stream,
        );
        final g = view(h.c, 'gone');
        await settle();
        g.close();
        final d = view(h.c, 'dismissed');
        await settle();
        d.close();
        await settle();
        expect(kept(h.c, 'gone'), isFalse);

        final keep = h.c.read(sessionKeepAliveProvider);
        keep.overviewSeen(const [], DateTime.now());
        await settle();
        expect(kept(h.c, 'dismissed'), isTrue, reason: 'one list can be wrong');
        keep.overviewSeen(const [], DateTime.now());
        await settle();
        expect(kept(h.c, 'dismissed'), isFalse);
      },
    );

    test(
      'hidden longer than the TTL, or under memory pressure, it goes',
      () async {
        final h = harness(
          const KeepAliveBudget(
            hidden: 3,
            bytes: 1 << 30,
            ttl: Duration(milliseconds: 50),
            grace: Duration.zero,
            evictOnMemoryPressure: true,
          ),
        );
        final a = view(h.c, 'a');
        await settle();
        a.close();
        await settle(100);
        expect(kept(h.c, 'a'), isFalse);

        final b = view(h.c, 'b');
        await settle();
        b.close();
        final v = view(h.c, 'v');
        await settle();
        h.c.read(sessionKeepAliveProvider).memoryPressure();
        await settle();
        expect((kept(h.c, 'b'), kept(h.c, 'v')), (false, true));
        v.close();
      },
    );

    test('in the background every stream closes; back, only the visible '
        'one reopens', () async {
      final h = harness(budget);
      final a = view(h.c, 'a');
      await settle();
      a.close();
      final v = view(h.c, 'v');
      await settle();
      final keep = h.c.read(sessionKeepAliveProvider);

      keep.background();
      await settle();
      expect(h.cancelled.toSet(), {'a', 'v'}, reason: 'the grace is cut short');
      keep.foreground();
      await settle();
      expect(h.opens, [('a', 0), ('v', 0), ('v', 6)]);
      v.close();
    });
  });

  group('switching sessions on the real shell (A-154-1, A-154-2)', () {
    setUp(() => SharedPreferences.setMockInitialValues({}));

    FakeAgentServer server() => FakeAgentServer(
      hosts: [agentHost()],
      sessions: [
        agentSession('ags_a', name: 'alpha'),
        agentSession('ags_b', name: 'bravo'),
      ],
    );

    ({
      List<(String, int)> opens,
      Stream<TerminalEvent> Function(String, int) open,
    })
    scripted() {
      final opens = <(String, int)>[];
      return (
        opens: opens,
        open: (id, offset) {
          opens.add((id, offset));
          final s = StreamController<TerminalEvent>();
          s.add(StatusEvent(record(id: id)));
          final text = bytes('$id history');
          if (offset == 0) s.add(OutputEvent(text.length, text));
          s.add(LiveEvent());
          return s.stream;
        },
      );
    }

    Future<void> go(WidgetTester tester, ProviderContainer c, String to) async {
      c.read(routerProvider).go(to);
      await tester.pump();
    }

    String screen(ProviderContainer c, String id) =>
        c.read(sessionControllerProvider(id))!.terminal.textForTest;

    for (final (name, size) in [
      ('desk', const Size(1400, 900)),
      ('phone', const Size(411, 900)),
    ]) {
      testWidgets('$name: a revisit inside the grace fetches nothing; after '
          'it, only what is new', (tester) async {
        final s = scripted();
        final c = shellContainer(
          agentClient: server().client,
          terminalStream: s.open,
          keepAlive: const KeepAliveBudget(
            hidden: 3,
            bytes: 48 << 20,
            ttl: null,
            grace: Duration(seconds: 2),
          ),
        );
        await pumpShell(tester, c, size: size);
        final phone = size.width < 900;

        await go(tester, c, '/agents/s/ags_a');
        await tester.pumpAndSettle();
        expect(screen(c, 'ags_a'), contains('ags_a history'));

        if (phone) await go(tester, c, '/agents');
        await go(tester, c, '/agents/s/ags_b');
        await tester.pumpAndSettle();
        if (phone) await go(tester, c, '/agents');
        await go(tester, c, '/agents/s/ags_a');
        expect(
          screen(c, 'ags_a'),
          contains('ags_a history'),
          reason: 'the kept screen, in the first frame',
        );
        await tester.pumpAndSettle();
        expect(s.opens, [('ags_a', 0), ('ags_b', 0)], reason: 'zero bytes');

        if (phone) await go(tester, c, '/agents');
        await go(tester, c, '/agents/s/ags_b');
        await tester.pumpAndSettle();
        await tester.pump(const Duration(seconds: 3));
        if (phone) await go(tester, c, '/agents');
        await go(tester, c, '/agents/s/ags_a');
        await tester.pumpAndSettle();
        expect(s.opens.last, ('ags_a', 'ags_a history'.length));
        expect(screen(c, 'ags_a'), contains('ags_a history'));
        await disposeShell(tester, c);
      });
    }
  });
}
