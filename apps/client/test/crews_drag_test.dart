import 'dart:async';
import 'dart:convert';
import 'dart:math' as math;

import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:shared_preferences/shared_preferences.dart';

import 'package:storm/agent/crew_gestures.dart';
import 'package:storm/router.dart';
import 'package:storm/ui/tokens.dart';

import 'agent_fakes.dart';
import 'real_fonts.dart';
import 'shell_harness.dart';

/// Decision 138, slice 138-5: the desktop sidebar's drag and drop and
/// multi-select, and the crew menu from ··· and right-click wherever a crew
/// is shown (138n, plan §4.4, §6.3).
void main() {
  const wide = Size(1400, 900);
  const phone = Size(411, 900);

  setUpAll(loadRealFonts);
  setUp(() => SharedPreferences.setMockInitialValues({}));

  /// Gateway release: two running members and an ended one; Relay hardening
  /// with one; three loose sessions, two running and one ended.
  FakeAgentServer crewServer() => FakeAgentServer(
    hosts: [agentHost()],
    crews: [
      agentCrew('crw_1'),
      agentCrew('crw_2', name: 'Relay hardening', color: 'clay', order: 1),
    ],
    sessions: [
      agentSession(
        'ags_impl',
        name: 'gateway-impl',
        crew: ('crw_1', 0),
        role: roleRef('k_coder', 'Storm Coder'),
      ),
      agentSession('ags_rev', name: 'review-gateway', crew: ('crw_1', 1)),
      agentSession(
        'ags_notes',
        name: 'release-notes',
        status: 'completed',
        crew: ('crw_1', 2),
      ),
      agentSession('ags_r1', name: 'relay-timeouts', crew: ('crw_2', 0)),
      agentSession(
        'ags_spike',
        name: 'spike-sync',
        role: roleRef('k_res', 'Storm Researcher'),
      ),
      agentSession('ags_shell', name: 'shell-1', provider: 'shell'),
      agentSession('ags_lint', name: 'lint-fix', status: 'failed'),
    ],
  );

  Future<ProviderContainer> open(
    WidgetTester t,
    FakeAgentServer agents, {
    Size size = wide,
    String at = Routes.agents,
  }) async {
    final c = shellContainer(agentClient: agents.client);
    await pumpShell(t, c, size: size);
    c.read(routerProvider).go(at);
    await t.pumpAndSettle();
    return c;
  }

  Finder row(String id) => find.byKey(Key('session-$id'));
  Finder header(String id) => find.byKey(Key('crew-$id'));

  List<http.Request> changes(FakeAgentServer a) => [
    for (final r in a.requests)
      if (r.method != 'GET') r,
  ];

  List<http.Request> roleRequests(FakeAgentServer a) => [
    for (final r in a.requests)
      if (r.url.path.endsWith('/role')) r,
  ];

  Map<String, dynamic> body(http.Request r) =>
      jsonDecode(r.body) as Map<String, dynamic>;

  /// Drags [from] with the mouse to [to], pausing there so the target sees
  /// where it is, and drops.
  Future<void> drag(WidgetTester t, Finder from, Offset to) async {
    final g = await t.startGesture(
      t.getCenter(from),
      kind: PointerDeviceKind.mouse,
    );
    await t.pump();
    await g.moveBy(const Offset(0, 24));
    await t.pump();
    await g.moveTo(to);
    await t.pump();
    await g.moveTo(to + const Offset(1, 0));
    await t.pump();
    await g.up();
    await t.pumpAndSettle();
  }

  Offset topBand(WidgetTester t, Finder f) =>
      t.getTopLeft(f) + Offset(t.getSize(f).width / 2, 2);
  Offset bottomBand(WidgetTester t, Finder f) =>
      t.getBottomLeft(f) + Offset(t.getSize(f).width / 2, -2);

  group('drag and drop', () {
    testWidgets('onto a loose row makes a crew named "New crew", its name '
        'open with the text selected', (t) async {
      final agents = crewServer();
      final c = await open(t, agents);
      await drag(t, row('ags_shell'), t.getCenter(row('ags_spike')));

      final made = changes(agents).single;
      expect(made.method, 'POST');
      expect(made.url.path, '/v1/agent/crews');
      expect(body(made)['name'], 'New crew');
      expect(body(made)['session_ids'], ['ags_spike', 'ags_shell']);

      final field = find.byKey(const Key('crew-rename-crw_new2'));
      expect(field, findsOneWidget);
      final edit = t.widget<TextField>(field).controller!;
      expect(edit.text, 'New crew');
      expect(edit.selection.start, 0);
      expect(edit.selection.end, 'New crew'.length);

      await t.enterText(field, 'Spike pair');
      await t.testTextInput.receiveAction(TextInputAction.done);
      await t.pumpAndSettle();
      final rename = changes(agents).last;
      expect(rename.method, 'PATCH');
      expect(rename.url.path, '/v1/agent/crews/crw_new2');
      expect(body(rename)['name'], 'Spike pair');
      expect(find.byKey(const Key('crew-rename-crw_new2')), findsNothing);
      expect(find.byKey(const Key('crew-name-crw_new2')), findsOneWidget);
      await disposeShell(t, c);
    });

    testWidgets('Escape leaves "New crew" as it is', (t) async {
      final agents = crewServer();
      final c = await open(t, agents);
      await drag(t, row('ags_shell'), t.getCenter(row('ags_spike')));
      await t.sendKeyEvent(LogicalKeyboardKey.escape);
      await t.pumpAndSettle();
      expect(find.byKey(const Key('crew-rename-crw_new2')), findsNothing);
      expect(changes(agents).map((r) => r.method), ['POST']);
      await disposeShell(t, c);
    });

    testWidgets('onto a crew header joins it, last', (t) async {
      final agents = crewServer();
      final c = await open(t, agents);
      await drag(t, row('ags_spike'), t.getCenter(header('crw_2')));
      final move = changes(agents).single;
      expect(move.url.path, '/v1/agent/sessions/ags_spike/crew');
      expect(body(move), {'crew_id': 'crw_2'});
      expect(agents.session('ags_spike')!['crew'], {'id': 'crw_2', 'order': 1});
      await disposeShell(t, c);
    });

    testWidgets('onto a member joins that member\'s crew', (t) async {
      final agents = crewServer();
      final c = await open(t, agents);
      await drag(t, row('ags_r1'), t.getCenter(row('ags_rev')));
      final move = changes(agents).single;
      expect(move.url.path, '/v1/agent/sessions/ags_r1/crew');
      expect(body(move), {'crew_id': 'crw_1'});
      // Relay hardening was left empty, and the server removed it.
      expect(find.byKey(const Key('crew-crw_2')), findsNothing);
      expect(
        find.text('Relay hardening was removed: its last session left it.'),
        findsOneWidget,
      );
      await disposeShell(t, c);
    });

    testWidgets('between members reorders', (t) async {
      final agents = crewServer();
      final c = await open(t, agents);
      // Into the gap above gateway-impl: in front of it.
      await drag(t, row('ags_rev'), topBand(t, row('ags_impl')));
      expect(body(changes(agents).last), {
        'crew_id': 'crw_1',
        'before': 'ags_impl',
      });
      expect(
        t.getTopLeft(row('ags_rev')).dy,
        lessThan(t.getTopLeft(row('ags_impl')).dy),
      );

      // Into the gap below review-gateway, now first: in front of the one
      // after it.
      await drag(t, row('ags_impl'), bottomBand(t, row('ags_rev')));
      expect(body(changes(agents).last), {
        'crew_id': 'crw_1',
        'before': 'ags_notes',
      });
      await disposeShell(t, c);
    });

    testWidgets('below the last member puts it last', (t) async {
      final agents = crewServer();
      final c = await open(t, agents);
      await drag(t, row('ags_impl'), bottomBand(t, row('ags_notes')));
      final move = changes(agents).single;
      expect(body(move), {'crew_id': 'crw_1'});
      expect(agents.session('ags_impl')!['crew'], {'id': 'crw_1', 'order': 2});
      await disposeShell(t, c);
    });

    testWidgets('onto its own crew\'s member, or itself, does nothing', (
      t,
    ) async {
      final agents = crewServer();
      final c = await open(t, agents);
      await drag(t, row('ags_impl'), t.getCenter(row('ags_rev')));
      await drag(t, row('ags_impl'), t.getCenter(row('ags_impl')));
      await drag(t, row('ags_impl'), t.getCenter(header('crw_1')));
      expect(changes(agents), isEmpty);
      await disposeShell(t, c);
    });

    testWidgets('onto RUNNING or ENDED leaves the crew', (t) async {
      final agents = crewServer();
      final c = await open(t, agents);
      await drag(
        t,
        row('ags_rev'),
        t.getCenter(find.byKey(const Key('section-running'))),
      );
      expect(body(changes(agents).last), {'crew_id': null});
      expect(agents.session('ags_rev')!['crew'], isNull);

      await drag(
        t,
        row('ags_notes'),
        t.getCenter(find.byKey(const Key('section-ended'))),
      );
      expect(changes(agents).last.url.path, endsWith('/ags_notes/crew'));
      expect(body(changes(agents).last), {'crew_id': null});
      // A loose session dropped on RUNNING was already there.
      final n = changes(agents).length;
      await drag(
        t,
        row('ags_lint'),
        t.getCenter(find.byKey(const Key('section-running'))),
      );
      expect(changes(agents), hasLength(n));
      await disposeShell(t, c);
    });

    testWidgets('RUNNING is there to drop on while dragging, even with no '
        'loose session running', (t) async {
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        crews: [agentCrew('crw_1')],
        sessions: [agentSession('ags_impl', crew: ('crw_1', 0))],
      );
      final c = await open(t, agents);
      expect(find.byKey(const Key('section-running')), findsNothing);
      final g = await t.startGesture(
        t.getCenter(row('ags_impl')),
        kind: PointerDeviceKind.mouse,
      );
      await g.moveBy(const Offset(0, 30));
      await t.pump();
      expect(find.byKey(const Key('section-running')), findsOneWidget);
      await g.moveTo(t.getCenter(find.byKey(const Key('section-running'))));
      await t.pump();
      await g.moveBy(const Offset(1, 0));
      await t.pump();
      await g.up();
      await t.pumpAndSettle();
      expect(body(changes(agents).single), {'crew_id': null});
      await disposeShell(t, c);
    });

    testWidgets('between headers reorders the crews', (t) async {
      final agents = crewServer();
      final c = await open(t, agents);
      await drag(t, header('crw_2'), topBand(t, header('crw_1')));
      final order = changes(agents).single;
      expect(order.method, 'PUT');
      expect(order.url.path, '/v1/agent/crews/order');
      expect(body(order), {
        'ids': ['crw_2', 'crw_1'],
      });
      expect(
        t.getTopLeft(header('crw_2')).dy,
        lessThan(t.getTopLeft(header('crw_1')).dy),
      );

      // Back below it.
      await drag(t, header('crw_2'), bottomBand(t, header('crw_1')));
      expect(body(changes(agents).last), {
        'ids': ['crw_1', 'crw_2'],
      });
      await disposeShell(t, c);
    });

    testWidgets('a drop never sets a role', (t) async {
      final agents = crewServer();
      final c = await open(t, agents);
      await drag(t, row('ags_spike'), t.getCenter(header('crw_1')));
      await drag(t, row('ags_impl'), topBand(t, row('ags_rev')));
      await drag(
        t,
        row('ags_impl'),
        t.getCenter(find.byKey(const Key('section-running'))),
      );
      await drag(t, row('ags_shell'), t.getCenter(row('ags_impl')));
      expect(changes(agents), hasLength(4));
      expect(roleRequests(agents), isEmpty);
      expect(
        agents.session('ags_spike')!['role'],
        roleRef('k_res', 'Storm Researcher'),
      );
      expect(
        agents.session('ags_impl')!['role'],
        roleRef('k_coder', 'Storm Coder'),
      );
      expect(agents.session('ags_shell')!['role'], isNull);
      await disposeShell(t, c);
    });

    testWidgets('the ghost is the row on surface2, turned -2°, with the '
        'shadow; the row left behind dims to .3', (t) async {
      final c = await open(t, crewServer());
      final g = await t.startGesture(
        t.getCenter(row('ags_spike')),
        kind: PointerDeviceKind.mouse,
      );
      await g.moveBy(const Offset(10, 40));
      await t.pump();
      final ghost = find.byKey(const Key('drag-ghost'));
      expect(ghost, findsOneWidget);
      final tk = t.element(ghost).tokens;
      final box = t.widget<Container>(ghost).decoration! as BoxDecoration;
      expect(box.color, tk.surface2);
      expect(box.boxShadow, tk.shadow);
      expect(
        find.descendant(of: ghost, matching: find.text('spike-sync')),
        findsOneWidget,
      );
      final turn = t.widget<Transform>(
        find.ancestor(of: ghost, matching: find.byType(Transform)).first,
      );
      final angle = math.atan2(
        turn.transform.entry(1, 0),
        turn.transform.entry(0, 0),
      );
      expect(angle, closeTo(-2 * math.pi / 180, 1e-6));
      final dim = t.widget<Opacity>(
        find
            .ancestor(
              of: row('ags_spike').first,
              matching: find.byType(Opacity),
            )
            .first,
      );
      expect(dim.opacity, 0.3);
      await g.up();
      await t.pumpAndSettle();
      expect(find.byKey(const Key('drag-ghost')), findsNothing);
      await disposeShell(t, c);
    });

    testWidgets('desktop only: the phone list has nothing to drag, and its '
        'rows still scroll', (t) async {
      final agents = crewServer();
      var c = await open(t, agents, size: phone);
      expect(find.byType(CrewDraggable<SessionDrag>), findsNothing);
      expect(find.byType(CrewDraggable<CrewDrag>), findsNothing);
      expect(find.byKey(const Key('drag-ghost')), findsNothing);
      await t.drag(row('ags_spike'), const Offset(0, -200));
      await t.pumpAndSettle();
      expect(find.byKey(const Key('drag-ghost')), findsNothing);
      expect(changes(agents), isEmpty);
      await disposeShell(t, c);

      c = await open(t, crewServer());
      expect(find.byType(CrewDraggable<SessionDrag>), findsNWidgets(7));
      expect(find.byType(CrewDraggable<CrewDrag>), findsNWidgets(2));
      await disposeShell(t, c);
    });
  });

  group('multi-select', () {
    Future<void> clickWith(
      WidgetTester t,
      LogicalKeyboardKey key,
      Finder f,
    ) async {
      await t.sendKeyDownEvent(key);
      await t.tap(f);
      await t.sendKeyUpEvent(key);
      await t.pumpAndSettle();
    }

    Finder bar() => find.byKey(const Key('selection-bar'));

    testWidgets('⌘-click selects without opening; Make crew makes "New '
        'crew" of them in sidebar order and opens its name', (t) async {
      final agents = crewServer();
      final c = await open(t, agents);
      expect(bar(), findsNothing);
      await clickWith(t, LogicalKeyboardKey.metaLeft, row('ags_shell'));
      await clickWith(t, LogicalKeyboardKey.metaLeft, row('ags_spike'));
      expect(c.read(routerProvider).state.uri.path, Routes.agents);
      expect(bar(), findsOneWidget);
      expect(find.text('2 selected'), findsOneWidget);
      // Toggled off again, and back.
      await clickWith(t, LogicalKeyboardKey.metaLeft, row('ags_spike'));
      expect(find.text('1 selected'), findsOneWidget);
      await clickWith(t, LogicalKeyboardKey.controlLeft, row('ags_spike'));
      expect(find.text('2 selected'), findsOneWidget);

      await t.tap(find.byKey(const Key('selection-make-crew')));
      await t.pumpAndSettle();
      final made = changes(agents).single;
      expect(body(made)['name'], 'New crew');
      // Sidebar order: newest first.
      expect(body(made)['session_ids'], ['ags_shell', 'ags_spike']);
      expect(bar(), findsNothing);
      expect(find.byKey(const Key('crew-rename-crw_new2')), findsOneWidget);
      expect(roleRequests(agents), isEmpty);
      await disposeShell(t, c);
    });

    testWidgets('Make crew opens the name even when the bar is gone first', (
      t,
    ) async {
      final agents = crewServer()..holdCrewChanges = Completer();
      final c = await open(t, agents);
      await clickWith(t, LogicalKeyboardKey.metaLeft, row('ags_shell'));
      await clickWith(t, LogicalKeyboardKey.metaLeft, row('ags_spike'));
      await t.tap(find.byKey(const Key('selection-make-crew')));
      await t.pumpAndSettle();
      expect(bar(), findsNothing);
      agents.holdCrewChanges!.complete();
      await t.pumpAndSettle();
      expect(changes(agents), hasLength(1));
      expect(find.byKey(const Key('crew-rename-crw_new2')), findsOneWidget);
      await disposeShell(t, c);
    });

    testWidgets('Shift-click selects the range from the open session; Move '
        'to… moves each', (t) async {
      final agents = crewServer();
      final c = await open(t, agents);
      await t.tap(row('ags_shell'));
      await t.pumpAndSettle();
      expect(
        c.read(routerProvider).state.uri.path,
        Routes.agentSession('ags_shell'),
      );
      await clickWith(t, LogicalKeyboardKey.shiftLeft, row('ags_lint'));
      expect(find.text('3 selected'), findsOneWidget);

      await t.tap(find.byKey(const Key('selection-move')));
      await t.pumpAndSettle();
      await t.tap(
        find.descendant(
          of: find.byKey(const Key('selection-move-menu')),
          matching: find.byKey(const Key('move-to-crw_2')),
        ),
      );
      await t.pumpAndSettle();
      expect(
        [for (final r in changes(agents)) r.url.path],
        [
          '/v1/agent/sessions/ags_shell/crew',
          '/v1/agent/sessions/ags_spike/crew',
          '/v1/agent/sessions/ags_lint/crew',
        ],
      );
      for (final r in changes(agents)) {
        expect(body(r), {'crew_id': 'crw_2'});
      }
      expect(bar(), findsNothing);
      await disposeShell(t, c);
    });

    testWidgets('Esc clears, from the key or the bar', (t) async {
      final agents = crewServer();
      final c = await open(t, agents);
      await clickWith(t, LogicalKeyboardKey.metaLeft, row('ags_shell'));
      expect(bar(), findsOneWidget);
      await t.sendKeyEvent(LogicalKeyboardKey.escape);
      await t.pumpAndSettle();
      expect(bar(), findsNothing);

      await clickWith(t, LogicalKeyboardKey.metaLeft, row('ags_shell'));
      await t.tap(find.byKey(const Key('selection-clear')));
      await t.pumpAndSettle();
      expect(bar(), findsNothing);
      // A plain click is a plain open again.
      await t.tap(row('ags_shell'));
      await t.pumpAndSettle();
      expect(
        c.read(routerProvider).state.uri.path,
        Routes.agentSession('ags_shell'),
      );
      expect(changes(agents), isEmpty);
      await disposeShell(t, c);
    });

    test('selectWith: toggle, range and a plain click', () {
      const order = ['a', 'b', 'c', 'd'];
      var s = selectWith(
        const SessionSelection(),
        'c',
        order,
        toggle: true,
        range: false,
        current: 'a',
      );
      expect(s.ids, ['a', 'c'], reason: 'the open row joins a first ⌘-click');
      s = selectWith(s, 'a', order, toggle: true, range: false);
      expect(s.ids, ['c']);
      s = selectWith(s, 'c', order, toggle: false, range: true);
      expect(s.ids, ['a', 'b', 'c'], reason: 'from the last ⌘-clicked row');
      s = selectWith(s, 'd', order, toggle: false, range: true);
      expect(s.ids, ['a', 'b', 'c', 'd']);
      s = selectWith(s, 'b', order, toggle: false, range: false);
      expect(s.ids, ['b']);
    });
  });

  group('the crew menu from ··· and right-click, everywhere (138n, Q4)', () {
    const items = [
      'Rename',
      'Colour',
      'New session in crew',
      'Dismiss ended',
      'End all running…',
    ];

    void expectMenu(WidgetTester t) {
      final menu = find.byKey(const Key('crew-menu'));
      expect(menu, findsOneWidget);
      for (final label in items) {
        expect(
          find.descendant(of: menu, matching: find.text(label)),
          findsOneWidget,
          reason: label,
        );
      }
    }

    Future<void> dismiss(WidgetTester t) async {
      await t.tapAt(const Offset(1390, 890));
      await t.pumpAndSettle();
      expect(find.byKey(const Key('crew-menu')), findsNothing);
    }

    testWidgets('the sidebar header', (t) async {
      final c = await open(t, crewServer());
      await t.tap(header('crw_1'), buttons: kSecondaryButton);
      await t.pumpAndSettle();
      expectMenu(t);
      await dismiss(t);
      final mouse = await t.createGesture(kind: PointerDeviceKind.mouse);
      await mouse.addPointer(location: Offset.zero);
      addTearDown(mouse.removePointer);
      await mouse.moveTo(t.getCenter(header('crw_1')));
      await t.pumpAndSettle();
      await t.tap(find.byKey(const Key('crew-menu-crw_1')));
      await t.pumpAndSettle();
      expectMenu(t);
      await dismiss(t);
      await disposeShell(t, c);
    });

    testWidgets('the overview card', (t) async {
      final c = await open(t, crewServer());
      await t.tap(
        find.byKey(const Key('crew-card-header-crw_1')),
        buttons: kSecondaryButton,
      );
      await t.pumpAndSettle();
      expectMenu(t);
      await dismiss(t);
      await t.tap(find.byKey(const Key('crew-card-menu-crw_1')));
      await t.pumpAndSettle();
      expectMenu(t);
      await dismiss(t);
      await disposeShell(t, c);
    });

    testWidgets('the crew page', (t) async {
      final c = await open(t, crewServer(), at: Routes.agentCrew('crw_1'));
      await t.tap(
        find.byKey(const Key('crew-page-name')),
        buttons: kSecondaryButton,
      );
      await t.pumpAndSettle();
      expectMenu(t);
      await dismiss(t);
      await t.tap(find.byKey(const Key('crew-page-menu')));
      await t.pumpAndSettle();
      expectMenu(t);
      await dismiss(t);
      await disposeShell(t, c);
    });
  });
}
