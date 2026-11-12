import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'package:storm/agent/agent_models.dart';
import 'package:storm/agent/agent_state.dart' show SessionTab;
import 'package:storm/agent/agent_widgets.dart' show clockAt;
import 'package:storm/agent/spawn_models.dart';
import 'package:storm/router.dart';

import 'agent_fakes.dart';
import 'shell_harness.dart';
import 'spawn_fakes.dart';

/// Decision 144 in the client, as the Agents tab (153i) shows it: each
/// child with its state in a word and why, never as working; since
/// decision 148 a child's denials in what its parent sent, and since 151 N5
/// its own prompt an ordinary permission prompt; a round's stats on the
/// entry that began it (153j).
void main() {
  const edge = Size(900, 700);
  const wide = Size(1400, 900);

  setUp(() => SharedPreferences.setMockInitialValues({}));

  Future<ProviderContainer> agentsTab(
    WidgetTester tester,
    FakeAgentServer agents,
    String id, {
    Size size = wide,
  }) async {
    final c = shellContainer(agentClient: agents.client);
    await pumpShell(tester, c, size: size);
    c
        .read(routerProvider)
        .go(Routes.agentSession(id, tab: SessionTab.agents.name));
    await tester.pumpAndSettle();
    if (find.byKey(const Key('agents-panel')).evaluate().isEmpty) {
      await tester.tap(find.byKey(const Key('inspector-toggle')));
      await tester.pumpAndSettle();
    }
    expect(find.byKey(const Key('agents-panel')), findsOneWidget);
    return c;
  }

  const startedAt = '2026-10-27T10:00:00Z';
  const promptAt = '2026-10-27T10:04:00Z';
  const deniedAt = '2026-10-27T10:05:00Z';

  List<Map<String, dynamic>> family() => [
    spawnSession('ags_p', spawn: {'enabled': true}),
    spawnSession(
      'ags_c',
      name: 'test-gateway',
      parent: 'ags_p',
      depth: 1,
      needsAttention: 'permission_prompt',
      childActivity: 'unknown',
      pendingEvents: 2,
      recentEvents: [
        childEvent(
          'ags_p',
          1,
          'ags_c',
          'started',
          source: 'host_process',
          at: startedAt,
        ),
        childEvent(
          'ags_p',
          2,
          'ags_c',
          'needs_attention',
          reason: 'permission_prompt',
          needsAttention: 'permission_prompt',
          source: 'provider_hook',
          confidence: 'reported',
          at: promptAt,
        ),
      ],
    ),
    spawnSession(
      'ags_w',
      name: 'spike-notes',
      parent: 'ags_p',
      depth: 1,
      childActivity: 'working',
      pendingEvents: 0,
      recentEvents: [
        childEvent(
          'ags_p',
          3,
          'ags_w',
          'idle_timeout',
          activity: 'idle',
          source: 'output_pattern',
          confidence: 'heuristic',
          at: promptAt,
        ),
        childEvent(
          'ags_p',
          4,
          'ags_w',
          'denied',
          tool: 'Bash',
          detail: 'curl https://example.com/[x](y)',
          count: 3,
          source: 'provider_hook',
          confidence: 'reported',
          at: deniedAt,
        ),
      ],
    ),
  ];

  for (final size in [wide, edge]) {
    testWidgets('a parent sees each child’s state and why at '
        '${size.width.toInt()}px', (tester) async {
      final agents = FakeAgentServer(hosts: [agentHost()], sessions: family());
      FakeSpawning(agents);
      final c = await agentsTab(tester, agents, 'ags_p', size: size);
      expect(
        find.descendant(
          of: find.byKey(const Key('agent-child-tag-ags_c')),
          matching: find.text('Needs you'),
        ),
        findsOneWidget,
      );
      expect(
        tester
            .widget<Text>(find.byKey(const Key('agent-child-reason-ags_c')))
            .data,
        startsWith('permission prompt'),
      );
      expect(
        find.descendant(
          of: find.byKey(const Key('agent-child-tag-ags_w')),
          matching: find.text('Running'),
        ),
        findsOneWidget,
      );
      await disposeShell(tester, c);
    });
  }

  testWidgets('A-148-2: what the parent sent lists its children’s denials', (
    tester,
  ) async {
    final agents = FakeAgentServer(hosts: [agentHost()], sessions: family());
    FakeSpawning(agents);
    final c = await agentsTab(tester, agents, 'ags_p');
    final row = find.byKey(const Key('act-denied-evt_ags_p_4'));
    await tester.ensureVisible(row);
    expect(row, findsOneWidget);
    expect(
      find.descendant(of: row, matching: find.text('DENIED spike-notes')),
      findsOneWidget,
    );
    expect(
      find.descendant(
        of: row,
        matching: find.text(
          '${clockAt(DateTime.parse(deniedAt))} · Denied Bash ×3',
        ),
      ),
      findsOneWidget,
    );
    expect(
      find.descendant(
        of: row,
        matching: find.text('curl https://example.com/[x](y)'),
      ),
      findsOneWidget,
      reason: 'the detail is the child’s own text, shown as it is',
    );
    expect(find.byKey(const Key('agents-empty')), findsNothing);
    await disposeShell(tester, c);
  });

  testWidgets('a child is never said to be working', (tester) async {
    final agents = FakeAgentServer(hosts: [agentHost()], sessions: family());
    FakeSpawning(agents);
    final c = await agentsTab(tester, agents, 'ags_p');
    final panel = find.byKey(const Key('agents-panel'));
    expect(
      find.descendant(of: panel, matching: find.textContaining('orking')),
      findsNothing,
    );
    await disposeShell(tester, c);
  });

  testWidgets('a session that started none says so', (tester) async {
    final agents = FakeAgentServer(
      hosts: [agentHost()],
      sessions: [
        spawnSession('ags_p', spawn: {'enabled': true}),
      ],
    );
    FakeSpawning(agents);
    final c = await agentsTab(tester, agents, 'ags_p');
    expect(find.byKey(const Key('agents-empty')), findsOneWidget);
    expect(find.text('STARTED BY THIS SESSION'), findsNothing);
    await disposeShell(tester, c);
  });

  testWidgets('a round shows what it took on the entry that began it, '
      'tokens included (145f, 153j)', (tester) async {
    final agents = FakeAgentServer(
      hosts: [agentHost()],
      sessions: [
        spawnSession('ags_p', spawn: {'enabled': true}),
        spawnSession(
          'ags_c',
          name: 'counter',
          parent: 'ags_p',
          depth: 1,
          recentEvents: [
            childEvent(
              'ags_p',
              1,
              'ags_c',
              'reported',
              reason: 'succeeded',
              round: 2,
              durationMs: 14200,
              usage: {
                'input_tokens': 10,
                'cache_read_tokens': 12000,
                'cache_write_tokens': 300,
                'output_tokens': 456,
              },
            ),
          ],
        ),
      ],
    );
    final spawning = FakeSpawning(agents);
    spawning.addAct(
      'ags_p',
      name: 'counter',
      session: 'ags_c',
      task: 'Count the lines.',
    );
    final message = spawning.addAct(
      'ags_p',
      kind: 'message',
      target: 'ags_c',
      text: 'Now count the words.',
      round: 2,
      outcome: 'delivered',
    );
    final c = await agentsTab(tester, agents, 'ags_p');
    final id = message['id'];
    expect(find.text('MESSAGE counter'), findsOneWidget);
    expect(
      find.descendant(
        of: find.byKey(Key('act-text-$id')),
        matching: find.text('Now count the words.'),
      ),
      findsOneWidget,
    );
    expect(
      tester.widget<Text>(find.byKey(Key('act-round-$id'))).data,
      'round 2 · 14s · 12k in · 456 out',
    );
    // The start has no round-1 event, so no round line of its own.
    expect(find.byKey(const Key('act-round-agr_1')), findsNothing);
    await disposeShell(tester, c);
  });

  test('the view’s events parse, and every label is plain', () {
    final facts = SpawnFacts.fromJson(family()[1]);
    expect(facts.childActivity, 'unknown');
    expect(facts.pendingEvents, 2);
    expect(facts.recentEvents.map((e) => e.kind), [
      'started',
      'needs_attention',
    ]);
    final prompt = facts.recentEvents[1];
    expect(prompt.id, 'evt_ags_p_2');
    expect(prompt.seq, 2);
    expect(prompt.needsAttention, 'permission_prompt');
    expect(prompt.certain, isFalse);
    expect(SpawnFacts.fromJson(family()[0]).recentEvents, isEmpty);

    expect(activityLabel('unknown'), 'Unknown');
    expect(activityLabel('something new'), 'Unknown');
    expect(activityLabel('working'), 'Working');
    expect(activityLabel('idle'), 'Idle');
    expect(activityLabel(null), isNull);
    expect(attentionLabel('usage_limit'), 'Out of usage');

    ChildEvent event(String kind, {String? reason}) => ChildEvent.fromJson(
      childEvent('ags_p', 1, 'ags_c', kind, reason: reason),
    );
    expect(
      childEventLabel(event('reported', reason: 'blocked')),
      'Reported: blocked',
    );
    expect(
      childEventLabel(event('needs_attention', reason: 'blocked')),
      'Blocked',
    );
    expect(
      childEventLabel(event('failed', reason: 'start_failure')),
      'Failed: start_failure',
    );
    expect(childEventLabel(event('unknown')), 'Lost touch');
    expect(childEventLabel(event('usage_limit')), 'Out of usage');
    expect(childEventLabel(event('parked')), 'Parked until its next message');
    expect(
      childEventLabel(event('needs_attention', reason: 'permission_prompt')),
      'Permission prompt',
    );
    final denied = ChildEvent.fromJson(
      childEvent('ags_p', 2, 'ags_c', 'denied', tool: 'Write', detail: '/x'),
    );
    expect(childEventLabel(denied), 'Denied Write');
    expect(denied.count, 1);
    expect(denied.detail, '/x');
    expect(evidenceLabel(event('started')), isNull);
    expect(roundLabel(event('started')), isNull);
    final noUsage = ChildEvent.fromJson(
      childEvent('ags_p', 3, 'ags_c', 'ended', round: 1, durationMs: 125000),
    );
    expect(roundLabel(noUsage), 'Round 1 · 2m 5s', reason: 'never guessed');

    final session = AgentSession.fromJson({
      ...agentSession('ags_x'),
      'activity': 'unknown',
    });
    expect(session.working, isFalse, reason: 'unknown is never working');
  });
}
