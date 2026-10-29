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

/// Decision 144 in the client: a parent's About lists each child's latest
/// events, how many its agent has not read, and where a less-than-certain
/// signal came from; a child's activity reads "Unknown" until a gated
/// signal says more, and never as working.
void main() {
  const edge = Size(900, 700);
  const wide = Size(1400, 900);

  setUp(() => SharedPreferences.setMockInitialValues({}));

  Future<ProviderContainer> about(
    WidgetTester tester,
    FakeAgentServer agents,
    String id, {
    Size size = wide,
  }) async {
    final c = shellContainer(agentClient: agents.client);
    await pumpShell(tester, c, size: size);
    c
        .read(routerProvider)
        .go(Routes.agentSession(id, tab: SessionTab.about.name));
    await tester.pumpAndSettle();
    if (find.byKey(const Key('session-about')).evaluate().isEmpty) {
      await tester.tap(find.byKey(const Key('inspector-toggle')));
      await tester.pumpAndSettle();
    }
    expect(find.byKey(const Key('session-about')), findsOneWidget);
    return c;
  }

  const startedAt = '2026-10-27T10:00:00Z';
  const promptAt = '2026-10-27T10:04:00Z';

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
      ],
    ),
  ];

  for (final size in [wide, edge]) {
    testWidgets(
      'a parent sees each child’s events, unread count and evidence at '
      '${size.width.toInt()}px',
      (tester) async {
        final agents = FakeAgentServer(
          hosts: [agentHost()],
          sessions: family(),
        );
        FakeSpawning(agents);
        final c = await about(tester, agents, 'ags_p', size: size);
        final tile = find.byKey(const Key('child-events-ags_c'));
        await tester.ensureVisible(tile);
        expect(tile, findsOneWidget);
        expect(
          find.descendant(
            of: find.byKey(const Key('child-pending-ags_c')),
            matching: find.text('2 unread events'),
          ),
          findsOneWidget,
        );
        expect(
          find.text('Started · ${clockAt(DateTime.parse(startedAt))}'),
          findsOneWidget,
        );
        expect(
          find.text('Needs you · ${clockAt(DateTime.parse(promptAt))}'),
          findsOneWidget,
        );
        // Certain evidence carries no badge; a hook's word says so.
        expect(
          find.byKey(const Key('event-evidence-evt_ags_p_1')),
          findsNothing,
        );
        expect(
          find.descendant(
            of: find.byKey(const Key('event-evidence-evt_ags_p_2')),
            matching: find.text('agent hook · reported'),
          ),
          findsOneWidget,
        );
        expect(
          find.descendant(
            of: find.byKey(const Key('event-evidence-evt_ags_p_3')),
            matching: find.text('output · heuristic'),
          ),
          findsOneWidget,
        );
        expect(find.byKey(const Key('child-pending-ags_w')), findsNothing);
        await disposeShell(tester, c);
      },
    );
  }

  testWidgets('a child’s unknown activity reads Unknown, never working', (
    tester,
  ) async {
    final agents = FakeAgentServer(hosts: [agentHost()], sessions: family());
    FakeSpawning(agents);
    final c = await about(tester, agents, 'ags_p');
    final unknown = find.byKey(const Key('child-activity-ags_c'));
    await tester.ensureVisible(unknown);
    expect(
      find.descendant(of: unknown, matching: find.text('Unknown')),
      findsOneWidget,
    );
    expect(
      find.descendant(of: unknown, matching: find.text('Working')),
      findsNothing,
    );
    expect(
      find.descendant(
        of: find.byKey(const Key('child-activity-ags_w')),
        matching: find.text('Working'),
      ),
      findsOneWidget,
      reason: 'only a gated signal says working',
    );
    await disposeShell(tester, c);
  });

  testWidgets('a session nobody started shows no events', (tester) async {
    final agents = FakeAgentServer(
      hosts: [agentHost()],
      sessions: [
        spawnSession('ags_p', spawn: {'enabled': true}),
      ],
    );
    FakeSpawning(agents);
    final c = await about(tester, agents, 'ags_p');
    expect(find.byKey(const Key('child-events-ags_p')), findsNothing);
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
    expect(evidenceLabel(event('started')), isNull);

    final session = AgentSession.fromJson({
      ...agentSession('ags_x'),
      'activity': 'unknown',
    });
    expect(session.working, isFalse, reason: 'unknown is never working');
  });
}
