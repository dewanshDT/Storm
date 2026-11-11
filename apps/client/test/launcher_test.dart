import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'package:storm/agent/agent_models.dart';
import 'package:storm/agent/launcher.dart';
import 'package:storm/router.dart';
import 'package:storm/ui/controls.dart';

import 'agent_fakes.dart';
import 'fake_server.dart';
import 'shell_harness.dart';

/// Decision 153k, slice 153-5: the launcher's three fields, its "This
/// session can" chips and More, rows that never come and go with the agent,
/// and the same launch request as before.
void main() {
  const wide = Size(1400, 900);
  const phone = Size(411, 900);
  const primary = FakeServer.primaryVault;

  setUp(() => SharedPreferences.setMockInitialValues({}));

  /// A host that passes C0, with every agent the rows have to survive.
  Map<String, dynamic> c0({
    String id = 'hst_1',
    String name = 'build-vm',
    List<String> workspaces = const ['storm', 'site'],
  }) => agentHost(
    id: id,
    name: name,
    providers: ['claude-code', 'opencode', 'shell', 'codex'],
    workspaces: workspaces,
    identity: 'verified',
    isolation: 'verified',
    execution: {'account': 'dewansh', 'isolation': 'per-session'},
    limits: {'max_children': 3, 'max_tree': 6},
  );

  Future<ProviderContainer> launcher(
    WidgetTester t,
    FakeAgentServer agents, {
    Size size = wide,
  }) async {
    final c = shellContainer(agentClient: agents.client);
    await pumpShell(t, c, size: size);
    c.read(routerProvider).go(Routes.agents);
    await t.pumpAndSettle();
    await t.tap(
      find.byKey(Key(size == phone ? 'new-session-pill' : 'new-session')),
    );
    await t.pumpAndSettle();
    return c;
  }

  Future<void> tapKey(WidgetTester t, String key) async {
    await t.ensureVisible(find.byKey(Key(key)));
    await t.pumpAndSettle();
    await t.tap(find.byKey(Key(key)));
    await t.pumpAndSettle();
  }

  Future<void> pick(WidgetTester t, String field, Finder item) async {
    await tapKey(t, field);
    await t.tap(item);
    await t.pumpAndSettle();
  }

  Finder inField(String key, String text) => find.descendant(
    of: find.byKey(Key(key)),
    matching: find.text(text),
    matchRoot: true,
  );

  Set<String> rows(WidgetTester t) => {
    for (final e
        in find
            .byWidgetPredicate(
              (w) =>
                  w.key is ValueKey<String> &&
                  (w.key! as ValueKey<String>).value.startsWith(
                    'launcher-row-',
                  ),
            )
            .evaluate())
      (e.widget.key! as ValueKey<String>).value.substring(13),
  };

  LauncherChipState chip(WidgetTester t, String name) =>
      t.widget<LauncherChip>(find.byKey(Key('launcher-chip-$name'))).state;

  bool onScreen(WidgetTester t, String key) {
    final r = t.getRect(find.byKey(Key(key)));
    return r.top >= 0 && r.bottom <= t.view.physicalSize.height;
  }

  group('the default view', () {
    testWidgets('From, Agent and Where, the chips, More and Launch; '
        'nothing else', (t) async {
      final agents = FakeAgentServer(hosts: [c0()]);
      final c = shellContainer(agentClient: agents.client);
      await pumpShell(t, c, size: wide);
      c.read(routerProvider).go(Routes.note(primary, 'n3'));
      await t.pumpAndSettle();
      await t.tap(find.byKey(const Key('start-session')));
      await t.pumpAndSettle();

      expect(rows(t), {'from', 'agent', 'where'});
      expect(inField('launcher-row-from', 'From'), findsOneWidget);
      expect(
        inField('launcher-agent', 'Claude Code · default'),
        findsOneWidget,
      );
      expect(inField('launcher-where', 'storm on build-vm'), findsOneWidget);
      expect(find.text('THIS SESSION CAN'), findsOneWidget);
      for (final name in ['writes', 'spawn', 'integrations', 'network']) {
        expect(find.byKey(Key('launcher-chip-$name')), findsOneWidget);
      }
      expect(
        find.text(
          'Tap one to change it. Everything else uses the agent’s defaults.',
        ),
        findsOneWidget,
      );
      expect(
        find.text('More: model, permission mode, crew, role ›'),
        findsOneWidget,
      );
      expect(find.byKey(const Key('launch')), findsOneWidget);
      // More's rows and the full sentence wait for More.
      expect(find.byKey(const Key('launcher-risk')), findsNothing);
      expect(find.byKey(const Key('launcher-model')), findsNothing);
      await disposeShell(t, c);
    });

    testWidgets('no host online points at Settings › Agents & hosts', (
      t,
    ) async {
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        sessions: [agentSession('ags_1')],
      );
      final c = shellContainer(agentClient: agents.client);
      await pumpShell(t, c, size: wide);
      c.read(routerProvider).go(Routes.agents);
      await t.pumpAndSettle();
      showNewSessionLauncher(
        t.element(find.byKey(const Key('new-session'))),
        hosts: [AgentHost.fromJson(agentHost(status: 'offline'))],
        wide: true,
      );
      await t.pumpAndSettle();
      expect(
        find.text(
          'No host is online. Start storm-runtime on one, or enroll a host '
          'in Settings › Agents & hosts.',
        ),
        findsOneWidget,
      );
      await disposeShell(t, c);
    });
  });

  group('Where', () {
    testWidgets('joins workspace and host, grouped by host, with Runs as '
        'and a held workspace under it', (t) async {
      final agents = FakeAgentServer(
        hosts: [
          c0(),
          c0(id: 'hst_2', name: 'codebox', workspaces: ['notes']),
        ],
        sessions: [agentSession('ags_1')],
      );
      final c = await launcher(t, agents);
      expect(inField('launcher-where', 'storm on build-vm'), findsOneWidget);
      // ags_1 runs in storm on build-vm.
      expect(
        inField(
          'launcher-host-execution-held',
          'Runs as dewansh · each session isolated · another session is '
              'working here',
        ),
        findsOneWidget,
      );

      await tapKey(t, 'launcher-where');
      final menu = find.byKey(const Key('where-menu'));
      Finder inMenu(String text) =>
          find.descendant(of: menu, matching: find.text(text));
      expect(inMenu('BUILD-VM'), findsOneWidget);
      expect(inMenu('CODEBOX'), findsOneWidget);
      double y(Finder f) => t.getTopLeft(f).dy;
      expect(
        y(find.byKey(const Key('where-hst_1-site'))),
        lessThan(y(inMenu('CODEBOX'))),
      );
      expect(
        y(inMenu('CODEBOX')),
        lessThan(y(find.byKey(const Key('where-hst_2-notes')))),
      );
      expect(inMenu('another session is working here'), findsOneWidget);
      expect(inMenu('New workspace on build-vm…'), findsOneWidget);
      expect(inMenu('New workspace on codebox…'), findsOneWidget);
      await t.tap(find.byKey(const Key('where-hst_2-notes')));
      await t.pumpAndSettle();
      expect(inField('launcher-where', 'notes on codebox'), findsOneWidget);
      expect(
        inField(
          'launcher-host-execution',
          'Runs as dewansh · each session isolated',
        ),
        findsOneWidget,
      );

      await tapKey(t, 'launch');
      expect(
        (
          agents.launches.single['host_id'],
          agents.launches.single['workspace'],
        ),
        ('hst_2', 'notes'),
      );
      await disposeShell(t, c);
    });

    testWidgets('New workspace… is a field under Where, never a dialog', (
      t,
    ) async {
      final agents = FakeAgentServer(hosts: [c0()]);
      final c = await launcher(t, agents);
      await tapKey(t, 'launcher-new-workspace');
      expect(find.byType(AlertDialog), findsNothing);
      expect(
        find.descendant(
          of: find.byKey(const Key('launcher-row-where')),
          matching: find.byKey(const Key('launcher-new-workspace-form')),
        ),
        findsOneWidget,
      );
      await t.enterText(find.byKey(const Key('new-workspace-name')), 'demo');
      await t.pump();
      await tapKey(t, 'new-workspace-create');
      expect(agents.newWorkspaces, [
        {'name': 'demo', 'git': true},
      ]);
      expect(inField('launcher-where', 'demo on build-vm'), findsOneWidget);

      // Cancel closes it and keeps what was chosen.
      await tapKey(t, 'launcher-new-workspace');
      await tapKey(t, 'new-workspace-cancel');
      expect(
        find.byKey(const Key('launcher-new-workspace-form')),
        findsNothing,
      );
      expect(inField('launcher-where', 'demo on build-vm'), findsOneWidget);
      await disposeShell(t, c);
    });
  });

  group('This session can', () {
    testWidgets('each chip says its state and opens its setting in More', (
      t,
    ) async {
      final agents = FakeAgentServer(hosts: [c0()]);
      final c = await launcher(t, agents);
      expect(chip(t, 'writes'), LauncherChipState.on);
      expect(inField('launcher-chip-writes', 'Writes to Primary'), findsOne);
      expect(chip(t, 'spawn'), LauncherChipState.on);
      expect(chip(t, 'integrations'), LauncherChipState.off);
      expect(
        inField('launcher-chip-integrations', 'Uses no integrations'),
        findsOne,
      );
      expect(chip(t, 'network'), LauncherChipState.off);
      expect(
        inField('launcher-chip-network', 'Network: build-vm’s policy'),
        findsOne,
      );

      for (final (name, target) in [
        ('writes', 'launcher-writes'),
        ('spawn', 'launcher-spawn'),
        ('integrations', 'launcher-risk'),
        ('network', 'launcher-risk'),
      ]) {
        await t.tap(find.byKey(Key('launcher-chip-$name')));
        await t.pumpAndSettle();
        expect(find.byKey(Key(target)), findsOneWidget, reason: name);
        expect(onScreen(t, target), isTrue, reason: name);
        await tapKey(t, 'launcher-fewer');
      }

      // Turned off in More, the chip goes neutral.
      await t.tap(find.byKey(const Key('launcher-chip-spawn')));
      await t.pumpAndSettle();
      await tapKey(t, 'launcher-spawn');
      await tapKey(t, 'launcher-fewer');
      expect(chip(t, 'spawn'), LauncherChipState.off);
      await disposeShell(t, c);
    });

    testWidgets('a chip the agent can not have stays, muted, and says why', (
      t,
    ) async {
      final agents = FakeAgentServer(hosts: [c0()]);
      final c = await launcher(t, agents);
      await pick(t, 'launcher-agent', find.text('Shell').last);
      for (final name in ['writes', 'spawn', 'integrations', 'network']) {
        expect(find.byKey(Key('launcher-chip-$name')), findsOneWidget);
      }
      expect(chip(t, 'writes'), LauncherChipState.unavailable);
      expect(chip(t, 'spawn'), LauncherChipState.unavailable);
      expect(chip(t, 'integrations'), LauncherChipState.unavailable);
      await t.tap(find.byKey(const Key('launcher-chip-writes')));
      await t.pumpAndSettle();
      expect(
        inField('launcher-chip-reason', "Shell can't reach Storm"),
        findsOneWidget,
      );
      expect(find.byKey(const Key('launcher-writes')), findsNothing);

      // Codex reaches Storm and starts agents; an old host can't offer it.
      await pick(t, 'launcher-agent', find.text('Codex').last);
      expect(chip(t, 'spawn'), LauncherChipState.on);
      await disposeShell(t, c);

      final old = FakeAgentServer(hosts: [agentHost()]);
      final c2 = await launcher(t, old);
      expect(chip(t, 'spawn'), LauncherChipState.unavailable);
      await t.tap(find.byKey(const Key('launcher-chip-spawn')));
      await t.pumpAndSettle();
      expect(
        inField(
          'launcher-chip-reason',
          'Update storm-runtime on this host to let agents start agents',
        ),
        findsOneWidget,
      );
      await disposeShell(t, c2);
    });
  });

  group('More', () {
    testWidgets('switching agents never adds or removes a row', (t) async {
      final agents = FakeAgentServer(hosts: [c0()]);
      final c = await launcher(t, agents);
      await tapKey(t, 'launcher-more');
      const all = {
        'agent',
        'where',
        'writes',
        'spawn',
        'children-write',
        'model',
        'permission-mode',
        'crew',
        'role',
      };
      expect(rows(t), all);
      for (final agent in [
        'OpenCode',
        'Shell',
        'Codex',
        'Claude Code · default',
      ]) {
        await pick(t, 'launcher-agent', find.text(agent).last);
        expect(rows(t), all, reason: agent);
      }

      await pick(t, 'launcher-agent', find.text('Shell').last);
      for (final (key, why) in [
        ('launcher-model-hint', 'Shell has no model'),
        (
          'launcher-permission-note',
          'Shell has only its own setting. Claude Code and OpenCode only.',
        ),
        ('launcher-writes-reason', "Shell can't reach Storm"),
        ('launcher-spawn-reason', "Shell can't start agents"),
        (
          'launcher-role-label-only',
          "Shell can't read Storm notes. A role would only be a label.",
        ),
        ('launcher-children-write-reason', 'Turn on Start agents first'),
      ]) {
        expect(inField(key, why), findsOneWidget, reason: key);
      }
      expect(
        t.widget<TextField>(find.byKey(const Key('launcher-model'))).enabled,
        isFalse,
      );
      for (final toggle in ['launcher-writes', 'launcher-spawn']) {
        expect(t.widget<StormToggle>(find.byKey(Key(toggle))).enabled, isFalse);
      }

      await pick(t, 'launcher-agent', find.text('Codex').last);
      expect(
        inField(
          'launcher-permission-note',
          'Codex has only its own setting. Claude Code and OpenCode only.',
        ),
        findsOneWidget,
      );
      await disposeShell(t, c);
    });

    testWidgets('the toggles, the host limits and the children-write rule', (
      t,
    ) async {
      final agents = FakeAgentServer(hosts: [c0()]);
      final c = await launcher(t, agents);
      await tapKey(t, 'launcher-more');
      expect(find.text('Up to 3 at once, 6 in its tree'), findsOneWidget);
      expect(
        find.text('Let the agents it starts write to Primary'),
        findsOneWidget,
      );
      expect(inField('launcher-write-vault', 'Primary'), findsOneWidget);
      bool live(String k) => t.widget<StormToggle>(find.byKey(Key(k))).enabled;
      expect(live('launcher-children-write'), isTrue);

      await tapKey(t, 'launcher-spawn');
      expect(live('launcher-children-write'), isFalse);
      expect(
        inField('launcher-children-write-reason', 'Turn on Start agents first'),
        findsOneWidget,
      );
      await tapKey(t, 'launcher-spawn');
      await tapKey(t, 'launcher-children-write');
      await tapKey(t, 'launch');
      final sent = agents.launches.single;
      expect((sent['spawn'], sent['children_write']), (true, true));
      await disposeShell(t, c);
    });

    testWidgets('the full risk sentence is in More', (t) async {
      final agents = FakeAgentServer(hosts: [c0()]);
      final c = await launcher(t, agents);
      await tapKey(t, 'launcher-more');
      expect(
        inField(
          'launcher-risk',
          'Network: inherits build-vm’s policy. This session can use your '
              'integrations and read your vaults, with that network access.',
        ),
        findsOneWidget,
      );
      await disposeShell(t, c);
    });

    testWidgets('Model offers opus, sonnet and haiku over the default', (
      t,
    ) async {
      final agents = FakeAgentServer(hosts: [c0()]);
      final c = await launcher(t, agents);
      await tapKey(t, 'launcher-more');
      final field = find.byKey(const Key('launcher-model'));
      expect(
        find.descendant(of: field, matching: find.text('The agent’s default')),
        findsOneWidget,
      );
      for (final m in ['opus', 'sonnet', 'haiku']) {
        expect(find.byKey(Key('launcher-model-$m')), findsOneWidget);
      }
      await tapKey(t, 'launcher-model-sonnet');
      expect(t.widget<TextField>(field).controller!.text, 'sonnet');
      // Only Claude Code's names are offered.
      await pick(t, 'launcher-agent', find.text('OpenCode').last);
      expect(find.byKey(const Key('launcher-model-opus')), findsNothing);
      await pick(t, 'launcher-agent', find.text('Claude Code · default').last);
      await tapKey(t, 'launch');
      expect(agents.launches.single['model'], 'sonnet');
      await disposeShell(t, c);
    });

    testWidgets('Permission mode says which agents have it and where it '
        'changes', (t) async {
      final agents = FakeAgentServer(hosts: [c0()]);
      final c = await launcher(t, agents);
      await tapKey(t, 'launcher-more');
      expect(
        inField(
          'launcher-permission-note',
          'Claude Code and OpenCode only. Tab and ⇧Tab change it in the '
              'terminal.',
        ),
        findsOneWidget,
      );
      await disposeShell(t, c);
    });
  });

  group('the launch request is the one sent before 153-5', () {
    testWidgets('the defaults', (t) async {
      final agents = FakeAgentServer(hosts: [c0()]);
      final c = await launcher(t, agents);
      await tapKey(t, 'launch');
      expect(
        agents.requests
            .where((r) => r.method == 'POST' && r.url.path.endsWith('sessions'))
            .single
            .body,
        '{"host_id":"hst_1","workspace":"storm","provider":"claude-code",'
        '"interaction":"terminal","terminal":{"cols":155,"rows":45},'
        '"write_vault_id":"$primary","spawn":true}',
      );
      await disposeShell(t, c);
    });

    testWidgets('every choice made', (t) async {
      final agents = FakeAgentServer(
        hosts: [c0()],
        crews: [agentCrew('crw_1')],
        roles: [kitRole('k_rev', 'Storm Reviewer')],
      );
      final c = await launcher(t, agents);
      await tapKey(t, 'launcher-more');
      await t.enterText(find.byKey(const Key('launcher-model')), ' opus ');
      await pick(t, 'launcher-permission-mode', find.text('Plan').last);
      await pick(t, 'launcher-crew', find.text('Gateway release').last);
      await pick(t, 'launcher-role', find.byKey(const Key('role-k_rev')));
      await tapKey(t, 'launcher-children-write');
      await tapKey(t, 'launch');
      expect(
        agents.requests
            .where((r) => r.method == 'POST' && r.url.path.endsWith('sessions'))
            .single
            .body,
        '{"host_id":"hst_1","workspace":"storm","provider":"claude-code",'
        '"interaction":"terminal","terminal":{"cols":155,"rows":45},'
        '"write_vault_id":"$primary","crew":{"id":"crw_1"},'
        '"role":{"vault_id":"v-kit","note_id":"k_rev"},"spawn":true,'
        '"children_write":true,"model":"opus","permission_mode":"plan"}',
      );
      await disposeShell(t, c);
    });

    testWidgets('a shell sends none of what it cannot have', (t) async {
      final agents = FakeAgentServer(hosts: [c0()]);
      final c = await launcher(t, agents);
      await tapKey(t, 'launcher-more');
      await t.enterText(find.byKey(const Key('launcher-model')), 'opus');
      await tapKey(t, 'launcher-children-write');
      await pick(t, 'launcher-agent', find.text('Shell').last);
      await tapKey(t, 'launch');
      expect(
        agents.requests
            .where((r) => r.method == 'POST' && r.url.path.endsWith('sessions'))
            .single
            .body,
        '{"host_id":"hst_1","workspace":"storm","provider":"shell",'
        '"interaction":"terminal","terminal":{"cols":155,"rows":45}}',
      );
      await disposeShell(t, c);
    });
  });

  testWidgets('the phone sheet: the same three fields and chips, labels '
      'above, More in the sheet', (t) async {
    final agents = FakeAgentServer(
      hosts: [c0()],
      sessions: [agentSession('ags_1')],
    );
    final c = await launcher(t, agents, size: phone);
    expect(find.byType(BottomSheet), findsOneWidget);
    expect(rows(t), {'agent', 'where'});
    for (final name in ['writes', 'spawn', 'integrations', 'network']) {
      expect(find.byKey(Key('launcher-chip-$name')), findsOneWidget);
    }
    final label = t.getRect(inField('launcher-row-where', 'Where'));
    final control = t.getRect(find.byKey(const Key('launcher-where')));
    expect(label.bottom, lessThanOrEqualTo(control.top));
    expect(label.left, closeTo(control.left, 1));
    expect(
      t.getSize(find.byKey(const Key('launch'))).width,
      closeTo(control.width, 1),
    );

    await tapKey(t, 'launcher-more');
    expect(
      find.descendant(
        of: find.byType(BottomSheet),
        matching: find.byKey(const Key('launcher-permission-mode')),
      ),
      findsOneWidget,
    );
    await disposeShell(t, c);
  });
}
