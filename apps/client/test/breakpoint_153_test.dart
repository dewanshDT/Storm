import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'package:storm/agent/agents_shell.dart' show AgentsSidebar;
import 'package:storm/router.dart';
import 'package:storm/ui/shell/activity_rail.dart' show ActivityRail;

import 'agent_fakes.dart';
import 'fake_server.dart';
import 'shell_harness.dart';

/// The one breakpoint end to end for the surfaces redesign round 2 touched
/// (153o, R2 Constraints): at 899 nothing desktop-only shows (no app bar,
/// rail, sidebar, inspector rail or grid) and the phone's own surfaces do;
/// at 900 the desktop layout shows.
void main() {
  const at899 = Size(899, 900);
  const at900 = Size(900, 900);

  setUp(() => SharedPreferences.setMockInitialValues({}));

  FakeAgentServer world() => FakeAgentServer(
    hosts: [agentHost()],
    sessions: [agentSession('ags_1', name: 'gateway-impl')],
  );

  Future<ProviderContainer> open(
    WidgetTester t,
    Size size,
    String route,
  ) async {
    final c = shellContainer(agentClient: world().client);
    await pumpShell(t, c, size: size);
    c.read(routerProvider).go(route);
    for (var i = 0; i < 20; i++) {
      await t.pump(const Duration(milliseconds: 50));
    }
    return c;
  }

  Future<void> close(WidgetTester t, ProviderContainer c) async {
    await t.pumpWidget(const SizedBox());
    c.dispose();
  }

  void desk(bool wide) {
    final m = wide ? findsOneWidget : findsNothing;
    expect(find.byKey(const Key('app-bar')), m, reason: 'app bar');
    expect(find.byType(ActivityRail), m, reason: 'activity rail');
  }

  for (final (size, wide) in [(at899, false), (at900, true)]) {
    final w = size.width.toInt();
    final m = wide ? findsOneWidget : findsNothing;
    final phone = wide ? findsNothing : findsOneWidget;

    group('at $w', () {
      testWidgets('the session view', (t) async {
        final c = await open(t, size, Routes.agentSession('ags_1'));
        desk(wide);
        expect(find.byType(AgentsSidebar), m);
        expect(find.byKey(const Key('inspector-rail')), m);
        expect(find.byKey(const Key('back-bubble')), phone);
        await close(t, c);
      });

      testWidgets('the Agents list and sidebar', (t) async {
        final c = await open(t, size, Routes.agents);
        desk(wide);
        expect(find.byType(AgentsSidebar), m);
        expect(find.byKey(const Key('new-session-pill')), phone);
        expect(find.byKey(const Key('places-bubble')), phone);
        await close(t, c);
      });

      testWidgets('the launcher', (t) async {
        final c = await open(t, size, Routes.agents);
        await t.tap(find.byKey(Key(wide ? 'new-session' : 'new-session-pill')));
        for (var i = 0; i < 20; i++) {
          await t.pump(const Duration(milliseconds: 50));
        }
        expect(find.byKey(const Key('launcher-chips')), findsOneWidget);
        // A sheet on a phone, a dialog at the desk.
        expect(find.byType(BottomSheet), phone);
        await close(t, c);
      });

      testWidgets('Trash', (t) async {
        final c = await open(t, size, Routes.trash(FakeServer.primaryVault));
        desk(wide);
        expect(find.byKey(const Key('places-bubble')), phone);
        await close(t, c);
      });

      testWidgets('Agents & hosts', (t) async {
        final c = await open(t, size, Routes.settingsPage('hosts'));
        desk(wide);
        expect(find.byKey(const Key('agents-grid')), m);
        expect(find.byKey(const Key('grid-hint')), m);
        expect(find.byKey(const Key('host-section-hst_1')), phone);
        await close(t, c);
      });
    });
  }
}
