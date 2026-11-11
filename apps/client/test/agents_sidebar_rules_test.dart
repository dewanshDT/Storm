import 'dart:convert';

import 'package:flutter/foundation.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'package:storm/agent/agent_state.dart';
import 'package:storm/agent/agents_shell.dart' show AgentsSidebar;
import 'package:storm/agent/output_seen.dart';
import 'package:storm/agent/spawn_widgets.dart' show LineageIndent;
import 'package:storm/router.dart';
import 'package:storm/ui/shell/sidebar_rows.dart' show UnseenDot;
import 'package:storm/ui/tokens.dart';

import 'agent_fakes.dart';
import 'shell_harness.dart';

/// Decision 153e/f (handoff §2, Sidebar Rules A): the one header row, one
/// signal per channel, the sort, roll-ups, the gaps it covers, the rail's
/// count and ⇧⌘N.
void main() {
  const wide = Size(1400, 900);

  setUp(() => SharedPreferences.setMockInitialValues({}));

  String ago(Duration d) =>
      DateTime.now().toUtc().subtract(d).toIso8601String();

  Map<String, dynamic> session(
    String id, {
    String status = 'running',
    String? activity,
    String? attention,
    Map<String, dynamic>? reason,
    String? parent,
    (String, int)? crew,
    int? outputEnd,
    Map<String, dynamic> extra = const {},
  }) => {
    ...agentSession(
      id,
      name: id,
      status: status,
      activity: activity,
      crew: crew,
    ),
    'needs_attention': ?attention,
    'attention': ?reason,
    if (parent != null) ...{'parent_session': parent, 'depth': 1},
    'output_end': ?outputEnd,
    ...extra,
  };

  Future<ProviderContainer> open(
    WidgetTester t,
    FakeAgentServer agents, {
    String at = Routes.agents,
  }) async {
    final c = shellContainer(agentClient: agents.client);
    await pumpShell(t, c, size: wide);
    c.read(routerProvider).go(at);
    await t.pumpAndSettle();
    return c;
  }

  Finder row(String id) => find.byKey(Key('session-$id'));
  double top(WidgetTester t, String id) => t.getTopLeft(row(id)).dy;
  String sub(WidgetTester t, String id) => t
      .widget<Text>(
        find
            .descendant(
              of: find.byKey(Key('sub-$id')),
              matching: find.byType(Text),
            )
            .first,
      )
      .data!;
  Material rowMaterial(WidgetTester t, String id) => t.widget<Material>(
    find.ancestor(of: row(id), matching: find.byType(Material)).first,
  );
  BorderSide outline(WidgetTester t, String id) =>
      (rowMaterial(t, id).shape! as RoundedRectangleBorder).side;
  StormTokens tokens(WidgetTester t) =>
      t.element(find.byType(AgentsSidebar)).tokens;

  group('the header row', () {
    testWidgets('"Agents" opens the overview and ＋ starts a session', (
      t,
    ) async {
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        sessions: [session('ags_1')],
      );
      final c = await open(t, agents, at: Routes.agentSession('ags_1'));
      final tk = tokens(t);
      final heading = t.widget<Text>(find.byKey(const Key('agents-heading')));
      expect(heading.data, 'Agents');
      expect(heading.style!.fontSize, tk.uiSize);
      expect(heading.style!.fontWeight, FontWeight.w600);
      // One 28px row, with no full-width button and no Overview row.
      expect(
        t.getSize(find.byKey(const Key('new-session'))),
        const Size(28, 28),
      );
      expect(find.text('New session'), findsNothing);
      expect(find.text('Overview'), findsNothing);
      final plus = t.widget<Material>(
        find
            .ancestor(
              of: find.byKey(const Key('new-session')),
              matching: find.byType(Material),
            )
            .first,
      );
      expect(plus.color, tk.accentSoft);
      expect(find.byTooltip('New session Ctrl+Shift+N'), findsOneWidget);

      await t.tap(find.byKey(const Key('agents-heading')));
      await t.pumpAndSettle();
      expect(
        c.read(routerProvider).routerDelegate.currentConfiguration.uri.path,
        Routes.agents,
      );

      await t.tap(find.byKey(const Key('new-session')));
      await t.pumpAndSettle();
      expect(find.byKey(const Key('launch')), findsOneWidget);
      await disposeShell(t, c);
    });

    testWidgets('the tooltip says ⇧⌘N on macOS', (t) async {
      debugDefaultTargetPlatformOverride = TargetPlatform.macOS;
      final c = await open(t, FakeAgentServer(hosts: [agentHost()]));
      expect(find.byTooltip('New session ⇧⌘N'), findsOneWidget);
      await disposeShell(t, c);
      debugDefaultTargetPlatformOverride = null;
    });

    testWidgets('with no host online ＋ stays, off, and says why', (t) async {
      final agents = FakeAgentServer(
        hosts: [agentHost(status: 'offline')],
        sessions: [session('ags_old', status: 'stopped')],
      );
      final c = await open(t, agents);
      final plus = t.widget<InkWell>(find.byKey(const Key('new-session')));
      expect(plus.onTap, isNull);
      expect(find.byTooltip('No host is online'), findsOneWidget);
      await t.tap(find.byKey(const Key('new-session')));
      await t.pumpAndSettle();
      expect(find.byKey(const Key('launch')), findsNothing);
      await disposeShell(t, c);
    });

    testWidgets('it keeps the old heading\'s line: rows start 12 + 28 + 4 '
        'under the bar', (t) async {
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        sessions: [session('ags_1')],
      );
      final c = await open(t, agents);
      final header = t.getRect(find.byKey(const Key('new-session')));
      final sidebarTop = t.getTopLeft(find.byType(AgentsSidebar)).dy;
      expect(header.top - sidebarTop, 12);
      expect(header.height, 28);
      await disposeShell(t, c);
    });
  });

  group('one signal per channel', () {
    FakeAgentServer states() => FakeAgentServer(
      hosts: [agentHost()],
      sessions: [
        session('ags_idle'),
        session(
          'ags_needs',
          attention: 'permission_prompt',
          reason: {
            'kind': 'permission_prompt',
            'since': ago(const Duration(minutes: 3)),
          },
        ),
        session('ags_blocked', attention: 'blocked'),
        session('ags_usage', attention: 'usage_limit'),
        session(
          'ags_offline',
          status: 'unknown',
          extra: {'host_last_seen': ago(const Duration(minutes: 9))},
        ),
        session(
          'ags_failed',
          status: 'failed',
          extra: {'end_reason': 'host_restart'},
        ),
        session('ags_parked', status: 'parked'),
      ],
    );

    testWidgets('the outline is for what needs you, and nothing else', (
      t,
    ) async {
      final c = await open(t, states());
      final tk = tokens(t);
      final needs = outline(t, 'ags_needs');
      expect(needs.color, tk.danger);
      expect(needs.width, tk.bw);
      expect(
        rowMaterial(t, 'ags_needs').color,
        Color.alphaBlend(tk.danger.withValues(alpha: 0.09), tk.surface),
      );
      for (final id in [
        'ags_failed',
        'ags_blocked',
        'ags_usage',
        'ags_offline',
        'ags_idle',
        'ags_parked',
      ]) {
        expect(outline(t, id), BorderSide.none, reason: id);
      }
      await disposeShell(t, c);
    });

    testWidgets('the badge says what the agent does; never amber', (t) async {
      final c = await open(t, states());
      final tk = tokens(t);
      BoxDecoration? badge(String id) {
        final f = find.descendant(
          of: find.descendant(
            of: find.byType(AgentsSidebar),
            matching: find.byKey(Key('badge-$id')),
          ),
          matching: find.byType(Container),
        );
        if (f.evaluate().isEmpty) return null;
        return t.widget<Container>(f).decoration! as BoxDecoration;
      }

      expect((badge('ags_idle')!.border! as Border).top.color, tk.accent);
      expect(badge('ags_failed')!.color, tk.danger);
      for (final id in ['ags_usage', 'ags_offline', 'ags_parked']) {
        expect(badge(id), isNull, reason: id);
      }
      // Their names go text2.
      for (final id in ['ags_usage', 'ags_offline', 'ags_parked']) {
        final name = t.widget<Text>(
          find.descendant(of: row(id), matching: find.text(id)),
        );
        expect(name.style!.color, tk.text2, reason: id);
      }
      // No amber anywhere in the sidebar, and no tag pills.
      for (final e
          in find
              .descendant(
                of: find.byType(AgentsSidebar),
                matching: find.byType(Container),
              )
              .evaluate()) {
        final d = (e.widget as Container).decoration;
        if (d is BoxDecoration) {
          expect(d.color, isNot(anyOf(tk.amber, tk.amberSoft)));
        }
      }
      for (final e
          in find
              .descendant(
                of: find.byType(AgentsSidebar),
                matching: find.byType(Text),
              )
              .evaluate()) {
        expect((e.widget as Text).style?.color, isNot(tk.amber));
      }
      expect(find.textContaining('Needs you'), findsNothing);
      await disposeShell(t, c);
    });

    testWidgets('the sub-line says why, in words', (t) async {
      final c = await open(t, states());
      expect(sub(t, 'ags_needs'), 'permission prompt · 3m');
      expect(sub(t, 'ags_blocked'), 'blocked');
      expect(sub(t, 'ags_usage'), 'out of usage');
      expect(sub(t, 'ags_offline'), 'host offline · last seen 9m');
      expect(sub(t, 'ags_failed'), 'failed · host restarted · now');
      expect(sub(t, 'ags_parked'), 'parked until its next message');
      expect(sub(t, 'ags_idle'), 'storm · now');
      final tk = tokens(t);
      Color? color(String id) => t
          .widget<Text>(
            find
                .descendant(
                  of: find.byKey(Key('sub-$id')),
                  matching: find.byType(Text),
                )
                .first,
          )
          .style
          ?.color;
      expect(color('ags_failed'), tk.danger);
      expect(color('ags_needs'), tk.text);
      expect(color('ags_blocked'), tk.text2);
      expect(color('ags_idle'), tk.text3);
      await disposeShell(t, c);
    });

    testWidgets('an older server: no reason, the kind words only', (t) async {
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        sessions: [
          session('ags_needs', attention: 'permission_prompt'),
          session('ags_offline', status: 'unknown'),
        ],
      );
      final c = await open(t, agents);
      expect(sub(t, 'ags_needs'), 'permission prompt');
      expect(sub(t, 'ags_offline'), 'host offline');
      await disposeShell(t, c);
    });
  });

  group('sorting', () {
    testWidgets('urgent rows lead RUNNING; the rest keep their order', (
      t,
    ) async {
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        sessions: [
          session('ags_a'),
          session('ags_b', attention: 'usage_limit'),
          session('ags_c', attention: 'blocked'),
          session('ags_d', attention: 'delete_approval'),
        ],
      );
      final c = await open(t, agents);
      final natural = [
        for (final s in c.read(agentOverviewProvider).value!.looseLive)
          if (s.id == 'ags_a' || s.id == 'ags_b') s.id,
      ];
      expect(top(t, 'ags_d'), lessThan(top(t, 'ags_c')));
      expect(top(t, 'ags_c'), lessThan(top(t, natural.first)));
      expect(top(t, natural.first), lessThan(top(t, natural.last)));
      await disposeShell(t, c);
    });

    testWidgets('the order holds while the pointer is in the sidebar', (
      t,
    ) async {
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        sessions: [session('ags_a'), session('ags_b'), session('ags_c')],
      );
      final c = await open(t, agents);
      final ids = ['ags_a', 'ags_b', 'ags_c']
        ..sort((x, y) => top(t, x).compareTo(top(t, y)));
      final last = ids.last;

      final mouse = await t.createGesture(kind: PointerDeviceKind.mouse);
      await mouse.addPointer(location: t.getCenter(find.byType(AgentsSidebar)));
      await t.pump();
      agents.session(last)!['needs_attention'] = 'permission_prompt';
      c.invalidate(agentOverviewProvider);
      await t.pumpAndSettle();
      expect(outline(t, last).color, tokens(t).danger, reason: 'redrawn');
      expect(top(t, last), greaterThan(top(t, ids.first)), reason: 'held');

      await mouse.moveTo(const Offset(1300, 400));
      await t.pumpAndSettle();
      expect(top(t, last), lessThan(top(t, ids.first)), reason: 'released');
      await mouse.removePointer();
      await disposeShell(t, c);
    });

    testWidgets('crews sort by their most urgent member', (t) async {
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        crews: [
          agentCrew('crw_1', name: 'Relay'),
          agentCrew('crw_2', name: 'Gateway', order: 1),
        ],
        sessions: [
          session('ags_r', crew: ('crw_1', 0)),
          session('ags_g1', crew: ('crw_2', 0)),
          session('ags_g2', crew: ('crw_2', 1), attention: 'permission_prompt'),
        ],
      );
      final c = await open(t, agents);
      double header(String id) => t.getTopLeft(find.byKey(Key('crew-$id'))).dy;
      expect(header('crw_2'), lessThan(header('crw_1')));
      // Inside it, the urgent member first, then the drag order.
      expect(top(t, 'ags_g2'), lessThan(top(t, 'ags_g1')));
      await disposeShell(t, c);
    });

    testWidgets('a tree moves as one, its children under it', (t) async {
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        sessions: [
          session('ags_p'),
          session('ags_c', parent: 'ags_p', attention: 'permission_prompt'),
          session('ags_x'),
          session('ags_y'),
        ],
      );
      final c = await open(t, agents);
      expect(top(t, 'ags_p'), lessThan(top(t, 'ags_c')));
      for (final other in ['ags_x', 'ags_y']) {
        expect(top(t, 'ags_c'), lessThan(top(t, other)));
      }
      expect(
        find.ancestor(of: row('ags_c'), matching: find.byType(LineageIndent)),
        findsOneWidget,
      );
      await disposeShell(t, c);
    });
  });

  group('roll-ups when collapsed', () {
    testWidgets('a crew: "1 needs you" outlined, "1 failed" in danger only', (
      t,
    ) async {
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        crews: [
          agentCrew('crw_1', name: 'Gateway'),
          agentCrew('crw_2', name: 'Relay', order: 1),
        ],
        sessions: [
          session('ags_g', crew: ('crw_1', 0), attention: 'permission_prompt'),
          session('ags_r', crew: ('crw_2', 0), status: 'failed'),
        ],
      );
      final c = await open(t, agents);
      final tk = tokens(t);
      for (final id in ['crw_1', 'crw_2']) {
        await t.tap(find.byKey(Key('crew-chevron-$id')));
        await t.pumpAndSettle();
      }
      Text count(String id) =>
          t.widget<Text>(find.byKey(Key('crew-count-$id')));
      BorderSide edge(String id) =>
          (t
                      .widget<Material>(
                        find
                            .ancestor(
                              of: find.byKey(Key('crew-$id')),
                              matching: find.byType(Material),
                            )
                            .first,
                      )
                      .shape!
                  as RoundedRectangleBorder)
              .side;
      expect(count('crw_1').data, '1 needs you');
      expect(count('crw_1').style!.color, tk.danger);
      expect(edge('crw_1').color, tk.danger);
      expect(count('crw_2').data, '1 failed');
      expect(count('crw_2').style!.color, tk.danger);
      expect(edge('crw_2'), BorderSide.none);
      await disposeShell(t, c);
    });

    testWidgets('a tree folds under its parent\'s chevron and rolls up', (
      t,
    ) async {
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        sessions: [
          session('ags_p'),
          session('ags_c1', parent: 'ags_p', attention: 'permission_prompt'),
          session('ags_c2', parent: 'ags_p'),
        ],
      );
      final c = await open(t, agents);
      // An open tree shows its chevron only under the pointer.
      expect(find.byKey(const Key('tree-chevron-ags_p')), findsNothing);
      final mouse = await t.createGesture(kind: PointerDeviceKind.mouse);
      await mouse.addPointer(location: t.getCenter(row('ags_p')));
      await t.pumpAndSettle();
      await t.tap(find.byKey(const Key('tree-chevron-ags_p')));
      await t.pumpAndSettle();
      await mouse.removePointer();
      await t.pumpAndSettle();
      expect(row('ags_c1'), findsNothing);
      expect(row('ags_c2'), findsNothing);
      expect(sub(t, 'ags_p'), '2 agents · 1 needs you');
      expect(outline(t, 'ags_p').color, tokens(t).danger);
      // Device-local, kept across restarts.
      final prefs = await SharedPreferences.getInstance();
      expect(prefs.getStringList('storm.agents.collapsed.trees'), ['ags_p']);
      await t.tap(find.byKey(const Key('tree-chevron-ags_p')));
      await t.pumpAndSettle();
      expect(row('ags_c1'), findsOneWidget);
      await disposeShell(t, c);
    });

    testWidgets('ENDED folds to its count, with Dismiss all', (t) async {
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        sessions: [
          session('ags_live'),
          session('ags_e1', status: 'completed'),
          session('ags_e2', status: 'stopped'),
        ],
      );
      final c = await open(t, agents);
      await t.tap(find.byKey(const Key('ended-toggle')));
      await t.pumpAndSettle();
      expect(find.text('ENDED · 2'), findsOneWidget);
      expect(row('ags_e1'), findsNothing);
      await t.tap(find.byKey(const Key('ended-dismiss-all')));
      await t.pumpAndSettle();
      expect(agents.session('ags_e1'), isNull);
      expect(agents.session('ags_e2'), isNull);
      expect(agents.session('ags_live'), isNotNull);
      await disposeShell(t, c);
    });
  });

  group('the gaps (§2 1–5)', () {
    testWidgets('1: unread output is a dot, per device, never on the open '
        'session', (t) async {
      SharedPreferences.setMockInitialValues({
        SeenOutput.key: jsonEncode({'': 1, 'ags_a': 50, 'ags_b': 100}),
      });
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        sessions: [
          session('ags_a', outputEnd: 100),
          session('ags_b', outputEnd: 100),
          session('ags_c'),
          session('ags_d', outputEnd: 10),
        ],
      );
      final c = await open(t, agents);
      expect(find.byKey(const Key('unread-ags_a')), findsOneWidget);
      expect(find.byKey(const Key('unread-ags_d')), findsOneWidget);
      expect(find.byKey(const Key('unread-ags_b')), findsNothing);
      expect(
        find.byKey(const Key('unread-ags_c')),
        findsNothing,
        reason: 'no output end: never unread',
      );
      final dot = t.widget<UnseenDot>(find.byKey(const Key('unread-ags_a')));
      expect(dot.message, UnseenDot.sessionMessage);

      c.read(routerProvider).go(Routes.agentSession('ags_a'));
      await t.pumpAndSettle();
      expect(find.byKey(const Key('unread-ags_a')), findsNothing);
      c.read(routerProvider).go(Routes.agents);
      await t.pumpAndSettle();
      expect(
        find.byKey(const Key('unread-ags_a')),
        findsNothing,
        reason: 'seen',
      );
      expect(c.read(seenOutputProvider).value!['ags_a'], 100);
      await disposeShell(t, c);
    });

    testWidgets('1: a new device takes what it finds as read', (t) async {
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        sessions: [session('ags_a', outputEnd: 100)],
      );
      final c = await open(t, agents);
      expect(find.byKey(const Key('unread-ags_a')), findsNothing);
      agents.session('ags_a')!['output_end'] = 140;
      c.invalidate(agentOverviewProvider);
      await t.pumpAndSettle();
      expect(find.byKey(const Key('unread-ags_a')), findsOneWidget);
      await disposeShell(t, c);
    });

    testWidgets('2: work to apply keeps it out of ENDED', (t) async {
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        sessions: [
          session('ags_p'),
          session(
            'ags_c',
            parent: 'ags_p',
            status: 'completed',
            extra: {'work_to_take': true, 'unapplied_commits': 2},
          ),
          session('ags_done', status: 'completed'),
        ],
      );
      final c = await open(t, agents);
      expect(sub(t, 'ags_c'), 'completed · 2 commits to apply');
      final ended = t.getTopLeft(find.byKey(const Key('section-ended'))).dy;
      expect(top(t, 'ags_c'), lessThan(ended));
      expect(top(t, 'ags_done'), greaterThan(ended));
      // Under its live parent.
      expect(
        find.ancestor(of: row('ags_c'), matching: find.byType(LineageIndent)),
        findsOneWidget,
      );
      await disposeShell(t, c);
    });

    testWidgets('3: a child in another crew from its parent: no indent, '
        '"from {parent}"', (t) async {
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        crews: [
          agentCrew('crw_1', name: 'Gateway'),
          agentCrew('crw_2', name: 'Relay', order: 1),
        ],
        sessions: [
          session('gateway-impl', crew: ('crw_1', 0)),
          session('bench-check', parent: 'gateway-impl', crew: ('crw_2', 0)),
        ],
      );
      final c = await open(t, agents);
      expect(sub(t, 'bench-check'), startsWith('from gateway-impl · '));
      expect(
        find.ancestor(
          of: row('bench-check'),
          matching: find.byType(LineageIndent),
        ),
        findsNothing,
      );
      await disposeShell(t, c);
    });

    testWidgets('5: a row\'s ··· has Add to crew…', (t) async {
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        crews: [agentCrew('crw_1', name: 'Gateway')],
        sessions: [
          session('ags_m', crew: ('crw_1', 0)),
          session('ags_loose'),
        ],
      );
      final c = await open(t, agents);
      final mouse = await t.createGesture(kind: PointerDeviceKind.mouse);
      await mouse.addPointer(location: t.getCenter(row('ags_loose')));
      await t.pumpAndSettle();
      await t.tap(find.byKey(const Key('session-menu-ags_loose')));
      await t.pumpAndSettle();
      expect(find.byKey(const Key('session-menu')), findsOneWidget);
      await t.tap(find.byKey(const Key('session-action-add-to-crew')));
      await t.pumpAndSettle();
      await t.tap(find.byKey(const Key('add-to-crew-crw_1')));
      await t.pumpAndSettle();
      expect((agents.session('ags_loose')!['crew'] as Map)['id'], 'crw_1');
      await mouse.removePointer();
      await disposeShell(t, c);
    });

    testWidgets('5: a crew\'s menu has Add sessions…, a checklist', (t) async {
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        crews: [agentCrew('crw_1', name: 'Gateway')],
        sessions: [
          session('ags_m', crew: ('crw_1', 0)),
          session('ags_x'),
          session('ags_y'),
          session('ags_old', status: 'completed'),
        ],
      );
      final c = await open(t, agents);
      await t.tap(
        find.byKey(const Key('crew-crw_1')),
        buttons: kSecondaryButton,
      );
      await t.pumpAndSettle();
      await t.tap(find.byKey(const Key('crew-action-addSessions')));
      await t.pumpAndSettle();
      expect(find.byKey(const Key('add-sessions')), findsOneWidget);
      expect(find.byKey(const Key('add-session-ags_m')), findsNothing);
      expect(find.byKey(const Key('add-session-ags_old')), findsNothing);
      expect(find.text('Add'), findsOneWidget);
      await t.tap(find.byKey(const Key('add-session-ags_x')));
      await t.pumpAndSettle();
      expect(find.text('Add 1 session'), findsOneWidget);
      await t.tap(find.byKey(const Key('add-sessions-confirm')));
      await t.pumpAndSettle();
      expect((agents.session('ags_x')!['crew'] as Map)['id'], 'crw_1');
      expect(agents.session('ags_y')!['crew'], isNull);
      await disposeShell(t, c);
    });
  });

  testWidgets('5: on a phone a crew\'s long-press sheet has Add sessions…, '
      'as a sheet', (t) async {
    final agents = FakeAgentServer(
      hosts: [agentHost()],
      crews: [agentCrew('crw_1', name: 'Gateway')],
      sessions: [
        session('ags_m', crew: ('crw_1', 0)),
        session('ags_x'),
      ],
    );
    final c = shellContainer(agentClient: agents.client);
    await pumpShell(t, c, size: const Size(411, 900));
    c.read(routerProvider).go(Routes.agents);
    await t.pumpAndSettle();
    await t.longPress(find.byKey(const Key('crew-crw_1')));
    await t.pumpAndSettle();
    await t.tap(find.byKey(const Key('crew-action-addSessions')));
    await t.pumpAndSettle();
    expect(find.byType(BottomSheet), findsOneWidget);
    await t.tap(find.byKey(const Key('add-session-ags_x')));
    await t.pumpAndSettle();
    await t.tap(find.byKey(const Key('add-sessions-confirm')));
    await t.pumpAndSettle();
    expect((agents.session('ags_x')!['crew'] as Map)['id'], 'crw_1');
    await disposeShell(t, c);
  });

  group('the rail counts needs you, and only that', () {
    testWidgets('two waiting, one failed, one blocked: 2', (t) async {
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        sessions: [
          session('ags_a', attention: 'permission_prompt'),
          session('ags_b', attention: 'delete_approval'),
          session('ags_c', status: 'failed'),
          session('ags_d', attention: 'blocked'),
        ],
      );
      final c = await open(t, agents);
      final badge = find.byKey(const Key('rail-needs-count'));
      expect(
        find.descendant(of: badge, matching: find.text('2')),
        findsOneWidget,
      );
      final box = t.widget<Container>(badge).decoration! as BoxDecoration;
      expect(box.color, tokens(t).danger);
      expect(find.byTooltip('Agents, 2 need you'), findsOneWidget);

      agents.session('ags_a')!['needs_attention'] = null;
      agents.session('ags_b')!['needs_attention'] = null;
      c.invalidate(agentOverviewProvider);
      await t.pumpAndSettle();
      expect(badge, findsNothing);
      await disposeShell(t, c);
    });
  });

  group('⇧⌘N (153f)', () {
    Future<void> chord(WidgetTester t) async {
      await t.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
      await t.sendKeyDownEvent(LogicalKeyboardKey.shiftLeft);
      await t.sendKeyEvent(LogicalKeyboardKey.keyN);
      await t.sendKeyUpEvent(LogicalKeyboardKey.shiftLeft);
      await t.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
      await t.pumpAndSettle();
    }

    testWidgets('starts a session in Agents, and is New folder in Notes', (
      t,
    ) async {
      debugDefaultTargetPlatformOverride = TargetPlatform.linux;
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        sessions: [session('ags_1')],
      );
      final c = await open(t, agents);
      await chord(t);
      expect(find.byKey(const Key('launch')), findsOneWidget);
      await t.sendKeyEvent(LogicalKeyboardKey.escape);
      await t.pumpAndSettle();
      expect(find.byKey(const Key('launch')), findsNothing);

      await openVault(t, c);
      await chord(t);
      expect(find.byKey(const Key('launch')), findsNothing);
      expect(find.text('New folder'), findsOneWidget);
      await disposeShell(t, c);
      debugDefaultTargetPlatformOverride = null;
    });

    testWidgets('does nothing in Agents while no host is online', (t) async {
      debugDefaultTargetPlatformOverride = TargetPlatform.linux;
      final c = await open(
        t,
        FakeAgentServer(hosts: [agentHost(status: 'offline')]),
      );
      await chord(t);
      expect(find.byKey(const Key('launch')), findsNothing);
      await disposeShell(t, c);
      debugDefaultTargetPlatformOverride = null;
    });
  });
}
