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
import 'package:storm/agent/launcher.dart';
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
    Object? hosts,
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
    'hosts': ?hosts,
  };

  /// Another connection, on one host only.
  Map<String, dynamic> scoped(String id, String name, String host) => {
    ...github(hosts: [host]),
    'id': id,
    'display_name': name,
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
      if (path == '/v1/agent/hosts') {
        return http.Response(
          jsonEncode([
            {'id': 'hst_1', 'name': 'build-vm', 'status': 'online'},
            {'id': 'hst_2', 'name': 'work-mac', 'status': 'offline'},
            {'id': 'hst_3', 'name': 'old-box', 'status': 'revoked'},
          ]),
          200,
        );
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
      agentApiFactoryProvider.overrideWithValue(
        () => AgentApi(baseUrl: 'http://s', token: 't', client: client),
      ),
      integrationsApiFactoryProvider.overrideWithValue(
        () => IntegrationsApi(baseUrl: 'http://s', token: 't', client: client),
      ),
      oauthSupportedProvider.overrideWithValue(oauth),
    ],
    // Desk width: the page sits beside the settings navigation there and
    // needs no router. The phone layout is covered in the shell
    // (settings_pages_test.dart).
    child: MaterialApp(
      theme: StormTheme.light(),
      home: Builder(
        builder: (context) => MediaQuery(
          data: MediaQuery.of(context).copyWith(size: const Size(1280, 800)),
          child: Scaffold(body: child),
        ),
      ),
    ),
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
    expect(find.textContaining('Read only'), findsOneWidget);
    expect(find.text('Needs sign-in'), findsOneWidget);
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

  /// The segment for `value` in the Add dialog's auth choice.
  ButtonSegment<String> segment(WidgetTester tester, String value) => tester
      .widget<SegmentedButton<String>>(
        find.byKey(const Key('integration-auth')),
      )
      .segments
      .firstWhere((s) => s.value == value);

  testWidgets('the web shows sign-in as unavailable and connects with a token '
      '(G-D13)', (tester) async {
    final (client, _) = server();
    await tester.pumpWidget(
      app(const IntegrationsScreen(), client, oauth: false),
    );
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('add-integration')));
    await tester.pumpAndSettle();
    expect(find.text('How Storm connects'), findsOneWidget);
    expect(segment(tester, 'oauth').enabled, isFalse);
    expect(segment(tester, 'static').enabled, isTrue);
    expect(find.text('No sign-in'), findsOneWidget);
    // Token is the default, and the note explaining why sits with the choice.
    expect(find.byKey(const Key('integration-token')), findsOneWidget);
    expect(find.byKey(const Key('integration-web-note')), findsOneWidget);
  });

  testWidgets('each auth choice says what it means', (tester) async {
    final (client, _) = server();
    await tester.pumpWidget(app(const IntegrationsScreen(), client));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('add-integration')));
    await tester.pumpAndSettle();
    expect(segment(tester, 'oauth').enabled, isTrue);
    expect(find.byKey(const Key('integration-web-note')), findsNothing);
    String help() => tester
        .widget<Text>(find.byKey(const Key('integration-auth-help')))
        .data!;
    expect(help(), contains('browser opens to sign in'));
    await tester.tap(find.text('Token'));
    await tester.pumpAndSettle();
    expect(help(), contains('token'));
    await tester.tap(find.text('No sign-in'));
    await tester.pumpAndSettle();
    expect(help(), contains('no credential'));
  });

  testWidgets('on the web, an OAuth integration is reconnected from an app, '
      'not by an action that can only fail', (tester) async {
    final notion = {
      ...github(status: 'needs_reauth'),
      'id': 'mcc_NO',
      'slug': 'notion',
      'display_name': 'Notion',
      'auth_kind': 'oauth',
    };
    final (client, seen) = server(items: [builtin(), notion]);
    await tester.pumpWidget(
      app(const IntegrationsScreen(), client, oauth: false),
    );
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('reconnect-mcc_NO')), findsNothing);
    expect(find.text('Reconnect from a Storm app'), findsOneWidget);
    await tester.tap(find.byKey(const Key('menu-mcc_NO')));
    await tester.pumpAndSettle();
    final item = tester.widget<PopupMenuItem<String>>(
      find.ancestor(
        of: find.text('Sign in again from a Storm app'),
        matching: find.byType(PopupMenuItem<String>),
      ),
    );
    expect(item.enabled, isFalse);
    expect(seen.where((r) => r.url.path.endsWith('/authorize')), isEmpty);
  });

  testWidgets('new tools start off and the owner chooses', (tester) async {
    final (client, seen) = server();
    await tester.pumpWidget(app(const IntegrationsScreen(), client));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('tools-mcc_GH')));
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

  test('a connection is on every host unless the server names some '
      '(decision 121)', () {
    expect(Integration.fromJson(github()).hosts, isNull);
    expect(Integration.fromJson(github(hosts: 'all')).hosts, isNull);
    final two = Integration.fromJson(github(hosts: ['hst_1', 'hst_2']));
    expect(two.hosts, ['hst_1', 'hst_2']);
    expect(two.availableOn('hst_2'), isTrue);
    expect(two.availableOn('hst_9'), isFalse);
    expect(two.availableOn(null), isFalse);
    expect(Integration.fromJson(github()).availableOn('hst_9'), isTrue);
  });

  test('a scope reads as all hosts, a host by name, or a count', () {
    const names = {'hst_1': 'work-mac'};
    expect(hostScopeLabel(null, names), 'All hosts');
    expect(hostScopeLabel(['hst_1'], names), 'work-mac');
    expect(hostScopeLabel(['hst_9'], names), 'No enrolled host');
    expect(hostScopeLabel(['hst_9'], const {}), '1 host');
    expect(hostScopeLabel(['hst_1', 'hst_2'], names), '2 hosts');
  });

  testWidgets('the owner scopes an integration to chosen hosts', (
    tester,
  ) async {
    final (client, seen) = server(
      items: [
        builtin(),
        github(hosts: ['hst_1']),
      ],
    );
    await tester.pumpWidget(app(const IntegrationsScreen(), client));
    await tester.pumpAndSettle();
    expect(find.text('Token · 1 tool on · build-vm'), findsOneWidget);
    await tester.tap(find.byKey(const Key('menu-mcc_GH')));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Choose hosts'));
    await tester.pumpAndSettle();
    // A revoked host is not offered.
    expect(find.byKey(const Key('host-hst_3')), findsNothing);
    FilledButton save() =>
        tester.widget<FilledButton>(find.byKey(const Key('save-hosts')));
    await tester.tap(find.byKey(const Key('host-hst_1')));
    await tester.pumpAndSettle();
    expect(save().onPressed, isNull, reason: 'an empty scope is refused');
    expect(find.byKey(const Key('hosts-empty')), findsOneWidget);
    await tester.tap(find.byKey(const Key('host-hst_2')));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('save-hosts')));
    await tester.pumpAndSettle();
    final patch = seen.firstWhere((r) => r.method == 'PATCH');
    expect(jsonDecode(patch.body), {
      'hosts': ['hst_2'],
    });
  });

  testWidgets('All hosts is sent as "all"', (tester) async {
    final (client, seen) = server(
      items: [
        builtin(),
        github(hosts: ['hst_1', 'hst_2']),
      ],
    );
    await tester.pumpWidget(app(const IntegrationsScreen(), client));
    await tester.pumpAndSettle();
    expect(find.text('Token · 1 tool on · 2 hosts'), findsOneWidget);
    // The built-in connection has no scope to choose.
    expect(find.textContaining('All hosts'), findsNothing);
    await tester.tap(find.byKey(const Key('menu-mcc_GH')));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Choose hosts'));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('hosts-all')));
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('host-hst_1')), findsNothing);
    await tester.tap(find.byKey(const Key('save-hosts')));
    await tester.pumpAndSettle();
    final patch = seen.firstWhere((r) => r.method == 'PATCH');
    expect(jsonDecode(patch.body), {'hosts': 'all'});
  });

  test('a session lists only the integrations scoped to its host', () {
    final off = {
      ...github(status: 'disabled'),
      'id': 'mcc_X',
      'display_name': 'Off',
    };
    final all = [
      builtin(),
      github(),
      scoped('mcc_LI', 'Linear', 'hst_1'),
      scoped('mcc_NO', 'Notion', 'hst_2'),
      off,
    ].map(Integration.fromJson).toList();
    expect(integrationNamesOn(all, 'hst_1'), ['GitHub', 'Linear']);
    expect(integrationNamesOn(all, 'hst_2'), ['GitHub', 'Notion']);
    expect(integrationNamesOn(all, null), ['GitHub']);
    expect(integrationNamesOn(null, 'hst_1'), isEmpty);
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

    Widget launcher(
      MockClient client, {
      List<Map<String, dynamic>>? integrations,
    }) => ProviderScope(
      overrides: [
        agentApiFactoryProvider.overrideWithValue(
          () => AgentApi(baseUrl: 'http://s', token: 't', client: client),
        ),
        if (integrations != null)
          integrationsApiFactoryProvider.overrideWithValue(
            () => IntegrationsApi(
              baseUrl: 'http://s',
              token: 't',
              client: MockClient(
                (_) async => http.Response(jsonEncode(integrations), 200),
              ),
            ),
          ),
      ],
      child: MaterialApp(
        theme: StormTheme.light(),
        home: Scaffold(body: NewSessionLauncher(hosts: hosts)),
      ),
    );

    testWidgets('an agent is told it can use your integrations', (
      tester,
    ) async {
      final (client, seen) = agentServer();
      await tester.pumpWidget(launcher(client));
      await tester.pumpAndSettle();
      expect(
        find.textContaining('This session can use your integrations'),
        findsOneWidget,
      );
      await tester.tap(find.byKey(const Key('launch')));
      await tester.pumpAndSettle();
      final launch = seen.firstWhere((r) => r.method == 'POST');
      expect(
        (jsonDecode(launch.body) as Map).containsKey('allow_vault_writes'),
        isFalse,
        reason: 'the pre-v2 toggle is never sent (decision 82)',
      );
    });

    testWidgets("the risk line names only the chosen host's integrations "
        '(decision 121)', (tester) async {
      final (client, _) = agentServer();
      await tester.pumpWidget(
        launcher(
          client,
          integrations: [
            builtin(),
            github(),
            scoped('mcc_LI', 'Linear', 'hst_1'),
            scoped('mcc_NO', 'Notion', 'hst_2'),
          ],
        ),
      );
      await tester.pumpAndSettle();
      expect(
        find.textContaining('your integrations (GitHub, Linear)'),
        findsOneWidget,
      );
      expect(find.textContaining('Notion'), findsNothing);
    });

    testWidgets('a shell gets no write field and no integrations (G-D9)', (
      tester,
    ) async {
      final (client, _) = agentServer();
      await tester.pumpWidget(launcher(client));
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('launcher-writes')), findsOneWidget);
      await tester.tap(find.byKey(const Key('launcher-agent')));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Shell').last);
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('launcher-writes')), findsNothing);
      expect(find.textContaining('A shell has no access'), findsOneWidget);
      expect(find.textContaining('can use your integrations'), findsNothing);
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
