import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'package:storm/agent/agent_models.dart';
import 'package:storm/agent/agent_state.dart' show SessionTab;
import 'package:storm/agent/spawn_models.dart';
import 'package:storm/router.dart';

import 'agent_fakes.dart';
import 'shell_harness.dart';
import 'spawn_fakes.dart';

/// Decision 146 in the client: the launcher's Permission mode (146b), the
/// mode on a session's row and in About, and Needs you for a child waiting
/// at its CLI's own permission prompt, on the child, its parent and its
/// crew (146c).
void main() {
  const wide = Size(1400, 900);
  const phone = Size(899, 900);

  setUp(() => SharedPreferences.setMockInitialValues({}));

  String location(ProviderContainer c) => c.read(routerProvider).state.uri.path;

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

  Future<ProviderContainer> launcher(
    WidgetTester t,
    FakeAgentServer agents,
  ) async {
    final c = await open(t, agents);
    await t.tap(find.byKey(const Key('new-session')));
    await t.pumpAndSettle();
    return c;
  }

  Future<void> pick(WidgetTester t, String field, Finder item) async {
    await t.tap(find.byKey(Key(field)));
    await t.pumpAndSettle();
    await t.tap(item);
    await t.pumpAndSettle();
  }

  FakeAgentServer server() => FakeAgentServer(
    hosts: [
      agentHost(providers: ['claude-code', 'opencode', 'codex', 'shell']),
    ],
    sessions: [agentSession('ags_1')],
  );

  group('Permission mode (146b)', () {
    for (final (label, mode) in [
      ('Accept edits', 'accept_edits'),
      ('Auto', 'auto'),
    ]) {
      testWidgets('A-146-2: the launcher sends $mode', (t) async {
        final agents = server();
        final c = await launcher(t, agents);
        await pick(t, 'launcher-permission-mode', find.text(label).last);
        await t.tap(find.byKey(const Key('launch')));
        await t.pumpAndSettle();
        expect(agents.launches.single['permission_mode'], mode);
        await disposeShell(t, c);
      });
    }

    testWidgets('A-146-2: default sends no permission_mode', (t) async {
      final agents = server();
      final c = await launcher(t, agents);
      expect(find.byKey(const Key('launcher-permission-mode')), findsOneWidget);
      await t.tap(find.byKey(const Key('launch')));
      await t.pumpAndSettle();
      expect(agents.launches.single.containsKey('permission_mode'), isFalse);
      await disposeShell(t, c);
    });

    testWidgets(
      'A-146-2: only the agent’s own modes are offered, and never a bypass',
      (t) async {
        final agents = server();
        final c = await launcher(t, agents);
        await t.tap(find.byKey(const Key('launcher-permission-mode')));
        await t.pumpAndSettle();
        expect(find.text('Default (the agent asks)'), findsWidgets);
        expect(find.text('Accept edits'), findsOneWidget);
        expect(find.text('Auto'), findsOneWidget);
        expect(
          find.textContaining(RegExp('bypass|skip', caseSensitive: false)),
          findsNothing,
        );
        await t.tap(find.text('Auto').last);
        await t.pumpAndSettle();

        // OpenCode offers no Auto: the choice falls back to the default.
        await pick(t, 'launcher-agent', find.text('OpenCode').last);
        await t.tap(find.byKey(const Key('launcher-permission-mode')));
        await t.pumpAndSettle();
        expect(find.text('Accept edits'), findsOneWidget);
        expect(find.text('Auto'), findsNothing);
        await t.tapAt(Offset.zero);
        await t.pumpAndSettle();

        for (final other in ['Codex', 'Shell']) {
          await pick(t, 'launcher-agent', find.text(other).last);
          expect(
            find.byKey(const Key('launcher-permission-mode')),
            findsNothing,
            reason: other,
          );
        }
        await pick(t, 'launcher-agent', find.text('OpenCode').last);
        await t.tap(find.byKey(const Key('launch')));
        await t.pumpAndSettle();
        expect(agents.launches.single.containsKey('permission_mode'), isFalse);
        await disposeShell(t, c);
      },
    );

    test('A-146-2: the modes per agent mirror the server, with no bypass', () {
      expect(permissionModesOf('claude-code'), [
        'default',
        'accept_edits',
        'auto',
      ]);
      expect(permissionModesOf('opencode'), ['default', 'accept_edits']);
      for (final other in ['codex', 'shell', 'cursor', null]) {
        expect(permissionModesOf(other), ['default'], reason: '$other');
      }
      expect(permissionModeOf('bypassPermissions'), isNull);
      expect(permissionModeOf('accept_edits'), 'accept_edits');
      expect(
        AgentSession.fromJson(
          spawnSession('ags_x', permissionMode: 'auto'),
        ).permissionMode,
        'auto',
      );
      expect(
        AgentSession.fromJson(spawnSession('ags_x')).permissionMode,
        isNull,
      );
    });

    testWidgets('the mode shows on the row and in About', (t) async {
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        sessions: [
          spawnSession(
            'ags_m',
            name: 'mode-run',
            permissionMode: 'accept_edits',
          ),
        ],
      );
      final c = await open(
        t,
        agents,
        at: Routes.agentSession('ags_m', tab: SessionTab.about.name),
      );
      expect(find.textContaining('accept edits'), findsWidgets);
      if (find.byKey(const Key('session-about')).evaluate().isEmpty) {
        await t.tap(find.byKey(const Key('inspector-toggle')));
        await t.pumpAndSettle();
      }
      expect(find.text('PERMISSION MODE'), findsOneWidget);
      expect(find.text('Accept edits'), findsOneWidget);
      await disposeShell(t, c);
    });
  });

  group('Needs you (146c)', () {
    Map<String, dynamic> inCrew(Map<String, dynamic> s) => {
      ...s,
      'crew': {'id': 'crw_1', 'order': 0},
    };

    List<Map<String, dynamic>> family({bool crew = false}) {
      final parent = spawnSession('ags_p', name: 'gateway-impl');
      final child = spawnSession(
        'ags_c',
        name: 'test-gateway',
        parent: 'ags_p',
        depth: 1,
        needsAttention: 'permission_prompt',
      );
      return crew ? [inCrew(parent), inCrew(child)] : [parent, child];
    }

    test('A-146-4: the label is Needs you', () {
      expect(attentionLabel('permission_prompt'), 'Needs you');
    });

    for (final size in [wide, phone]) {
      testWidgets(
        'A-146-4: the child and its parent show Needs you, and a tap opens '
        'the child at ${size.width.toInt()}px',
        (t) async {
          final agents = FakeAgentServer(
            hosts: [agentHost()],
            sessions: family(),
          );
          final spawning = FakeSpawning(agents);
          final c = await open(t, agents, size: size);
          expect(
            find.byKey(const Key('attention-permission_prompt')),
            findsWidgets,
          );
          final badge = find.byKey(const Key('needs-you-ags_c'));
          expect(badge, findsOneWidget, reason: 'on the parent’s row');
          await t.tap(badge);
          await t.pumpAndSettle();
          expect(location(c), '/agents/s/ags_c');
          expect(spawning.terminalWrites, 0);
          await disposeShell(t, c);
        },
      );

      testWidgets('A-146-4: a crew holding the child shows Needs you at '
          '${size.width.toInt()}px', (t) async {
        final agents = FakeAgentServer(
          hosts: [agentHost()],
          crews: [agentCrew('crw_1')],
          sessions: family(crew: true),
        );
        FakeSpawning(agents);
        final c = await open(t, agents, size: size);
        final crewHeader = find.byKey(const Key('crew-crw_1'));
        expect(crewHeader, findsWidgets);
        expect(
          find.descendant(
            of: crewHeader.first,
            matching: find.byKey(const Key('needs-you-ags_c')),
          ),
          findsOneWidget,
        );
        await t.tap(
          find
              .descendant(
                of: crewHeader.first,
                matching: find.byKey(const Key('needs-you-ags_c')),
              )
              .first,
        );
        await t.pumpAndSettle();
        expect(location(c), '/agents/s/ags_c');
        await disposeShell(t, c);
      });
    }

    testWidgets('nothing waits once the prompt is answered', (t) async {
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        sessions: [
          spawnSession('ags_p', name: 'gateway-impl'),
          spawnSession(
            'ags_c',
            name: 'test-gateway',
            parent: 'ags_p',
            depth: 1,
          ),
        ],
      );
      FakeSpawning(agents);
      final c = await open(t, agents);
      expect(find.byKey(const Key('needs-you-ags_c')), findsNothing);
      expect(
        find.byKey(const Key('attention-permission_prompt')),
        findsNothing,
      );
      await disposeShell(t, c);
    });
  });
}
