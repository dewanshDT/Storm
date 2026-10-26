import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'package:storm/router.dart';

import 'agent_fakes.dart';
import 'fake_server.dart';
import 'real_fonts.dart';
import 'shell_harness.dart';

/// Decision 138, slice 138-4: the launcher's Crew and Role rows, its quiet
/// line, the Role row for an agent that can't read Storm notes, and a chevron
/// on every select (138g, 138h; plan §4.3, §6.3).
void main() {
  const wide = Size(1400, 900);
  const primary = FakeServer.primaryVault;

  setUpAll(loadRealFonts);
  setUp(() => SharedPreferences.setMockInitialValues({}));

  List<Map<String, dynamic>> roles() => [
    kitRole(
      'k_lead',
      'Storm Lead',
      summary: 'Runs the orchestrator loop; the single writer to work/.',
    ),
    kitRole(
      'k_rev',
      'Storm Reviewer',
      summary: 'Checks finished work; writes nothing.',
    ),
    kitRole(
      'k_sop',
      'SOP Reviewer',
      folder: 'projects/sop/agents',
      summary: 'Reviews a written procedure for gaps.',
    ),
  ];

  FakeAgentServer server({
    Map<String, dynamic>? host,
    String defaultProvider = 'claude-code',
    List<Map<String, dynamic>>? sessions,
  }) => FakeAgentServer(
    hosts: [host ?? agentHost()],
    sessions: sessions ?? [agentSession('ags_1')],
    crews: [agentCrew('crw_1')],
    roles: roles(),
    defaultProvider: defaultProvider,
  );

  Future<ProviderContainer> launcher(
    WidgetTester t,
    FakeAgentServer agents,
  ) async {
    final c = shellContainer(agentClient: agents.client);
    serverOf(c).addVault('v-work', 'Work');
    await pumpShell(t, c, size: wide);
    c.read(routerProvider).go(Routes.agents);
    await t.pumpAndSettle();
    await t.tap(find.byKey(const Key('new-session')));
    await t.pumpAndSettle();
    return c;
  }

  Finder inField(String key, String text) =>
      find.descendant(of: find.byKey(Key(key)), matching: find.text(text));

  Future<void> pick(WidgetTester t, String field, Finder item) async {
    await t.tap(find.byKey(Key(field)));
    await t.pumpAndSettle();
    await t.tap(item);
    await t.pumpAndSettle();
  }

  double top(WidgetTester t, String key) =>
      t.getTopLeft(find.byKey(Key(key))).dy;

  testWidgets('every select has a chevron; Crew and Role come after Can '
      'write to, both None', (t) async {
    final c = await launcher(t, server());
    for (final key in [
      'launcher-host',
      'launcher-workspace',
      'launcher-agent',
      'launcher-write-vault',
      'launcher-crew',
      'launcher-role',
    ]) {
      expect(
        find.descendant(
          of: find.byKey(Key(key)),
          matching: find.byKey(const Key('select-chevron')),
        ),
        findsOneWidget,
        reason: key,
      );
    }
    expect(find.byKey(const Key('select-chevron')), findsNWidgets(6));
    expect(top(t, 'launcher-write-vault'), lessThan(top(t, 'launcher-crew')));
    expect(top(t, 'launcher-crew'), lessThan(top(t, 'launcher-role')));
    expect(inField('launcher-crew', 'None'), findsOneWidget);
    expect(inField('launcher-role', 'None'), findsOneWidget);
    await disposeShell(t, c);
  });

  testWidgets('roles are grouped by folder with their summaries, and the '
      'one picked is sent', (t) async {
    final agents = server();
    final c = await launcher(t, agents);
    await t.tap(find.byKey(const Key('launcher-role')));
    await t.pumpAndSettle();
    final menu = find.byKey(const Key('role-menu'));
    Finder inMenu(String text) =>
        find.descendant(of: menu, matching: find.text(text));
    expect(inMenu('A plain agent'), findsOneWidget);
    expect(inMenu('AGENTS'), findsOneWidget);
    expect(inMenu('PROJECTS / SOP / AGENTS'), findsOneWidget);
    expect(inMenu('Checks finished work; writes nothing.'), findsOneWidget);
    expect(inMenu('Reviews a written procedure for gaps.'), findsOneWidget);
    double y(String text) => t.getTopLeft(inMenu(text)).dy;
    expect(y('AGENTS'), lessThan(y('Storm Lead')));
    expect(y('Storm Reviewer'), lessThan(y('PROJECTS / SOP / AGENTS')));
    expect(y('PROJECTS / SOP / AGENTS'), lessThan(y('SOP Reviewer')));

    await t.tap(find.byKey(const Key('role-k_rev')));
    await t.pumpAndSettle();
    expect(inField('launcher-role', 'Storm Reviewer'), findsOneWidget);
    // A write vault is granted by default.
    expect(
      find.text('The role note may tell the agent not to write.'),
      findsOneWidget,
    );
    await t.tap(find.byKey(const Key('launcher-writes')));
    await t.pumpAndSettle();
    expect(
      find.text('The role note may tell the agent not to write.'),
      findsNothing,
    );

    await t.tap(find.byKey(const Key('launch')));
    await t.pumpAndSettle();
    final body = agents.launches.single;
    expect(body['role'], {'vault_id': 'v-kit', 'note_id': 'k_rev'});
    expect(body.containsKey('crew'), isFalse);
    await disposeShell(t, c);
  });

  group("an agent that can't read Storm notes (138g)", () {
    testWidgets('Shell: Role disabled with its reason, never sent', (t) async {
      final agents = server(defaultProvider: 'shell');
      final c = await launcher(t, agents);
      expect(
        find.text(
          "Shell can't read Storm notes. A role would only be a label.",
        ),
        findsOneWidget,
      );
      await t.tap(find.byKey(const Key('launcher-role')));
      await t.pumpAndSettle();
      expect(find.byKey(const Key('role-menu')), findsNothing);
      // Crew still works for it.
      expect(find.byKey(const Key('launcher-crew')), findsOneWidget);
      await t.tap(find.byKey(const Key('launch')));
      await t.pumpAndSettle();
      expect(agents.launches.single.containsKey('role'), isFalse);
      await disposeShell(t, c);
    });

    testWidgets('decided by what the host says, never by the name', (t) async {
      // Claude Code without Storm's connection is refused a role; Cursor
      // with it is offered one.
      final c = await launcher(
        t,
        server(
          host: agentHost(
            providers: ['claude-code', 'cursor'],
            mcp: {'claude-code': false, 'cursor': true},
          ),
        ),
      );
      expect(
        find.text(
          "Claude Code can't read Storm notes. A role would only be a label.",
        ),
        findsOneWidget,
      );
      await pick(t, 'launcher-agent', find.text('Cursor').last);
      expect(find.byKey(const Key('launcher-role-label-only')), findsNothing);
      await t.tap(find.byKey(const Key('launcher-role')));
      await t.pumpAndSettle();
      expect(find.byKey(const Key('role-menu')), findsOneWidget);
      await disposeShell(t, c);
    });
  });

  group('the Crew row', () {
    Map<String, dynamic> ended() => agentSession(
      'ags_done',
      status: 'completed',
      context: {'vault_id': primary, 'note_id': 'n3', 'title': 'Design'},
    );

    Future<ProviderContainer> runAgain(
      WidgetTester t,
      FakeAgentServer agents,
    ) async {
      final c = shellContainer(agentClient: agents.client);
      await pumpShell(t, c, size: wide);
      c.read(routerProvider).go(Routes.agentSession('ags_done'));
      await t.pumpAndSettle();
      await t.tap(find.byKey(const Key('run-again')));
      await t.pumpAndSettle();
      return c;
    }

    testWidgets('None, "New crew: {context note}", then the crews; a new '
        'crew is sent by name', (t) async {
      final agents = server(sessions: [ended()]);
      final c = await runAgain(t, agents);
      await t.tap(find.byKey(const Key('launcher-crew')));
      await t.pumpAndSettle();
      final none = find.text('None').last;
      final made = find.text('New crew: Design');
      final existing = find.text('Gateway release').last;
      expect(made, findsOneWidget);
      expect(t.getTopLeft(none).dy, lessThan(t.getTopLeft(made).dy));
      expect(t.getTopLeft(made).dy, lessThan(t.getTopLeft(existing).dy));
      await t.tap(made);
      await t.pumpAndSettle();
      await t.tap(find.byKey(const Key('launch')));
      await t.pumpAndSettle();
      expect(agents.launches.single['crew'], {'new_name': 'Design'});
      await disposeShell(t, c);
    });

    testWidgets('an existing crew is sent by id', (t) async {
      final agents = server(sessions: [ended()]);
      final c = await runAgain(t, agents);
      await pick(t, 'launcher-crew', find.text('Gateway release').last);
      await t.tap(find.byKey(const Key('launch')));
      await t.pumpAndSettle();
      expect(agents.launches.single['crew'], {'id': 'crw_1'});
      await disposeShell(t, c);
    });

    testWidgets('no context note, no New crew', (t) async {
      final c = await launcher(t, server());
      await t.tap(find.byKey(const Key('launcher-crew')));
      await t.pumpAndSettle();
      expect(find.textContaining('New crew'), findsNothing);
      expect(find.text('Gateway release'), findsWidgets);
      await disposeShell(t, c);
    });

    testWidgets('a server before crews: no Crew row, Role offers only None', (
      t,
    ) async {
      final agents = server()..crewsSupported = false;
      final c = await launcher(t, agents);
      expect(find.byKey(const Key('launcher-crew')), findsNothing);
      await t.tap(find.byKey(const Key('launcher-role')));
      await t.pumpAndSettle();
      expect(find.byKey(const Key('role-none')), findsOneWidget);
      expect(find.byKey(const Key('role-k_rev')), findsNothing);
      await t.tapAt(const Offset(10, 10));
      await t.pumpAndSettle();
      await t.tap(find.byKey(const Key('launch')));
      await t.pumpAndSettle();
      expect(agents.launches.single.containsKey('crew'), isFalse);
      expect(agents.launches.single.containsKey('role'), isFalse);
      await disposeShell(t, c);
    });
  });

  group('"Allow this session to start agents" (141e)', () {
    Map<String, dynamic> c0({String identity = 'verified'}) => agentHost(
      providers: ['claude-code', 'opencode', 'shell', 'cursor'],
      mcp: {'cursor': false},
      identity: identity,
      isolation: 'verified',
    );

    testWidgets('off by default, sent only when turned on', (t) async {
      final agents = server(host: c0());
      final c = await launcher(t, agents);
      expect(find.byKey(const Key('launcher-spawn')), findsOneWidget);
      expect(
        find.text('You approve each start, message and end.'),
        findsOneWidget,
      );
      await t.tap(find.byKey(const Key('launch')));
      await t.pumpAndSettle();
      expect(agents.launches.single.containsKey('spawn'), isFalse);
      await disposeShell(t, c);

      final again = server(host: c0());
      final c2 = await launcher(t, again);
      await t.tap(find.byKey(const Key('launcher-spawn')));
      await t.pumpAndSettle();
      await t.tap(find.byKey(const Key('launch')));
      await t.pumpAndSettle();
      expect(again.launches.single['spawn'], true);
      await disposeShell(t, c2);
    });

    testWidgets('only for a host that passes C0 and an agent that can '
        'start agents', (t) async {
      for (final host in [
        agentHost(),
        agentHost(identity: 'verified'),
        agentHost(isolation: 'verified'),
        c0(identity: 'unavailable'),
      ]) {
        final c = await launcher(t, server(host: host));
        expect(find.byKey(const Key('launcher-spawn')), findsNothing);
        await disposeShell(t, c);
      }
      final agents = server(host: c0());
      final c = await launcher(t, agents);
      await t.tap(find.byKey(const Key('launcher-spawn')));
      await t.pumpAndSettle();
      for (final other in ['Shell', 'Cursor']) {
        await pick(t, 'launcher-agent', find.text(other).last);
        expect(
          find.byKey(const Key('launcher-spawn')),
          findsNothing,
          reason: other,
        );
      }
      await t.tap(find.byKey(const Key('launch')));
      await t.pumpAndSettle();
      expect(agents.launches.single.containsKey('spawn'), isFalse);
      await disposeShell(t, c);
    });
  });
}
