import 'dart:async';
import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'package:storm/agent/agent_state.dart';
import 'package:storm/agent/spawn_models.dart';
import 'package:storm/agent/spawn_widgets.dart';
import 'package:storm/router.dart';
import 'package:storm/ui/controls.dart';

import 'agent_fakes.dart';
import 'shell_harness.dart';
import 'spawn_fakes.dart';

/// Decision 134-10: the spawning client on the real router and shell, on
/// both sides of the 900px breakpoint. Everything it draws comes from the
/// server's views, so a host that cannot offer spawning shows nothing of it.
void main() {
  const phone = Size(899, 900);
  const edge = Size(900, 900);
  const wide = Size(1400, 900);

  setUp(() => SharedPreferences.setMockInitialValues({}));

  Future<ProviderContainer> open(
    WidgetTester tester,
    FakeAgentServer agents, {
    Size size = wide,
    String at = Routes.agents,
  }) async {
    final c = shellContainer(agentClient: agents.client);
    serverOf(c).addVault('v-work', 'Work');
    await pumpShell(tester, c, size: size);
    c.read(routerProvider).go(at);
    await tester.pumpAndSettle();
    return c;
  }

  Uri where(ProviderContainer c) => c.read(routerProvider).state.uri;

  /// A session's About on a wide window, where the switch lives (141e).
  Future<ProviderContainer> about(
    WidgetTester tester,
    FakeAgentServer agents,
    String id,
  ) async {
    final c = await open(
      tester,
      agents,
      at: Routes.agentSession(id, tab: SessionTab.about.name),
    );
    if (find.byKey(const Key('session-about')).evaluate().isEmpty) {
      await tester.tap(find.byKey(const Key('inspector-toggle')));
      await tester.pumpAndSettle();
    }
    expect(find.byKey(const Key('session-about')), findsOneWidget);
    return c;
  }

  List<Map<String, dynamic>> sent(FakeAgentServer a, String method, String p) =>
      [
        for (final r in a.requests)
          if (r.method == method && r.url.path == p)
            r.body.isEmpty ? {} : jsonDecode(r.body) as Map<String, dynamic>,
      ];

  /// A parent whose host passed its gates, its switch on.
  Map<String, dynamic> parent({String? needs, String? writeVault}) =>
      spawnSession(
        'ags_p',
        spawn: {'enabled': true},
        needsAttention: needs,
        writeVaultId: writeVault,
      );

  group('dark: a host that cannot offer spawning shows none of it', () {
    for (final size in [wide, edge, phone]) {
      testWidgets('at ${size.width.toInt()}px', (tester) async {
        // Today's view: no `spawn`, no lineage, no attention, and a host
        // whose identity and isolation are unverified.
        final agents = FakeAgentServer(
          hosts: [agentHost()],
          sessions: [
            {
              ...agentSession('ags_1'),
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
        final c = await open(
          tester,
          agents,
          size: size,
          at: Routes.agentSession('ags_1', tab: SessionTab.about.name),
        );
        if (size.width < 900) {
          await tester.tap(find.byKey(const Key('session-header')));
          await tester.pumpAndSettle();
        } else if (find.byKey(const Key('session-about')).evaluate().isEmpty) {
          await tester.tap(find.byKey(const Key('inspector-toggle')));
          await tester.pumpAndSettle();
        }
        expect(find.byKey(const Key('spawn-switch')), findsNothing);
        expect(find.text('Allow this session to start agents'), findsNothing);
        expect(find.text('Requests'), findsNothing);
        expect(find.text('REQUESTS'), findsNothing);
        expect(find.text('Work'), findsNothing);
        expect(find.textContaining('↳'), findsNothing);
        expect(find.byKey(const Key('approval-banner')), findsNothing);
        expect(find.byType(ApprovalCard), findsNothing);
        expect(find.byType(AttentionTag), findsNothing);
        expect(find.byKey(const Key('attention-approval')), findsNothing);
        // The labels 134 asks for are the host's own: identity, isolation.
        expect(
          find.textContaining('Session identity not verified'),
          findsWidgets,
        );
        // Nothing even asks for spawning's routes.
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
        await disposeShell(tester, c);
      });
    }
  });

  group('the spawn switch', () {
    testWidgets('on asks first, with what it allows; off is at once', (
      tester,
    ) async {
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        sessions: [
          spawnSession('ags_p', spawn: {'enabled': false}),
        ],
      );
      final spawning = FakeSpawning(agents);
      final c = await about(tester, agents, 'ags_p');
      expect(find.text('Allow this session to start agents'), findsOneWidget);
      expect(find.byKey(const Key('spawn-switch-note')), findsOneWidget);
      expect(find.text('Off'), findsOneWidget);
      await tester.tap(find.byKey(const Key('spawn-switch')));
      await tester.pumpAndSettle();
      expect(
        find.text(
          'Allow gateway-impl to start agents? It can then ask to start, '
          'message and end sessions on build-vm. Each request waits for '
          'you, and children never get more access than gateway-impl. You '
          'can turn this off at any time: requests still waiting are '
          'refused then, and children keep running.',
        ),
        findsOneWidget,
      );
      expect(sent(agents, 'PUT', '/v1/agent/sessions/ags_p/spawn'), isEmpty);
      await tester.ensureVisible(find.byKey(const Key('banner-action')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const Key('banner-action')));
      await tester.pumpAndSettle();
      expect(sent(agents, 'PUT', '/v1/agent/sessions/ags_p/spawn'), [
        {'enabled': true},
      ]);
      expect(
        find.text(
          'On · you approve each start, message and end · Turn off any time',
        ),
        findsOneWidget,
      );

      // Off refuses what waits, at once, and says so (138p point 6).
      spawning.addRequest('ags_p', task: 'write tests');
      await tester.tap(find.byKey(const Key('spawn-switch')));
      await tester.pumpAndSettle();
      expect(sent(agents, 'PUT', '/v1/agent/sessions/ags_p/spawn').last, {
        'enabled': false,
      });
      expect(
        find.text(
          'Starting agents is off for gateway-impl. 1 waiting request was '
          'refused.',
        ),
        findsOneWidget,
      );
      expect(spawning.requests.single['reason'], 'revoked');
      await disposeShell(tester, c);
    });

    testWidgets('a specific reason where it cannot turn on (138p point 9)', (
      tester,
    ) async {
      for (final (code, text) in [
        ('workspace_held', 'Another session is working in this workspace'),
        (
          'launched_without_asks',
          'It started before this host could offer it. Start a new session '
              'to allow it',
        ),
        ('provider_cannot_spawn', "Claude Code can't start agents"),
      ]) {
        final agents = FakeAgentServer(
          hosts: [agentHost()],
          sessions: [
            spawnSession('ags_p', spawn: {'enabled': false, 'refused': code}),
          ],
        );
        FakeSpawning(agents);
        final c = await about(tester, agents, 'ags_p');
        expect(find.text('Unavailable · $text'), findsOneWidget);
        await tester.tap(find.byKey(const Key('spawn-switch')));
        await tester.pumpAndSettle();
        expect(find.byKey(const Key('spawn-on-confirm')), findsNothing);
        expect(sent(agents, 'PUT', '/v1/agent/sessions/ags_p/spawn'), isEmpty);
        await disposeShell(tester, c);
      }
    });

    testWidgets('141e: launched without it, About says how to get it', (
      tester,
    ) async {
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        sessions: [
          spawnSession(
            'ags_p',
            spawn: {
              'enabled': false,
              'launched_on': false,
              'refused': 'launched_without_spawn',
            },
          ),
        ],
      );
      FakeSpawning(agents);
      final c = await about(tester, agents, 'ags_p');
      expect(
        find.text(
          'To let an agent start agents, start a new session with "Allow '
          'this session to start agents" on.',
        ),
        findsOneWidget,
      );
      final toggle = tester.widget<StormToggle>(
        find.byKey(const Key('spawn-switch')),
      );
      expect(toggle.enabled, isFalse);
      await tester.tap(find.byKey(const Key('spawn-switch')));
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('spawn-on-confirm')), findsNothing);
      expect(sent(agents, 'PUT', '/v1/agent/sessions/ags_p/spawn'), isEmpty);
      await disposeShell(tester, c);
    });

    testWidgets('141e: launched with it, off and on again from About', (
      tester,
    ) async {
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        sessions: [
          spawnSession('ags_p', spawn: {'enabled': false, 'launched_on': true}),
        ],
      );
      FakeSpawning(agents);
      final c = await about(tester, agents, 'ags_p');
      expect(find.text('Off'), findsOneWidget);
      await tester.tap(find.byKey(const Key('spawn-switch')));
      await tester.pumpAndSettle();
      await tester.ensureVisible(find.byKey(const Key('banner-action')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const Key('banner-action')));
      await tester.pumpAndSettle();
      expect(sent(agents, 'PUT', '/v1/agent/sessions/ags_p/spawn'), [
        {'enabled': true},
      ]);
      await disposeShell(tester, c);
    });

    testWidgets('141e: not in the session header', (tester) async {
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        sessions: [
          spawnSession('ags_p', spawn: {'enabled': true, 'launched_on': true}),
        ],
      );
      FakeSpawning(agents);
      final c = await open(
        tester,
        agents,
        at: Routes.agentSession('ags_p', tab: SessionTab.context.name),
      );
      expect(find.byKey(const Key('session-bar')), findsOneWidget);
      expect(
        find.descendant(
          of: find.byKey(const Key('session-bar')),
          matching: find.byKey(const Key('spawn-switch')),
        ),
        findsNothing,
      );
      expect(
        find.descendant(
          of: find.byKey(const Key('session-bar')),
          matching: find.text('Allow this session to start agents'),
        ),
        findsNothing,
      );
      await disposeShell(tester, c);
    });

    testWidgets('phone: in the details sheet, below 900', (tester) async {
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        sessions: [
          spawnSession('ags_p', spawn: {'enabled': false}),
        ],
      );
      FakeSpawning(agents);
      final c = await open(
        tester,
        agents,
        size: phone,
        at: Routes.agentSession('ags_p'),
      );
      expect(find.byKey(const Key('spawn-switch')), findsNothing);
      await tester.tap(find.byKey(const Key('session-header')));
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('spawn-switch')), findsOneWidget);
      await disposeShell(tester, c);
    });
  });

  group('approval cards (134o)', () {
    testWidgets('a start: exact text, the host states, write off by default', (
      tester,
    ) async {
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        sessions: [parent(needs: 'approval', writeVault: 'v-work')],
      );
      final spawning = FakeSpawning(agents);
      const task =
          'Write tests for token refresh in gateway/oauth.rs. Run cargo test '
          '-p gateway and report failures.';
      spawning.addRequest('ags_p', task: task, name: 'test-gateway');
      final c = await open(tester, agents, at: Routes.agentSession('ags_p'));
      expect(find.byType(ApprovalCard), findsOneWidget);
      expect(find.text('START'), findsOneWidget);
      expect(
        find.text('gateway-impl wants to start a session'),
        findsOneWidget,
      );
      expect(
        find.textContaining(RegExp(r'^expires in 8:4\d$')),
        findsOneWidget,
      );
      expect(
        find.text('gateway-impl · Claude Code · build-vm'),
        findsOneWidget,
      );
      expect(find.text(task), findsOneWidget);
      expect(
        find.text(
          'Claude Code on build-vm, in a workspace of its own: a copy of '
          'storm’s last commit (uncommitted changes are not included), or '
          'storm read-only with a scratch folder if it isn’t a git '
          'repository',
        ),
        findsOneWidget,
      );
      expect(
        find.text('build-vm · identity verified · isolates sessions'),
        findsOneWidget,
      );
      expect(find.text('Let the child write to Work'), findsOneWidget);
      final toggle = tester.widget<StormToggle>(
        find.byKey(const Key('approval-write')),
      );
      expect(toggle.value, isFalse, reason: 'D2: off by default');

      await tester.tap(find.byKey(const Key('approval-approve')));
      await tester.pumpAndSettle();
      expect(sent(agents, 'POST', '/v1/agent/requests/agr_1/approve'), [
        {'write_vault': false},
      ]);
      expect(find.text('Approved: test-gateway is starting.'), findsOneWidget);
      expect(spawning.terminalWrites, 0);
      await disposeShell(tester, c);
    });

    testWidgets('a shared workspace is said on the card (141b)', (
      tester,
    ) async {
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        sessions: [parent(needs: 'approval')],
      );
      final spawning = FakeSpawning(agents);
      spawning.addRequest('ags_p', task: 't', sharedWith: 2);
      final c = await open(tester, agents, at: Routes.agentSession('ags_p'));
      expect(find.text('WORKSPACE'), findsOneWidget);
      expect(
        find.text('storm is shared with 2 other sessions'),
        findsOneWidget,
      );
      await disposeShell(tester, c);
    });

    testWidgets('an unshared workspace says nothing of it', (tester) async {
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        sessions: [parent(needs: 'approval')],
      );
      FakeSpawning(agents).addRequest('ags_p', task: 't', sharedWith: 0);
      final c = await open(tester, agents, at: Routes.agentSession('ags_p'));
      expect(find.byType(ApprovalCard), findsOneWidget);
      expect(find.textContaining('is shared with'), findsNothing);
      await disposeShell(tester, c);
    });

    testWidgets('ticking the write toggle is what grants it', (tester) async {
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        sessions: [parent(needs: 'approval', writeVault: 'v-work')],
      );
      final spawning = FakeSpawning(agents);
      spawning.addRequest('ags_p', task: 't');
      final c = await open(tester, agents, at: Routes.agentSession('ags_p'));
      await tester.tap(find.byKey(const Key('approval-write')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const Key('approval-approve')));
      await tester.pumpAndSettle();
      expect(sent(agents, 'POST', '/v1/agent/requests/agr_1/approve'), [
        {'write_vault': true},
      ]);
      await disposeShell(tester, c);
    });

    testWidgets('no toggle when the parent writes nowhere', (tester) async {
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        sessions: [parent(needs: 'approval')],
      );
      FakeSpawning(agents).addRequest('ags_p', task: 't');
      final c = await open(tester, agents, at: Routes.agentSession('ags_p'));
      expect(find.byKey(const Key('approval-write')), findsNothing);
      expect(find.textContaining('Let the child write'), findsNothing);
      await disposeShell(tester, c);
    });

    testWidgets('a message and an end: their exact text and target', (
      tester,
    ) async {
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        sessions: [
          parent(needs: 'approval'),
          spawnSession(
            'ags_c',
            name: 'test-gateway',
            parent: 'ags_p',
            depth: 1,
          ),
        ],
      );
      final spawning = FakeSpawning(agents);
      spawning.addRequest(
        'ags_p',
        kind: 'message',
        target: 'ags_c',
        text: 'Tests are on storm/test-gateway.\n  Review `refresh.rs`.',
      );
      final c = await open(tester, agents, at: Routes.agentSession('ags_p'));
      expect(find.text('MESSAGE'), findsWidgets);
      expect(
        find.text('gateway-impl wants to message test-gateway'),
        findsOneWidget,
      );
      expect(find.text('test-gateway, which it started'), findsOneWidget);
      expect(
        find.text('Tests are on storm/test-gateway.\n  Review `refresh.rs`.'),
        findsOneWidget,
      );
      await tester.tap(find.byKey(const Key('approval-deny')));
      await tester.pumpAndSettle();
      expect(
        sent(agents, 'POST', '/v1/agent/requests/agr_1/deny'),
        hasLength(1),
      );
      expect(find.text('Denied.'), findsOneWidget);

      spawning.addRequest('ags_p', kind: 'end', target: 'ags_c');
      c.invalidate(agentOverviewProvider);
      await tester.pump(const Duration(seconds: 5));
      await tester.pumpAndSettle();
      expect(find.text('END'), findsWidgets);
      expect(
        find.text('gateway-impl wants to end test-gateway'),
        findsOneWidget,
      );
      expect(spawning.terminalWrites, 0);
      await disposeShell(tester, c);
    });

    testWidgets('phone: Needs approval opens the sheet; full-width buttons', (
      tester,
    ) async {
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        sessions: [parent(needs: 'approval', writeVault: 'v-work')],
      );
      FakeSpawning(agents).addRequest('ags_p', task: 'bench the relay');
      final c = await open(
        tester,
        agents,
        size: phone,
        at: Routes.agentSession('ags_p'),
      );
      expect(find.byType(ApprovalCard), findsNothing);
      expect(
        find.text('gateway-impl is asking to start a session'),
        findsOneWidget,
      );
      await tester.tap(find.byKey(const Key('approval-banner')));
      await tester.pumpAndSettle();
      expect(find.byType(ApprovalCard), findsOneWidget);
      expect(
        find.text('Nothing happens until you decide here.'),
        findsOneWidget,
      );
      expect(find.text('bench the relay'), findsOneWidget);
      expect(find.text('Let the child write to Work'), findsOneWidget);
      final approve = tester.getSize(find.byKey(const Key('approval-approve')));
      final card = tester.getSize(find.byType(ApprovalCard));
      expect(approve.width, greaterThan(card.width * 0.9));
      expect(
        tester.getSize(find.byKey(const Key('approval-deny'))).width,
        approve.width,
      );
      await tester.tap(find.byKey(const Key('approval-approve')));
      await tester.pumpAndSettle();
      expect(sent(agents, 'POST', '/v1/agent/requests/agr_1/approve'), [
        {'write_vault': false},
      ]);
      expect(find.byType(ApprovalCard), findsNothing);
      await disposeShell(tester, c);
    });

    testWidgets('desk at 900: the card is over the terminal', (tester) async {
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        sessions: [parent(needs: 'approval')],
      );
      FakeSpawning(agents).addRequest('ags_p', task: 't');
      final c = await open(
        tester,
        agents,
        size: edge,
        at: Routes.agentSession('ags_p'),
      );
      expect(find.byType(ApprovalCard), findsOneWidget);
      expect(find.byKey(const Key('approval-banner')), findsNothing);
      await disposeShell(tester, c);
    });
  });

  testWidgets('a decision finished after the card is gone still refreshes', (
    tester,
  ) async {
    final agents = FakeAgentServer(
      hosts: [agentHost()],
      sessions: [parent(needs: 'approval')],
    );
    final spawning = FakeSpawning(agents)..holdDecisions = Completer();
    spawning.addRequest('ags_p', task: 't', name: 'test-gateway');
    final c = await open(tester, agents, at: Routes.agentSession('ags_p'));
    await tester.tap(find.byKey(const Key('approval-approve')));
    await tester.pump();
    c.read(routerProvider).go(Routes.agents);
    await tester.pumpAndSettle();
    expect(find.byType(ApprovalCard), findsNothing);
    final before = agents.requests.length;
    spawning.holdDecisions!.complete();
    await tester.pumpAndSettle();
    expect(tester.takeException(), isNull);
    expect(spawning.requests.single['outcome'], 'started');
    final after = agents.requests.sublist(before);
    expect(
      after.where(
        (r) => r.method == 'GET' && r.url.path == '/v1/agent/sessions',
      ),
      isNotEmpty,
    );
    await disposeShell(tester, c);
  });

  testWidgets('a held crew change keeps the approval card, Requests and the '
      'parent line', (tester) async {
    final agents = FakeAgentServer(
      hosts: [agentHost()],
      crews: [agentCrew('crw_1')],
      sessions: [
        spawnSession('ags_p'),
        spawnSession(
          'ags_c',
          name: 'test-gateway',
          parent: 'ags_p',
          depth: 1,
          spawn: {'enabled': true},
        ),
      ],
    );
    FakeSpawning(agents).addRequest('ags_c', task: 't', name: 'grandchild');
    final c = await open(
      tester,
      agents,
      at: Routes.agentSession('ags_c', tab: SessionTab.about.name),
    );
    void onScreen() {
      expect(find.byType(ApprovalCard), findsOneWidget);
      expect(find.text('Requests'), findsOneWidget);
      expect(find.text('↳ started by gateway-impl ›'), findsOneWidget);
    }

    onScreen();
    agents.holdCrewChanges = Completer();
    await tester.tap(find.byKey(const Key('move-to-crew')));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('move-to-crw_1')));
    await tester.pumpAndSettle();
    onScreen();
    agents.holdCrewChanges!.complete();
    await tester.pumpAndSettle();
    expect((agents.session('ags_c')!['crew'] as Map)['id'], 'crw_1');
    onScreen();
    await disposeShell(tester, c);
  });

  testWidgets('the Requests tab lists every outcome to the person', (
    tester,
  ) async {
    final agents = FakeAgentServer(
      hosts: [agentHost()],
      sessions: [parent(needs: 'approval')],
    );
    final spawning = FakeSpawning(agents);
    for (final (outcome, reason, cap) in [
      ('started', null, null),
      ('denied', null, null),
      ('expired', null, null),
      ('refused', 'limit', 'children'),
    ]) {
      final r = spawning.addRequest('ags_p', task: 'job $outcome');
      r
        ..['state'] = 'decided'
        ..['outcome'] = outcome
        ..['reason'] = reason
        ..['cap'] = cap
        ..['decided_at'] = DateTime.now().toUtc().toIso8601String();
    }
    spawning.addRequest('ags_p', task: 'write the tests');
    final c = await open(
      tester,
      agents,
      at: Routes.agentSession('ags_p', tab: SessionTab.requests.name),
    );
    final list = find.byKey(const Key('requests-list'));
    expect(
      find.descendant(of: list, matching: find.textContaining('WAITING · 8:4')),
      findsOneWidget,
    );
    for (final word in ['STARTED', 'DENIED', 'EXPIRED', 'REFUSED']) {
      expect(
        find.descendant(of: list, matching: find.textContaining('$word · ')),
        findsOneWidget,
        reason: word,
      );
    }
    expect(
      find.text('Start: “job refused” · limit: 3 live children'),
      findsOneWidget,
    );
    expect(
      find.text('Start: “job expired” · no answer in 10 min'),
      findsOneWidget,
    );
    expect(spawning.terminalWrites, 0);
    await disposeShell(tester, c);
  });

  group('ending (C1, C5, C9)', () {
    List<Map<String, dynamic>> family() => [
      spawnSession('ags_p', spawn: {'enabled': true}),
      spawnSession('ags_c', name: 'test-gateway', parent: 'ags_p', depth: 1),
      spawnSession(
        'ags_d',
        name: 'spike-notes',
        parent: 'ags_p',
        depth: 1,
        status: 'completed',
      ),
    ];

    for (final size in [wide, phone]) {
      testWidgets(
        'a parent lists its children first at ${size.width.toInt()}px',
        (tester) async {
          final agents = FakeAgentServer(
            hosts: [agentHost()],
            sessions: family(),
          );
          final spawning = FakeSpawning(agents);
          final c = await open(
            tester,
            agents,
            size: size,
            at: Routes.agentSession('ags_p'),
          );
          if (size.width < 900) {
            await tester.tap(find.byKey(const Key('session-header')));
            await tester.pumpAndSettle();
          }
          await tester.ensureVisible(find.byKey(const Key('end-session')));
          await tester.tap(find.byKey(const Key('end-session')));
          await tester.pumpAndSettle();
          expect(
            find.text('End gateway-impl? Its children end with it.'),
            findsOneWidget,
          );
          expect(find.text('RUNNING'), findsWidgets);
          expect(find.text('test-gateway'), findsWidgets);
          expect(find.text('ALREADY ENDED'), findsOneWidget);
          expect(find.text(kReportFirst), findsOneWidget);
          await tester.ensureVisible(find.byKey(const Key('report-first')));
          await tester.tap(find.byKey(const Key('report-first')));
          await tester.pumpAndSettle();
          await tester.ensureVisible(find.text('End 2 sessions'));
          await tester.tap(find.text('End 2 sessions'));
          await tester.pumpAndSettle();
          expect(spawning.ends.single.$1, 'end:ags_p');
          expect(spawning.ends.single.$2, {
            'descendants': ['ags_c'],
            'report_first': true,
          });
          await disposeShell(tester, c);
        },
      );
    }

    testWidgets('a child the list did not know yet: the server asks first', (
      tester,
    ) async {
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        sessions: [
          spawnSession('ags_p', spawn: {'enabled': true}),
        ],
      );
      final spawning = FakeSpawning(agents);
      final c = await open(tester, agents, at: Routes.agentSession('ags_p'));
      // Started after the list was read.
      agents.sessions.add(
        spawnSession('ags_c', name: 'test-gateway', parent: 'ags_p', depth: 1),
      );
      await tester.tap(find.byKey(const Key('end-session')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const Key('confirm-action')));
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('end-tree-confirm')), findsOneWidget);
      expect(spawning.ends, isEmpty);
      await tester.tap(find.text('End 2 sessions'));
      await tester.pumpAndSettle();
      expect(spawning.ends.single.$2, {
        'descendants': ['ags_c'],
        'report_first': false,
      });
      await disposeShell(tester, c);
    });

    testWidgets('End all children leaves the parent running', (tester) async {
      final agents = FakeAgentServer(hosts: [agentHost()], sessions: family());
      final spawning = FakeSpawning(agents);
      final c = await open(
        tester,
        agents,
        at: Routes.agentSession('ags_p', tab: SessionTab.about.name),
      );
      expect(
        find.text('test-gateway (running) · spike-notes (ended)'),
        findsOneWidget,
      );
      await tester.ensureVisible(find.byKey(const Key('end-children')));
      await tester.tap(find.byKey(const Key('end-children')));
      await tester.pumpAndSettle();
      expect(
        find.text('End gateway-impl’s children? gateway-impl keeps running.'),
        findsOneWidget,
      );
      await tester.ensureVisible(find.text('End 1 session'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('End 1 session'));
      await tester.pumpAndSettle();
      expect(spawning.ends.single.$1, 'children:ags_p');
      expect(agents.session('ags_p')!['status'], 'running');
      await disposeShell(tester, c);
    });
  });

  group('tree dismissal (C8)', () {
    for (final size in [wide, phone]) {
      testWidgets(
        'warns that unapplied child work goes at ${size.width.toInt()}px',
        (tester) async {
          final agents = FakeAgentServer(
            hosts: [agentHost()],
            sessions: [
              spawnSession('ags_p', status: 'completed'),
              spawnSession(
                'ags_c',
                name: 'test-gateway',
                parent: 'ags_p',
                depth: 1,
                status: 'completed',
              ),
            ],
          );
          FakeSpawning(agents);
          final c = await open(
            tester,
            agents,
            size: size,
            at: Routes.agentSession('ags_c'),
          );
          await tester.tap(find.byKey(const Key('dismiss-session')));
          await tester.pumpAndSettle();
          expect(find.text(kTreeDismissWarning), findsOneWidget);
          expect(
            find.text(
              'Dismiss gateway-impl and everything it started? 2 sessions '
              'go together.',
            ),
            findsOneWidget,
          );
          expect(agents.requests.where((r) => r.method == 'DELETE'), isEmpty);
          await tester.tap(find.text('Dismiss 2 sessions'));
          await tester.pumpAndSettle();
          final delete = agents.requests.singleWhere(
            (r) => r.method == 'DELETE',
          );
          expect(delete.url.queryParameters['tree'], 'true');
          expect(agents.sessions, isEmpty);
          expect(where(c).path, Routes.agents);
          await disposeShell(tester, c);
        },
      );
    }

    testWidgets('a live member: only together, once every one has ended', (
      tester,
    ) async {
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        sessions: [
          spawnSession('ags_p'),
          spawnSession(
            'ags_c',
            name: 'test-gateway',
            parent: 'ags_p',
            depth: 1,
            status: 'completed',
          ),
        ],
      );
      FakeSpawning(agents);
      final c = await open(tester, agents, at: Routes.agentSession('ags_c'));
      await tester.tap(find.byKey(const Key('dismiss-session')));
      await tester.pumpAndSettle();
      expect(find.text('STILL RUNNING'), findsOneWidget);
      expect(find.byKey(const Key('banner-action')), findsNothing);
      expect(agents.requests.where((r) => r.method == 'DELETE'), isEmpty);
      await disposeShell(tester, c);
    });
  });

  group('a child’s work (137e, 137g)', () {
    Map<String, dynamic> cloneWork() => {
      'mode': 'clone',
      'tip': 'f00dfacecafe0000000000000000000000000000',
      'base': 'abc1234def000000000000000000000000000000',
      'base_found': true,
      'commits': [
        {
          'id': 'f00dfacecafe0000000000000000000000000000',
          'subject': 'tests for token refresh',
        },
      ],
      'more_commits': false,
      'files': [
        {'path': 'gateway/tests/refresh.rs', 'added': 142, 'removed': 0},
        {'path': 'gateway/tests/common.rs', 'added': 31, 'removed': 4},
      ],
      'more_files': false,
      'patch': '+fn refresh() {}\n',
      'patch_truncated': false,
    };

    List<Map<String, dynamic>> pair(String mode) => [
      spawnSession('ags_p', status: 'completed'),
      spawnSession(
        'ags_c',
        name: 'test-gateway',
        parent: 'ags_p',
        depth: 1,
        status: 'completed',
        childWorkspace: {'mode': mode, 'commit': 'abc1234def0000'},
      ),
    ];

    testWidgets('a clone: its diff, Apply as storm/<child>, once', (
      tester,
    ) async {
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        sessions: pair('clone'),
      );
      final spawning = FakeSpawning(agents)..work['ags_c'] = cloneWork();
      final c = await open(
        tester,
        agents,
        at: Routes.agentSession('ags_c', tab: SessionTab.work.name),
      );
      expect(
        find.text(
          'Nothing has reached storm yet. Apply creates the branch '
          'storm/ags_c there from its commits. It never merges or '
          'overwrites.',
        ),
        findsOneWidget,
      );
      expect(
        find.text('From abc1234, the commit it started from'),
        findsOneWidget,
      );
      expect(find.text('f00dfac  tests for token refresh'), findsOneWidget);
      expect(find.text('+142 −0'), findsOneWidget);
      expect(find.text('+fn refresh() {}\n'), findsOneWidget);
      await tester.tap(find.byKey(const Key('work-apply')));
      await tester.pumpAndSettle();
      expect(spawning.applied, ['storm/ags_c']);
      expect(
        find.text('Applied: the branch storm/ags_c at f00dfac is in storm.'),
        findsOneWidget,
      );
      await disposeShell(tester, c);

      // Again: refused, never over what exists.
      final again = await open(
        tester,
        agents,
        at: Routes.agentSession('ags_c', tab: SessionTab.work.name),
      );
      await tester.tap(find.byKey(const Key('work-apply')));
      await tester.pumpAndSettle();
      expect(
        find.textContaining('storm/ags_c already exists in storm'),
        findsOneWidget,
      );
      expect(spawning.applied, ['storm/ags_c']);
      await disposeShell(tester, again);
    });

    testWidgets('a scratch child: the name prefilled <ws>-<child>, and '
        'refused where it exists (138p point 9)', (tester) async {
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        sessions: pair('scratch'),
      );
      final spawning = FakeSpawning(agents)
        ..work['ags_c'] = {
          'mode': 'scratch',
          'files': [
            {'path': 'notes/answer.md', 'size': 2048},
          ],
          'more_files': false,
          'file_count': 1,
          'bytes': 2048,
          'skipped': 1,
        }
        ..existing.add('storm-ags_c');
      final c = await open(
        tester,
        agents,
        at: Routes.agentSession('ags_c', tab: SessionTab.work.name),
      );
      final field = tester.widget<TextField>(
        find.byKey(const Key('work-name')),
      );
      expect(field.controller!.text, 'storm-ags_c');
      expect(scratchApplyName('storm', 'ags_c'), 'storm-ags_c');
      expect(find.text('2.0 KB'), findsOneWidget);
      await tester.tap(find.byKey(const Key('work-apply')));
      await tester.pumpAndSettle();
      expect(
        find.text(
          'Something named storm-ags_c already exists in storm. Choose '
          'another name; nothing was changed.',
        ),
        findsOneWidget,
      );
      expect(
        sent(agents, 'POST', '/v1/agent/sessions/ags_c/child-work/apply'),
        [
          {'name': 'storm-ags_c'},
        ],
      );
      await tester.enterText(
        find.byKey(const Key('work-name')),
        'spike-answer',
      );
      await tester.tap(find.byKey(const Key('work-apply')));
      await tester.pumpAndSettle();
      expect(spawning.applied, ['spike-answer']);
      expect(
        find.text('Applied: the folder spike-answer is in storm.'),
        findsOneWidget,
      );
      await disposeShell(tester, c);
    });

    testWidgets('delete asks first and says what goes', (tester) async {
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        sessions: pair('clone'),
      );
      final spawning = FakeSpawning(agents)..work['ags_c'] = cloneWork();
      final c = await open(
        tester,
        agents,
        at: Routes.agentSession('ags_c', tab: SessionTab.work.name),
      );
      await tester.tap(find.byKey(const Key('work-delete')));
      await tester.pumpAndSettle();
      expect(
        find.textContaining('with any of its work you haven’t applied'),
        findsOneWidget,
      );
      expect(spawning.deleted, isEmpty);
      await tester.tap(find.byKey(const Key('work-delete-confirmed')));
      await tester.pumpAndSettle();
      expect(spawning.deleted, ['ags_c']);
      await disposeShell(tester, c);
    });

    testWidgets('phone: the work is in the details sheet', (tester) async {
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        sessions: pair('clone'),
      );
      FakeSpawning(agents).work['ags_c'] = cloneWork();
      final c = await open(
        tester,
        agents,
        size: phone,
        at: Routes.agentSession('ags_c'),
      );
      await tester.tap(find.byKey(const Key('session-header')));
      await tester.pumpAndSettle();
      expect(find.text('WORK'), findsOneWidget);
      expect(find.byKey(const Key('work-apply')), findsOneWidget);
      await disposeShell(tester, c);
    });

    testWidgets('a live child: nothing to apply yet', (tester) async {
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        sessions: [
          spawnSession('ags_p'),
          spawnSession(
            'ags_c',
            name: 'test-gateway',
            parent: 'ags_p',
            depth: 1,
          ),
        ],
      );
      FakeSpawning(agents);
      final c = await open(
        tester,
        agents,
        at: Routes.agentSession('ags_c', tab: SessionTab.work.name),
      );
      expect(find.byKey(const Key('work-live')), findsOneWidget);
      expect(find.byKey(const Key('work-apply')), findsNothing);
      await disposeShell(tester, c);
    });
  });

  group('lineage and attention in the lists', () {
    for (final size in [wide, phone]) {
      testWidgets('a child sits under its parent at ${size.width.toInt()}px', (
        tester,
      ) async {
        final agents = FakeAgentServer(
          hosts: [agentHost()],
          sessions: [
            parent(needs: 'approval'),
            spawnSession('ags_x', name: 'unrelated'),
            spawnSession(
              'ags_c',
              name: 'test-gateway',
              parent: 'ags_p',
              depth: 1,
              childWorkspace: {'mode': 'clone'},
            ),
          ],
        );
        FakeSpawning(agents);
        final c = await open(tester, agents, size: size);
        expect(
          find.text('↳ from gateway-impl · own copy of storm'),
          findsOneWidget,
        );
        expect(find.byType(LineageIndent), findsOneWidget);
        expect(find.byKey(const Key('attention-approval')), findsOneWidget);
        final rows = [
          for (final id in ['ags_p', 'ags_c', 'ags_x'])
            tester.getTopLeft(find.byKey(Key('session-$id'))).dy,
        ];
        // The child right under its parent; the rest keep their places.
        expect(rows[1], greaterThan(rows[0]));
        expect(rows[2] < rows[0] || rows[2] > rows[1], isTrue);
        await disposeShell(tester, c);
      });
    }

    testWidgets('a child’s header names who started it, and About its copy', (
      tester,
    ) async {
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        sessions: [
          spawnSession('ags_p'),
          spawnSession(
            'ags_c',
            name: 'test-gateway',
            parent: 'ags_p',
            depth: 1,
            childWorkspace: {'mode': 'clone', 'commit': 'abc1234def'},
          ),
        ],
      );
      FakeSpawning(agents);
      final c = await open(
        tester,
        agents,
        at: Routes.agentSession('ags_c', tab: SessionTab.about.name),
      );
      expect(find.text('↳ started by gateway-impl ›'), findsOneWidget);
      expect(
        find.text(
          'Its own copy of storm from abc1234 on build-vm (committed work '
          'only)',
        ),
        findsOneWidget,
      );
      await tester.tap(find.byKey(const Key('started-by')));
      await tester.pumpAndSettle();
      expect(where(c).path, Routes.agentSession('ags_p'));
      await disposeShell(tester, c);
    });
  });

  testWidgets('the whole flow on a fake server: approve, the child runs, '
      'ends, and its work is applied', (tester) async {
    final agents = FakeAgentServer(
      hosts: [agentHost()],
      sessions: [parent(needs: 'approval', writeVault: 'v-work')],
    );
    final spawning = FakeSpawning(agents);
    spawning.addRequest('ags_p', task: 'write the tests', name: 'test-gateway');
    final c = await open(tester, agents, at: Routes.agentSession('ags_p'));

    await tester.tap(find.byKey(const Key('approval-approve')));
    await tester.pumpAndSettle();
    final child = spawning.requests.single['session'] as String;
    expect(spawning.requests.single['outcome'], 'started');
    expect(find.byType(ApprovalCard), findsNothing);
    expect(find.byKey(const Key('attention-approval')), findsNothing);

    // The host runs it, then it ends having committed its work.
    agents.session(child)!
      ..['status'] = 'running'
      ..['child_workspace'] = {'mode': 'clone', 'commit': 'abc1234def'};
    c.invalidate(agentOverviewProvider);
    await tester.pumpAndSettle();
    expect(
      find.text('↳ from gateway-impl · own copy of storm'),
      findsOneWidget,
    );
    agents.session(child)!['status'] = 'completed';
    spawning.work[child] = {
      'mode': 'clone',
      'tip': 'f00dfacecafe',
      'base': 'abc1234def',
      'base_found': true,
      'commits': [
        {'id': 'f00dfacecafe', 'subject': 'the child’s work'},
      ],
      'files': [
        {'path': 'WORK.md', 'added': 1, 'removed': 0},
      ],
      'patch': '+done\n',
    };
    c.invalidate(agentOverviewProvider);
    await tester.pumpAndSettle();

    await tester.tap(find.byKey(Key('session-$child')));
    await tester.pumpAndSettle();
    c
        .read(routerProvider)
        .go(Routes.agentSession(child, tab: SessionTab.work.name));
    await tester.pumpAndSettle();
    expect(find.textContaining('the child’s work'), findsOneWidget);
    await tester.tap(find.byKey(const Key('work-apply')));
    await tester.pumpAndSettle();
    expect(spawning.applied, ['storm/$child']);
    expect(
      find.text('Applied: the branch storm/$child at f00dfac is in storm.'),
      findsOneWidget,
    );
    // Every outcome reached the person, none a terminal (138p point 2).
    expect(spawning.terminalWrites, 0);
    await disposeShell(tester, c);
  });

  test('the outcome words and reasons', () {
    expect(
      requestRefusalText('limit', 'tree'),
      'limit: 6 live sessions in a tree',
    );
    expect(
      requestRefusalText('isolation_unverified', null),
      'the host doesn’t isolate sessions',
    );
    expect(
      hostStatesLine('unavailable', 'unavailable'),
      'identity not verified · doesn’t isolate sessions',
    );
    expect(attentionLabel('blocked'), 'Blocked');
    expect(attentionLabel(null), isNull);
    expect(clockLeft(const Duration(minutes: 9, seconds: 5)), '9:05');
  });
}
