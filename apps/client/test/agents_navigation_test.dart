import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'package:storm/agent/agent_api.dart';
import 'package:storm/agent/agent_models.dart';
import 'package:storm/agent/agent_state.dart';
import 'package:storm/agent/agents_screen.dart';
import 'package:storm/router.dart';
import 'package:storm/ui/theme.dart';

import 'shell_harness.dart';

/// Agents as a space beside Notes (decision 78), on the real router and
/// shell. Every layout claim is asserted at both widths, and every entry
/// point is asserted absent for an account that is not the owner (AC-S1).
void main() {
  const phone = Size(411, 900);
  const wide = Size(1400, 900);

  Map<String, dynamic> host({
    String id = 'hst_1',
    String name = 'build-vm',
    String status = 'online',
  }) => {
    'id': id,
    'name': name,
    'status': status,
    'egress': 'host',
    'capabilities': {
      'providers': [
        {'id': 'shell', 'kind': 'cli', 'available': true},
      ],
      'workspaces': ['storm'],
      'max_sessions': 8,
    },
  };

  Map<String, dynamic> session(
    String id, {
    String status = 'running',
    int? exitCode,
  }) => {
    'id': id,
    'host_id': 'hst_1',
    'workspace': 'storm',
    'provider': 'claude-code',
    'status': status,
    'end_reason': null,
    'exit_code': exitCode,
    'signal': null,
    'cols': 80,
    'rows': 24,
    'created_at': DateTime.now().toUtc().toIso8601String(),
    'provider_fallback': null,
  };

  /// An agent server. [down] answers like a server that is not there.
  MockClient agentServer({
    List<Map<String, dynamic>> hosts = const [],
    List<Map<String, dynamic>> sessions = const [],
    bool down = false,
  }) => MockClient((req) async {
    if (down) return http.Response('', 503);
    final path = req.url.path;
    if (path == '/v1/agent/hosts') {
      return http.Response(jsonEncode(hosts), 200);
    }
    if (path == '/v1/agent/sessions') {
      // The server lists oldest first; the client shows newest first.
      return http.Response(jsonEncode(sessions.reversed.toList()), 200);
    }
    if (path.startsWith('/v1/agent/sessions/')) {
      final id = path.split('/')[4];
      final s = sessions.where((s) => s['id'] == id).firstOrNull;
      return s == null
          ? http.Response('{"error":"no such session"}', 404)
          : http.Response(jsonEncode(s), 200);
    }
    if (path == '/v1/config/agent') {
      return http.Response('{"default_provider":"shell"}', 200);
    }
    if (path.endsWith('/workspaces')) {
      return http.Response('[{"name":"storm","live_sessions":0}]', 200);
    }
    return http.Response('{"error":"nope"}', 404);
  });

  setUp(() => SharedPreferences.setMockInitialValues({}));

  String location(ProviderContainer c) => c.read(routerProvider).state.uri.path;

  double top(WidgetTester tester, Finder f) => tester.getTopLeft(f).dy;

  group('an account that is not the owner sees the app it always saw', () {
    testWidgets('no band on the phone dashboard', (tester) async {
      final c = shellContainer(agents: false);
      await pumpShell(tester, c, size: phone);
      expect(find.text('AGENTS'), findsNothing);
      expect(find.text('RECENTLY OPENED'), findsOneWidget);
      await disposeShell(tester, c);
    });

    testWidgets('no space switch beside the notes', (tester) async {
      final c = shellContainer(agents: false);
      await pumpShell(tester, c, size: wide);
      await openVault(tester, c);
      expect(find.byKey(const Key('space-agents')), findsNothing);
      expect(find.byKey(const Key('space-notes')), findsNothing);
      await disposeShell(tester, c);
    });

    testWidgets('and /agents by URL returns to the dashboard', (tester) async {
      final c = shellContainer(agents: false);
      await pumpShell(tester, c, size: phone);
      c.read(routerProvider).go(Routes.agents);
      await tester.pumpAndSettle();
      expect(location(c), Routes.dashboard);
      c.read(routerProvider).go(Routes.agentHosts);
      await tester.pumpAndSettle();
      expect(location(c), Routes.dashboard);
      await disposeShell(tester, c);
    });

    testWidgets('Server settings offers nothing agent-shaped', (tester) async {
      final c = shellContainer(agents: false);
      await pumpShell(tester, c, size: phone);
      c.read(routerProvider).push(Routes.serverSettings);
      await tester.pumpAndSettle();
      // The list is lazy: scroll past where the section would be, or
      // "nothing found" would only mean "not built yet".
      await tester.scrollUntilVisible(find.text('New vault'), 300);
      expect(find.byKey(const Key('open-hosts')), findsNothing);
      await disposeShell(tester, c);
    });
  });

  group('the owner\'s dashboard band', () {
    testWidgets(
      'shows live sessions above recents, and a tap lands in the terminal',
      (tester) async {
        final c = shellContainer(
          agents: true,
          agentClient: agentServer(
            hosts: [host()],
            sessions: [
              session('ags_live'),
              session('ags_done', status: 'completed', exitCode: 0),
            ],
          ),
        );
        await pumpShell(tester, c, size: phone);

        expect(find.byKey(const Key('band-ags_live')), findsOneWidget);
        expect(
          find.byKey(const Key('band-ags_done')),
          findsNothing,
          reason: 'the band is what is still happening',
        );
        expect(
          top(tester, find.text('AGENTS')),
          lessThan(top(tester, find.text('RECENTLY OPENED'))),
        );

        await tester.tap(find.byKey(const Key('band-ags_live')));
        await tester.pumpAndSettle();
        expect(location(c), Routes.agents);
        expect(find.byKey(const Key('key-esc')), findsOneWidget);
        expect(c.read(activeAgentTabProvider), 'ags_live');

        // System back: first to the list, then home — never out of the app.
        await tester.binding.handlePopRoute();
        await tester.pumpAndSettle();
        expect(location(c), Routes.agents);
        expect(find.byKey(const Key('session-ags_live')), findsOneWidget);
        await tester.binding.handlePopRoute();
        await tester.pumpAndSettle();
        expect(location(c), Routes.dashboard);

        await disposeShell(tester, c);
      },
    );

    // Idle, the band is one row naming the next step, and it is in the same
    // place it would be with something running.
    final idle = <String, (MockClient, String, String)>{
      'no hosts': (agentServer(), 'agents-band-setup', 'Set up a host'),
      'the only host offline': (
        agentServer(hosts: [host(status: 'offline')]),
        'agents-band-host-offline',
        'build-vm is offline',
      ),
      'a host online, nothing running': (
        agentServer(
          hosts: [host()],
          sessions: [session('ags_old', status: 'stopped')],
        ),
        'agents-band-idle',
        'No agents running',
      ),
      'the server unreachable': (
        agentServer(down: true),
        'agents-band-offline',
        'Agents need the server',
      ),
    };
    for (final MapEntry(key: name, value: (client, key, text))
        in idle.entries) {
      testWidgets('idle: $name', (tester) async {
        final c = shellContainer(agents: true, agentClient: client);
        await pumpShell(tester, c, size: phone);
        expect(find.byKey(Key(key)), findsOneWidget);
        expect(find.text(text), findsOneWidget);
        expect(
          top(tester, find.text('AGENTS')),
          lessThan(top(tester, find.text('RECENTLY OPENED'))),
        );
        await disposeShell(tester, c);
      });
    }

    testWidgets('setting up a host goes to Hosts', (tester) async {
      final c = shellContainer(agents: true, agentClient: agentServer());
      await pumpShell(tester, c, size: phone);
      await tester.tap(find.byKey(const Key('agents-band-setup')));
      await tester.pumpAndSettle();
      expect(location(c), Routes.agentHosts);
      await disposeShell(tester, c);
    });
  });

  group('the Agents space on a phone', () {
    testWidgets('running then ended, with New session as a pill', (
      tester,
    ) async {
      final c = shellContainer(
        agents: true,
        agentClient: agentServer(
          hosts: [host()],
          sessions: [
            session('ags_live'),
            session('ags_done', status: 'completed', exitCode: 2),
          ],
        ),
      );
      await pumpShell(tester, c, size: phone);
      c.read(routerProvider).push(Routes.agents);
      await tester.pumpAndSettle();

      expect(find.byKey(const Key('new-session-pill')), findsOneWidget);
      expect(find.byType(FloatingActionButton), findsNothing);
      expect(
        top(tester, find.text('RUNNING')),
        lessThan(top(tester, find.text('ENDED'))),
      );
      expect(find.text('Exited (2)'), findsOneWidget);
      // No sidebar below the breakpoint: the switch is a wide-screen thing.
      expect(find.byKey(const Key('space-notes')), findsNothing);
      await disposeShell(tester, c);
    });
  });

  group('wide: Notes | Agents atop the sidebar', () {
    testWidgets('switches between the two spaces', (tester) async {
      final c = shellContainer(
        agents: true,
        agentClient: agentServer(
          hosts: [host()],
          sessions: [session('ags_live')],
        ),
      );
      await pumpShell(tester, c, size: wide);
      await openVault(tester, c);

      await tester.tap(find.byKey(const Key('space-agents')));
      await tester.pumpAndSettle();
      expect(location(c), Routes.agents);
      // The list is in the sidebar; the pane waits for a choice.
      expect(find.byKey(const Key('session-ags_live')), findsOneWidget);
      expect(find.text('No session open'), findsOneWidget);
      expect(find.byKey(const Key('new-session-pill')), findsNothing);

      await tester.tap(find.byKey(const Key('session-ags_live')));
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('tab-ags_live')), findsOneWidget);
      expect(
        find.byKey(const Key('key-esc')),
        findsNothing,
        reason: 'a keyboard has the keys the phone row stands in for',
      );

      // Hosts opens in the pane, beside the same sidebar.
      await tester.tap(find.byKey(const Key('open-hosts')));
      await tester.pumpAndSettle();
      expect(location(c), Routes.agentHosts);
      expect(find.byKey(const Key('space-agents')), findsOneWidget);

      await tester.tap(find.byKey(const Key('space-notes')));
      await tester.pumpAndSettle();
      expect(location(c), startsWith('/v/'));
      await disposeShell(tester, c);
    });

    testWidgets('Server settings keeps hosts, not sessions', (tester) async {
      final c = shellContainer(
        agents: true,
        agentClient: agentServer(hosts: [host()]),
      );
      await pumpShell(tester, c, size: phone);
      c.read(routerProvider).push(Routes.serverSettings);
      await tester.pumpAndSettle();
      await tester.scrollUntilVisible(find.text('New vault'), 300);
      expect(find.byKey(const Key('open-hosts')), findsOneWidget);
      expect(find.byKey(const Key('open-agents')), findsNothing);
      await disposeShell(tester, c);
    });
  });

  testWidgets('the launcher preselects the host this device used last', (
    tester,
  ) async {
    SharedPreferences.setMockInitialValues({'agent.lastHost': 'hst_2'});
    final client = agentServer(
      hosts: [
        host(),
        host(id: 'hst_2', name: 'codebox'),
      ],
    );
    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          agentApiFactoryProvider.overrideWithValue(
            () => AgentApi(baseUrl: 'http://s', token: 't', client: client),
          ),
        ],
        child: MaterialApp(
          theme: StormTheme.dark(),
          home: Scaffold(
            body: launcherForTest([
              AgentHost.fromJson(host()),
              AgentHost.fromJson(host(id: 'hst_2', name: 'codebox')),
            ]),
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();
    final chip = tester.widget<ChoiceChip>(
      find.ancestor(
        of: find.text('codebox'),
        matching: find.byType(ChoiceChip),
      ),
    );
    expect(chip.selected, isTrue);
    // Its only workspace is chosen too, so Launch is one tap away.
    final launch = tester.widget<FilledButton>(find.byKey(const Key('launch')));
    expect(launch.onPressed, isNotNull);
  });
}
