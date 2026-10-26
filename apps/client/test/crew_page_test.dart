import 'dart:convert';

import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart' show RenderParagraph;
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'package:storm/agent/agent_models.dart';
import 'package:storm/agent/session_crew.dart';
import 'package:storm/router.dart';

import 'agent_fakes.dart';
import 'fake_server.dart';
import 'real_fonts.dart';
import 'shell_harness.dart';

/// Decision 138, slice 138-4: the crew page, End all running and Dismiss
/// ended, and a crew member's session view (plan §4.3, §6.3; handoff 4d, 4e).
void main() {
  const phone = Size(411, 900);
  const wide = Size(1400, 900);
  const long = 'Install guide and relay docs refresh, Q4'; // 40 characters
  const primary = FakeServer.primaryVault;

  setUpAll(loadRealFonts);
  setUp(() => SharedPreferences.setMockInitialValues({}));

  Map<String, dynamic> spec() => {
    'vault_id': primary,
    'note_id': 'n3',
    'title': 'Gateway spec',
  };

  Map<String, dynamic> write(
    String noteId,
    String title, {
    String kind = 'edited',
    int? version,
    required String at,
  }) => {
    'vault_id': primary,
    'note_id': noteId,
    'title': title,
    'path': 'projects/storm/$title.md',
    'kind': kind,
    'version': version,
    'at': at,
  };

  /// Gateway release on build-vm and mac-mini, which is offline: two
  /// running members it can stop, one it can't reach, one ended. One loose
  /// session besides.
  FakeAgentServer crewServer({String crewName = 'Gateway release'}) =>
      FakeAgentServer(
        hosts: [
          agentHost(),
          agentHost(id: 'hst_2', name: 'mac-mini', status: 'offline'),
        ],
        crews: [agentCrew('crw_1', name: crewName)],
        roles: [
          kitRole('k_rev', 'Storm Reviewer'),
          kitRole('k_sop', 'SOP Reviewer', folder: 'projects/sop/agents'),
        ],
        sessions: [
          agentSession(
            'ags_notes',
            name: 'release-notes',
            workspace: 'site',
            status: 'completed',
            crew: ('crw_1', 3),
            wroteCount: 2,
          ),
          agentSession(
            'ags_impl',
            name: 'gateway-impl',
            crew: ('crw_1', 0),
            context: spec(),
            wroteCount: 1,
            role: roleRef('k_coder', 'Storm Coder'),
            launchRole: roleRef('k_coder', 'Storm Coder'),
          ),
          agentSession(
            'ags_rev',
            name: 'review-gateway',
            crew: ('crw_1', 1),
            context: spec(),
            role: roleRef('k_sop', 'SOP Reviewer'),
            launchRole: roleRef('k_rev', 'Storm Reviewer'),
          ),
          agentSession(
            'ags_probe',
            name: 'relay-probe',
            hostId: 'hst_2',
            workspace: 'relay-bench',
            status: 'unknown',
            crew: ('crw_1', 2),
          ),
          agentSession('ags_loose', name: 'spike-sync'),
        ],
        writes: {
          'ags_impl': [
            write('n_board', 'BOARD', version: 53, at: '2026-10-25T10:00:00Z'),
          ],
          'ags_notes': [
            write(
              'n_rel',
              'Release notes 0.4',
              kind: 'created',
              at: '2026-10-25T09:00:00Z',
            ),
            write('n_board', 'BOARD', version: 50, at: '2026-10-25T08:00:00Z'),
          ],
        },
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

  Uri where(ProviderContainer c) => c.read(routerProvider).state.uri;
  double top(WidgetTester t, Finder f) => t.getTopLeft(f).dy;
  Finder within(String key, Finder f) =>
      find.descendant(of: find.byKey(Key(key)), matching: f);

  group('getting there', () {
    testWidgets('the sidebar header and the overview card open the crew '
        'page; the chevron still folds', (t) async {
      final c = await open(t, crewServer());
      await t.tap(find.byKey(const Key('crew-card-header-crw_1')));
      await t.pumpAndSettle();
      expect(where(c).path, Routes.agentCrew('crw_1'));
      expect(find.byKey(const Key('crew-page-crw_1')), findsOneWidget);

      await t.tap(find.byKey(const Key('agents-overview')));
      await t.pumpAndSettle();
      await t.tap(find.byKey(const Key('crew-name-crw_1')));
      await t.pumpAndSettle();
      expect(where(c).path, Routes.agentCrew('crw_1'));

      await t.tap(find.byKey(const Key('crew-chevron-crw_1')));
      await t.pumpAndSettle();
      expect(where(c).path, Routes.agentCrew('crw_1'));
      expect(find.byKey(const Key('session-ags_impl')), findsNothing);
      await disposeShell(t, c);
    });

    testWidgets('Open crew page in the menu', (t) async {
      final c = await open(t, crewServer());
      await t.tap(
        find.byKey(const Key('crew-crw_1')),
        buttons: kSecondaryButton,
      );
      await t.pumpAndSettle();
      await t.tap(find.byKey(const Key('crew-action-openPage')));
      await t.pumpAndSettle();
      expect(where(c).path, Routes.agentCrew('crw_1'));
      await disposeShell(t, c);
    });

    testWidgets('a crew that is gone says so', (t) async {
      final c = await open(t, crewServer(), at: Routes.agentCrew('crw_x'));
      expect(find.byKey(const Key('crew-gone')), findsOneWidget);
      expect(find.text('This crew is gone'), findsOneWidget);
      await disposeShell(t, c);
    });
  });

  group('the page (4d)', () {
    testWidgets('header, hosts, members, Started from and Wrote', (t) async {
      final c = await open(t, crewServer(), at: Routes.agentCrew('crw_1'));
      expect(find.text('Gateway release'), findsWidgets);
      expect(
        t.widget<Text>(find.byKey(const Key('crew-page-summary'))).data,
        '3 running · 1 ended · build-vm, mac-mini',
      );
      // Running members first, each in crew order; mark 28.
      final rows = [
        for (final id in ['ags_impl', 'ags_rev', 'ags_probe', 'ags_notes'])
          within('crew-page-members', find.byKey(Key('crew-row-$id'))),
      ];
      for (var i = 1; i < rows.length; i++) {
        expect(top(t, rows[i - 1]), lessThan(top(t, rows[i])));
      }
      expect(
        t
            .getSize(
              within(
                'crew-page-members',
                find.byKey(const Key('mark-ags_impl')),
              ),
            )
            .width,
        28,
      );
      expect(
        within('crew-page-members', find.text('storm coder')),
        findsOneWidget,
      );

      // Started from: the one note two members started from.
      expect(within('crew-started-from', find.text('Gateway spec')), findsOne);
      expect(within('crew-started-from', find.text('2 sessions')), findsOne);

      // Wrote: both members' lists merged, each note once at its newest.
      expect(within('crew-wrote', find.text('BOARD')), findsOneWidget);
      expect(within('crew-wrote', find.text('v53 · gateway-impl')), findsOne);
      expect(within('crew-wrote', find.text('new · release-notes')), findsOne);
      expect(
        top(t, within('crew-wrote', find.text('BOARD'))),
        lessThan(top(t, within('crew-wrote', find.text('Release notes 0.4')))),
      );

      await t.tap(find.byKey(const Key('crew-wrote-n_board')));
      await t.pumpAndSettle();
      expect(where(c).path, Uri.parse(Routes.note(primary, 'n_board')).path);
      expect(where(c).queryParameters['session'], 'ags_impl');
      await disposeShell(t, c);
    });

    testWidgets('a 40-character name wraps rather than ellipsizing', (t) async {
      final c = await open(
        t,
        crewServer(crewName: long),
        at: Routes.agentCrew('crw_1'),
      );
      final name = t.renderObject<RenderParagraph>(
        find.byKey(const Key('crew-page-name')),
      );
      expect(name.didExceedMaxLines, isFalse);
      expect(find.byKey(const Key('crew-page-name')), findsOneWidget);
      await disposeShell(t, c);
    });

    testWidgets('New session in crew opens the launcher with the crew set', (
      t,
    ) async {
      final agents = crewServer();
      final c = await open(t, agents, at: Routes.agentCrew('crw_1'));
      await t.tap(find.byKey(const Key('crew-new-session')));
      await t.pumpAndSettle();
      expect(
        within('launcher-crew', find.text('Gateway release')),
        findsOneWidget,
      );
      await t.tap(find.byKey(const Key('launch')));
      await t.pumpAndSettle();
      expect(agents.launches.single['crew'], {'id': 'crw_1'});
      expect(
        (agents.sessions.last['crew'] as Map)['id'],
        'crw_1',
        reason: 'the launched session is in the crew',
      );
      await disposeShell(t, c);
    });

    testWidgets('and from the sidebar menu', (t) async {
      final agents = crewServer();
      final c = await open(t, agents);
      await t.tap(
        find.byKey(const Key('crew-crw_1')),
        buttons: kSecondaryButton,
      );
      await t.pumpAndSettle();
      await t.tap(find.byKey(const Key('crew-action-newSession')));
      await t.pumpAndSettle();
      expect(
        within('launcher-crew', find.text('Gateway release')),
        findsOneWidget,
      );
      await disposeShell(t, c);
    });
  });

  group('End all running (138k)', () {
    Future<void> openBanner(WidgetTester t) async {
      await t.tap(find.byKey(const Key('crew-page-menu')));
      await t.pumpAndSettle();
      await t.tap(find.byKey(const Key('crew-action-endAll')));
      await t.pumpAndSettle();
    }

    testWidgets('lists Stops, Can\'t reach now and Already ended, and skips '
        'the unreachable host', (t) async {
      final agents = crewServer();
      final c = await open(t, agents, at: Routes.agentCrew('crw_1'));
      await openBanner(t);
      final banner = find.byKey(const Key('end-all-banner'));
      expect(banner, findsOneWidget);
      expect(
        within(
          'end-all-banner',
          find.text('End all running in Gateway release?'),
        ),
        findsOneWidget,
      );
      expect(
        within('end-all-stops', find.text('gateway-impl · build-vm')),
        findsOneWidget,
      );
      expect(
        within('end-all-stops', find.text('review-gateway · build-vm')),
        findsOneWidget,
      );
      expect(
        within(
          'end-all-unreachable',
          find.text(
            'relay-probe · mac-mini is offline. Skipped: end it once the '
            'host is back.',
          ),
        ),
        findsOneWidget,
      );
      expect(within('end-all-ended', find.text('release-notes')), findsOne);
      expect(within('end-all-banner', find.text('End 2 sessions')), findsOne);
      // Under the header, above the members.
      expect(
        top(t, find.byKey(const Key('crew-page-summary'))),
        lessThan(top(t, banner)),
      );
      expect(
        top(t, banner),
        lessThan(top(t, find.byKey(const Key('crew-page-members')))),
      );

      await t.tap(find.byKey(const Key('end-all-confirm')));
      await t.pumpAndSettle();
      final ended = [
        for (final r in agents.requests)
          if (r.method == 'POST' && r.url.path.endsWith('/end'))
            r.url.pathSegments[3],
      ];
      expect(ended, ['ags_impl', 'ags_rev'], reason: 'one call each, in order');
      expect(agents.session('ags_probe')!['status'], 'unknown');
      expect(banner, findsNothing);
      await disposeShell(t, c);
    });

    testWidgets('Cancel ends nothing', (t) async {
      final agents = crewServer();
      final c = await open(t, agents, at: Routes.agentCrew('crw_1'));
      await openBanner(t);
      await t.tap(find.byKey(const Key('end-all-cancel')));
      await t.pumpAndSettle();
      expect(find.byKey(const Key('end-all-banner')), findsNothing);
      expect(
        agents.requests.where((r) => r.url.path.endsWith('/end')),
        isEmpty,
      );
      await disposeShell(t, c);
    });

    testWidgets('from the sidebar menu it opens on the crew page', (t) async {
      final c = await open(t, crewServer());
      await t.tap(
        find.byKey(const Key('crew-crw_1')),
        buttons: kSecondaryButton,
      );
      await t.pumpAndSettle();
      await t.tap(find.byKey(const Key('crew-action-endAll')));
      await t.pumpAndSettle();
      expect(where(c).path, Routes.agentCrew('crw_1'));
      expect(find.byKey(const Key('end-all-banner')), findsOneWidget);
      await disposeShell(t, c);
    });

    testWidgets('Dismiss ended loops over dismiss', (t) async {
      final agents = crewServer();
      agents.session('ags_probe')!['status'] = 'failed';
      final c = await open(t, agents, at: Routes.agentCrew('crw_1'));
      await t.tap(find.byKey(const Key('crew-page-menu')));
      await t.pumpAndSettle();
      expect(
        within('crew-action-dismissEnded', find.text('2')),
        findsOneWidget,
      );
      await t.tap(find.byKey(const Key('crew-action-dismissEnded')));
      await t.pumpAndSettle();
      final dismissed = [
        for (final r in agents.requests)
          if (r.method == 'DELETE') r.url.pathSegments[3],
      ];
      expect(dismissed.toSet(), {'ags_notes', 'ags_probe'});
      expect(agents.session('ags_impl'), isNotNull);
      await disposeShell(t, c);
    });
  });

  group('both sides of 900', () {
    testWidgets('a phone shows the crew page alone, with back to Agents', (
      t,
    ) async {
      final c = await open(
        t,
        crewServer(),
        size: phone,
        at: Routes.agentCrew('crw_1'),
      );
      expect(find.byKey(const Key('crew-page-crw_1')), findsOneWidget);
      expect(find.byKey(const Key('agents-overview')), findsNothing);
      // Started from and Wrote stack.
      expect(
        top(t, find.byKey(const Key('crew-started-from'))),
        lessThan(top(t, find.byKey(const Key('crew-wrote')))),
      );
      await disposeShell(t, c);

      final w = await open(t, crewServer(), at: Routes.agentCrew('crw_1'));
      expect(find.byKey(const Key('agents-overview')), findsOneWidget);
      expect(
        top(t, find.byKey(const Key('crew-started-from'))),
        top(t, find.byKey(const Key('crew-wrote'))),
      );
      await disposeShell(t, w);
    });
  });

  group('the session view (4e)', () {
    testWidgets('the meta line has the crew chip and the role tag; the chip '
        'opens the crew', (t) async {
      final c = await open(t, crewServer(), at: Routes.agentSession('ags_rev'));
      final meta = find.byKey(const Key('session-meta'));
      final chip = find.byKey(const Key('session-crew-chip'));
      expect(chip, findsOneWidget);
      expect(t.getTopLeft(chip).dx, greaterThan(t.getTopRight(meta).dx));
      expect(
        find.descendant(
          of: find.byKey(const Key('role-tag-k_sop')),
          matching: find.text('sop reviewer'),
        ),
        findsWidgets,
      );
      await t.tap(chip);
      await t.pumpAndSettle();
      expect(where(c).path, Routes.agentCrew('crw_1'));
      await disposeShell(t, c);
    });

    testWidgets('About shows the launch role and the label when they differ', (
      t,
    ) async {
      final c = await open(
        t,
        crewServer(),
        at: Routes.agentSession('ags_rev', tab: 'about'),
      );
      expect(
        t.widget<Text>(find.byKey(const Key('about-role-text'))).data,
        'Launched as storm reviewer (the agent read this note) · Label: sop '
        'reviewer',
      );
      expect(within('about-crew', find.text('Gateway release')), findsOne);
      // Agent, Workspace, Crew, Role, then what it may touch.
      expect(
        top(t, find.text('WORKSPACE')),
        lessThan(top(t, find.text('CREW'))),
      );
      expect(top(t, find.text('CREW')), lessThan(top(t, find.text('ROLE'))));
      expect(
        top(t, find.text('ROLE')),
        lessThan(top(t, find.text('VAULT ACCESS'))),
      );
      await disposeShell(t, c);
    });

    testWidgets('Change label sets only the label', (t) async {
      final agents = crewServer();
      final c = await open(
        t,
        agents,
        at: Routes.agentSession('ags_impl', tab: 'about'),
      );
      await t.tap(find.byKey(const Key('change-label')));
      await t.pumpAndSettle();
      expect(find.text('No label'), findsOneWidget);
      expect(find.text('PROJECTS / SOP / AGENTS'), findsOneWidget);
      await t.tap(find.byKey(const Key('role-k_sop')));
      await t.pumpAndSettle();
      final put = agents.requests.lastWhere(
        (r) => r.url.path.endsWith('/role'),
      );
      expect(put.url.path, '/v1/agent/sessions/ags_impl/role');
      expect(jsonDecode(put.body), {'vault_id': 'v-kit', 'note_id': 'k_sop'});
      expect(
        t.widget<Text>(find.byKey(const Key('about-role-text'))).data,
        'Launched as storm coder (the agent read this note) · Label: sop '
        'reviewer',
      );
      await disposeShell(t, c);
    });

    testWidgets('Move to crew moves it out, and says when that empties a '
        'crew', (t) async {
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        crews: [
          agentCrew('crw_1'),
          agentCrew('crw_2', name: 'Docs', order: 1),
        ],
        sessions: [
          agentSession('ags_1', crew: ('crw_1', 0)),
          agentSession('ags_2', crew: ('crw_2', 0)),
        ],
      );
      final c = await open(
        t,
        agents,
        at: Routes.agentSession('ags_1', tab: 'about'),
      );
      await t.tap(find.byKey(const Key('move-to-crew')));
      await t.pumpAndSettle();
      await t.tap(find.byKey(const Key('move-to-crw_2')));
      await t.pumpAndSettle();
      expect((agents.session('ags_1')!['crew'] as Map)['id'], 'crw_2');
      expect(agents.crew('crw_1'), isNull);
      expect(
        find.text('Gateway release was removed: its last session left it.'),
        findsOneWidget,
      );
      expect(within('about-crew', find.text('Docs')), findsOneWidget);

      await t.tap(find.byKey(const Key('move-to-crew')));
      await t.pumpAndSettle();
      await t.tap(find.byKey(const Key('move-to-none')));
      await t.pumpAndSettle();
      expect(agents.session('ags_1')!['crew'], isNull);
      expect(within('about-crew', find.text('Not in a crew')), findsOneWidget);
      await disposeShell(t, c);
    });
  });

  group('About\'s Role line', () {
    AgentSession session({
      String provider = 'claude-code',
      Map<String, dynamic>? role,
      Map<String, dynamic>? launchRole,
    }) => AgentSession.fromJson(
      agentSession(
        'ags_1',
        provider: provider,
        role: role,
        launchRole: launchRole,
      ),
    );

    test('none, the same note, and both when they differ', () {
      expect(aboutRoleText(session(), readsStorm: true), 'None');
      final rev = roleRef('k_rev', 'Storm Reviewer');
      expect(
        aboutRoleText(session(role: rev, launchRole: rev), readsStorm: true),
        'Launched as storm reviewer (the agent read this note)',
      );
      expect(
        aboutRoleText(session(launchRole: rev), readsStorm: true),
        'Launched as storm reviewer (the agent read this note) · No label',
      );
      expect(
        aboutRoleText(
          session(role: roleRef('k_sop', 'SOP Reviewer'), launchRole: rev),
          readsStorm: true,
        ),
        'Launched as storm reviewer (the agent read this note) · Label: sop '
        'reviewer',
      );
    });

    test('a label only, for an agent that cannot read Storm notes', () {
      expect(
        aboutRoleText(
          session(provider: 'shell', role: roleRef('k_rev', 'Storm Reviewer')),
          readsStorm: false,
        ),
        "Label: storm reviewer. Shell can't read Storm notes, so this is only "
        'a label.',
      );
      expect(
        aboutRoleText(
          session(role: roleRef('k_rev', 'Storm Reviewer')),
          readsStorm: true,
        ),
        "Label: storm reviewer. Set after launch: the agent didn't read this "
        'note.',
      );
    });

    test('a deleted note', () {
      final launch = roleRef('k_rev', 'Storm Reviewer');
      expect(
        aboutRoleText(
          session(role: roleRef('k_rev', null), launchRole: launch),
          readsStorm: true,
        ),
        'Launched as storm reviewer (the agent read this note). Its note has '
        'since been deleted.',
      );
      expect(
        aboutRoleText(
          session(role: roleRef('k_sop', null), launchRole: launch),
          readsStorm: true,
        ),
        'Launched as storm reviewer (the agent read this note) · Label: '
        'deleted role (its note was deleted)',
      );
    });
  });
}
