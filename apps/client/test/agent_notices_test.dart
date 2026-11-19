import 'dart:async';
import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'package:storm/agent/agent_events.dart';
import 'package:storm/agent/agent_models.dart';
import 'package:storm/agent/agent_notices.dart';
import 'package:storm/agent/output_seen.dart';
import 'package:storm/router.dart';

import 'agent_fakes.dart';
import 'shell_harness.dart';

/// Decision 154, slice 154-4: in-app toasts from live session changes.
void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  AgentSession view(
    String id, {
    String name = 'alpha',
    String status = 'running',
    String? activity,
    String? attention,
    int? outputEnd = 50,
    List<Map<String, dynamic>>? decisions,
    Map<String, dynamic>? program,
  }) => AgentSession.fromJson({
    ...agentSession(id, name: name, status: status, activity: activity),
    'needs_attention': ?attention,
    'output_end': outputEnd,
    'pending_decisions': ?decisions,
    'program_status': ?program,
  });

  AgentSession working(String id, {int? outputEnd = 50}) =>
      view(id, activity: 'working', outputEnd: outputEnd);

  const t1 = '2026-11-14T12:00:00Z';
  const t2 = '2026-11-14T12:05:00Z';

  /// A session whose program reports its own state (decision 155), as the
  /// server derives activity and attention from it.
  AgentSession reporting(
    String id,
    String state, {
    String name = 'alpha',
    String? kind,
    String since = t1,
    String? activity,
    int? outputEnd = 90,
  }) => view(
    id,
    name: name,
    activity: activity ?? (state == 'working' ? 'working' : 'idle'),
    attention: state == 'blocked'
        ? switch (kind) {
            'permission' => 'permission_prompt',
            'auth' => 'sign_in',
            _ => 'question',
          }
        : null,
    outputEnd: outputEnd,
    program: {'state': state, 'kind': ?kind, 'since': since},
  );

  group('which changes say something', () {
    test('entering needs-you, from any state', () {
      for (final before in [
        view('a'),
        working('a'),
        view('a', attention: 'blocked'),
      ]) {
        expect(
          noticeOf(before, view('a', attention: 'permission_prompt')),
          AgentNotice.needsYou,
        );
      }
      expect(
        noticeOf(
          view('a', attention: 'question'),
          view('a', attention: 'permission_prompt'),
        ),
        isNull,
        reason: 'already in the tier',
      );
    });

    test('a delete approval is left to its own toast (153h)', () {
      final asking = view(
        'a',
        attention: 'delete_approval',
        decisions: [
          {'kind': 'delete_request', 'id': 'adr_1'},
        ],
      );
      expect(noticeOf(view('a'), asking), isNull);
    });

    test('working to idle, completed, failed or stopped', () {
      for (final after in [
        view('a', activity: 'idle'),
        view('a', status: 'completed'),
        view('a', status: 'failed'),
        view('a', status: 'stopped'),
      ]) {
        expect(noticeOf(working('a'), after), AgentNotice.stoppedWorking);
      }
      expect(noticeOf(view('a'), view('a', status: 'completed')), isNull);
      expect(noticeOf(working('a'), working('a')), isNull);
      expect(
        noticeOf(view('a', status: 'starting'), view('a', activity: 'idle')),
        isNull,
      );
    });

    test('working to unknown says nothing (155-R1 safeguard)', () {
      expect(noticeOf(working('a'), view('a')), isNull);
      expect(
        noticeOf(reporting('a', 'working'), reporting('a', 'done').withUnknown),
        isNull,
      );
      expect(
        noticeOf(
          reporting('a', 'working'),
          reporting('a', 'error').withUnknown,
        ),
        isNull,
      );
      expect(
        noticeOf(reporting('a', 'working'), reporting('a', 'idle').withUnknown),
        isNull,
      );
    });

    test('decision 155: what the program said, in its words', () {
      final busy = reporting('a', 'working');
      expect(noticeOf(busy, reporting('a', 'done')), AgentNotice.finishedTurn);
      expect(noticeOf(busy, reporting('a', 'error')), AgentNotice.hitError);
      for (final kind in ['permission', 'question', 'auth', null]) {
        expect(
          noticeOf(busy, reporting('a', 'blocked', kind: kind)),
          AgentNotice.needsYou,
          reason: '$kind',
        );
      }
      expect(
        noticeOf(busy, reporting('a', 'idle')),
        AgentNotice.stoppedWorking,
        reason: 'Esc or Ctrl+C: idle without done',
      );
      expect(
        noticeOf(
          reporting('a', 'blocked', kind: 'permission'),
          reporting('a', 'done'),
        ),
        AgentNotice.finishedTurn,
      );
      expect(noticeOf(busy, busy), isNull);
      expect(
        noticeOf(working('a'), reporting('a', 'done')),
        AgentNotice.finishedTurn,
        reason: 'the first record after the title said working',
      );
      expect(
        noticeOf(busy, view('a', status: 'completed')),
        AgentNotice.stoppedWorking,
        reason: 'an ended session has no program status',
      );
    });

    test('a done or an error is said once per record', () {
      final done = reporting('a', 'done');
      expect(noticeOf(done, reporting('a', 'done', outputEnd: 120)), isNull);
      expect(
        noticeOf(done, reporting('a', 'done', since: t2)),
        AgentNotice.finishedTurn,
      );
      expect(
        noticeOf(reporting('a', 'error'), reporting('a', 'error')),
        isNull,
      );
      expect(
        noticeOf(done, reporting('a', 'idle')),
        isNull,
        reason: 'not working before',
      );
      expect(
        noticeOf(
          reporting('a', 'working'),
          reporting('a', 'done', activity: 'working'),
        ),
        isNull,
        reason: 'still working, whatever the record says',
      );
      expect(
        noticeKey(AgentNotice.finishedTurn, done),
        noticeKey(
          AgentNotice.finishedTurn,
          reporting('a', 'done', outputEnd: 500),
        ),
      );
      expect(
        noticeKey(AgentNotice.finishedTurn, done),
        isNot(
          noticeKey(
            AgentNotice.finishedTurn,
            reporting('a', 'done', since: t2),
          ),
        ),
      );
    });

    test('a stop is keyed by where its output ended', () {
      final at50 = noticeKey(AgentNotice.stoppedWorking, view('a'));
      expect(noticeKey(AgentNotice.stoppedWorking, view('a')), at50);
      expect(
        noticeKey(AgentNotice.stoppedWorking, view('a', outputEnd: 90)),
        isNot(at50),
      );
    });
  });

  group('the app', () {
    const wide = Size(1400, 900);
    late List<StreamController<AgentEvent>> opens;

    Stream<AgentEvent> open(AgentPresence presence) {
      final c = StreamController<AgentEvent>();
      opens.add(c);
      return c.stream;
    }

    setUp(() {
      opens = [];
      SharedPreferences.setMockInitialValues({
        SeenOutput.key: jsonEncode({'': 1, 'ags_1': 50, 'ags_2': 50}),
      });
    });

    Future<ProviderContainer> mount(
      WidgetTester t, {
      String at = Routes.agents,
      List<AgentSession>? snapshot,
    }) async {
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        sessions: [
          agentSession('ags_1', name: 'alpha'),
          agentSession('ags_2', name: 'beta'),
        ],
      );
      final c = shellContainer(agentClient: agents.client, agentEvents: open);
      await pumpShell(t, c, size: wide);
      c.read(routerProvider).go(at);
      await t.pumpAndSettle();
      opens.last.add(
        AgentSnapshot(
          'run_1',
          snapshot ?? [working('ags_1'), view('ags_2', name: 'beta')],
        ),
      );
      await t.pumpAndSettle();
      return c;
    }

    Future<void> push(WidgetTester t, AgentSession s) async {
      opens.last.add(AgentSessionChanged(s));
      await t.pumpAndSettle();
    }

    Finder toast() => find.byType(SnackBar);
    Finder dot(String id) => find.byKey(Key('unread-$id'));

    testWidgets('needs you: one toast, and Open goes to the session', (
      t,
    ) async {
      final c = await mount(t);
      expect(toast(), findsNothing, reason: 'nothing on a snapshot');
      await push(t, view('ags_2', name: 'beta', attention: 'question'));
      expect(find.text('beta needs you'), findsOneWidget);
      expect(toast(), findsOneWidget);
      await t.tap(find.byKey(const Key('notice-toast-open')));
      await t.pumpAndSettle();
      expect(c.read(routerProvider).state.uri.path, '/agents/s/ags_2');
      await disposeShell(t, c);
    });

    for (final (what, after) in [
      ('idle', view('ags_1', activity: 'idle', outputEnd: 90)),
      ('completed', view('ags_1', status: 'completed', outputEnd: 90)),
      ('failed', view('ags_1', status: 'failed', outputEnd: 90)),
      ('stopped', view('ags_1', status: 'stopped', outputEnd: 90)),
    ]) {
      testWidgets('working to $what, unread: "stopped working", and its dot '
          'agrees', (t) async {
        final c = await mount(t, snapshot: [working('ags_1', outputEnd: 90)]);
        await push(t, after);
        expect(find.text('alpha stopped working'), findsOneWidget);
        expect(find.textContaining('done'), findsNothing);
        expect(c.read(unreadSessionsProvider), contains('ags_1'));
        if (what == 'idle') expect(dot('ags_1'), findsOneWidget);
        await disposeShell(t, c);
      });
    }

    for (final (what, after, words) in [
      ('done', reporting('ags_1', 'done'), 'alpha finished its turn'),
      ('error', reporting('ags_1', 'error'), 'alpha hit an error'),
      (
        'blocked kind=auth',
        reporting('ags_1', 'blocked', kind: 'auth'),
        'alpha needs you',
      ),
      (
        'blocked kind=permission',
        reporting('ags_1', 'blocked', kind: 'permission'),
        'alpha needs you',
      ),
      ('idle (Esc)', reporting('ags_1', 'idle'), 'alpha stopped working'),
    ]) {
      testWidgets('decision 155: working to $what says "$words"', (t) async {
        final c = await mount(t, snapshot: [reporting('ags_1', 'working')]);
        await push(t, after);
        expect(find.text(words), findsOneWidget);
        expect(toast(), findsOneWidget);
        await disposeShell(t, c);
      });
    }

    testWidgets('decision 155: a turn already seen says nothing, a need '
        'still does', (t) async {
      final c = await mount(
        t,
        snapshot: [reporting('ags_1', 'working', outputEnd: 50)],
      );
      await push(t, reporting('ags_1', 'done', outputEnd: 50));
      await push(t, reporting('ags_1', 'error', since: t2, outputEnd: 50));
      expect(toast(), findsNothing);
      await push(t, reporting('ags_1', 'blocked', kind: 'question'));
      expect(find.text('alpha needs you'), findsOneWidget);
      await disposeShell(t, c);
    });

    testWidgets('decision 155: one "finished its turn" per record, by its '
        'since', (t) async {
      final c = await mount(t, snapshot: [reporting('ags_1', 'working')]);
      await push(t, reporting('ags_1', 'done'));
      expect(find.text('alpha finished its turn'), findsOneWidget);
      await push(t, reporting('ags_1', 'done', outputEnd: 95));
      await push(t, reporting('ags_1', 'working', outputEnd: 95));
      await push(t, reporting('ags_1', 'done', outputEnd: 100));
      final prefs = await SharedPreferences.getInstance();
      expect(prefs.getStringList(kNoticeToastedKey), [
        noticeKey(AgentNotice.finishedTurn, reporting('ags_1', 'done')),
      ]);
      await push(t, reporting('ags_1', 'working', outputEnd: 100));
      await push(t, reporting('ags_1', 'done', since: t2, outputEnd: 110));
      expect(prefs.getStringList(kNoticeToastedKey), hasLength(2));
      expect(find.text('alpha finished its turn'), findsOneWidget);
      await disposeShell(t, c);
    });

    testWidgets('decision 155: unknown activity never toasts', (t) async {
      final c = await mount(t, snapshot: [reporting('ags_1', 'working')]);
      await push(t, view('ags_1', outputEnd: 90));
      await push(t, reporting('ags_1', 'done').withUnknown);
      expect(toast(), findsNothing);
      await disposeShell(t, c);
    });

    testWidgets('a title-only session keeps "stopped working"', (t) async {
      final c = await mount(t, snapshot: [working('ags_1')]);
      await push(t, view('ags_1', activity: 'idle', outputEnd: 90));
      expect(find.text('alpha stopped working'), findsOneWidget);
      expect(find.textContaining('finished'), findsNothing);
      await disposeShell(t, c);
    });

    testWidgets('working to idle when already seen: no toast and no dot', (
      t,
    ) async {
      final c = await mount(t);
      await push(t, view('ags_1', activity: 'idle'));
      expect(toast(), findsNothing);
      expect(dot('ags_1'), findsNothing);
      expect(c.read(unreadSessionsProvider), isNot(contains('ags_1')));
      await disposeShell(t, c);
    });

    testWidgets('nothing for the session on screen', (t) async {
      final c = await mount(t, at: Routes.agentSession('ags_1'));
      await push(t, view('ags_1', activity: 'idle', outputEnd: 90));
      await push(t, view('ags_1', attention: 'permission_prompt'));
      expect(toast(), findsNothing);
      await disposeShell(t, c);
    });

    testWidgets('on a notes screen too', (t) async {
      final c = await mount(t, at: '/');
      await push(t, view('ags_1', activity: 'idle', outputEnd: 90));
      expect(find.text('alpha stopped working'), findsOneWidget);
      await disposeShell(t, c);
    });

    testWidgets('nothing on a snapshot after a resync, or after a '
        'reconnect', (t) async {
      final c = await mount(t);
      opens.last
        ..add(AgentResync())
        ..add(
          AgentSnapshot('run_1', [
            view('ags_1', activity: 'idle', outputEnd: 90),
            view('ags_2', name: 'beta', attention: 'question'),
          ]),
        );
      await t.pumpAndSettle();
      expect(toast(), findsNothing);

      await opens.last.close();
      await t.pump(const Duration(seconds: 2));
      await t.pumpAndSettle();
      expect(opens, hasLength(2));
      opens.last.add(
        AgentSnapshot('run_2', [
          view('ags_1', status: 'failed', outputEnd: 120),
          view('ags_2', name: 'beta', attention: 'permission_prompt'),
        ]),
      );
      await t.pumpAndSettle();
      expect(toast(), findsNothing);
      await disposeShell(t, c);
    });

    testWidgets('A-154-7: nothing after the app comes back, then exactly one '
        'per transition', (t) async {
      final c = await mount(t);
      for (final s in [
        AppLifecycleState.inactive,
        AppLifecycleState.hidden,
        AppLifecycleState.paused,
        AppLifecycleState.hidden,
        AppLifecycleState.inactive,
        AppLifecycleState.resumed,
      ]) {
        t.binding.handleAppLifecycleStateChanged(s);
        await t.pump();
      }
      await t.pumpAndSettle();
      expect(opens, hasLength(2), reason: 'a fresh stream on return');
      opens.last.add(
        AgentSnapshot('run_1', [
          view('ags_1', activity: 'idle', outputEnd: 90),
          view('ags_2', name: 'beta', attention: 'question'),
        ]),
      );
      await t.pumpAndSettle();
      expect(toast(), findsNothing);

      await push(t, working('ags_1', outputEnd: 90));
      await push(t, view('ags_1', activity: 'idle', outputEnd: 120));
      expect(find.text('alpha stopped working'), findsOneWidget);
      // The same view again, and a flap that wrote nothing, say nothing more.
      await push(t, view('ags_1', activity: 'idle', outputEnd: 120));
      await push(t, working('ags_1', outputEnd: 120));
      await push(t, view('ags_1', activity: 'idle', outputEnd: 120));
      final prefs = await SharedPreferences.getInstance();
      expect(prefs.getStringList(kNoticeToastedKey), [
        noticeKey(
          AgentNotice.stoppedWorking,
          view('ags_1', activity: 'idle', outputEnd: 120),
        ),
      ]);
      await disposeShell(t, c);
    });

    testWidgets('once per device across a restart', (t) async {
      final c = await mount(t);
      await push(t, view('ags_1', activity: 'idle', outputEnd: 90));
      expect(toast(), findsOneWidget);
      await disposeShell(t, c);

      final again = await mount(t);
      await push(t, view('ags_1', activity: 'idle', outputEnd: 90));
      expect(toast(), findsNothing, reason: 'that stop was said here');
      await disposeShell(t, again);
    });

    testWidgets('the toasted keys are capped at 200', (t) async {
      SharedPreferences.setMockInitialValues({
        SeenOutput.key: jsonEncode({'': 1, 'ags_1': 50, 'ags_2': 50}),
        kNoticeToastedKey: [for (var i = 0; i < 200; i++) 'old_$i'],
      });
      final c = await mount(t);
      await push(t, view('ags_1', activity: 'idle', outputEnd: 90));
      expect(toast(), findsOneWidget);
      final kept = (await SharedPreferences.getInstance()).getStringList(
        kNoticeToastedKey,
      )!;
      expect(kept, hasLength(200));
      expect(kept.first, 'old_1');
      await disposeShell(t, c);
    });

    testWidgets('a newer toast replaces the one showing, never stacked', (
      t,
    ) async {
      final c = await mount(t);
      await push(t, view('ags_1', activity: 'idle', outputEnd: 90));
      expect(find.text('alpha stopped working'), findsOneWidget);
      await push(t, view('ags_2', name: 'beta', attention: 'question'));
      expect(toast(), findsOneWidget);
      expect(find.text('beta needs you'), findsOneWidget);
      expect(find.text('alpha stopped working'), findsNothing);
      expect(find.byType(AlertDialog), findsNothing);
      await disposeShell(t, c);
    });

    testWidgets('a need answered elsewhere takes its toast with it', (t) async {
      final c = await mount(t);
      await push(t, view('ags_2', name: 'beta', attention: 'question'));
      expect(toast(), findsOneWidget);
      await push(t, view('ags_2', name: 'beta'));
      expect(toast(), findsNothing);
      await disposeShell(t, c);
    });

    testWidgets('opening the session takes its toast away', (t) async {
      final c = await mount(t);
      await push(t, view('ags_1', activity: 'idle', outputEnd: 90));
      expect(toast(), findsOneWidget);
      c.read(routerProvider).go(Routes.agentSession('ags_1'));
      await t.pumpAndSettle();
      expect(toast(), findsNothing);
      await disposeShell(t, c);
    });
  });
}

extension on AgentSession {
  /// This view with its activity unknown, as a host whose program's records
  /// stopped parsing posts it.
  AgentSession get withUnknown => AgentSession.fromJson({
    'id': id,
    'host_id': hostId,
    'workspace': workspace,
    'provider': provider,
    'status': status,
    'name': name,
    'activity': null,
    'output_end': outputEnd,
    if (programStatus case final p?)
      'program_status': {
        'state': p.state,
        'kind': ?p.kind,
        'since': p.since?.toIso8601String(),
      },
  });
}
