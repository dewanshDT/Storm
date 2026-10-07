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
import 'package:storm/agent/integrations_api.dart';
import 'package:storm/agent/integrations_screen.dart';
import 'package:storm/agent/oauth_links.dart';
import 'package:storm/ui/theme.dart';

/// MCP Gateway, client side (decision 81h).
void main() {
  Map<String, dynamic> builtin() => {
    'id': 'storm',
    'slug': 'storm',
    'display_name': 'Storm vaults',
    'url': null,
    'auth_kind': 'none',
    'status': 'connected',
    'builtin': true,
    'has_credential': false,
    'tool_allowlist': <String>[],
    'vault_writes_available': false,
  };

  Map<String, dynamic> github({
    String status = 'connected',
    String? error,
    List<String> newTools = const [],
  }) => {
    'id': 'mcc_GH',
    'slug': 'github',
    'display_name': 'GitHub',
    'url': 'https://api.example.com/mcp/',
    'auth_kind': 'static',
    'status': status,
    'builtin': false,
    'has_credential': true,
    'tool_allowlist': ['search'],
    'known_tools': ['search', 'delete_repo'],
    'new_tools': newTools,
    'last_error_code': error,
  };

  Map<String, dynamic> check(Map<String, dynamic> integration) => {
    'ok': true,
    'error_code': null,
    'tool_count': 2,
    'new_tools': <String>[],
    'integration': integration,
  };

  /// A fake server that records every request.
  (MockClient, List<http.Request>) server({
    int listStatus = 200,
    List<Map<String, dynamic>>? items,
  }) {
    final seen = <http.Request>[];
    final client = MockClient((req) async {
      seen.add(req);
      final path = req.url.path;
      if (path == '/v1/integrations/connections' && req.method == 'GET') {
        if (listStatus != 200) {
          return http.Response('{"error":"forbidden"}', listStatus);
        }
        return http.Response(jsonEncode(items ?? [builtin(), github()]), 200);
      }
      if (path == '/v1/integrations/connections' && req.method == 'POST') {
        return http.Response(jsonEncode(github()), 201);
      }
      if (path.endsWith('/test') || path == '/v1/integrations/oauth/callback') {
        return http.Response(jsonEncode(check(github())), 200);
      }
      if (path.endsWith('/tools')) {
        return http.Response(
          jsonEncode([
            {'name': 'search', 'allowed': true, 'new': false},
            {'name': 'delete_repo', 'allowed': false, 'new': true},
          ]),
          200,
        );
      }
      if (req.method == 'PATCH') {
        return http.Response(jsonEncode(github()), 200);
      }
      if (req.method == 'DELETE') return http.Response('', 204);
      return http.Response('{"error":"nope"}', 404);
    });
    return (client, seen);
  }

  Widget app(
    Widget child,
    MockClient client, {
    bool oauth = true,
    OAuthLinks? links,
  }) => ProviderScope(
    overrides: [
      oauthLinksProvider.overrideWithValue(links ?? OAuthLinks.detached()),
      integrationsApiFactoryProvider.overrideWithValue(
        () => IntegrationsApi(baseUrl: 'http://s', token: 't', client: client),
      ),
      oauthSupportedProvider.overrideWithValue(oauth),
    ],
    child: MaterialApp(theme: StormTheme.light(), home: child),
  );

  setUp(() => SharedPreferences.setMockInitialValues({}));

  testWidgets('the list shows the built-in connection and each status', (
    tester,
  ) async {
    final (client, _) = server(
      items: [
        builtin(),
        github(status: 'needs_reauth', error: 'upstream_unauthorized'),
      ],
    );
    await tester.pumpWidget(app(const IntegrationsScreen(), client));
    await tester.pumpAndSettle();
    expect(find.text('Storm vaults'), findsOneWidget);
    expect(find.text('Built in'), findsOneWidget);
    expect(find.textContaining('read only'), findsOneWidget);
    expect(find.text('Needs reconnecting'), findsOneWidget);
    expect(find.textContaining('refused the credential'), findsOneWidget);
    expect(find.byKey(const Key('reconnect-mcc_GH')), findsOneWidget);
    // The built-in connection has no menu: it cannot be changed or removed.
    expect(find.byKey(const Key('menu-storm')), findsNothing);
  });

  testWidgets("a member sees the server's refusal, not an empty list", (
    tester,
  ) async {
    final (client, _) = server(listStatus: 403);
    await tester.pumpWidget(app(const IntegrationsScreen(), client));
    await tester.pumpAndSettle();
    expect(find.textContaining('server owner only'), findsOneWidget);
    expect(find.byKey(const Key('add-integration')), findsNothing);
  });

  testWidgets('a token is sent once, as a Bearer header, and then tested', (
    tester,
  ) async {
    final (client, seen) = server();
    await tester.pumpWidget(app(const IntegrationsScreen(), client));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('add-integration')));
    await tester.pumpAndSettle();
    await tester.enterText(find.byKey(const Key('integration-name')), 'GitHub');
    await tester.enterText(
      find.byKey(const Key('integration-url')),
      'https://api.example.com/mcp/',
    );
    await tester.tap(find.text('Token'));
    await tester.pumpAndSettle();
    await tester.enterText(find.byKey(const Key('integration-token')), 'ghp_x');
    await tester.tap(find.byKey(const Key('integration-connect')));
    await tester.pumpAndSettle();
    final create = seen.firstWhere(
      (r) => r.method == 'POST' && r.url.path == '/v1/integrations/connections',
    );
    final body = jsonDecode(create.body) as Map<String, dynamic>;
    expect(body['auth_kind'], 'static');
    expect(body['credential'], {
      'header': 'Authorization',
      'value': 'Bearer ghp_x',
    });
    expect(seen.any((r) => r.url.path.endsWith('/mcc_GH/test')), isTrue);
    // And the token is not on screen anywhere afterwards.
    expect(find.textContaining('ghp_x'), findsNothing);
  });

  testWidgets('a plain http address is refused before anything is sent', (
    tester,
  ) async {
    final (client, seen) = server();
    await tester.pumpWidget(app(const IntegrationsScreen(), client));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('add-integration')));
    await tester.pumpAndSettle();
    await tester.enterText(find.byKey(const Key('integration-name')), 'X');
    await tester.enterText(
      find.byKey(const Key('integration-url')),
      'http://192.168.1.20/mcp',
    );
    await tester.tap(find.byKey(const Key('integration-connect')));
    await tester.pumpAndSettle();
    expect(find.textContaining('https://'), findsWidgets);
    expect(seen.where((r) => r.method == 'POST'), isEmpty);
  });

  testWidgets('the web offers tokens only (G-D13)', (tester) async {
    final (client, _) = server();
    await tester.pumpWidget(
      app(const IntegrationsScreen(), client, oauth: false),
    );
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('add-integration')));
    await tester.pumpAndSettle();
    expect(find.text('Sign in'), findsNothing);
    expect(find.textContaining('connect with a token'), findsOneWidget);
  });

  testWidgets('new tools start off and the owner chooses', (tester) async {
    final (client, seen) = server();
    await tester.pumpWidget(app(const IntegrationsScreen(), client));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('menu-mcc_GH')));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Choose tools'));
    await tester.pumpAndSettle();
    expect(find.text('delete_repo  (new)'), findsOneWidget);
    final off = tester.widget<CheckboxListTile>(
      find.byKey(const Key('tool-delete_repo')),
    );
    expect(off.value, isFalse);
    await tester.tap(find.byKey(const Key('save-tools')));
    await tester.pumpAndSettle();
    final patch = seen.firstWhere((r) => r.method == 'PATCH');
    expect(jsonDecode(patch.body), {
      'tool_allowlist': ['search'],
    });
  });

  testWidgets('new upstream tools are announced, and reviewing them enables '
      'nothing (spec §9)', (tester) async {
    final (client, seen) = server(
      items: [
        builtin(),
        github(newTools: ['delete_repo', 'merge_pr']),
      ],
    );
    await tester.pumpWidget(app(const IntegrationsScreen(), client));
    await tester.pumpAndSettle();
    expect(find.text('2 new tools — review'), findsOneWidget);
    // The built-in connection never has one.
    expect(find.byKey(const Key('review-tools-storm')), findsNothing);
    await tester.tap(find.byKey(const Key('review-tools-mcc_GH')));
    await tester.pumpAndSettle();
    expect(find.text('delete_repo  (new)'), findsOneWidget);
    await tester.tap(find.byKey(const Key('save-tools')));
    await tester.pumpAndSettle();
    final patch = seen.firstWhere((r) => r.method == 'PATCH');
    expect(jsonDecode(patch.body), {
      'tool_allowlist': ['search'],
    }, reason: 'reviewing must not turn a new tool on');
  });

  testWidgets('one new tool is singular', (tester) async {
    final (client, _) = server(
      items: [
        github(newTools: ['merge_pr']),
      ],
    );
    await tester.pumpWidget(app(const IntegrationsScreen(), client));
    await tester.pumpAndSettle();
    expect(find.text('1 new tool — review'), findsOneWidget);
  });

  test('an upstream tool name is cleaned and bounded before it is shown', () {
    // A right-to-left override would make the name read as something else.
    expect(displayToolName('delete\u202Erepo'), 'deleterepo');
    expect(displayToolName('zero\u200Bwidth'), 'zerowidth');
    expect(displayToolName('two\nlines\there'), 'two lines here');
    expect(displayToolName('\u0007\u200B'), '(unnamed tool)');
    final long = 'x' * 100;
    expect(displayToolName(long), '${'x' * 64}…');
    expect(displayToolName('search'), 'search');
  });

  testWidgets('a sign-in redirect no sign-in was waiting for is relayed '
      '(spec §10.4, 81l)', (tester) async {
    // The app was killed while the browser was open, or Android handed the
    // link to a fresh instance: the redirect arrives as an orphan, and the
    // Integrations screen finishes the sign-in with it.
    final (client, seen) = server();
    final links = OAuthLinks.detached()
      ..deliver(Uri.parse('storm://oauth/callback?state=s9&code=c9'));
    await tester.pumpWidget(
      app(const IntegrationsScreen(), client, links: links),
    );
    await tester.pumpAndSettle();
    final relay = seen.where(
      (r) => r.url.path == '/v1/integrations/oauth/callback',
    );
    expect(relay, hasLength(1));
    expect(jsonDecode(relay.single.body), {'state': 's9', 'code': 'c9'});
    expect(find.textContaining('Connected: 2 tool(s)'), findsOneWidget);
    expect(links.orphans.value, isEmpty);
  });

  testWidgets('disconnecting a token says Storm cannot revoke it', (
    tester,
  ) async {
    final (client, seen) = server();
    await tester.pumpWidget(app(const IntegrationsScreen(), client));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('menu-mcc_GH')));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Disconnect'));
    await tester.pumpAndSettle();
    expect(find.textContaining('revoke it at the service'), findsOneWidget);
    await tester.tap(find.byKey(const Key('confirm-disconnect')));
    await tester.pumpAndSettle();
    expect(seen.any((r) => r.method == 'DELETE'), isTrue);
  });

  group('the launcher', () {
    final hosts = [
      AgentHost.fromJson({
        'id': 'hst_1',
        'name': 'build-vm',
        'status': 'online',
        'egress': 'host',
        'capabilities': {
          'providers': [
            {'id': 'claude-code', 'kind': 'cli', 'available': true},
            {'id': 'shell', 'kind': 'cli', 'available': true},
          ],
          'workspaces': ['storm'],
          'max_sessions': 8,
        },
      }),
    ];

    (MockClient, List<http.Request>) agentServer({String? notice}) {
      final seen = <http.Request>[];
      final client = MockClient((req) async {
        seen.add(req);
        final path = req.url.path;
        if (path == '/v1/config/agent') {
          return http.Response('{"default_provider":"claude-code"}', 200);
        }
        if (path == '/v1/agent/hosts/hst_1/workspaces') {
          return http.Response('[{"name":"storm","live_sessions":0}]', 200);
        }
        if (path == '/v1/agent/sessions' && req.method == 'POST') {
          return http.Response(
            jsonEncode({
              'id': 'ags_1',
              'host_id': 'hst_1',
              'workspace': 'storm',
              'provider': 'claude-code',
              'status': 'starting',
              'cols': 80,
              'rows': 24,
              'created_at': '2026-10-05T00:00:00Z',
              'mcp': {
                'connections': <Object>[],
                'allow_vault_writes': false,
                'notice': notice,
              },
            }),
            200,
          );
        }
        return http.Response('{"error":"nope"}', 404);
      });
      return (client, seen);
    }

    Widget launcher(MockClient client) => ProviderScope(
      overrides: [
        agentApiFactoryProvider.overrideWithValue(
          () => AgentApi(baseUrl: 'http://s', token: 't', client: client),
        ),
        agentAccessProvider.overrideWith((ref) async => true),
      ],
      child: MaterialApp(
        theme: StormTheme.light(),
        home: Scaffold(body: launcherForTest(hosts)),
      ),
    );

    testWidgets('vault writes are off unless the owner turns them on', (
      tester,
    ) async {
      final (client, seen) = agentServer();
      await tester.pumpWidget(launcher(client));
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('egress-integrations')), findsOneWidget);
      final toggle = tester.widget<SwitchListTile>(
        find.byKey(const Key('allow-vault-writes')),
      );
      expect(toggle.value, isFalse);
      await tester.tap(find.byKey(const Key('allow-vault-writes')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const Key('launch')));
      await tester.pumpAndSettle();
      final launch = seen.firstWhere((r) => r.method == 'POST');
      expect(jsonDecode(launch.body)['allow_vault_writes'], isTrue);
    });

    testWidgets('a shell gets no toggle and no integrations line (G-D9)', (
      tester,
    ) async {
      final (client, _) = agentServer();
      await tester.pumpWidget(launcher(client));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const Key('provider-shell')));
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('allow-vault-writes')), findsNothing);
      expect(find.byKey(const Key('egress-integrations')), findsNothing);
    });

    test('an old host is announced on the launch answer', () {
      final s = AgentSession.fromJson({
        'id': 'ags_1',
        'host_id': 'hst_1',
        'workspace': 'storm',
        'provider': 'claude-code',
        'status': 'starting',
        'mcp': {
          'notice': "build-vm can't use integrations — update storm-runtime",
        },
      });
      expect(s.launchNotice, contains('update storm-runtime'));
    });
  });
}
