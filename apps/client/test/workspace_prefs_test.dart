import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'package:storm/agent/agent_models.dart';
import 'package:storm/agent/launcher.dart';
import 'package:storm/router.dart';

import 'agent_fakes.dart';
import 'fake_server.dart';
import 'shell_harness.dart';

/// Decision 159: the launcher preselects Write to from what was last used
/// in the chosen workspace, and Settings › Agents & hosts lists and forgets
/// it.
void main() {
  const wide = Size(1400, 900);
  const primary = FakeServer.primaryVault;
  const work = 'v-work';

  setUp(() => SharedPreferences.setMockInitialValues({}));

  Map<String, dynamic> c0() => agentHost(
    id: 'hst_1',
    name: 'build-vm',
    providers: ['claude-code', 'opencode', 'shell'],
    workspaces: ['storm', 'site'],
    identity: 'verified',
    isolation: 'verified',
    execution: {'account': 'dewansh', 'isolation': 'per-session'},
    limits: {'max_children': 3, 'max_tree': 6},
  );

  Future<void> tapKey(WidgetTester t, String key) async {
    await t.ensureVisible(find.byKey(Key(key)));
    await t.pumpAndSettle();
    await t.tap(find.byKey(Key(key)));
    await t.pumpAndSettle();
  }

  /// The launcher over the Agents page, with a second vault, Work.
  Future<ProviderContainer> open(
    WidgetTester t,
    FakeAgentServer agents, {
    LaunchPrefill? prefill,
  }) async {
    final c = shellContainer(agentClient: agents.client);
    serverOf(c).addVault(work, 'Work');
    await pumpShell(t, c, size: wide);
    c.read(routerProvider).go(Routes.agents);
    await t.pumpAndSettle();
    if (prefill == null) {
      await t.tap(find.byKey(const Key('new-session')));
    } else {
      showNewSessionLauncher(
        t.element(find.byKey(const Key('new-session'))),
        hosts: [AgentHost.fromJson(c0())],
        wide: true,
        prefill: prefill,
      );
    }
    await t.pumpAndSettle();
    return c;
  }

  LauncherChipState chip(WidgetTester t) => t
      .widget<LauncherChip>(find.byKey(const Key('launcher-chip-writes')))
      .state;

  Finder chipSays(String text) => find.descendant(
    of: find.byKey(const Key('launcher-chip-writes')),
    matching: find.text(text),
  );

  final hint = find.byKey(const Key('launcher-writes-remembered'));

  Future<Map<String, dynamic>> launch(
    WidgetTester t,
    FakeAgentServer agents,
  ) async {
    await tapKey(t, 'launch');
    return agents.launches.last;
  }

  Future<void> where(WidgetTester t, String workspace) async {
    await tapKey(t, 'launcher-where');
    await t.tap(find.byKey(Key('where-hst_1-$workspace')));
    await t.pumpAndSettle();
  }

  group('the launcher', () {
    testWidgets('remembered on: its vault, with the hint', (t) async {
      final agents = FakeAgentServer(hosts: [c0()]);
      agents.workspacePrefs['hst_1/storm'] = {'writes': true, 'vault_id': work};
      final c = await open(t, agents);
      expect(chip(t), LauncherChipState.on);
      expect(chipSays('Writes to Work'), findsOneWidget);
      expect(hint, findsOneWidget);
      await tapKey(t, 'launcher-more');
      expect(hint, findsOneWidget, reason: 'next to the vault in More');
      final sent = await launch(t, agents);
      expect(sent['write_vault_id'], work);
      expect(sent.containsKey('rerun_of'), isFalse);
      await disposeShell(t, c);
    });

    testWidgets('remembered off: the toggle off, no hint', (t) async {
      final agents = FakeAgentServer(hosts: [c0()]);
      agents.workspacePrefs['hst_1/storm'] = {
        'writes': false,
        'vault_id': null,
      };
      final c = await open(t, agents);
      expect(chip(t), LauncherChipState.off);
      expect(hint, findsNothing);
      final sent = await launch(t, agents);
      expect(sent.containsKey('write_vault_id'), isFalse);
      await disposeShell(t, c);
    });

    testWidgets('remembered beats the context note', (t) async {
      final agents = FakeAgentServer(hosts: [c0()]);
      agents.workspacePrefs['hst_1/storm'] = {
        'writes': true,
        'vault_id': primary,
      };
      final c = await open(
        t,
        agents,
        prefill: const LaunchPrefill(context: (vaultId: work, noteId: 'n9')),
      );
      expect(chipSays('Writes to Primary'), findsOneWidget);
      expect(hint, findsOneWidget);
      expect((await launch(t, agents))['write_vault_id'], primary);
      await disposeShell(t, c);
    });

    testWidgets('the context note alone: its vault, no hint', (t) async {
      final agents = FakeAgentServer(hosts: [c0()]);
      final c = await open(
        t,
        agents,
        prefill: const LaunchPrefill(context: (vaultId: work, noteId: 'n9')),
      );
      expect(chipSays('Writes to Work'), findsOneWidget);
      expect(hint, findsNothing);
      expect((await launch(t, agents))['write_vault_id'], work);
      await disposeShell(t, c);
    });

    testWidgets('nothing: as before, no hint', (t) async {
      final agents = FakeAgentServer(hosts: [c0()]);
      final c = await open(t, agents);
      expect(chip(t), LauncherChipState.on);
      expect(chipSays('Writes to Primary'), findsOneWidget);
      expect(hint, findsNothing);
      expect((await launch(t, agents))['write_vault_id'], primary);
      await disposeShell(t, c);
    });

    testWidgets('a rerun keeps its own and says it is one', (t) async {
      final agents = FakeAgentServer(hosts: [c0()]);
      agents.workspacePrefs['hst_1/storm'] = {
        'writes': true,
        'vault_id': primary,
      };
      final old = AgentSession.fromJson(
        agentSession('ags_old', status: 'completed', writeVaultId: work),
      );
      final c = await open(t, agents, prefill: LaunchPrefill.runAgain(old));
      expect(chipSays('Writes to Work'), findsOneWidget);
      expect(hint, findsNothing);
      final sent = await launch(t, agents);
      expect((sent['write_vault_id'], sent['rerun_of']), (work, 'ags_old'));
      await disposeShell(t, c);
    });

    testWidgets('changing Where applies the new workspace’s', (t) async {
      final agents = FakeAgentServer(hosts: [c0()]);
      agents.workspacePrefs['hst_1/storm'] = {'writes': true, 'vault_id': work};
      agents.workspacePrefs['hst_1/site'] = {'writes': false, 'vault_id': null};
      final c = await open(t, agents);
      expect(chipSays('Writes to Work'), findsOneWidget);
      await where(t, 'site');
      expect(chip(t), LauncherChipState.off);
      expect(hint, findsNothing);
      await where(t, 'storm');
      expect(chip(t), LauncherChipState.on);
      expect(chipSays('Writes to Work'), findsOneWidget);
      expect(hint, findsOneWidget);
      await disposeShell(t, c);
    });

    testWidgets('unless Write to was touched in this launch', (t) async {
      final agents = FakeAgentServer(hosts: [c0()]);
      agents.workspacePrefs['hst_1/storm'] = {'writes': true, 'vault_id': work};
      agents.workspacePrefs['hst_1/site'] = {'writes': false, 'vault_id': null};
      final c = await open(t, agents);
      await tapKey(t, 'launcher-more');
      await tapKey(t, 'launcher-write-vault');
      await t.tap(find.text('Primary').last);
      await t.pumpAndSettle();
      expect(hint, findsNothing, reason: 'no longer the remembered one');
      await where(t, 'site');
      final sent = await launch(t, agents);
      expect((sent['workspace'], sent['write_vault_id']), ('site', primary));
      await disposeShell(t, c);
    });
  });

  group('Settings › Agents & hosts', () {
    testWidgets('lists each remembered workspace and forgets one', (t) async {
      final agents = FakeAgentServer(hosts: [c0()]);
      agents.workspacePrefs['hst_1/storm'] = {'writes': true, 'vault_id': work};
      agents.workspacePrefs['hst_1/site'] = {'writes': false, 'vault_id': null};
      final c = shellContainer(agentClient: agents.client);
      serverOf(c).addVault(work, 'Work');
      await pumpShell(t, c, size: wide);
      c.read(routerProvider).go(Routes.settingsPage('hosts'));
      await t.pumpAndSettle();

      String line(String ws) => t
          .widget<Text>(find.byKey(Key('workspace-pref-line-hst_1/$ws')))
          .data!;
      expect(line('storm'), 'Writes to Work');
      expect(line('site'), 'No writes');

      await tapKey(t, 'forget-pref-hst_1/storm');
      expect(
        agents.count('DELETE', '/v1/agent/hosts/hst_1/workspaces/storm/prefs'),
        1,
      );
      expect(find.byKey(const Key('workspace-pref-hst_1/storm')), findsNothing);
      expect(line('site'), 'No writes');
      await disposeShell(t, c);
    });
  });
}
