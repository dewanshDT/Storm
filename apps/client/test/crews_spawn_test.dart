import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'package:storm/agent/agent_state.dart';
import 'package:storm/agent/crew_widgets.dart';
import 'package:storm/agent/spawn_widgets.dart';
import 'package:storm/router.dart';

import 'agent_fakes.dart';
import 'shell_harness.dart';
import 'spawn_fakes.dart';

/// Where crews (decision 138) meet spawning (134-10): End all running sends
/// the descendants the person was shown, Undo after a tree dismissal brings
/// back every crew it emptied, a child in a crew reads as one, and none of
/// spawning shows for a crew on a host that cannot offer it.
void main() {
  const phone = Size(899, 900);
  const wide = Size(1400, 900);

  setUp(() => SharedPreferences.setMockInitialValues({}));

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

  Finder within(String key, Finder f) =>
      find.descendant(of: find.byKey(Key(key)), matching: f);

  Map<String, dynamic> inCrew(Map<String, dynamic> s, String crew, int order) =>
      {
        ...s,
        'crew': {'id': crew, 'order': order},
      };

  group('End all running reaches children (134-7 C1, 4r)', () {
    for (final size in [wide, phone]) {
      testWidgets('it lists them and sends them at ${size.width.toInt()}px', (
        t,
      ) async {
        final agents = FakeAgentServer(
          hosts: [agentHost()],
          crews: [agentCrew('crw_1')],
          sessions: [
            inCrew(spawnSession('ags_p', name: 'gateway-impl'), 'crw_1', 0),
            spawnSession(
              'ags_c',
              name: 'test-gateway',
              parent: 'ags_p',
              depth: 1,
            ),
            inCrew(
              spawnSession(
                'ags_c2',
                name: 'docs-pass',
                parent: 'ags_p',
                depth: 1,
              ),
              'crw_1',
              1,
            ),
          ],
        );
        final spawning = FakeSpawning(agents);
        final c = await open(
          t,
          agents,
          size: size,
          at: Routes.agentCrew('crw_1'),
        );
        await t.tap(find.byKey(const Key('crew-page-menu')));
        await t.pumpAndSettle();
        await t.tap(find.byKey(const Key('crew-action-endAll')));
        await t.pumpAndSettle();

        expect(find.byKey(const Key('end-all-reaches')), findsOneWidget);
        if (size == wide) {
          expect(
            within(
              'end-all-reaches',
              find.text('test-gateway · started by gateway-impl'),
            ),
            findsOneWidget,
          );
        } else {
          expect(
            within('end-all-reaches', find.text('test-gateway')),
            findsOneWidget,
          );
          expect(
            within('end-all-reaches', find.text('started by gateway-impl')),
            findsOneWidget,
          );
        }
        // docs-pass is a stop of its own, not listed twice.
        expect(
          within('end-all-reaches', find.textContaining('docs-pass')),
          findsNothing,
        );
        expect(find.text('End 2 sessions'), findsOneWidget);

        await t.ensureVisible(find.byKey(const Key('end-all-confirm')));
        await t.pumpAndSettle();
        await t.tap(find.byKey(const Key('end-all-confirm')));
        await t.pumpAndSettle();

        // One end, with the list the person was shown; docs-pass ends with
        // its parent and is not sent again.
        expect(spawning.ends, hasLength(1));
        expect(spawning.ends.single.$1, 'end:ags_p');
        expect(spawning.ends.single.$2, {
          'descendants': ['ags_c', 'ags_c2'],
          'report_first': false,
        });
        for (final id in ['ags_p', 'ags_c', 'ags_c2']) {
          expect(agents.session(id)!['status'], 'stopped', reason: id);
        }
        expect(find.textContaining("Couldn't end"), findsNothing);
        expect(spawning.terminalWrites, 0);
        await disposeShell(t, c);
      });
    }

    testWidgets('a crew with no children asks for no descendants', (t) async {
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        crews: [agentCrew('crw_1')],
        sessions: [
          agentSession('ags_1', crew: ('crw_1', 0)),
          agentSession('ags_2', name: 'review', crew: ('crw_1', 1)),
        ],
      );
      final c = await open(t, agents, at: Routes.agentCrew('crw_1'));
      await t.tap(find.byKey(const Key('crew-page-menu')));
      await t.pumpAndSettle();
      await t.tap(find.byKey(const Key('crew-action-endAll')));
      await t.pumpAndSettle();
      expect(find.byKey(const Key('end-all-reaches')), findsNothing);
      await t.tap(find.byKey(const Key('end-all-confirm')));
      await t.pumpAndSettle();
      expect(agents.count('POST', '/v1/agent/sessions/ags_1/end'), 1);
      expect(agents.count('POST', '/v1/agent/sessions/ags_2/end'), 1);
      expect(
        agents.requests.where((r) => r.url.path.endsWith('/descendants')),
        isEmpty,
      );
      await disposeShell(t, c);
    });
  });

  testWidgets('Undo after a tree dismissal brings back every crew it emptied '
      '(138c with C8)', (t) async {
    final agents = FakeAgentServer(
      hosts: [agentHost()],
      crews: [
        agentCrew('crw_1'),
        agentCrew('crw_2', name: 'Docs', color: 'clay', order: 1),
        agentCrew('crw_3', name: 'Relay hardening', color: 'sky', order: 2),
      ],
      sessions: [
        inCrew(spawnSession('ags_p', status: 'completed'), 'crw_1', 0),
        inCrew(
          spawnSession(
            'ags_c',
            name: 'test-gateway',
            parent: 'ags_p',
            depth: 1,
            status: 'completed',
          ),
          'crw_2',
          0,
        ),
        agentSession('ags_x', name: 'relay-probe', crew: ('crw_3', 0)),
      ],
    );
    FakeSpawning(agents);
    final c = await open(t, agents, at: Routes.agentSession('ags_c'));
    await t.tap(find.byKey(const Key('dismiss-session')));
    await t.pumpAndSettle();
    expect(find.text(kTreeDismissWarning), findsOneWidget);
    await t.tap(find.text('Dismiss 2 sessions'));
    await t.pumpAndSettle();
    expect(agents.crews.map((c) => c['id']), ['crw_3']);
    expect(
      find.text(
        'Gateway release and Docs were removed: their last sessions were '
        'dismissed.',
      ),
      findsOneWidget,
    );

    await t.tap(find.text('Undo'));
    await t.pumpAndSettle();
    final made = [
      for (final r in agents.requests)
        if (r.method == 'POST' && r.url.path == '/v1/agent/crews')
          jsonDecode(r.body),
    ];
    expect(made, [
      {'name': 'Gateway release', 'color': 'sage', 'session_ids': <String>[]},
      {'name': 'Docs', 'color': 'clay', 'session_ids': <String>[]},
    ]);
    final crews = [...agents.crews]
      ..sort((a, b) => (a['order'] as int).compareTo(b['order'] as int));
    expect(crews.map((c) => c['name']), [
      'Gateway release',
      'Docs',
      'Relay hardening',
    ]);
    await disposeShell(t, c);
  });

  group('a child in a crew', () {
    for (final size in [wide, phone]) {
      testWidgets(
        'keeps its lineage line and role at ${size.width.toInt()}px',
        (t) async {
          final agents = FakeAgentServer(
            hosts: [agentHost()],
            crews: [agentCrew('crw_1')],
            sessions: [
              spawnSession('ags_p', name: 'gateway-impl'),
              {
                ...inCrew(
                  spawnSession(
                    'ags_c',
                    name: 'test-gateway',
                    parent: 'ags_p',
                    depth: 1,
                    childWorkspace: {'mode': 'clone', 'commit': 'abc1234'},
                  ),
                  'crw_1',
                  0,
                ),
                'role': roleRef('k_rev', 'Storm Reviewer'),
              },
            ],
          );
          FakeSpawning(agents);
          final c = await open(t, agents, size: size);
          final row = find.byKey(const Key('session-ags_c'));
          expect(row, findsOneWidget);
          expect(
            find.descendant(
              of: row,
              matching: find.text('↳ from gateway-impl · own copy of storm'),
            ),
            findsOneWidget,
          );
          expect(
            find.descendant(of: row, matching: find.byType(RoleTag)),
            findsOneWidget,
          );
          // Under its crew, on the crew's solid edge, not the lineage indent.
          expect(
            find.ancestor(of: row, matching: find.byType(LineageIndent)),
            findsNothing,
          );
          expect(
            find.ancestor(of: row, matching: find.byType(CrewMembers)),
            findsOneWidget,
          );
          await disposeShell(t, c);
        },
      );
    }

    testWidgets('on the crew page its sub-line names its own copy, not '
        'children/<id>', (t) async {
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        crews: [agentCrew('crw_1')],
        sessions: [
          spawnSession('ags_p', name: 'gateway-impl'),
          inCrew(
            spawnSession(
              'ags_c',
              name: 'test-gateway',
              parent: 'ags_p',
              depth: 1,
              childWorkspace: {'mode': 'clone', 'commit': 'abc1234'},
            ),
            'crw_1',
            0,
          ),
        ],
      );
      FakeSpawning(agents);
      final c = await open(t, agents, at: Routes.agentCrew('crw_1'));
      final row = find.byKey(const Key('crew-row-ags_c'));
      expect(row, findsOneWidget);
      expect(
        find.descendant(of: row, matching: find.textContaining('own copy')),
        findsOneWidget,
      );
      expect(find.textContaining('children/ags_c'), findsNothing);
      await disposeShell(t, c);
    });
  });

  group('a crew on a host that cannot offer spawning shows none of it', () {
    for (final size in [wide, phone]) {
      testWidgets('at ${size.width.toInt()}px', (t) async {
        final agents = FakeAgentServer(
          hosts: [agentHost()],
          crews: [agentCrew('crw_1')],
          sessions: [
            {
              ...agentSession('ags_1', crew: ('crw_1', 0)),
              'identity': {'state': 'unavailable'},
              'isolation': {'state': 'unavailable', 'reason': 'off'},
              'needs_attention': null,
              'parent_session': null,
              'root_session': 'ags_1',
              'depth': 0,
            },
          ],
        );
        final spawning = FakeSpawning(agents);
        final c = await open(t, agents, size: size);
        for (final at in [
          Routes.agents,
          Routes.agentCrew('crw_1'),
          Routes.agentSession('ags_1', tab: SessionTab.about.name),
        ]) {
          c.read(routerProvider).go(at);
          await t.pumpAndSettle();
          expect(
            find.byKey(const Key('spawn-switch')),
            findsNothing,
            reason: at,
          );
          expect(find.textContaining('↳'), findsNothing, reason: at);
          expect(find.byType(AttentionTag), findsNothing, reason: at);
          expect(find.byType(ApprovalCard), findsNothing, reason: at);
          expect(find.text('Requests'), findsNothing, reason: at);
        }
        c.read(routerProvider).go(Routes.agentCrew('crw_1'));
        await t.pumpAndSettle();
        await t.tap(find.byKey(const Key('crew-page-menu')));
        await t.pumpAndSettle();
        await t.tap(find.byKey(const Key('crew-action-endAll')));
        await t.pumpAndSettle();
        expect(find.byKey(const Key('end-all-reaches')), findsNothing);
        expect(
          agents.requests.where(
            (r) =>
                r.url.path.contains('/requests') ||
                r.url.path.contains('/descendants') ||
                r.url.path.contains('/child-work'),
          ),
          isEmpty,
        );
        expect(spawning.requests, isEmpty);
        await disposeShell(t, c);
      });
    }
  });
}
