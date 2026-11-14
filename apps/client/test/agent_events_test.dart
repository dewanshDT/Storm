import 'dart:async';
import 'dart:convert';
import 'dart:typed_data';

import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'package:storm/agent/agent_api.dart';
import 'package:storm/agent/agent_events.dart';
import 'package:storm/agent/agent_models.dart';
import 'package:storm/agent/agent_state.dart';
import 'package:storm/agent/agent_widgets.dart' show SessionRow;
import 'package:storm/agent/session_keep_alive.dart';
import 'package:storm/agent/terminal_events.dart';
import 'package:storm/router.dart';

import 'agent_fakes.dart';
import 'shell_harness.dart';

/// Decision 154, slice 154-3: lists follow `GET /v1/agent/events`.
void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  setUp(() => SharedPreferences.setMockInitialValues({}));

  AgentSession view(
    String id, {
    String name = 'gateway-spec',
    String status = 'running',
    String? activity,
    String? attention,
    int wroteCount = 0,
    int? outputEnd,
  }) => AgentSession.fromJson({
    ...agentSession(
      id,
      name: name,
      status: status,
      activity: activity,
      wroteCount: wroteCount,
    ),
    'needs_attention': ?attention,
    'output_end': outputEnd,
  });

  group('the wire', () {
    test('SSE lines decode to events; keepalives and unknown events do '
        'not', () {
      final lines = AgentEventLines();
      final snapshot = jsonEncode({
        'epoch': 'run_1',
        'sessions': [agentSession('ags_1')],
      });
      final text =
          ':\n\n'
          'event: snapshot\ndata: $snapshot\n\n'
          'event: session\ndata: ${jsonEncode(agentSession('ags_2'))}\n\n'
          'event: removed\ndata: {"id":"ags_1"}\n\n'
          'event: resync\ndata:\n\n'
          'event: later\ndata: {}\n\n';
      final out = [
        for (final l in const LineSplitter().convert(text)) ?lines.add(l),
      ];
      expect(out.map((e) => e.runtimeType), [
        AgentSnapshot,
        AgentSessionChanged,
        AgentSessionRemoved,
        AgentResync,
      ]);
      final s = out.first as AgentSnapshot;
      expect((s.epoch, s.sessions.single.id), ('run_1', 'ags_1'));
      expect((out[1] as AgentSessionChanged).session.id, 'ags_2');
      expect((out[2] as AgentSessionRemoved).id, 'ags_1');
    });

    test('a WebSocket frame is {event, data}; resync carries null', () {
      expect(
        decodeAgentFrame('{"event":"resync","data":null}'),
        isA<AgentResync>(),
      );
      final e = decodeAgentFrame(
        jsonEncode({'event': 'session', 'data': agentSession('ags_9')}),
      );
      expect((e as AgentSessionChanged).session.id, 'ags_9');
      expect(decodeAgentFrame('{"event":"session","data":{}}'), isNull);
      expect(decodeAgentFrame('not json'), isNull);
    });
  });

  group('the pushed state', () {
    late List<StreamController<AgentEvent>> opens;
    late AgentEvents events;

    setUp(() {
      opens = [];
      events = AgentEvents(
        () {
          final c = StreamController<AgentEvent>();
          opens.add(c);
          return c.stream;
        },
        retryMin: const Duration(milliseconds: 5),
      )..start(followLifecycle: false);
    });
    tearDown(() => events.dispose());

    Future<void> settle() => Future<void>.delayed(Duration.zero);

    test('a list read asked before a push draws the push; one asked after it '
        'wins', () async {
      opens.last.add(AgentSnapshot('run_1', [view('ags_1')]));
      await settle();
      final before = events.now;
      opens.last.add(
        AgentSessionChanged(view('ags_1', activity: 'working', name: 'x')),
      );
      await settle();
      final polled = [view('ags_1')];
      expect(events.merge(polled, before).single.working, isTrue);
      expect(events.merge(polled, events.now).single.working, isFalse);
    });

    test('a removal or a snapshot after the read is newer than it', () async {
      opens.last.add(AgentSnapshot('run_1', [view('ags_1'), view('ags_2')]));
      await settle();
      final before = events.now;
      opens.last
        ..add(AgentSessionRemoved('ags_1'))
        ..add(AgentSessionChanged(view('ags_3')));
      await settle();
      final polled = [view('ags_2'), view('ags_1')];
      expect(events.merge(polled, before).map((s) => s.id), ['ags_3', 'ags_2']);

      opens.last.add(AgentSnapshot('run_1', [view('ags_4')]));
      await settle();
      expect(events.merge(polled, before).map((s) => s.id), ['ags_4']);
      expect(events.resolve('ags_2', view('ags_2'), before), isNull);
    });

    test('a dropped stream reconnects and its snapshot replaces what was '
        'held; a refusal stops asking', () async {
      opens.last.add(AgentSnapshot('run_1', [view('ags_1')]));
      await settle();
      expect(events.state.value, AgentEventsState.open);
      await opens.last.close();
      await settle();
      expect(events.state.value, AgentEventsState.closed);
      await Future<void>.delayed(const Duration(milliseconds: 30));
      expect(opens, hasLength(2));
      expect(events.state.value, AgentEventsState.connecting);
      opens.last.add(AgentSnapshot('run_1', [view('ags_2')]));
      await settle();
      expect(events.sessions!.map((s) => s.id), ['ags_2']);
      expect(events.isOpen, isTrue);

      opens.last
        ..addError(AgentEventsRefused(404))
        ..close();
      await Future<void>.delayed(const Duration(milliseconds: 50));
      expect(events.state.value, AgentEventsState.unsupported);
      expect(opens, hasLength(2));
    });

    test('back in front it connects again, a refusal forgotten', () async {
      opens.last
        ..addError(AgentEventsRefused(404))
        ..close();
      await settle();
      events.background();
      events.foreground();
      expect(opens, hasLength(2));
      expect(events.state.value, AgentEventsState.connecting);
      events.background();
      await settle();
      expect(events.state.value, AgentEventsState.closed);
    });
  });

  group('kept sessions', () {
    Uint8List bytes(String s) => Uint8List.fromList(utf8.encode(s));
    Future<void> settle([int ms = 10]) =>
        Future<void>.delayed(Duration(milliseconds: ms));

    ({
      ProviderContainer c,
      List<(String, int)> terminalOpens,
      List<StreamController<AgentEvent>> eventOpens,
    })
    harness() {
      final terminalOpens = <(String, int)>[];
      final eventOpens = <StreamController<AgentEvent>>[];
      final c = ProviderContainer(
        overrides: [
          sessionCredentialsProvider.overrideWithValue(('http://s', 't')),
          agentApiFactoryProvider.overrideWithValue(
            () => AgentApi(
              baseUrl: 'http://s',
              token: 't',
              client: MockClient((_) async => http.Response('', 204)),
            ),
          ),
          terminalStreamFactoryProvider.overrideWithValue((id, offset, _) {
            terminalOpens.add((id, offset));
            final s = StreamController<TerminalEvent>()
              ..add(StatusEvent(view(id)));
            if (offset == 0) s.add(OutputEvent(10, bytes('0123456789')));
            s.add(LiveEvent());
            return s.stream;
          }),
          sessionKeepAliveBudgetProvider.overrideWithValue(
            const KeepAliveBudget(
              hidden: 3,
              bytes: 1 << 30,
              ttl: null,
              grace: Duration.zero,
            ),
          ),
          agentEventsTransportProvider.overrideWithValue(() {
            final s = StreamController<AgentEvent>();
            eventOpens.add(s);
            return s.stream;
          }),
        ],
      );
      addTearDown(c.dispose);
      c.listen(agentEventsProvider, (_, _) {});
      return (c: c, terminalOpens: terminalOpens, eventOpens: eventOpens);
    }

    test('a restarted server\'s snapshot ending below a kept offset reopens '
        'it from 0; the same epoch at the offset does not', () async {
      final h = harness();
      h.eventOpens.last.add(
        AgentSnapshot('run_1', [view('ags_1', outputEnd: 10)]),
      );
      final v = h.c.listen(sessionControllerProvider('ags_1'), (_, _) {});
      await settle();
      v.close();
      await settle();

      h.eventOpens.last.add(
        AgentSnapshot('run_1', [view('ags_1', outputEnd: 10)]),
      );
      await settle();
      final again = h.c.listen(sessionControllerProvider('ags_1'), (_, _) {});
      await settle();
      expect(h.terminalOpens, [('ags_1', 0), ('ags_1', 10)]);
      again.close();
      await settle();

      h.eventOpens.last
        ..add(AgentResync())
        ..add(AgentSnapshot('run_2', [view('ags_1', outputEnd: 4)]));
      await settle();
      final third = h.c.listen(sessionControllerProvider('ags_1'), (_, _) {});
      await settle();
      expect(h.terminalOpens.last, ('ags_1', 0), reason: 'renumbered');
      third.close();
    });

    test('a pushed removal lets a hidden kept session go at once', () async {
      final h = harness();
      h.eventOpens.last.add(AgentSnapshot('run_1', [view('ags_1')]));
      final v = h.c.listen(sessionControllerProvider('ags_1'), (_, _) {});
      await settle();
      v.close();
      await settle();
      expect(h.c.exists(sessionControllerProvider('ags_1')), isTrue);
      h.eventOpens.last.add(AgentSessionRemoved('ags_1'));
      await settle();
      expect(h.c.exists(sessionControllerProvider('ags_1')), isFalse);
    });
  });

  group('the app', () {
    const wide = Size(1400, 900);
    const sessions = '/v1/agent/sessions';

    late List<StreamController<AgentEvent>> opens;
    Object? refuse;

    Stream<AgentEvent> open() {
      final c = StreamController<AgentEvent>();
      opens.add(c);
      if (refuse case final e?) {
        c
          ..addError(e)
          ..close();
      }
      return c.stream;
    }

    setUp(() {
      opens = [];
      refuse = null;
    });

    Future<ProviderContainer> mount(
      WidgetTester t,
      FakeAgentServer agents, {
      String at = Routes.agents,
      Stream<TerminalEvent> Function(String, int, bool)? terminal,
      bool settle = true,
    }) async {
      final c = shellContainer(
        agentClient: agents.client,
        agentEvents: open,
        terminalStream: terminal,
      );
      await pumpShell(t, c, size: wide);
      c.read(routerProvider).go(at);
      if (settle) {
        await t.pumpAndSettle();
      } else {
        for (var i = 0; i < 20; i++) {
          await t.pump(const Duration(milliseconds: 50));
        }
      }
      return c;
    }

    Finder row(String id) => find.byKey(Key('session-$id'));

    testWidgets('a snapshot, a new session and a removal are drawn without '
        'asking the server', (t) async {
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        sessions: [agentSession('ags_1', name: 'alpha')],
      );
      final c = await mount(t, agents);
      expect(agents.count('GET', sessions), 1);
      expect(row('ags_1'), findsWidgets);

      opens.last.add(
        AgentSnapshot('run_1', [
          view('ags_1', name: 'alpha'),
          view('ags_2', name: 'beta'),
        ]),
      );
      await t.pumpAndSettle();
      expect(row('ags_2'), findsWidgets);

      opens.last.add(AgentSessionChanged(view('ags_3', name: 'gamma')));
      await t.pumpAndSettle();
      expect(row('ags_3'), findsWidgets);

      opens.last.add(AgentSessionRemoved('ags_1'));
      await t.pumpAndSettle();
      expect(row('ags_1'), findsNothing);
      expect(agents.count('GET', sessions), 1);
      await disposeShell(t, c);
    });

    testWidgets('the list asks every 60 s while the stream is open and every '
        '15 s otherwise', (t) async {
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        sessions: [agentSession('ags_1')],
      );
      final c = await mount(t, agents);
      opens.last.add(AgentSnapshot('run_1', [view('ags_1')]));
      await t.pumpAndSettle();

      await t.pump(agentRefreshInterval + const Duration(seconds: 1));
      await t.pumpAndSettle();
      expect(agents.count('GET', sessions), 1);
      await t.pump(const Duration(seconds: 45));
      await t.pumpAndSettle();
      expect(agents.count('GET', sessions), 2);

      // Dropped after 15 s of the next wait: that ask is due at once.
      await t.pump(agentRefreshInterval + const Duration(seconds: 1));
      await opens.last.close();
      await t.pumpAndSettle();
      expect(agents.count('GET', sessions), 3);
      await t.pump(const Duration(seconds: 2));
      await t.pumpAndSettle();
      expect(opens, hasLength(2), reason: 'it reconnects on its backoff');
      await t.pump(agentRefreshInterval + const Duration(seconds: 1));
      await t.pumpAndSettle();
      expect(agents.count('GET', sessions), 4);
      await disposeShell(t, c);
    });

    testWidgets('a server without the stream keeps the 15 s poll and is not '
        'asked again', (t) async {
      refuse = AgentEventsRefused(404);
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        sessions: [agentSession('ags_1')],
      );
      final c = await mount(t, agents);
      for (var i = 0; i < 3; i++) {
        await t.pump(agentRefreshInterval + const Duration(seconds: 1));
        await t.pumpAndSettle();
      }
      expect(agents.count('GET', sessions), 4);
      expect(opens, hasLength(1));
      await disposeShell(t, c);
    });

    testWidgets('a dropped stream reconnects and re-snapshots', (t) async {
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        sessions: [agentSession('ags_1', name: 'alpha')],
      );
      final c = await mount(t, agents);
      opens.last.add(AgentSnapshot('run_1', [view('ags_1', name: 'alpha')]));
      await t.pumpAndSettle();
      await opens.last.close();
      await t.pump(const Duration(seconds: 2));
      await t.pumpAndSettle();
      expect(opens, hasLength(2));
      opens.last.add(
        AgentSnapshot('run_1', [
          view('ags_1', name: 'alpha'),
          view('ags_4', name: 'delta'),
        ]),
      );
      await t.pumpAndSettle();
      expect(row('ags_4'), findsWidgets);
      await disposeShell(t, c);
    });

    testWidgets('a resync is quiet: nothing changes until its snapshot, on '
        'the same stream, and nothing is announced', (t) async {
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        sessions: [agentSession('ags_1', name: 'alpha')],
      );
      final c = await mount(t, agents);
      opens.last.add(AgentSnapshot('run_1', [view('ags_1', name: 'alpha')]));
      await t.pumpAndSettle();
      opens.last.add(AgentResync());
      await t.pumpAndSettle();
      expect(row('ags_1'), findsWidgets);
      opens.last.add(
        AgentSnapshot('run_1', [
          view('ags_1', name: 'alpha', status: 'stopped'),
          view('ags_5', name: 'epsilon'),
        ]),
      );
      await t.pumpAndSettle();
      expect(row('ags_5'), findsWidgets);
      expect(find.byType(SnackBar), findsNothing);
      expect(opens, hasLength(1));
      await disposeShell(t, c);
    });

    testWidgets('the rail\'s needs-you count follows a pushed session', (
      t,
    ) async {
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        sessions: [agentSession('ags_1')],
      );
      final c = await mount(t, agents);
      opens.last.add(AgentSnapshot('run_1', [view('ags_1')]));
      await t.pumpAndSettle();
      expect(find.byKey(const Key('rail-needs-count')), findsNothing);

      opens.last.add(
        AgentSessionChanged(view('ags_1', attention: 'permission_prompt')),
      );
      await t.pumpAndSettle();
      expect(
        find.descendant(
          of: find.byKey(const Key('rail-needs-count')),
          matching: find.text('1'),
        ),
        findsOneWidget,
      );

      opens.last.add(AgentSessionChanged(view('ags_1')));
      await t.pumpAndSettle();
      expect(find.byKey(const Key('rail-needs-count')), findsNothing);
      await disposeShell(t, c);
    });

    testWidgets('the open session follows its pushed record, re-reading only '
        'every 30 s', (t) async {
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        sessions: [agentSession('ags_1')],
      );
      final c = await mount(t, agents, at: Routes.agentSession('ags_1'));
      const detail = '$sessions/ags_1';
      expect(agents.count('GET', detail), 1);
      opens.last.add(AgentSnapshot('run_1', [view('ags_1')]));
      await t.pumpAndSettle();
      opens.last.add(AgentSessionChanged(view('ags_1', wroteCount: 2)));
      await t.pumpAndSettle();
      expect(
        c.read(sessionDetailProvider('ags_1')).value?.wroteCount,
        2,
        reason: 'from the push',
      );
      await t.pump(sessionRefreshInterval + const Duration(seconds: 1));
      await t.pumpAndSettle();
      expect(agents.count('GET', detail), 1);
      await t.pump(const Duration(seconds: 26));
      await t.pumpAndSettle();
      expect(agents.count('GET', detail), 2);
      await disposeShell(t, c);
    });

    testWidgets('the open session\'s row is drawn from its own stream', (
      t,
    ) async {
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        sessions: [agentSession('ags_1'), agentSession('ags_2')],
      );
      final c = await mount(
        t,
        agents,
        at: Routes.agentSession('ags_1'),
        settle: false,
        terminal: (id, _, _) =>
            (StreamController<TerminalEvent>()
                  ..add(StatusEvent(view(id, activity: 'working'))))
                .stream,
      );
      SessionRow drawn(String id) => t.widget<SessionRow>(
        find.ancestor(of: row(id), matching: find.byType(SessionRow)).first,
      );
      expect(drawn('ags_1').session.working, isTrue);
      expect(drawn('ags_2').session.working, isFalse);
      await disposeShell(t, c);
    });
  });
}
