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

/// Agents as an activity (Storm v2), on the real router and shell, at both
/// widths.
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

  group('live sessions are carried by the rail and the place picker', () {
    MockClient running() => agentServer(
      hosts: [host()],
      sessions: [
        session('ags_live'),
        session('ags_done', status: 'completed', exitCode: 0),
      ],
    );

    testWidgets('wide: the rail badge counts live sessions', (tester) async {
      final c = shellContainer(agentClient: running());
      await pumpShell(tester, c, size: wide);

      final badge = find.byKey(const Key('rail-badge'));
      expect(badge, findsOneWidget);
      expect(
        find.descendant(of: badge, matching: find.text('1')),
        findsOneWidget,
      );

      await tester.tap(find.byKey(const Key('rail-agents')));
      await tester.pumpAndSettle();
      expect(location(c), Routes.agents);
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

      await tester.tap(find.byKey(const Key('rail-notes')));
      await tester.pumpAndSettle();
      expect(location(c), startsWith('/v/'));
      await disposeShell(tester, c);
    });

    testWidgets('no badge when nothing is live', (tester) async {
      final c = shellContainer(
        agentClient: agentServer(
          hosts: [host()],
          sessions: [session('ags_old', status: 'stopped')],
        ),
      );
      await pumpShell(tester, c, size: wide);
      expect(find.byKey(const Key('rail-badge')), findsNothing);
      await disposeShell(tester, c);
    });

    testWidgets(
      'phone: the picker says what is running, and a session opens full screen',
      (tester) async {
        final c = shellContainer(agentClient: running());
        await pumpShell(tester, c, size: phone);
        final notes = location(c);

        await tester.tap(find.byKey(const Key('places-bubble')));
        await tester.pumpAndSettle();
        expect(find.text('1 running'), findsOneWidget);
        await tester.tap(find.byKey(const Key('place-agents')));
        await tester.pumpAndSettle();
        expect(location(c), Routes.agents);

        await tester.tap(find.byKey(const Key('session-ags_live')));
        await tester.pumpAndSettle();
        expect(find.byKey(const Key('switch-session')), findsOneWidget);
        expect(c.read(activeAgentTabProvider), 'ags_live');

        // The keys row rides on the keyboard: absent while it is down.
        expect(find.byKey(const Key('key-esc')), findsNothing);
        tester.view.viewInsets = const FakeViewPadding(bottom: 300);
        await tester.pumpAndSettle();
        expect(find.byKey(const Key('key-esc')), findsOneWidget);
        expect(find.byKey(const Key('key-shift')), findsOneWidget);
        expect(find.byKey(const Key('keys-done')), findsOneWidget);
        tester.view.resetViewInsets();
        await tester.pumpAndSettle();

        // System back: to the list, then to the last Notes location.
        expect(await tester.binding.handlePopRoute(), isTrue);
        await tester.pumpAndSettle();
        expect(location(c), Routes.agents);
        expect(find.byKey(const Key('session-ags_live')), findsOneWidget);
        expect(await tester.binding.handlePopRoute(), isTrue);
        await tester.pumpAndSettle();
        expect(location(c), notes);

        await disposeShell(tester, c);
      },
    );

    testWidgets('with no host, the list sends you to Hosts in Settings', (
      tester,
    ) async {
      final c = shellContainer(agentClient: agentServer());
      await pumpShell(tester, c, size: phone);
      c.read(routerProvider).go(Routes.agents);
      await tester.pumpAndSettle();
      expect(find.text('No hosts yet'), findsOneWidget);
      await tester.tap(find.text('Hosts'));
      await tester.pumpAndSettle();
      expect(location(c), Routes.settingsPage('hosts'));
      await disposeShell(tester, c);
    });
  });

  group('the Agents space on a phone', () {
    testWidgets('running then ended, with New session as a pill', (
      tester,
    ) async {
      final c = shellContainer(
        agentClient: agentServer(
          hosts: [host()],
          sessions: [
            session('ags_live'),
            session('ags_done', status: 'completed', exitCode: 2),
          ],
        ),
      );
      await pumpShell(tester, c, size: phone);
      c.read(routerProvider).go(Routes.agents);
      await tester.pumpAndSettle();

      expect(find.byKey(const Key('new-session-pill')), findsOneWidget);
      expect(find.byType(FloatingActionButton), findsNothing);
      expect(
        top(tester, find.text('RUNNING')),
        lessThan(top(tester, find.text('ENDED'))),
      );
      expect(find.text('Exited (2)'), findsOneWidget);
      expect(find.byKey(const Key('places-bubble')), findsOneWidget);
      expect(find.byKey(const Key('rail-agents')), findsNothing);
      await disposeShell(tester, c);
    });
  });

  testWidgets('Vaults settings link to Hosts, not to sessions', (tester) async {
    final c = shellContainer(agentClient: agentServer(hosts: [host()]));
    await pumpShell(tester, c, size: phone);
    c.read(routerProvider).go(Routes.settingsPage('vaults'));
    await tester.pumpAndSettle();
    await tester.scrollUntilVisible(find.text('New vault'), 300);
    expect(find.byKey(const Key('open-hosts')), findsOneWidget);
    expect(find.byKey(const Key('open-agents')), findsNothing);
    await disposeShell(tester, c);
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
