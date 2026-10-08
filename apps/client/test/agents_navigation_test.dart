import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'package:storm/router.dart';

import 'agent_fakes.dart';
import 'shell_harness.dart';

/// Agents as an activity (Storm v2), on the real router and shell, at both
/// widths. Sessions are routes (`/agents/s/:id`, plan §6.1); there are no
/// per-device tabs.
void main() {
  const phone = Size(411, 900);
  const wide = Size(1400, 900);

  setUp(() => SharedPreferences.setMockInitialValues({}));

  String location(ProviderContainer c) => c.read(routerProvider).state.uri.path;

  double top(WidgetTester tester, Finder f) => tester.getTopLeft(f).dy;

  FakeAgentServer running() => FakeAgentServer(
    hosts: [agentHost()],
    sessions: [
      agentSession('ags_done', status: 'completed', name: 'test-sweep'),
      agentSession('ags_live', name: 'gateway-spec'),
    ],
  );

  group('live sessions are carried by the rail and the place picker', () {
    testWidgets('wide: the rail badge counts live sessions', (tester) async {
      final c = shellContainer(agentClient: running().client);
      await pumpShell(tester, c, size: wide);

      final badge = find.byKey(const Key('rail-badge'));
      expect(badge, findsOneWidget);
      expect(
        find.descendant(of: badge, matching: find.text('1')),
        findsOneWidget,
      );

      await tester.tap(find.byKey(const Key('rail-agents')));
      await tester.pumpAndSettle();
      expect(location(c), Routes.agents);
      expect(find.byKey(const Key('session-ags_live')), findsOneWidget);
      expect(find.text('WORK'), findsOneWidget, reason: 'the overview');
      expect(find.byKey(const Key('new-session-pill')), findsNothing);

      await tester.tap(find.byKey(const Key('session-ags_live')));
      await tester.pumpAndSettle();
      expect(location(c), '/agents/s/ags_live');
      expect(
        find.byKey(const Key('key-esc')),
        findsNothing,
        reason: 'a keyboard has the keys the phone row stands in for',
      );

      await tester.tap(find.byKey(const Key('rail-notes')));
      await tester.pumpAndSettle();
      expect(location(c), startsWith('/v/'));
      await disposeShell(tester, c);
    });

    testWidgets('no badge when nothing is live', (tester) async {
      final c = shellContainer(
        agentClient: FakeAgentServer(
          hosts: [agentHost()],
          sessions: [agentSession('ags_old', status: 'stopped')],
        ).client,
      );
      await pumpShell(tester, c, size: wide);
      expect(find.byKey(const Key('rail-badge')), findsNothing);
      await disposeShell(tester, c);
    });

    testWidgets(
      'phone: the picker says what is running, and a session is a route',
      (tester) async {
        final c = shellContainer(agentClient: running().client);
        await pumpShell(tester, c, size: phone);
        final notes = location(c);

        await tester.tap(find.byKey(const Key('places-bubble')));
        await tester.pumpAndSettle();
        expect(find.text('1 running'), findsOneWidget);
        await tester.tap(find.byKey(const Key('place-agents')));
        await tester.pumpAndSettle();
        expect(location(c), Routes.agents);

        await tester.tap(find.byKey(const Key('session-ags_live')));
        await tester.pumpAndSettle();
        expect(location(c), '/agents/s/ags_live');
        expect(find.byKey(const Key('chip-details')), findsOneWidget);

        // Live: the extra keys are there with the keyboard down, and Done
        // comes with the keyboard.
        expect(find.byKey(const Key('key-esc')), findsOneWidget);
        expect(find.byKey(const Key('key-shift')), findsOneWidget);
        expect(find.byKey(const Key('keys-done')), findsNothing);
        tester.view.viewInsets = const FakeViewPadding(bottom: 300);
        await tester.pumpAndSettle();
        expect(find.byKey(const Key('keys-done')), findsOneWidget);
        tester.view.resetViewInsets();
        await tester.pumpAndSettle();

        // System back: to the list, then to the last Notes location.
        expect(await tester.binding.handlePopRoute(), isTrue);
        await tester.pumpAndSettle();
        expect(location(c), Routes.agents);
        expect(find.byKey(const Key('session-ags_live')), findsOneWidget);
        expect(await tester.binding.handlePopRoute(), isTrue);
        await tester.pumpAndSettle();
        expect(location(c), notes);

        await disposeShell(tester, c);
      },
    );

    testWidgets('with no host, the steps send you to Hosts in Settings', (
      tester,
    ) async {
      final c = shellContainer(agentClient: FakeAgentServer().client);
      await pumpShell(tester, c, size: wide);
      c.read(routerProvider).go(Routes.agents);
      await tester.pumpAndSettle();
      expect(find.text('Agents run on a machine you own'), findsOneWidget);
      expect(find.byKey(const Key('new-session')), findsNothing);
      await tester.tap(find.byKey(const Key('enroll-host-step')));
      await tester.pumpAndSettle();
      expect(location(c), Routes.settingsPage('hosts'));
      await disposeShell(tester, c);
    });
  });

  group('the Agents list on a phone', () {
    testWidgets('running then ended, with New session as a pill', (
      tester,
    ) async {
      final c = shellContainer(agentClient: running().client);
      await pumpShell(tester, c, size: phone);
      c.read(routerProvider).go(Routes.agents);
      await tester.pumpAndSettle();

      expect(find.byKey(const Key('new-session-pill')), findsOneWidget);
      expect(find.byType(FloatingActionButton), findsNothing);
      expect(
        top(tester, find.text('RUNNING')),
        lessThan(top(tester, find.text('ENDED'))),
      );
      expect(find.text('storm · Claude Code · completed'), findsOneWidget);
      expect(find.byKey(const Key('places-bubble')), findsOneWidget);
      expect(find.byKey(const Key('rail-agents')), findsNothing);
      expect(find.text('WORK'), findsNothing, reason: 'no grouping on a phone');
      await disposeShell(tester, c);
    });

    testWidgets('no pill while no host is online', (tester) async {
      final c = shellContainer(
        agentClient: FakeAgentServer(
          hosts: [agentHost(status: 'offline')],
          sessions: [agentSession('ags_old', status: 'stopped')],
        ).client,
      );
      await pumpShell(tester, c, size: phone);
      c.read(routerProvider).go(Routes.agents);
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('new-session-pill')), findsNothing);
      await disposeShell(tester, c);
    });
  });

  testWidgets('Hosts are a Settings page, reached from the list', (
    tester,
  ) async {
    final c = shellContainer(
      agentClient: FakeAgentServer(hosts: [agentHost()]).client,
    );
    await pumpShell(tester, c, size: phone);
    c.read(routerProvider).go(Routes.settings);
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('settings-row-hosts')));
    await tester.pumpAndSettle();
    expect(location(c), Routes.settingsPage('hosts'));
    expect(find.text('Hosts & default agent'), findsOneWidget);
    await disposeShell(tester, c);
  });
}
