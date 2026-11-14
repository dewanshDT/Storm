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
  }) => AgentSession.fromJson({
    ...agentSession(id, name: name, status: status, activity: activity),
    'needs_attention': ?attention,
    'output_end': outputEnd,
    'pending_decisions': ?decisions,
  });

  AgentSession working(String id, {int? outputEnd = 50}) =>
      view(id, activity: 'working', outputEnd: outputEnd);

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
        view('a'),
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

    Stream<AgentEvent> open() {
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
