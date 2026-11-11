import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'package:storm/agent/agent_state.dart';
import 'package:storm/agent/session_screen.dart' show terminalHint;
import 'package:storm/router.dart';
import 'package:storm/ui/tokens.dart';

import 'agent_fakes.dart';
import 'shell_harness.dart';
import 'spawn_fakes.dart';

/// Decision 153i and 153j: the session inspector's icon rail, the Agents
/// tab, About slimmed down, and the terminal's hint, on both sides of the
/// 900px breakpoint.
void main() {
  const wide = Size(1400, 900);
  const phone = Size(411, 900);

  setUp(() => SharedPreferences.setMockInitialValues({}));

  Future<ProviderContainer> open(
    WidgetTester tester,
    FakeAgentServer agents, {
    Size size = wide,
    required String at,
  }) async {
    final c = shellContainer(agentClient: agents.client);
    await pumpShell(tester, c, size: size);
    c.read(routerProvider).go(at);
    await tester.pumpAndSettle();
    return c;
  }

  Uri where(ProviderContainer c) => c.read(routerProvider).state.uri;

  Finder rail(String tab) => find.byKey(Key('inspector-rail-$tab'));
  Finder badge(String tab) => find.byKey(Key('inspector-badge-$tab'));

  BoxDecoration decoration(WidgetTester tester, Finder f) =>
      tester.widget<Container>(f).decoration! as BoxDecoration;

  /// A parent that started three children: one waits on its permission
  /// prompt, one is parked, one ended with two commits to apply.
  FakeAgentServer family({int wrote = 2}) {
    final agents = FakeAgentServer(
      hosts: [agentHost()],
      sessions: [
        {
          ...spawnSession('ags_p', spawn: {'enabled': true}),
          'wrote_count': wrote,
        },
        spawnSession(
          'ags_a',
          name: 'test-gateway',
          parent: 'ags_p',
          depth: 1,
          needsAttention: 'permission_prompt',
        ),
        spawnSession(
          'ags_b',
          name: 'bench-check',
          parent: 'ags_p',
          depth: 1,
          status: 'parked',
        ),
        {
          ...spawnSession(
            'ags_c',
            name: 'spike-notes',
            parent: 'ags_p',
            depth: 1,
            status: 'completed',
          ),
          'work_to_take': true,
          'unapplied_commits': 2,
        },
      ],
    );
    return agents;
  }

  group('the rail', () {
    testWidgets('Context, Wrote, Agents and About, each with its name', (
      tester,
    ) async {
      final agents = family();
      FakeSpawning(agents);
      final c = await open(tester, agents, at: Routes.agentSession('ags_p'));
      for (final (tab, name) in [
        ('context', 'Context'),
        ('wrote', 'Wrote'),
        ('agents', 'Agents'),
        ('about', 'About'),
      ]) {
        expect(rail(tab), findsOneWidget, reason: tab);
        expect(
          find.ancestor(
            of: rail(tab),
            matching: find.byWidgetPredicate(
              (w) => w is Tooltip && w.message == name,
            ),
          ),
          findsOneWidget,
          reason: tab,
        );
      }
      expect(rail('work'), findsNothing, reason: 'not a sandbox child');
      // The rail sits on the inspector's left edge, 48px wide.
      final r = tester.getRect(find.byKey(const Key('inspector-rail')));
      expect(r.width, 48);
      expect(
        r.right,
        lessThanOrEqualTo(
          tester.getRect(find.byKey(const Key('inspector-panel'))).left,
        ),
      );
      await disposeShell(tester, c);
    });

    testWidgets('Work only for a child with its own workspace (151 N6)', (
      tester,
    ) async {
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        sessions: [
          spawnSession('ags_p', spawn: {'enabled': true}),
          spawnSession('ags_own', name: 'own', parent: 'ags_p', depth: 1),
          spawnSession(
            'ags_shared',
            name: 'shared',
            parent: 'ags_p',
            depth: 1,
            ownWorkspace: false,
          ),
        ],
      );
      FakeSpawning(agents);
      final c = await open(tester, agents, at: Routes.agentSession('ags_own'));
      expect(rail('work'), findsOneWidget);
      // A child starts none, so it has no Agents.
      expect(rail('agents'), findsNothing);
      c.read(routerProvider).go(Routes.agentSession('ags_shared'));
      await tester.pumpAndSettle();
      expect(rail('work'), findsNothing);
      await disposeShell(tester, c);
    });

    testWidgets('the active icon is accent on accentSoft; clicking it closes '
        'the panel and the rail and counts stay', (tester) async {
      final agents = family();
      FakeSpawning(agents);
      final c = await open(
        tester,
        agents,
        at: Routes.agentSession('ags_p', tab: SessionTab.agents.name),
      );
      final t = tester.element(rail('agents')).tokens;
      Material fill(String tab) => tester.widget<Material>(
        find.ancestor(of: rail(tab), matching: find.byType(Material)).first,
      );
      Icon icon(String tab) => tester.widget<Icon>(
        find.descendant(of: rail(tab), matching: find.byType(Icon)),
      );
      expect(fill('agents').color, t.accentSoft);
      expect(icon('agents').color, t.accent);
      expect(fill('about').color, Colors.transparent);
      expect(find.byKey(const Key('agents-panel')), findsOneWidget);
      final before = tester.getSize(find.byKey(const Key('terminal-hint')));

      await tester.tap(rail('agents'));
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('inspector-panel')), findsNothing);
      expect(find.byKey(const Key('inspector-rail')), findsOneWidget);
      expect(badge('agents'), findsOneWidget);
      expect(badge('wrote'), findsOneWidget);
      expect(fill('agents').color, Colors.transparent);
      expect(
        tester.getSize(find.byKey(const Key('terminal-hint'))).width,
        greaterThan(before.width),
        reason: 'the terminal takes the panel’s width',
      );

      // Another icon opens its own tab.
      await tester.tap(rail('about'));
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('session-about')), findsOneWidget);
      expect(where(c).queryParameters['tab'], 'about');
      expect(
        tester.widget<Text>(find.byKey(const Key('inspector-title'))).data,
        'About',
      );
      // The header's Details toggle still closes and opens it.
      await tester.tap(find.byKey(const Key('inspector-toggle')));
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('inspector-panel')), findsNothing);
      await tester.tap(find.byKey(const Key('inspector-toggle')));
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('session-about')), findsOneWidget);
      await disposeShell(tester, c);
    });

    testWidgets('Agents counts in danger when a child needs you; Wrote in '
        'surface2 and text2', (tester) async {
      final agents = family(wrote: 3);
      FakeSpawning(agents);
      final c = await open(tester, agents, at: Routes.agentSession('ags_p'));
      final t = tester.element(rail('agents')).tokens;
      expect(
        find.descendant(of: badge('agents'), matching: find.text('1')),
        findsOneWidget,
      );
      expect(decoration(tester, badge('agents')).color, t.danger);
      expect(
        find.descendant(of: badge('wrote'), matching: find.text('3')),
        findsOneWidget,
      );
      expect(decoration(tester, badge('wrote')).color, t.surface2);
      expect(
        tester
            .widget<Text>(
              find.descendant(of: badge('wrote'), matching: find.text('3')),
            )
            .style!
            .color,
        t.text2,
      );
      expect(tester.getSize(badge('wrote')).height, 15);
      // With nobody waiting, the live children are a quiet count.
      agents.session('ags_a')!['needs_attention'] = null;
      c.invalidate(agentOverviewProvider);
      await tester.pumpAndSettle();
      expect(decoration(tester, badge('agents')).color, t.surface2);
      await disposeShell(tester, c);
    });

    testWidgets('an old Activity link opens the Agents tab', (tester) async {
      final agents = family();
      FakeSpawning(agents);
      final c = await open(
        tester,
        agents,
        at: Routes.agentSession('ags_p', tab: 'requests'),
      );
      expect(find.byKey(const Key('agents-panel')), findsOneWidget);
      await disposeShell(tester, c);
    });
  });

  group('the Agents tab', () {
    testWidgets('each child with its state in a word, why, and Open or '
        'Work', (tester) async {
      final agents = family();
      FakeSpawning(agents);
      final c = await open(
        tester,
        agents,
        at: Routes.agentSession('ags_p', tab: SessionTab.agents.name),
      );
      final t = tester.element(rail('agents')).tokens;
      expect(find.text('STARTED BY THIS SESSION'), findsOneWidget);
      void tag(String id, String word, Color color) {
        final f = find.byKey(Key('agent-child-tag-$id'));
        expect(
          find.descendant(of: f, matching: find.text(word)),
          findsOneWidget,
          reason: id,
        );
        expect(
          tester
              .widget<Text>(find.descendant(of: f, matching: find.text(word)))
              .style!
              .color,
          color,
          reason: id,
        );
      }

      tag('ags_a', 'Needs you', t.danger);
      tag('ags_b', 'Parked', t.text3);
      tag('ags_c', 'Ended', t.text3);
      String reason(String id) =>
          tester.widget<Text>(find.byKey(Key('agent-child-reason-$id'))).data!;
      expect(reason('ags_a'), startsWith('permission prompt'));
      expect(reason('ags_b'), 'until its next message');
      expect(reason('ags_c'), '2 commits to apply');
      Finder link(String id, String word) => find.descendant(
        of: find.byKey(Key('agent-child-open-$id')),
        matching: find.text(word),
      );
      expect(link('ags_a', 'Open'), findsOneWidget);
      expect(link('ags_b', 'Open'), findsOneWidget);
      expect(link('ags_c', 'Work'), findsOneWidget);

      await tester.tap(find.byKey(const Key('agent-child-open-ags_c')));
      await tester.pumpAndSettle();
      expect(where(c).path, Routes.agentSession('ags_c'));
      expect(where(c).queryParameters['tab'], SessionTab.work.name);
      await disposeShell(tester, c);
    });

    testWidgets('End all of them… opens the end-children strip', (
      tester,
    ) async {
      final agents = family();
      final spawning = FakeSpawning(agents);
      final c = await open(
        tester,
        agents,
        at: Routes.agentSession('ags_p', tab: SessionTab.agents.name),
      );
      final end = find.byKey(const Key('end-children'));
      await tester.ensureVisible(end);
      await tester.tap(end);
      await tester.pumpAndSettle();
      final strip = find.byKey(const Key('end-children-confirm'));
      expect(strip, findsOneWidget);
      expect(
        tester.getTopLeft(strip).dy,
        lessThan(tester.getTopLeft(find.byKey(const Key('agents-panel'))).dy),
        reason: 'in the strip under the header',
      );
      expect(spawning.ends, isEmpty);
      await disposeShell(tester, c);
    });

    testWidgets('what it sent, quoted exactly, with the round it began', (
      tester,
    ) async {
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        sessions: [
          spawnSession('ags_p', spawn: {'enabled': true}),
          spawnSession(
            'ags_a',
            name: 'test-gateway',
            parent: 'ags_p',
            depth: 1,
            recentEvents: [
              childEvent(
                'ags_p',
                1,
                'ags_a',
                'reported',
                reason: 'succeeded',
                round: 1,
                durationMs: 9000,
              ),
            ],
          ),
        ],
      );
      final spawning = FakeSpawning(agents);
      final start = spawning.addAct(
        'ags_p',
        name: 'test-gateway',
        session: 'ags_a',
        agent: 'claude-code',
        task: 'Write tests for token refresh in gateway/oauth.rs.',
      );
      final message = spawning.addAct(
        'ags_p',
        kind: 'message',
        target: 'ags_a',
        text: 'Also cover the expired-token path.',
        round: 2,
        outcome: 'delivered',
      );
      final end = spawning.addAct(
        'ags_p',
        kind: 'end',
        target: 'ags_a',
        outcome: 'ended',
      );
      final c = await open(
        tester,
        agents,
        at: Routes.agentSession('ags_p', tab: SessionTab.agents.name),
      );
      expect(find.text('WHAT IT SENT THEM'), findsOneWidget);
      expect(find.text('START test-gateway'), findsOneWidget);
      expect(find.text('MESSAGE test-gateway'), findsOneWidget);
      expect(find.text('END test-gateway'), findsOneWidget);
      // Oldest first, as it happened.
      double top(String text) => tester.getTopLeft(find.text(text)).dy;
      expect(top('START test-gateway'), lessThan(top('MESSAGE test-gateway')));
      expect(top('MESSAGE test-gateway'), lessThan(top('END test-gateway')));
      String text(Map<String, dynamic> act) => tester
          .widget<Text>(
            find.descendant(
              of: find.byKey(Key('act-text-${act['id']}')),
              matching: find.byType(Text),
            ),
          )
          .data!;
      expect(text(start), 'Write tests for token refresh in gateway/oauth.rs.');
      expect(text(message), 'Also cover the expired-token path.');
      expect(find.byKey(Key('act-text-${end['id']}')), findsNothing);
      String? round(Map<String, dynamic> act) {
        final f = find.byKey(Key('act-round-${act['id']}'));
        return f.evaluate().isEmpty ? null : tester.widget<Text>(f).data;
      }

      expect(round(start), 'round 1 · 9s');
      expect(round(message), 'round 2');
      expect(round(end), isNull);
      final t = tester.element(rail('agents')).tokens;
      final line = tester.widget<Text>(
        find.byKey(Key('act-round-${message['id']}')),
      );
      expect(line.style!.color, t.text3);
      expect(line.style!.fontFamily, StormTokens.monoFamily);
      await disposeShell(tester, c);
    });

    testWidgets('a session that can start agents but has not says so', (
      tester,
    ) async {
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        sessions: [
          spawnSession('ags_p', spawn: {'enabled': true}),
        ],
      );
      FakeSpawning(agents);
      final c = await open(
        tester,
        agents,
        at: Routes.agentSession('ags_p', tab: SessionTab.agents.name),
      );
      expect(rail('agents'), findsOneWidget);
      expect(badge('agents'), findsNothing);
      expect(
        find.text(
          'It hasn’t started any agents. Each one it starts shows here, '
          'with what it sent.',
        ),
        findsOneWidget,
      );
      await disposeShell(tester, c);
    });

    testWidgets('a session that cannot start agents has no Agents tab', (
      tester,
    ) async {
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        sessions: [agentSession('ags_1')],
      );
      final c = await open(
        tester,
        agents,
        at: Routes.agentSession('ags_1', tab: SessionTab.agents.name),
      );
      expect(rail('agents'), findsNothing);
      // The route's tab falls back to About.
      expect(find.byKey(const Key('session-about')), findsOneWidget);
      await disposeShell(tester, c);
    });
  });

  group('About, slimmed down', () {
    testWidgets('exactly Crew, Role, Starts agents, Ran as and Started by, '
        'and a plugin problem in one line', (tester) async {
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        sessions: [
          {
            ...spawnSession(
              'ags_p',
              spawn: {'enabled': true, 'launched_on': true},
              writeVaultId: 'v-work',
            ),
            'mcp_skipped': [
              {
                'name': 'figma',
                'source': 'plugin figma',
                'reason': 'it needs its address',
              },
            ],
          },
        ],
      );
      FakeSpawning(agents);
      final c = await open(
        tester,
        agents,
        at: Routes.agentSession('ags_p', tab: SessionTab.about.name),
      );
      final about = find.byKey(const Key('session-about'));
      final said = [
        for (final w in tester.widgetList<Text>(
          find.descendant(of: about, matching: find.byType(Text)),
        ))
          w.data ?? '',
      ];
      expect(
        [
          for (final x in said)
            if (RegExp(r'^[A-Z][A-Z ]+$').hasMatch(x)) x,
        ],
        ['CREW', 'ROLE', 'STARTS AGENTS', 'RAN AS', 'STARTED BY'],
      );
      expect(said, contains('figma isn’t loaded: it needs its address.'));
      expect(said, contains('Fix in Integrations ›'));
      for (final gone in [
        'Never deletes',
        'Vault access',
        'Identity',
        'Isolation',
        'Plugin servers',
        'Integrations',
        'Network',
      ]) {
        expect(
          said.where((x) => x.toLowerCase() == gone.toLowerCase()),
          isEmpty,
          reason: gone,
        );
      }
      expect(said.where((x) => x.contains('Never deletes')), isEmpty);
      await tester.tap(find.byKey(const Key('fix-in-integrations')));
      await tester.pumpAndSettle();
      expect(where(c).path, Routes.integrations);
      await disposeShell(tester, c);
    });
  });

  group('the terminal', () {
    testWidgets('says how to leave it, in text3 at the 12px step', (
      tester,
    ) async {
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        sessions: [agentSession('ags_1')],
      );
      final c = await open(tester, agents, at: Routes.agentSession('ags_1'));
      final hint = tester.widget<Text>(find.byKey(const Key('terminal-hint')));
      expect(
        hint.data,
        'Tab and Shift+Tab go to the agent · Esc, then Tab, leaves the '
        'terminal',
      );
      final t = tester.element(find.byKey(const Key('terminal-hint'))).tokens;
      expect(hint.style!.color, t.text3);
      expect(hint.style!.fontSize, t.codeSize);
      // Under the terminal.
      expect(
        tester.getTopLeft(find.byKey(const Key('terminal-hint'))).dy,
        greaterThan(tester.getTopLeft(rail('context')).dy),
      );
      await disposeShell(tester, c);
    });

    test('macOS writes Shift as ⇧', () {
      debugDefaultTargetPlatformOverride = TargetPlatform.macOS;
      try {
        expect(
          terminalHint,
          'Tab and ⇧Tab go to the agent · Esc, then Tab, leaves the terminal',
        );
      } finally {
        debugDefaultTargetPlatformOverride = null;
      }
    });

    testWidgets('an ended session has no hint', (tester) async {
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        sessions: [agentSession('ags_1', status: 'completed', exitCode: 0)],
      );
      final c = await open(tester, agents, at: Routes.agentSession('ags_1'));
      expect(find.byKey(const Key('terminal-hint')), findsNothing);
      await disposeShell(tester, c);
    });
  });

  group('phone', () {
    testWidgets('no rail; the details sheet has the same Agents content', (
      tester,
    ) async {
      final agents = family();
      final spawning = FakeSpawning(agents);
      spawning.addAct(
        'ags_p',
        kind: 'message',
        target: 'ags_a',
        text: 'Also cover the expired-token path.',
        round: 2,
        outcome: 'delivered',
      );
      final c = await open(
        tester,
        agents,
        size: phone,
        at: Routes.agentSession('ags_p'),
      );
      expect(find.byKey(const Key('inspector-rail')), findsNothing);
      expect(find.byKey(const Key('terminal-hint')), findsNothing);
      await tester.tap(find.byKey(const Key('session-header')));
      await tester.pumpAndSettle();
      final sheet = find.byKey(const Key('details-agents'));
      expect(sheet, findsOneWidget);
      Finder inSheet(String text) =>
          find.descendant(of: sheet, matching: find.text(text));
      expect(inSheet('STARTED BY THIS SESSION'), findsOneWidget);
      expect(inSheet('Needs you'), findsOneWidget);
      expect(inSheet('Parked'), findsOneWidget);
      expect(inSheet('Work'), findsOneWidget);
      expect(inSheet('WHAT IT SENT THEM'), findsOneWidget);
      expect(inSheet('Also cover the expired-token path.'), findsOneWidget);
      expect(find.text('ACTIVITY'), findsNothing);
      expect(find.text('CHILDREN'), findsNothing);
      // End all of them asks in place on a phone.
      final end = find.byKey(const Key('end-children'));
      await tester.ensureVisible(end);
      await tester.pumpAndSettle();
      await tester.tap(end);
      await tester.pumpAndSettle();
      expect(
        find.descendant(
          of: sheet,
          matching: find.byKey(const Key('end-children-confirm')),
        ),
        findsOneWidget,
      );
      // About is slimmed the same way.
      expect(find.text('STARTS AGENTS'), findsOneWidget);
      expect(find.text('VAULT ACCESS'), findsNothing);
      await disposeShell(tester, c);
    });
  });
}
