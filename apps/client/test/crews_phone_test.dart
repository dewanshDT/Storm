import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:shared_preferences/shared_preferences.dart';

import 'package:storm/router.dart';

import 'agent_fakes.dart';
import 'real_fonts.dart';
import 'shell_harness.dart';

/// Decision 138, slice 138-5: crews on a phone (handoff §3 Phone, 4j–4l):
/// the list, the long-press sheet with independent Crew and Role, the same
/// in the details sheet, the session header's chip and tag, the crew screen
/// and its End all running sheet. No drag below 900.
void main() {
  const phone = Size(411, 900);
  const wide = Size(1400, 900);

  setUpAll(loadRealFonts);
  setUp(() => SharedPreferences.setMockInitialValues({}));

  FakeAgentServer crewServer() => FakeAgentServer(
    hosts: [
      agentHost(),
      agentHost(id: 'hst_2', name: 'mac-mini', status: 'offline'),
    ],
    crews: [agentCrew('crw_1')],
    roles: [
      kitRole(
        'k_coder',
        'Storm Coder',
        summary: 'Implements one task; writes nothing in the vault.',
      ),
      kitRole(
        'k_rev',
        'Storm Reviewer',
        summary: 'Checks finished work; writes nothing.',
      ),
      kitRole('k_res', 'Storm Researcher', summary: 'Proves a spike.'),
    ],
    sessions: [
      agentSession(
        'ags_impl',
        name: 'gateway-impl',
        crew: ('crw_1', 0),
        role: roleRef('k_coder', 'Storm Coder'),
        launchRole: roleRef('k_coder', 'Storm Coder'),
      ),
      agentSession('ags_rev', name: 'review-gateway', crew: ('crw_1', 1)),
      agentSession(
        'ags_probe',
        name: 'relay-probe',
        hostId: 'hst_2',
        status: 'unknown',
        crew: ('crw_1', 2),
      ),
      agentSession(
        'ags_notes',
        name: 'release-notes',
        status: 'completed',
        crew: ('crw_1', 3),
      ),
      agentSession(
        'ags_spike',
        name: 'spike-sync',
        role: roleRef('k_res', 'Storm Researcher'),
        launchRole: roleRef('k_res', 'Storm Researcher'),
      ),
      agentSession('ags_shell', name: 'shell-1', provider: 'shell'),
      agentSession('ags_lint', name: 'lint-fix', status: 'failed'),
    ],
  );

  Future<ProviderContainer> open(
    WidgetTester t,
    FakeAgentServer agents, {
    Size size = phone,
    String at = Routes.agents,
  }) async {
    final c = shellContainer(agentClient: agents.client);
    await pumpShell(t, c, size: size);
    c.read(routerProvider).go(at);
    await t.pumpAndSettle();
    return c;
  }

  Uri where(ProviderContainer c) => c.read(routerProvider).state.uri;
  double top(WidgetTester t, Finder f) => t.getTopLeft(f).dy;
  Finder row(String id) => find.byKey(Key('session-$id'));
  Finder sheet() => find.byKey(const Key('session-crew-sheet'));

  List<http.Request> changes(FakeAgentServer a) => [
    for (final r in a.requests)
      if (r.method != 'GET') r,
  ];

  Map<String, dynamic> body(http.Request r) =>
      jsonDecode(r.body) as Map<String, dynamic>;

  Future<void> tapIn(WidgetTester t, Finder scope, Key key) async {
    final f = find.descendant(of: scope, matching: find.byKey(key));
    await t.ensureVisible(f);
    await t.pumpAndSettle();
    await t.tap(f);
    await t.pumpAndSettle();
  }

  bool checked(WidgetTester t, Key key) => find
      .descendant(
        of: find.byKey(key),
        matching: find.byKey(const Key('sheet-check')),
      )
      .evaluate()
      .isNotEmpty;

  group('the list (4j)', () {
    testWidgets('CREWS, the crew and its members behind the edge, then '
        'RUNNING and ENDED with only loose sessions', (t) async {
      final c = await open(t, crewServer());
      final header = find.byKey(const Key('crew-crw_1'));
      expect(
        top(t, find.byKey(const Key('crews-section'))),
        lessThan(top(t, header)),
      );
      expect(top(t, header), lessThan(top(t, row('ags_impl'))));
      // Running before ended inside the crew.
      expect(top(t, row('ags_probe')), lessThan(top(t, row('ags_notes'))));
      expect(top(t, row('ags_notes')), lessThan(top(t, find.text('RUNNING'))));
      expect(top(t, find.text('RUNNING')), lessThan(top(t, row('ags_spike'))));
      expect(top(t, row('ags_spike')), lessThan(top(t, find.text('ENDED'))));
      // Members sit in from the header, behind the left edge.
      expect(
        t.getTopLeft(row('ags_impl')).dx,
        greaterThan(t.getTopLeft(header).dx),
      );
      expect(t.getTopLeft(row('ags_spike')).dx, t.getTopLeft(header).dx);
      expect(row('ags_impl'), findsOneWidget, reason: 'listed once');
      // Roles on phone rows; marks 36.
      expect(find.byKey(const Key('role-tag-k_coder')), findsOneWidget);
      expect(t.getSize(find.byKey(const Key('mark-ags_impl'))).width, 36);
      expect(
        t.widget<Text>(find.byKey(const Key('crew-count-crw_1'))).data,
        '3 running',
      );

      await t.tap(find.byKey(const Key('crew-chevron-crw_1')));
      await t.pumpAndSettle();
      expect(row('ags_impl'), findsNothing);
      expect(
        t.widget<Text>(find.byKey(const Key('crew-count-crw_1'))).data,
        '3 · 1',
      );
      await t.tap(header);
      await t.pumpAndSettle();
      expect(where(c).path, Routes.agentCrew('crw_1'));
      await disposeShell(t, c);
    });

    testWidgets('with no crews it is the list as before', (t) async {
      final agents = crewServer()..crews.clear();
      for (final s in agents.sessions) {
        s.remove('crew');
      }
      final c = await open(t, agents);
      expect(find.byKey(const Key('crews-section')), findsNothing);
      expect(find.text('RUNNING'), findsOneWidget);
      expect(row('ags_impl'), findsOneWidget);
      await disposeShell(t, c);
    });

    testWidgets('a long-press on a crew header raises its actions', (t) async {
      final c = await open(t, crewServer());
      await t.longPress(find.byKey(const Key('crew-crw_1')));
      await t.pumpAndSettle();
      final menu = find.byKey(const Key('crew-menu'));
      expect(menu, findsOneWidget);
      expect(find.byType(BottomSheet), findsOneWidget);
      expect(
        find.descendant(of: menu, matching: find.text('Open crew page')),
        findsOneWidget,
      );
      await disposeShell(t, c);
    });
  });

  group('the long-press sheet (4k)', () {
    testWidgets('Crew and Role are independent: a role sets only the label, '
        'a crew moves only the crew', (t) async {
      final agents = crewServer();
      final c = await open(t, agents);
      await t.longPress(row('ags_spike'));
      await t.pumpAndSettle();
      expect(sheet(), findsOneWidget);
      expect(find.text('CREW'), findsOneWidget);
      expect(find.text('ROLE'), findsOneWidget);
      expect(
        find.text(
          'spike-sync launched as Storm Researcher and keeps that note. '
          'Changing the role here only changes the label.',
        ),
        findsOneWidget,
      );
      expect(checked(t, const Key('sheet-crew-none')), isTrue);
      expect(checked(t, const Key('sheet-role-k_res')), isTrue);
      expect(
        find.descendant(of: sheet(), matching: find.text('Proves a spike.')),
        findsOneWidget,
      );

      await tapIn(t, sheet(), const Key('sheet-role-k_rev'));
      expect(
        changes(agents).single.url.path,
        '/v1/agent/sessions/ags_spike/role',
      );
      expect(body(changes(agents).single), {
        'vault_id': 'v-kit',
        'note_id': 'k_rev',
      });
      expect(agents.session('ags_spike')!['crew'], isNull);
      expect(checked(t, const Key('sheet-role-k_rev')), isTrue);
      expect(checked(t, const Key('sheet-role-k_res')), isFalse);
      expect(checked(t, const Key('sheet-crew-none')), isTrue);

      await tapIn(t, sheet(), const Key('sheet-crew-crw_1'));
      final move = changes(agents).last;
      expect(move.url.path, '/v1/agent/sessions/ags_spike/crew');
      expect(body(move), {'crew_id': 'crw_1'});
      expect(changes(agents), hasLength(2));
      expect((agents.session('ags_spike')!['role'] as Map)['note_id'], 'k_rev');
      expect(checked(t, const Key('sheet-crew-crw_1')), isTrue);
      expect(checked(t, const Key('sheet-role-k_rev')), isTrue);

      await tapIn(t, sheet(), const Key('sheet-role-none'));
      expect(changes(agents).last.method, 'DELETE');
      expect(agents.session('ags_spike')!['crew'], isNotNull);
      await disposeShell(t, c);
    });

    testWidgets('a loose session can carry a role', (t) async {
      final agents = crewServer();
      final c = await open(t, agents);
      await t.longPress(row('ags_lint'));
      await t.pumpAndSettle();
      await tapIn(t, sheet(), const Key('sheet-role-k_coder'));
      expect(agents.session('ags_lint')!['crew'], isNull);
      expect(
        (agents.session('ags_lint')!['role'] as Map)['note_id'],
        'k_coder',
      );
      Navigator.of(t.element(sheet())).pop();
      await t.pumpAndSettle();
      expect(
        find.descendant(
          of: row('ags_lint'),
          matching: find.text('storm coder'),
        ),
        findsOneWidget,
      );
      // Still in ENDED, not in a crew.
      expect(top(t, row('ags_lint')), greaterThan(top(t, find.text('ENDED'))));
      await disposeShell(t, c);
    });

    testWidgets('New crew… makes a crew of the session', (t) async {
      final agents = crewServer();
      final c = await open(t, agents);
      await t.longPress(row('ags_spike'));
      await t.pumpAndSettle();
      await tapIn(t, sheet(), const Key('sheet-crew-new'));
      await t.enterText(find.byType(TextField).last, 'Spike pair');
      await t.tap(find.text('OK').last);
      await t.pumpAndSettle();
      final made = changes(agents).single;
      expect(made.url.path, '/v1/agent/crews');
      expect(body(made)['name'], 'Spike pair');
      expect(body(made)['session_ids'], ['ags_spike']);
      expect(checked(t, const Key('sheet-crew-crw_new1')), isTrue);
      await disposeShell(t, c);
    });

    testWidgets('an agent that can\'t read Storm says a role is only a label', (
      t,
    ) async {
      final c = await open(t, crewServer());
      await t.longPress(row('ags_shell'));
      await t.pumpAndSettle();
      expect(
        find.text(
          "Shell can't read Storm notes, so a role here is only a label.",
        ),
        findsOneWidget,
      );
      await disposeShell(t, c);
    });

    testWidgets('on a desk a long-press raises no sheet', (t) async {
      final c = await open(t, crewServer(), size: wide);
      await t.longPress(row('ags_spike'));
      await t.pumpAndSettle();
      expect(sheet(), findsNothing);
      await disposeShell(t, c);
    });
  });

  group('the session', () {
    testWidgets('the details sheet has the same Crew and Role', (t) async {
      final agents = crewServer();
      final c = await open(t, agents, at: Routes.agentSession('ags_rev'));
      await t.tap(find.byKey(const Key('session-header')));
      await t.pumpAndSettle();
      final sections = find.byKey(const Key('crew-role-sections'));
      expect(sections, findsOneWidget);
      expect(checked(t, const Key('sheet-crew-crw_1')), isTrue);
      await tapIn(t, sections, const Key('sheet-role-k_rev'));
      expect(
        changes(agents).single.url.path,
        '/v1/agent/sessions/ags_rev/role',
      );
      expect(agents.session('ags_rev')!['crew'], {'id': 'crw_1', 'order': 1});
      await disposeShell(t, c);
    });

    testWidgets('the phone header carries the crew chip and the role tag; '
        'the desk keeps its own', (t) async {
      var c = await open(t, crewServer(), at: Routes.agentSession('ags_impl'));
      final header = find.byKey(const Key('session-header'));
      expect(
        find.descendant(
          of: header,
          matching: find.byKey(const Key('phone-crew-chip')),
        ),
        findsOneWidget,
      );
      expect(
        find.descendant(of: header, matching: find.text('Gateway release')),
        findsOneWidget,
      );
      expect(
        find.descendant(
          of: header,
          matching: find.byKey(const Key('role-tag-k_coder')),
        ),
        findsOneWidget,
      );
      expect(find.byKey(const Key('session-crew-chip')), findsNothing);
      await disposeShell(t, c);

      c = await open(
        t,
        crewServer(),
        size: wide,
        at: Routes.agentSession('ags_impl'),
      );
      expect(find.byKey(const Key('phone-crew-chip')), findsNothing);
      expect(find.byKey(const Key('session-crew-chip')), findsOneWidget);
      await disposeShell(t, c);
    });
  });

  group('the crew screen (4l)', () {
    testWidgets('pushed with Back in the corner, its members as rows, ＋ New '
        'session in crew in the pill', (t) async {
      final c = await open(t, crewServer(), at: Routes.agentCrew('crw_1'));
      expect(find.byKey(const Key('back-bubble')), findsOneWidget);
      expect(find.byKey(const Key('crew-new-session')), findsNothing);
      expect(find.byKey(const Key('crew-new-session-pill')), findsOneWidget);
      expect(find.text('New session in crew'), findsOneWidget);
      expect(
        find.descendant(
          of: find.byKey(const Key('crew-page-members')),
          matching: row('ags_impl'),
        ),
        findsOneWidget,
      );
      await t.longPress(row('ags_rev'));
      await t.pumpAndSettle();
      expect(sheet(), findsOneWidget);
      Navigator.of(t.element(sheet())).pop();
      await t.pumpAndSettle();

      await t.tap(find.byKey(const Key('back-bubble')));
      await t.pumpAndSettle();
      expect(where(c).path, Routes.agents);
      await disposeShell(t, c);
    });

    testWidgets('··· raises the crew actions as a sheet', (t) async {
      final c = await open(t, crewServer(), at: Routes.agentCrew('crw_1'));
      await t.tap(find.byKey(const Key('crew-page-menu')));
      await t.pumpAndSettle();
      final menu = find.byKey(const Key('crew-menu'));
      expect(menu, findsOneWidget);
      expect(
        find.ancestor(of: menu, matching: find.byType(BottomSheet)),
        findsOneWidget,
      );
      for (final label in [
        'Rename',
        'Colour',
        'Collapse',
        'New session in crew',
        'Dismiss ended',
        'End all running…',
      ]) {
        expect(
          find.descendant(of: menu, matching: find.text(label)),
          findsOneWidget,
          reason: label,
        );
      }
      expect(
        find.descendant(of: menu, matching: find.text('Open crew page')),
        findsNothing,
        reason: 'it is the crew page',
      );
      await disposeShell(t, c);
    });

    testWidgets('End all running is a sheet: Stops, Can\'t reach now and '
        'Already ended; it ends only what it can reach', (t) async {
      final agents = crewServer();
      final c = await open(t, agents, at: Routes.agentCrew('crw_1'));
      await t.tap(find.byKey(const Key('crew-page-menu')));
      await t.pumpAndSettle();
      await t.tap(find.byKey(const Key('crew-action-endAll')));
      await t.pumpAndSettle();
      final s = find.byKey(const Key('end-all-sheet'));
      expect(s, findsOneWidget);
      expect(find.byKey(const Key('end-all-banner')), findsNothing);
      expect(find.text('End all running?'), findsOneWidget);
      Finder inGroup(String key, String text) =>
          find.descendant(of: find.byKey(Key(key)), matching: find.text(text));
      expect(inGroup('end-all-stops', 'gateway-impl'), findsOneWidget);
      expect(inGroup('end-all-stops', 'review-gateway'), findsOneWidget);
      expect(inGroup('end-all-unreachable', 'relay-probe'), findsOneWidget);
      expect(
        inGroup(
          'end-all-unreachable',
          'mac-mini is offline. Skipped: end it once the host is back.',
        ),
        findsOneWidget,
      );
      expect(inGroup('end-all-ended', 'release-notes'), findsOneWidget);
      expect(find.text('End 2 sessions'), findsOneWidget);

      await t.ensureVisible(find.byKey(const Key('end-all-confirm')));
      await t.pumpAndSettle();
      await t.tap(find.byKey(const Key('end-all-confirm')));
      await t.pumpAndSettle();
      expect(agents.count('POST', '/v1/agent/sessions/ags_impl/end'), 1);
      expect(agents.count('POST', '/v1/agent/sessions/ags_rev/end'), 1);
      expect(agents.count('POST', '/v1/agent/sessions/ags_probe/end'), 0);
      expect(find.byKey(const Key('end-all-sheet')), findsNothing);
      await disposeShell(t, c);
    });

    testWidgets('from the list\'s header it opens on the crew screen', (
      t,
    ) async {
      final c = await open(t, crewServer());
      await t.longPress(find.byKey(const Key('crew-crw_1')));
      await t.pumpAndSettle();
      await t.tap(find.byKey(const Key('crew-action-endAll')));
      await t.pumpAndSettle();
      expect(where(c).path, Routes.agentCrew('crw_1'));
      expect(find.byKey(const Key('end-all-sheet')), findsOneWidget);
      await t.tap(find.byKey(const Key('end-all-cancel')));
      await t.pumpAndSettle();
      expect(find.byKey(const Key('end-all-sheet')), findsNothing);
      await disposeShell(t, c);
    });

    testWidgets('the desk keeps its inline banner', (t) async {
      final c = await open(
        t,
        crewServer(),
        size: wide,
        at: Routes.agentCrew('crw_1'),
      );
      await t.tap(find.byKey(const Key('crew-page-menu')));
      await t.pumpAndSettle();
      await t.tap(find.byKey(const Key('crew-action-endAll')));
      await t.pumpAndSettle();
      expect(find.byKey(const Key('end-all-banner')), findsOneWidget);
      expect(find.byKey(const Key('end-all-sheet')), findsNothing);
      expect(find.byType(BottomSheet), findsNothing);
      await disposeShell(t, c);
    });
  });
}
