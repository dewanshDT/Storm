import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'package:storm/agent/agents_screen.dart';
import 'package:storm/agent/agent_state.dart';
import 'package:storm/router.dart';

import 'agent_fakes.dart';
import 'fake_server.dart';
import 'shell_harness.dart';

/// Storm v2 slice 6: the Agents activity on the real router and shell, at
/// both sides of the breakpoint (handoff §2.6–§2.8, §3.3–§3.7; plan §6.1).
void main() {
  const phone = Size(411, 900);
  const wide = Size(1400, 900);
  const primary = FakeServer.primaryVault;

  setUp(() => SharedPreferences.setMockInitialValues({}));

  Uri where(ProviderContainer c) => c.read(routerProvider).state.uri;

  /// `n3` is `Projects/Storm/Design.md` in the shell harness's fixture.
  Map<String, dynamic> designNote() => {
    'vault_id': primary,
    'note_id': 'n3',
    'title': 'Design',
  };

  Future<ProviderContainer> open(
    WidgetTester tester,
    FakeAgentServer agents, {
    Size size = wide,
    String at = Routes.agents,
    bool workVault = true,
  }) async {
    final c = shellContainer(agentClient: agents.client);
    if (workVault) serverOf(c).addVault('v-work', 'Work');
    await pumpShell(tester, c, size: size);
    c.read(routerProvider).go(at);
    await tester.pumpAndSettle();
    return c;
  }

  Finder inField(String key, String text) =>
      find.descendant(of: find.byKey(Key(key)), matching: find.text(text));

  group('a session is a route', () {
    testWidgets('desk: a row opens it beside the sidebar; Overview returns', (
      tester,
    ) async {
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        sessions: [agentSession('ags_1')],
      );
      final c = await open(tester, agents);
      await tester.tap(find.byKey(const Key('session-ags_1')));
      await tester.pumpAndSettle();
      expect(where(c).path, '/agents/s/ags_1');
      expect(find.byKey(const Key('session-name')), findsOneWidget);
      expect(find.byKey(const Key('agents-overview')), findsOneWidget);
      await tester.tap(find.byKey(const Key('agents-overview')));
      await tester.pumpAndSettle();
      expect(where(c).path, Routes.agents);
      expect(find.text('WORK'), findsOneWidget);
      await disposeShell(tester, c);
    });

    testWidgets('phone: a deep link backs out to /agents', (tester) async {
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        sessions: [agentSession('ags_1')],
      );
      final c = await open(
        tester,
        agents,
        size: phone,
        at: Routes.agentSession('ags_1'),
      );
      expect(find.text('gateway-spec'), findsOneWidget);
      expect(find.byKey(const Key('back-to-agents')), findsOneWidget);
      expect(await tester.binding.handlePopRoute(), isTrue);
      await tester.pumpAndSettle();
      expect(where(c).path, Routes.agents);
      await disposeShell(tester, c);
    });

    testWidgets('a session that is gone says so, at both widths', (
      tester,
    ) async {
      for (final size in [wide, phone]) {
        final c = await open(
          tester,
          FakeAgentServer(hosts: [agentHost()]),
          size: size,
          at: Routes.agentSession('ags_gone'),
        );
        expect(find.text('This session is gone'), findsOneWidget);
        await disposeShell(tester, c);
      }
    });
  });

  group('the desk panel', () {
    testWidgets('Context shows the note read only, and the tab is the route', (
      tester,
    ) async {
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        sessions: [
          agentSession('ags_1', context: designNote(), writeVaultId: primary),
        ],
      );
      final c = await open(tester, agents, at: Routes.agentSession('ags_1'));
      expect(find.byKey(const Key('panel-note-title')), findsOneWidget);
      expect(find.text('Design'), findsWidgets);
      expect(find.text('Primary / Projects / Storm'), findsOneWidget);
      expect(find.text('Open in Notes ›'), findsOneWidget);

      await tester.tap(find.text('Wrote 0'));
      await tester.pumpAndSettle();
      expect(where(c).queryParameters['tab'], 'wrote');
      expect(
        find.text(
          'Nothing written yet. Notes appear here as the session writes them.',
        ),
        findsOneWidget,
      );

      await tester.tap(find.byKey(const Key('panel-tab-SessionTab.context')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const Key('open-in-notes')));
      await tester.pumpAndSettle();
      expect(where(c).path, Routes.note(primary, 'n3'));
      await disposeShell(tester, c);
    });

    testWidgets('the empty copies say why', (tester) async {
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        sessions: [agentSession('ags_1')],
      );
      final c = await open(tester, agents, at: Routes.agentSession('ags_1'));
      expect(
        find.text(
          'Started without a note. Notes this session writes appear under '
          'Wrote.',
        ),
        findsOneWidget,
      );
      c.read(routerProvider).go(Routes.agentSession('ags_1', tab: 'wrote'));
      await tester.pumpAndSettle();
      expect(find.text('This session can’t write to your vaults.'), findsOne);
      await disposeShell(tester, c);
    });

    testWidgets('About says what the session may touch', (tester) async {
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        sessions: [
          agentSession('ags_1', writeVaultId: 'v-work'),
          agentSession('ags_sh', provider: 'shell', name: 'storm-2'),
        ],
      );
      final c = await open(
        tester,
        agents,
        at: Routes.agentSession('ags_1', tab: 'about'),
      );
      expect(find.text('storm on build-vm'), findsOneWidget);
      expect(
        find.text('Reads all vaults. Writes to Work. Never deletes.'),
        findsOneWidget,
      );
      expect(find.text('Inherits build-vm’s policy'), findsOneWidget);
      c.read(routerProvider).go(Routes.agentSession('ags_sh', tab: 'about'));
      await tester.pumpAndSettle();
      expect(
        find.text('None. Shell sessions have no Storm access.'),
        findsOneWidget,
      );
      await disposeShell(tester, c);
    });
  });

  group('End is confirmed in place, never in a dialog (H11)', () {
    testWidgets('desk: Cancel keeps it running; End session ends it', (
      tester,
    ) async {
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        sessions: [agentSession('ags_1')],
      );
      final c = await open(tester, agents, at: Routes.agentSession('ags_1'));
      await tester.tap(find.byKey(const Key('end-session')));
      await tester.pumpAndSettle();
      expect(find.byType(AlertDialog), findsNothing);
      expect(find.text('End gateway-spec'), findsOneWidget);
      expect(
        find.text(
          'The agent and everything it started are stopped on the host.',
        ),
        findsOneWidget,
      );
      // One compact row at desk width: the title, its message and the actions
      // share a line, and the banner stays a small part of the terminal's height.
      final title = tester.getRect(find.byKey(const Key('confirm-title')));
      final action = tester.getRect(find.byKey(const Key('confirm-action')));
      expect((title.center.dy - action.center.dy).abs(), lessThan(2));
      expect(
        tester.getSize(find.byKey(const Key('end-confirm'))).height,
        lessThan(64),
      );
      await tester.tap(find.byKey(const Key('confirm-cancel')));
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('end-confirm')), findsNothing);
      expect(agents.count('POST', '/v1/agent/sessions/ags_1/end'), 0);

      await tester.tap(find.byKey(const Key('end-session')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const Key('confirm-action')));
      await tester.pumpAndSettle();
      expect(agents.count('POST', '/v1/agent/sessions/ags_1/end'), 1);
      await disposeShell(tester, c);
    });

    testWidgets('phone: the details sheet holds End and its confirmation', (
      tester,
    ) async {
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        sessions: [agentSession('ags_1')],
      );
      final c = await open(
        tester,
        agents,
        size: phone,
        at: Routes.agentSession('ags_1'),
      );
      await tester.tap(find.byKey(const Key('chip-details')));
      await tester.pumpAndSettle();
      expect(find.text('AGENT'), findsOneWidget);
      await tester.tap(find.byKey(const Key('end-session')));
      await tester.pumpAndSettle();
      expect(
        find.text(
          'The agent and everything it started are stopped on the host.',
        ),
        findsOneWidget,
      );
      await tester.tap(find.byKey(const Key('confirm-action')));
      await tester.pumpAndSettle();
      expect(agents.count('POST', '/v1/agent/sessions/ags_1/end'), 1);
      await disposeShell(tester, c);
    });
  });

  group('Run again and Dismiss (handoff §3.7)', () {
    Map<String, dynamic> ended({String host = 'hst_1'}) => agentSession(
      'ags_done',
      status: 'completed',
      exitCode: 0,
      name: 'design',
      hostId: host,
      workspace: 'site',
      provider: 'opencode',
      context: designNote(),
      writeVaultId: 'v-work',
    );

    testWidgets('Run again prefills everything and still waits for Launch', (
      tester,
    ) async {
      final agents = FakeAgentServer(hosts: [agentHost()], sessions: [ended()]);
      final c = await open(tester, agents, at: Routes.agentSession('ags_done'));
      expect(find.byKey(const Key('ended-footer')), findsOneWidget);
      await tester.tap(find.byKey(const Key('run-again')));
      await tester.pumpAndSettle();

      expect(inField('launcher-host', 'build-vm · online'), findsOneWidget);
      expect(inField('launcher-workspace', 'site'), findsOneWidget);
      expect(inField('launcher-agent', 'OpenCode'), findsOneWidget);
      expect(inField('launcher-write-vault', 'Work'), findsOneWidget);
      expect(inField('launcher-context', 'Design'), findsOneWidget);
      expect(agents.launches, isEmpty);

      await tester.tap(find.byKey(const Key('launch')));
      await tester.pumpAndSettle();
      final body = agents.launches.single;
      expect(body['host_id'], 'hst_1');
      expect(body['workspace'], 'site');
      expect(body['provider'], 'opencode');
      expect(body['context'], {'vault_id': primary, 'note_id': 'n3'});
      expect(body['write_vault_id'], 'v-work');
      expect(body.containsKey('allow_vault_writes'), isFalse);
      expect(where(c).path, startsWith('/agents/s/ags_new'));
      expect(where(c).queryParameters['tab'], 'context');
      await disposeShell(tester, c);
    });

    testWidgets('an offline original host gives way to an online one', (
      tester,
    ) async {
      final agents = FakeAgentServer(
        hosts: [
          agentHost(id: 'hst_old', name: 'mac-mini', status: 'offline'),
          agentHost(id: 'hst_2', name: 'codebox'),
        ],
        sessions: [ended(host: 'hst_old')],
      );
      final c = await open(tester, agents, at: Routes.agentSession('ags_done'));
      await tester.tap(find.byKey(const Key('run-again')));
      await tester.pumpAndSettle();
      expect(inField('launcher-host', 'codebox · online'), findsOneWidget);
      await disposeShell(tester, c);
    });

    testWidgets('Dismiss removes it and returns to the list', (tester) async {
      final agents = FakeAgentServer(hosts: [agentHost()], sessions: [ended()]);
      final c = await open(
        tester,
        agents,
        size: phone,
        at: Routes.agentSession('ags_done'),
      );
      expect(find.byKey(const Key('key-esc')), findsNothing);
      await tester.tap(find.byKey(const Key('dismiss-session')));
      await tester.pumpAndSettle();
      expect(agents.count('DELETE', '/v1/agent/sessions/ags_done'), 1);
      expect(where(c).path, Routes.agents);
      await disposeShell(tester, c);
    });
  });

  testWidgets('Wrote is fetched again when wrote_count moves, and only then', (
    tester,
  ) async {
    final agents = FakeAgentServer(
      hosts: [agentHost()],
      sessions: [agentSession('ags_1', writeVaultId: primary)],
    );
    final c = await open(
      tester,
      agents,
      at: Routes.agentSession('ags_1', tab: 'wrote'),
    );
    const writes = '/v1/agent/sessions/ags_1/writes';
    expect(agents.count('GET', writes), 1);
    expect(find.textContaining('Nothing written yet'), findsOneWidget);

    await tester.pump(sessionRefreshInterval + const Duration(seconds: 1));
    await tester.pumpAndSettle();
    expect(agents.count('GET', writes), 1, reason: 'the count did not move');

    agents.session('ags_1')!['wrote_count'] = 2;
    agents.writes['ags_1'] = [
      {
        'vault_id': primary,
        'note_id': 'n4',
        'title': 'Plan',
        'path': 'Agents/Plan.md',
        'kind': 'created',
        'version': 3,
        'at': '2026-10-08T10:00:00Z',
      },
      {
        'vault_id': primary,
        'note_id': null,
        'title': 'tool.sh',
        'path': 'scripts/tool.sh',
        'kind': 'script_edited',
        'version': null,
        'at': '2026-10-08T09:00:00Z',
      },
    ];
    await tester.pump(sessionRefreshInterval + const Duration(seconds: 1));
    await tester.pumpAndSettle();
    expect(agents.count('GET', writes), 2);
    expect(find.text('Plan'), findsOneWidget);
    expect(find.text('new'), findsOneWidget);
    expect(find.text('Wrote 2'), findsOneWidget);
    // A kit script is a file under its path, never a note link.
    expect(find.text('tool.sh'), findsOneWidget);
    expect(find.text('Primary / scripts'), findsOneWidget);
    await tester.tap(find.text('tool.sh'));
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('back-to-wrote')), findsNothing);
    await tester.tap(find.text('Plan'));
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('back-to-wrote')), findsOneWidget);
    await disposeShell(tester, c);
  });

  testWidgets('Start session on a note launches with it as context', (
    tester,
  ) async {
    final agents = FakeAgentServer(hosts: [agentHost()]);
    final c = await open(tester, agents, at: Routes.note(primary, 'n3'));
    await tester.tap(find.byKey(const Key('start-session')));
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('launcher-context')), findsOneWidget);
    expect(inField('launcher-write-vault', 'Primary'), findsOneWidget);
    await tester.tap(find.byKey(const Key('launch')));
    await tester.pumpAndSettle();
    expect(agents.launches.single['context'], {
      'vault_id': primary,
      'note_id': 'n3',
    });
    expect(where(c).path, startsWith('/agents/s/'));
    expect(where(c).queryParameters['tab'], 'context');
    await disposeShell(tester, c);
  });

  group('the launcher (handoff §3.5)', () {
    Future<void> launchFrom(
      WidgetTester tester, {
      ({String vaultId, String noteId})? note,
    }) async {
      final ref = tester.element(find.byType(AgentsScreen)) as WidgetRef;
      launchAgentSession(
        tester.element(find.byType(AgentsScreen)),
        ref,
        contextNote: note,
      );
      await tester.pumpAndSettle();
    }

    testWidgets(
      'from a note: the last host, its first workspace, the default agent, '
      'and the note’s vault to write to',
      (tester) async {
        SharedPreferences.setMockInitialValues({'agent.lastHost': 'hst_2'});
        final agents = FakeAgentServer(
          hosts: [
            agentHost(),
            agentHost(
              id: 'hst_2',
              name: 'codebox',
              workspaces: ['notes', 'site'],
            ),
          ],
          sessions: [agentSession('ags_1')],
        );
        final c = await open(tester, agents);
        await launchFrom(tester, note: (vaultId: 'v-work', noteId: 'w1'));

        expect(inField('launcher-host', 'codebox · online'), findsOneWidget);
        expect(inField('launcher-workspace', 'notes'), findsOneWidget);
        expect(
          inField('launcher-agent', 'Claude Code · default'),
          findsOneWidget,
        );
        expect(inField('launcher-write-vault', 'Work'), findsOneWidget);
        expect(find.text('The agent reads the note through Storm.'), findsOne);

        await tester.tap(find.byKey(const Key('launch')));
        await tester.pumpAndSettle();
        final body = agents.launches.single;
        expect(body['context'], {'vault_id': 'v-work', 'note_id': 'w1'});
        expect(body['write_vault_id'], 'v-work');
        expect(body.containsKey('allow_vault_writes'), isFalse);
        expect(jsonEncode(body), isNot(contains('allow_vault_writes')));
        await disposeShell(tester, c);
      },
    );

    testWidgets('without a note it writes to the first vault, or not at all', (
      tester,
    ) async {
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        sessions: [agentSession('ags_1')],
      );
      final c = await open(tester, agents);
      await tester.tap(find.byKey(const Key('new-session')));
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('launcher-context')), findsNothing);
      expect(inField('launcher-write-vault', 'Primary'), findsOneWidget);

      await tester.tap(find.byKey(const Key('launcher-writes')));
      await tester.pumpAndSettle();
      expect(find.text('Read only'), findsOneWidget);
      await tester.tap(find.byKey(const Key('launch')));
      await tester.pumpAndSettle();
      expect(agents.launches.single.containsKey('write_vault_id'), isFalse);
      expect(agents.launches.single.containsKey('context'), isFalse);
      await disposeShell(tester, c);
    });

    testWidgets('writes off in Settings: the toggle is disabled and says so', (
      tester,
    ) async {
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        sessions: [agentSession('ags_1')],
        agentWrites: false,
      );
      final c = await open(tester, agents);
      await tester.tap(find.byKey(const Key('new-session')));
      await tester.pumpAndSettle();
      expect(find.text('Off in Settings › AI access'), findsOneWidget);
      expect(find.byKey(const Key('launcher-write-vault')), findsNothing);
      await tester.tap(find.byKey(const Key('launcher-writes')));
      await tester.pumpAndSettle();
      expect(find.text('Off in Settings › AI access'), findsOneWidget);
      await tester.tap(find.byKey(const Key('launch')));
      await tester.pumpAndSettle();
      expect(agents.launches.single.containsKey('write_vault_id'), isFalse);
      await disposeShell(tester, c);
    });

    testWidgets('a shell has no write field, and never sends a vault', (
      tester,
    ) async {
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        sessions: [agentSession('ags_1')],
        defaultProvider: 'shell',
      );
      final c = await open(tester, agents);
      await tester.tap(find.byKey(const Key('new-session')));
      await tester.pumpAndSettle();
      expect(inField('launcher-agent', 'Shell · default'), findsOneWidget);
      expect(find.byKey(const Key('launcher-writes')), findsNothing);
      await tester.tap(find.byKey(const Key('launch')));
      await tester.pumpAndSettle();
      expect(agents.launches.single.containsKey('write_vault_id'), isFalse);
      expect(where(c).queryParameters['tab'], 'about');
      await disposeShell(tester, c);
    });

    testWidgets('a centred modal at desk width, a bottom sheet on a phone', (
      tester,
    ) async {
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        sessions: [agentSession('ags_1')],
      );
      var c = await open(tester, agents);
      await tester.tap(find.byKey(const Key('new-session')));
      await tester.pumpAndSettle();
      expect(find.byType(BottomSheet), findsNothing);
      expect(
        tester.getSize(find.byKey(const Key('launch'))).width,
        lessThan(200),
      );
      final box = tester.getRect(find.byKey(const Key('launcher-host')));
      expect(box.center.dx, closeTo(wide.width / 2 + 48, 120));
      await disposeShell(tester, c);

      c = await open(tester, agents, size: phone);
      await tester.tap(find.byKey(const Key('new-session-pill')));
      await tester.pumpAndSettle();
      expect(find.byType(BottomSheet), findsOneWidget);
      await disposeShell(tester, c);
    });
  });

  group('the states, at both sides of 900', () {
    testWidgets('first session: start from a note, or without one', (
      tester,
    ) async {
      final agents = FakeAgentServer(hosts: [agentHost()]);
      var c = await open(tester, agents);
      expect(find.text('Start your first session'), findsOneWidget);
      expect(
        find.textContaining('build-vm is online with Claude Code and OpenCode'),
        findsOneWidget,
      );
      expect(find.byKey(const Key('first-new-session')), findsOneWidget);
      expect(find.byKey(const Key('new-session')), findsOneWidget);
      await disposeShell(tester, c);

      c = await open(tester, agents, size: phone);
      expect(
        find.text('build-vm is online. Start from a note, or tap ＋.'),
        findsOneWidget,
      );
      expect(find.byKey(const Key('new-session-pill')), findsOneWidget);
      expect(find.byKey(const Key('new-session')), findsNothing);
      await disposeShell(tester, c);
    });

    testWidgets('no host: three steps; the button only at desk width', (
      tester,
    ) async {
      var c = await open(tester, FakeAgentServer());
      expect(find.byKey(const Key('step-1')), findsOneWidget);
      expect(find.byKey(const Key('enroll-host-step')), findsOneWidget);
      await disposeShell(tester, c);

      c = await open(tester, FakeAgentServer(), size: phone);
      expect(
        find.text(
          'Agents run on a machine you own. Sessions keep going when this '
          'phone is off.',
        ),
        findsOneWidget,
      );
      expect(find.byKey(const Key('step-3')), findsOneWidget);
      expect(find.byKey(const Key('enroll-host-step')), findsNothing);
      expect(find.byKey(const Key('new-session-pill')), findsNothing);
      await disposeShell(tester, c);
    });

    testWidgets('the overview groups work and says what each one is doing', (
      tester,
    ) async {
      final agents = FakeAgentServer(
        hosts: [
          agentHost(),
          agentHost(id: 'hst_2', name: 'mac-mini', status: 'offline'),
        ],
        sessions: [
          agentSession(
            'ags_ctx',
            context: designNote(),
            wroteCount: 2,
            writeVaultId: primary,
          ),
          agentSession('ags_site', workspace: 'site', name: 'site-1'),
          agentSession(
            'ags_fail',
            status: 'failed',
            endReason: 'host_restart',
            hostId: 'hst_2',
            name: 'storm-1',
          ),
        ],
      );
      final c = await open(tester, agents);
      expect(find.text('Design'), findsOneWidget, reason: 'context chip');
      expect(find.text('wrote 2'), findsOneWidget);
      expect(find.text('on build-vm'), findsNWidgets(2));
      expect(find.text('on mac-mini'), findsOneWidget);
      expect(find.byKey(const Key('agent-card-claude-code')), findsOneWidget);
      expect(find.byKey(const Key('agent-card-shell')), findsOneWidget);
      expect(find.textContaining('1 of 2 hosts online'), findsOneWidget);
      expect(find.text('mac-mini · storm · failed'), findsOneWidget);
      await tester.tap(find.byKey(const Key('work-ags_site')));
      await tester.pumpAndSettle();
      expect(where(c).path, '/agents/s/ags_site');
      await disposeShell(tester, c);
    });
  });
}
