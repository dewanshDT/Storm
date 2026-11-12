import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'package:storm/router.dart';
import 'package:storm/ui/breakpoints.dart';

import 'agent_fakes.dart';
import 'shell_harness.dart';

/// Desk pages fill the pane (153 follow-up): no 680, 760 or 640 column.
/// Only a paragraph keeps a reading measure.
void main() {
  const wide = Size(1600, 900);
  setUp(() => SharedPreferences.setMockInitialValues({}));

  FakeAgentServer world() => FakeAgentServer(
    hosts: [agentHost()],
    crews: [agentCrew('crw_g', name: 'Gateway release', color: 'sage')],
    sessions: [
      agentSession('ags_a', name: 'gateway-impl', crew: ('crw_g', 0)),
      agentSession('ags_b', name: 'migrate-db'),
    ],
  );

  Future<double> widthAt(
    WidgetTester tester,
    String route,
    Key key, {
    FakeAgentServer? agents,
  }) async {
    final c = shellContainer(agentClient: (agents ?? world()).client);
    await pumpShell(tester, c, size: wide);
    c.read(routerProvider).go(route);
    await tester.pumpAndSettle();
    final w = tester.getSize(find.byKey(key)).width;
    await disposeShell(tester, c);
    return w;
  }

  testWidgets('the Agents & hosts card is wider than the old 680 page', (
    tester,
  ) async {
    final w = await widthAt(
      tester,
      Routes.settingsPage('hosts'),
      const Key('agents-grid'),
    );
    expect(w, greaterThan(1000));
  });

  testWidgets('the overview cards are wider than the old 760 column', (
    tester,
  ) async {
    final w = await widthAt(
      tester,
      Routes.agents,
      const Key('crew-card-crw_g'),
    );
    expect(w, greaterThan(1000));
  });

  testWidgets('the crew page fills the pane too', (tester) async {
    final w = await widthAt(
      tester,
      Routes.agentCrew('crw_g'),
      const Key('crew-page-members'),
    );
    expect(w, greaterThan(1000));
  });

  testWidgets('a settings intro keeps a reading measure while rows fill', (
    tester,
  ) async {
    final c = shellContainer(agentClient: world().client);
    await pumpShell(tester, c, size: const Size(2200, 1200));
    c.read(routerProvider).go(Routes.settingsPage('hosts'));
    await tester.pumpAndSettle();
    final intro = tester.getSize(
      find.text(
        'The machines agents run on, and which agent CLIs each one has.',
      ),
    );
    final grid = tester.getSize(find.byKey(const Key('agents-grid')));
    expect(intro.width, lessThan(kEditorMeasure));
    expect(grid.width, greaterThan(1500));
    await disposeShell(tester, c);
  });
}
