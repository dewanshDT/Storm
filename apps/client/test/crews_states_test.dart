import 'dart:async';
import 'dart:convert';

import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'package:storm/agent/agent_models.dart';
import 'package:storm/agent/agent_state.dart';
import 'package:storm/agent/agents_shell.dart' show AgentsSidebar;
import 'package:storm/agent/crew_changes.dart';
import 'package:storm/router.dart';
import 'package:storm/ui/states.dart' show SkeletonRows;

import 'agent_fakes.dart';
import 'real_fonts.dart';
import 'shell_harness.dart';

/// Decision 138, slice 138-6: a change shows at once and a refused one snaps
/// back (138j), Undo for a removed crew (138c), the loading and no-crews
/// states, and every crews surface on both sides of 900.
void main() {
  const wide = Size(1400, 900);
  const phone = Size(411, 900);

  setUpAll(loadRealFonts);
  setUp(() => SharedPreferences.setMockInitialValues({}));

  FakeAgentServer crewServer() => FakeAgentServer(
    hosts: [agentHost()],
    crews: [
      agentCrew('crw_1'),
      agentCrew('crw_2', name: 'Docs', color: 'clay', order: 1),
      agentCrew('crw_3', name: 'Relay hardening', color: 'sky', order: 2),
    ],
    roles: [kitRole('k_rev', 'Storm Reviewer')],
    sessions: [
      agentSession('ags_impl', name: 'gateway-impl', crew: ('crw_1', 0)),
      agentSession('ags_rev', name: 'review-gateway', crew: ('crw_1', 1)),
      agentSession(
        'ags_notes',
        name: 'release-notes',
        status: 'completed',
        crew: ('crw_2', 0),
      ),
      agentSession('ags_r1', name: 'relay-timeouts', crew: ('crw_3', 0)),
      agentSession('ags_spike', name: 'spike-sync'),
      agentSession('ags_shell', name: 'shell-1', provider: 'shell'),
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
  double top(WidgetTester t, Finder f) => t.getTopLeft(f).dy;

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

  Future<void> rightClick(WidgetTester t, Finder f) async {
    await t.tap(f, buttons: kSecondaryButton);
    await t.pumpAndSettle();
  }

  /// Under [crew]'s header and above the next thing after its members.
  bool inCrew(WidgetTester t, String session, String crew, Finder next) =>
      top(t, row(session)) > top(t, header(crew)) &&
      top(t, row(session)) < top(t, next);

  group('a change shows at once (138j)', () {
    testWidgets('a drop moves the row before the server answers, and it '
        'stays there once it has', (t) async {
      final agents = crewServer()..holdCrewChanges = Completer();
      final c = await open(t, agents);
      await drag(t, row('ags_spike'), t.getCenter(header('crw_3')));
      // Not answered yet: the row is already in Relay hardening.
      expect(agents.session('ags_spike')!['crew'], isNotNull);
      expect(
        inCrew(
          t,
          'ags_spike',
          'crw_3',
          find.byKey(const Key('section-running')),
        ),
        isTrue,
      );
      expect(c.read(agentOverviewProvider).value!.pending, {'ags_spike'});

      agents.holdCrewChanges!.complete();
      await t.pumpAndSettle();
      expect(
        inCrew(
          t,
          'ags_spike',
          'crw_3',
          find.byKey(const Key('section-running')),
        ),
        isTrue,
      );
      expect(c.read(crewPatchesProvider), isEmpty);
      expect(agents.count('PUT', '/v1/agent/sessions/ags_spike/crew'), 1);
      await disposeShell(t, c);
    });

    testWidgets('a refused drop snaps back with an outline and the reason', (
      t,
    ) async {
      final agents = crewServer()
        ..holdCrewChanges = Completer()
        ..refuseCrewChange = (409, 'it was dismissed on Pixel 8');
      final c = await open(t, agents);
      final running = find.byKey(const Key('section-running'));
      await drag(t, row('ags_spike'), t.getCenter(header('crw_3')));
      expect(inCrew(t, 'ags_spike', 'crw_3', running), isTrue);

      agents.holdCrewChanges!.complete();
      for (var i = 0; i < 3; i++) {
        await t.pump(const Duration(milliseconds: 100));
      }
      expect(top(t, row('ags_spike')), greaterThan(top(t, running)));
      expect(find.byKey(const Key('snap-back-ags_spike')), findsOneWidget);
      expect(
        find.text("Couldn't move spike-sync: it was dismissed on Pixel 8"),
        findsOneWidget,
      );
      await t.pumpAndSettle();
      expect(find.byKey(const Key('snap-back-ags_spike')), findsNothing);
      await disposeShell(t, c);
    });

    testWidgets('a rename shows at once; a refused one puts the old name '
        'back', (t) async {
      final agents = crewServer()..holdCrewChanges = Completer();
      final c = await open(t, agents);
      await rightClick(t, header('crw_1'));
      await t.tap(find.byKey(const Key('crew-action-rename')));
      await t.pumpAndSettle();
      await t.enterText(find.byKey(const Key('prompt-field')), 'Gateway 0.14');
      await t.tap(find.byKey(const Key('prompt-ok')));
      await t.pumpAndSettle();
      Finder named(String name) =>
          find.descendant(of: header('crw_1'), matching: find.text(name));
      expect(named('Gateway 0.14'), findsOneWidget);
      agents.holdCrewChanges!.complete();
      await t.pumpAndSettle();
      expect(named('Gateway 0.14'), findsOneWidget);

      agents
        ..holdCrewChanges = Completer()
        ..refuseCrewChange = (404, 'no such crew');
      await rightClick(t, header('crw_1'));
      await t.tap(find.byKey(const Key('crew-swatch-sky')));
      await t.pumpAndSettle();
      expect(c.read(agentOverviewProvider).value!.crews!.first.color, 'sky');
      agents.holdCrewChanges!.complete();
      await t.pumpAndSettle();
      expect(c.read(agentOverviewProvider).value!.crews!.first.color, 'sage');
      expect(
        find.text("Couldn't recolour Gateway 0.14: no such crew"),
        findsOneWidget,
      );
      await disposeShell(t, c);
    });

    testWidgets('a label shows at once on the session and its row', (t) async {
      final agents = crewServer()..holdCrewChanges = Completer();
      final c = await open(t, agents, size: phone);
      await t.longPress(row('ags_spike'));
      await t.pumpAndSettle();
      await t.tap(find.byKey(const Key('sheet-role-k_rev')));
      await t.pump();
      await t.pump();
      expect(
        c.read(agentOverviewProvider).value!.byId('ags_spike')!.role?.title,
        'Storm Reviewer',
      );
      expect(
        find.descendant(
          of: find.byKey(const Key('sheet-role-k_rev')),
          matching: find.byKey(const Key('sheet-check')),
        ),
        findsOneWidget,
      );
      agents.holdCrewChanges!.complete();
      await t.pumpAndSettle();
      expect(agents.session('ags_spike')!['role']['note_id'], 'k_rev');
      await disposeShell(t, c);
    });

    test('the patches draw what the server will do', () {
      final o = AgentOverview(
        sessions: [
          AgentSession.fromJson(agentSession('a', crew: ('c1', 0))),
          AgentSession.fromJson(agentSession('b', crew: ('c2', 0))),
          AgentSession.fromJson(agentSession('d', crew: ('c2', 1))),
          AgentSession.fromJson(agentSession('e')),
        ],
        hosts: const [],
        crews: [
          Crew.fromJson(agentCrew('c1')),
          Crew.fromJson(agentCrew('c2', name: 'Two', color: 'coral', order: 1)),
          Crew.fromJson(agentCrew('c3', name: 'Empty', color: 'peach')),
        ],
      );
      // Moving a's only membership away removes c1, not the empty c3.
      final moved = movePatch('a', 'c2', before: 'd').apply(o);
      expect(moved.crews!.map((c) => c.id), ['c2', 'c3']);
      expect(moved.membersOf(moved.crews!.first).map((s) => s.id), [
        'b',
        'a',
        'd',
      ]);
      final made = newCrewPatch('New crew', ['e', 'b']).apply(o);
      expect(made.crews!.last.name, 'New crew');
      expect(made.crews!.last.color, 'sand');
      expect(made.membersOf(made.crews!.last).map((s) => s.id), ['e', 'b']);
      final ordered = crewOrderPatch(['c3', 'c1', 'c2']).apply(o);
      expect(ordered.crews!.map((c) => c.id), ['c3', 'c1', 'c2']);
      expect(rolePatch('e', null).apply(o).byId('e')!.role, isNull);
    });
  });

  group('Undo for a removed crew (138c)', () {
    testWidgets('Dismiss ended removes the crew; Undo brings it back with '
        'its name, colour and place', (t) async {
      final agents = crewServer();
      final c = await open(t, agents);
      await rightClick(t, header('crw_2'));
      await t.tap(find.byKey(const Key('crew-action-dismissEnded')));
      await t.pumpAndSettle();
      expect(agents.crew('crw_2'), isNull);
      expect(
        find.text('Docs was removed: its last session was dismissed.'),
        findsOneWidget,
      );
      await t.tap(find.text('Undo'));
      await t.pumpAndSettle();

      final crews = [...agents.crews]
        ..sort((a, b) => (a['order'] as int).compareTo(b['order'] as int));
      expect(crews.map((c) => c['name']), [
        'Gateway release',
        'Docs',
        'Relay hardening',
      ]);
      expect(crews[1]['color'], 'clay');
      final made = agents.requests.lastWhere(
        (r) => r.method == 'POST' && r.url.path == '/v1/agent/crews',
      );
      expect(jsonDecode(made.body), {
        'name': 'Docs',
        'color': 'clay',
        'session_ids': <String>[],
      });
      // And the sidebar draws it there.
      final back = crews[1]['id'] as String;
      expect(top(t, header(back)), greaterThan(top(t, header('crw_1'))));
      expect(top(t, header(back)), lessThan(top(t, header('crw_3'))));
      await disposeShell(t, c);
    });

    testWidgets('a move that empties a crew: Undo moves the session back', (
      t,
    ) async {
      final agents = crewServer();
      final c = await open(
        t,
        agents,
        at: Routes.agentSession('ags_r1', tab: 'about'),
      );
      await t.tap(find.byKey(const Key('move-to-crew')));
      await t.pumpAndSettle();
      await t.tap(find.byKey(const Key('move-to-crw_1')));
      await t.pumpAndSettle();
      expect(agents.crew('crw_3'), isNull);
      expect(
        find.text('Relay hardening was removed: its last session left it.'),
        findsOneWidget,
      );
      await t.tap(find.text('Undo'));
      await t.pumpAndSettle();
      final back = agents.crews.firstWhere(
        (x) => x['name'] == 'Relay hardening',
      );
      expect(back['color'], 'sky');
      expect((agents.session('ags_r1')!['crew'] as Map)['id'], back['id']);
      expect(
        find.descendant(
          of: find.byKey(const Key('about-crew')),
          matching: find.text('Relay hardening'),
        ),
        findsOneWidget,
      );
      await disposeShell(t, c);
    });

    testWidgets('dismissing a crew\'s last session from its own page', (
      t,
    ) async {
      final agents = crewServer();
      final c = await open(t, agents, at: Routes.agentSession('ags_notes'));
      await t.tap(find.text('Dismiss'));
      await t.pumpAndSettle();
      expect(agents.crew('crw_2'), isNull);
      await t.tap(find.text('Undo'));
      await t.pumpAndSettle();
      expect(agents.crews.where((x) => x['name'] == 'Docs'), hasLength(1));
      await disposeShell(t, c);
    });
  });

  group('loading and no crews', () {
    testWidgets('headers draw only once crews have loaded', (t) async {
      final agents = crewServer()..holdSessionList = Completer();
      final c = shellContainer(agentClient: agents.client);
      await pumpShell(t, c, size: wide);
      c.read(routerProvider).go(Routes.agents);
      await t.pump();
      await t.pump(const Duration(milliseconds: 100));
      expect(find.byKey(const Key('crews-section')), findsNothing);
      expect(header('crw_1'), findsNothing);
      expect(find.byType(SkeletonRows), findsWidgets);

      agents.holdSessionList!.complete();
      await t.pumpAndSettle();
      expect(header('crw_1'), findsOneWidget);
      expect(find.byType(SkeletonRows), findsNothing);
      await disposeShell(t, c);
    });

    testWidgets('the hint stays on the overview until the first crew', (
      t,
    ) async {
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        sessions: [
          agentSession('ags_a', name: 'spike-sync'),
          agentSession('ags_b', name: 'lint-fix'),
        ],
      );
      final c = await open(t, agents);
      final hint = find.byKey(const Key('no-crews-hint'));
      expect(hint, findsOneWidget);
      expect(
        find.textContaining('Drag one session onto another to make a crew'),
        findsOneWidget,
      );
      await drag(t, row('ags_a'), t.getCenter(row('ags_b')));
      expect(agents.crews, hasLength(1));
      expect(hint, findsNothing);
      await disposeShell(t, c);
    });

    testWidgets('no hint from a server before crews, or on a phone', (t) async {
      final old = FakeAgentServer(
        hosts: [agentHost()],
        sessions: [agentSession('ags_a')],
        crewsSupported: false,
      );
      final c = await open(t, old);
      expect(find.byKey(const Key('no-crews-hint')), findsNothing);
      await disposeShell(t, c);

      final p = await open(
        t,
        FakeAgentServer(hosts: [agentHost()], sessions: [agentSession('a')]),
        size: phone,
      );
      expect(find.byKey(const Key('no-crews-hint')), findsNothing);
      await disposeShell(t, p);
    });
  });

  group('both sides of 900 (the M12 invariant)', () {
    const at900 = Size(900, 900);
    const at899 = Size(899, 900);

    bool any(Key k) => find.byKey(k).evaluate().isNotEmpty;
    bool draggable() =>
        find.byWidgetPredicate((w) => w is Draggable).evaluate().isNotEmpty;

    testWidgets('the Agents list: sidebar, cards and drag at 900; the phone '
        'list with crews and no drag at 899', (t) async {
      final w = await open(t, crewServer(), size: at900);
      expect(any(const Key('agents-overview')), isTrue);
      expect(any(const Key('crew-card-crw_1')), isTrue);
      expect(any(const Key('no-crews-hint')), isFalse);
      expect(draggable(), isTrue);
      // The handoff's 900 frame: the sidebar at its 260 floor.
      expect(t.getSize(find.byType(AgentsSidebar)).width, 260);
      await t.longPress(row('ags_spike'));
      await t.pumpAndSettle();
      expect(any(const Key('session-crew-sheet')), isFalse);
      await disposeShell(t, w);

      final p = await open(t, crewServer(), size: at899);
      expect(any(const Key('agents-overview')), isFalse);
      expect(any(const Key('crew-card-crw_1')), isFalse);
      expect(header('crw_1'), findsOneWidget);
      expect(draggable(), isFalse);
      await t.longPress(row('ags_spike'));
      await t.pumpAndSettle();
      expect(any(const Key('session-crew-sheet')), isTrue);
      await disposeShell(t, p);
    });

    testWidgets('the crew page and End all: a banner beside the sidebar at '
        '900, a sheet on the crew screen at 899', (t) async {
      for (final (size, wideSide) in [(at900, true), (at899, false)]) {
        final c = await open(
          t,
          crewServer(),
          size: size,
          at: Routes.agentCrew('crw_1'),
        );
        expect(any(const Key('crew-page-crw_1')), isTrue);
        expect(any(const Key('agents-overview')), wideSide);
        expect(any(const Key('crew-new-session-pill')), !wideSide);
        await t.tap(find.byKey(const Key('crew-page-menu')));
        await t.pumpAndSettle();
        await t.tap(find.byKey(const Key('crew-action-endAll')));
        await t.pumpAndSettle();
        expect(any(const Key('end-all-banner')), wideSide, reason: '$size');
        expect(any(const Key('end-all-sheet')), !wideSide, reason: '$size');
        await disposeShell(t, c);
      }
    });

    testWidgets('a member\'s session: the desk chip at 900, the phone '
        'header\'s at 899', (t) async {
      for (final (size, wideSide) in [(at900, true), (at899, false)]) {
        final c = await open(
          t,
          crewServer(),
          size: size,
          at: Routes.agentSession('ags_impl'),
        );
        expect(any(const Key('session-crew-chip')), wideSide, reason: '$size');
        expect(any(const Key('phone-crew-chip')), !wideSide, reason: '$size');
        await disposeShell(t, c);
      }
    });

    testWidgets('Undo and snap-back work the same on a phone', (t) async {
      final agents = crewServer()..refuseCrewChange = (409, 'gone');
      final c = await open(t, agents, size: at899);
      await t.longPress(row('ags_spike'));
      await t.pumpAndSettle();
      await t.tap(find.byKey(const Key('sheet-crew-crw_1')));
      await t.pump();
      await t.pump(const Duration(milliseconds: 100));
      expect(find.text("Couldn't move spike-sync: gone"), findsOneWidget);
      expect(agents.session('ags_spike')!['crew'], isNull);
      await t.pumpAndSettle();
      await t.tap(find.byKey(const Key('sheet-crew-crw_2')));
      await t.pumpAndSettle();
      expect((agents.session('ags_spike')!['crew'] as Map)['id'], 'crw_2');
      await disposeShell(t, c);
    });
  });
}
