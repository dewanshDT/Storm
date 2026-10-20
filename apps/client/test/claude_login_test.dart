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
import 'package:storm/agent/claude_login_api.dart';
import 'package:storm/agent/claude_login_screen.dart';
import 'package:storm/agent/launcher.dart';
import 'package:storm/ui/theme.dart';

/// Claude subscription token sync, client side (decision 124, slice 124q).
/// The server is mocked to the §0.3 shapes.
void main() {
  const canary = 'sk-ant-oat01-STORMCANARY_abcdefghijklmnopqrstuvwxyz0123';

  Map<String, dynamic> synced({
    String status = 'active',
    String? warn,
    int days = 365,
    List<String> hosts = const [],
  }) => {
    'kind': kClaudeTokenKind,
    'present': true,
    'status': status,
    'fingerprint': '3f9a01c2',
    'minted_at': '2026-10-16T09:00:00Z',
    'expires_at': '2027-10-16T09:00:00Z',
    'days_left': days,
    'warn': warn,
    'hosts': hosts,
  };

  Map<String, dynamic> none() => {
    'kind': kClaudeTokenKind,
    'present': false,
    'hosts': <String>[],
  };

  Map<String, dynamic> hostJson(
    String id,
    String name, {
    bool online = true,
    bool receives = true,
  }) => {
    'id': id,
    'name': name,
    'status': online ? 'online' : 'offline',
    if (online)
      'capabilities': {
        'providers': [
          {'id': 'claude-code', 'kind': 'cli', 'available': true},
          {'id': 'shell', 'kind': 'cli', 'available': true},
        ],
        'workspaces': ['storm'],
        'max_sessions': 8,
        if (receives) 'provider_credentials': [kClaudeTokenKind],
      },
  };

  /// A fake server holding one view, recording every request.
  (MockClient, List<http.Request>) server({
    Map<String, dynamic>? view,
    int getStatus = 200,
  }) {
    final seen = <http.Request>[];
    var current = view ?? none();
    final client = MockClient((req) async {
      seen.add(req);
      final path = req.url.path;
      const base = '/v1/provider-credentials/claude-subscription';
      if (path == base && req.method == 'GET') {
        if (getStatus != 200) {
          return http.Response('{"error":"forbidden"}', getStatus);
        }
        return http.Response(jsonEncode(current), 200);
      }
      if (path == base && req.method == 'PUT') {
        final body = jsonDecode(req.body) as Map;
        if (cleanClaudeToken(body['token'] as String) == null) {
          return http.Response('{"error":"bad_token_shape"}', 400);
        }
        current = {...synced(), 'hosts': current['hosts']};
        return http.Response(jsonEncode(current), 200);
      }
      if (path == base && req.method == 'DELETE') {
        current = none();
        return http.Response('', 204);
      }
      if (path == '$base/hosts' && req.method == 'PATCH') {
        final hosts = (jsonDecode(req.body) as Map)['hosts'] as List;
        current = {...current, 'hosts': hosts};
        return http.Response(jsonEncode(current), 200);
      }
      if (path == '/v1/agent/hosts') {
        return http.Response(
          jsonEncode([
            hostJson('hst_1', 'build-vm'),
            hostJson('hst_2', 'old-mac', receives: false),
            hostJson('hst_3', 'laptop', online: false),
            {'id': 'hst_4', 'name': 'gone', 'status': 'revoked'},
          ]),
          200,
        );
      }
      return http.Response('{"error":"nope"}', 404);
    });
    return (client, seen);
  }

  Widget app(Widget child, MockClient client) => ProviderScope(
    overrides: [
      agentApiFactoryProvider.overrideWithValue(
        () => AgentApi(baseUrl: 'http://s', token: 't', client: client),
      ),
      claudeLoginApiFactoryProvider.overrideWithValue(
        () => ClaudeLoginApi(baseUrl: 'http://s', token: 't', client: client),
      ),
    ],
    child: MaterialApp(
      theme: StormTheme.light(),
      home: Builder(
        builder: (context) => MediaQuery(
          data: MediaQuery.of(context).copyWith(size: const Size(1280, 1600)),
          child: Scaffold(body: child),
        ),
      ),
    ),
  );

  Future<void> pumpPage(WidgetTester tester, MockClient client) async {
    tester.view.physicalSize = const Size(1280, 1600);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    await tester.pumpWidget(app(const ClaudeLoginScreen(), client));
    await tester.pumpAndSettle();
  }

  Finder holding(String text) => find.byWidgetPredicate(
    (w) => w is EditableText && w.controller.text.contains(text),
  );

  bool ticked(WidgetTester tester, String id) => tester
      .widget<CheckboxListTile>(find.byKey(Key('claude-host-$id')))
      .value!;

  setUp(() => SharedPreferences.setMockInitialValues({}));

  test(
    'the token shape is the spec\'s, after paste markers and whitespace',
    () {
      expect(cleanClaudeToken('  \x1b[200~$canary\x1b[201~\r\n'), canary);
      expect(
        cleanClaudeToken('sk-ant-api03-abcdefghijklmnopqrstuvwxyz'),
        isNull,
      );
      expect(
        cleanClaudeToken('sk-ant-oat01-abc def ghijklmnopqrstuvwxyz'),
        isNull,
      );
      expect(cleanClaudeToken('sk-ant-oat01-short'), isNull);
      expect(cleanClaudeToken('sk-ant-oat01-${'a' * 401}'), isNull);
    },
  );

  group('the token field', () {
    testWidgets('is obscured, sent once, and never shown after save', (
      tester,
    ) async {
      final (client, seen) = server();
      await pumpPage(tester, client);
      expect(find.byKey(const Key('claude-login-pill-notSynced')), findsOne);

      await tester.tap(find.byKey(const Key('claude-sync')));
      await tester.pumpAndSettle();
      expect(
        find.byKey(const Key('claude-scrollback-warning')),
        findsOneWidget,
        reason: 'the outside-Storm warning comes before the field',
      );
      final field = tester.widget<TextField>(
        find.byKey(const Key('claude-token-field')),
      );
      expect(field.obscureText, isTrue);
      expect(field.autocorrect, isFalse);
      expect(field.enableSuggestions, isFalse);

      await tester.enterText(
        find.byKey(const Key('claude-token-field')),
        canary,
      );
      await tester.tap(find.byKey(const Key('claude-token-save')));
      await tester.pumpAndSettle();

      final put = seen.singleWhere((r) => r.method == 'PUT');
      expect(jsonDecode(put.body), {'token': canary});
      expect(find.byKey(const Key('claude-token-field')), findsNothing);
      expect(holding(canary), findsNothing);
      expect(find.textContaining('STORMCANARY'), findsNothing);
      expect(find.byKey(const Key('claude-login-pill-active')), findsOne);
      expect(
        find.textContaining('Synced · fingerprint 3f9a01c2'),
        findsOneWidget,
      );

      await tester.tap(find.byKey(const Key('claude-sync')));
      await tester.pumpAndSettle();
      expect(find.text('Replace token'), findsWidgets);
      expect(holding(canary), findsNothing, reason: 'reopened empty');
    });

    testWidgets('a wrong shape is refused without echoing it or sending it', (
      tester,
    ) async {
      final (client, seen) = server();
      await pumpPage(tester, client);
      await tester.tap(find.byKey(const Key('claude-sync')));
      await tester.pumpAndSettle();
      await tester.enterText(
        find.byKey(const Key('claude-token-field')),
        'sk-ant-api03-NOTOAUTH_abcdefghijklmnopqrstuvwxyz',
      );
      await tester.tap(find.byKey(const Key('claude-token-save')));
      await tester.pumpAndSettle();
      expect(find.text(kClaudeTokenRule), findsOneWidget);
      expect(find.textContaining('NOTOAUTH'), findsNothing);
      expect(holding('sk-ant'), findsNothing, reason: 'cleared, not kept');
      expect(seen.where((r) => r.method == 'PUT'), isEmpty);
    });

    testWidgets('an earlier mint date is sent', (tester) async {
      final (client, seen) = server();
      await pumpPage(tester, client);
      await tester.tap(find.byKey(const Key('claude-sync')));
      await tester.pumpAndSettle();
      await tester.enterText(
        find.byKey(const Key('claude-token-field')),
        canary,
      );
      await tester.tap(find.byKey(const Key('claude-minted-change')));
      await tester.pumpAndSettle();
      final now = DateTime.now();
      await tester.tap(find.text('1').last);
      await tester.tap(find.text('OK'));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const Key('claude-token-save')));
      await tester.pumpAndSettle();
      final body =
          jsonDecode(seen.singleWhere((r) => r.method == 'PUT').body) as Map;
      if (now.day == 1) {
        expect(body.containsKey('minted_at'), isFalse, reason: 'today');
      } else {
        final minted = DateTime.parse(body['minted_at'] as String).toLocal();
        expect(
          (minted.year, minted.month, minted.day),
          (now.year, now.month, 1),
        );
      }
    });

    testWidgets('remove says it does not revoke, then deletes', (tester) async {
      final (client, seen) = server(view: synced(hosts: ['hst_1']));
      await pumpPage(tester, client);
      await tester.tap(find.byKey(const Key('claude-remove')));
      await tester.pumpAndSettle();
      expect(
        find.textContaining('revoke it in your Claude account settings'),
        findsOneWidget,
      );
      await tester.tap(find.byKey(const Key('claude-remove-confirm')));
      await tester.pumpAndSettle();
      expect(seen.where((r) => r.method == 'DELETE'), hasLength(1));
      expect(find.byKey(const Key('claude-login-pill-notSynced')), findsOne);
    });
  });

  group('the host picker', () {
    testWidgets('defaults to none and offers no "all hosts"', (tester) async {
      final (client, seen) = server();
      await pumpPage(tester, client);
      expect(ticked(tester, 'hst_1'), isFalse);
      expect(ticked(tester, 'hst_2'), isFalse);
      expect(ticked(tester, 'hst_3'), isFalse);
      expect(find.byKey(const Key('claude-host-hst_4')), findsNothing);
      expect(find.textContaining('All hosts'), findsNothing);
      expect(find.byKey(const Key('claude-hosts-warning')), findsOneWidget);

      await tester.tap(find.byKey(const Key('claude-sync')));
      await tester.pumpAndSettle();
      await tester.enterText(
        find.byKey(const Key('claude-token-field')),
        canary,
      );
      await tester.tap(find.byKey(const Key('claude-token-save')));
      await tester.pumpAndSettle();
      expect(ticked(tester, 'hst_1'), isFalse, reason: 'syncing ticks nothing');
      expect(seen.where((r) => r.method == 'PATCH'), isEmpty);
    });

    testWidgets('saves exactly the ticked hosts, and marks an old one', (
      tester,
    ) async {
      final (client, seen) = server(view: synced());
      await pumpPage(tester, client);
      expect(
        tester
            .widget<Text>(find.byKey(const Key('claude-host-detail-hst_2')))
            .data,
        contains('update storm-runtime to use this'),
      );
      expect(
        tester
            .widget<Text>(find.byKey(const Key('claude-host-detail-hst_1')))
            .data,
        isNot(contains('update storm-runtime')),
      );
      await tester.tap(find.byKey(const Key('claude-host-hst_1')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const Key('claude-hosts-save')));
      await tester.pumpAndSettle();
      final patch = seen.singleWhere((r) => r.method == 'PATCH');
      expect(patch.url.path, endsWith('/claude-subscription/hosts'));
      expect(jsonDecode(patch.body), {
        'hosts': ['hst_1'],
      });
      expect(ticked(tester, 'hst_1'), isTrue);
    });
  });

  group('expiry states', () {
    for (final (view, state, banner) in [
      (synced(), 'active', false),
      (synced(warn: '30d', days: 21), 'expiring30', true),
      (synced(warn: '7d', days: 5), 'expiring7', true),
      (synced(warn: 'expired', days: 0), 'expired', true),
      (synced(status: 'needs_resync'), 'needsResync', true),
      (none(), 'notSynced', false),
    ]) {
      testWidgets(state, (tester) async {
        final (client, _) = server(view: view);
        await pumpPage(tester, client);
        expect(find.byKey(Key('claude-login-pill-$state')), findsOneWidget);
        expect(
          find.byKey(Key('claude-login-banner-$state')),
          banner ? findsOneWidget : findsNothing,
        );
      });
    }

    testWidgets('the banners say what to do', (tester) async {
      final (client, _) = server(view: synced(warn: '7d', days: 5));
      await pumpPage(tester, client);
      expect(find.textContaining('expires in 5 days'), findsOneWidget);
      expect(find.text('Expires in 5 days'), findsOneWidget);
    });

    test('the nav dot is on for 30d, 7d, expired and unreadable only', () {
      bool on(Map<String, dynamic> j) => ClaudeLogin.fromJson(j).needsAttention;
      expect(on(synced()), isFalse);
      expect(on(none()), isFalse);
      expect(on(synced(warn: '30d')), isTrue);
      expect(on(synced(warn: '7d')), isTrue);
      expect(on(synced(warn: 'expired')), isTrue);
      expect(on(synced(status: 'needs_resync')), isTrue);
    });
  });

  testWidgets('the page is the owner\'s: a refusal shows no controls', (
    tester,
  ) async {
    final (client, _) = server(getStatus: 403);
    await pumpPage(tester, client);
    expect(
      find.text('Claude login is managed by the server owner only.'),
      findsOneWidget,
    );
    expect(find.byKey(const Key('claude-sync')), findsNothing);
    expect(find.byKey(const Key('claude-host-hst_1')), findsNothing);
    expect(find.byKey(const Key('claude-login-notice')), findsOneWidget);
  });

  testWidgets('the notice is the single-user, terms and revocation text', (
    tester,
  ) async {
    final (client, _) = server();
    await pumpPage(tester, client);
    final notice = tester
        .widget<Text>(
          find.descendant(
            of: find.byKey(const Key('claude-login-notice')),
            matching: find.byType(Text),
          ),
        )
        .data!;
    expect(notice, contains('one person, on their own machines'));
    expect(
      notice,
      contains(
        '"may not collect, store, or intermediate Claude.ai credentials or '
        'session tokens"',
      ),
    );
    expect(notice, contains('claude.ai'));
  });

  group('the launcher line', () {
    (MockClient, List<http.Request>) launchServer(Map<String, dynamic> view) {
      final seen = <http.Request>[];
      final client = MockClient((req) async {
        seen.add(req);
        final path = req.url.path;
        if (path == '/v1/config/agent') {
          return http.Response('{"default_provider":"claude-code"}', 200);
        }
        if (path.endsWith('/workspaces')) {
          return http.Response('[{"name":"storm","live_sessions":0}]', 200);
        }
        if (path == '/v1/provider-credentials/claude-subscription') {
          return http.Response(jsonEncode(view), 200);
        }
        return http.Response('{"error":"nope"}', 404);
      });
      return (client, seen);
    }

    Future<void> pumpLauncher(
      WidgetTester tester,
      Map<String, dynamic> view, {
      bool receives = true,
    }) async {
      final (client, _) = launchServer(view);
      await tester.pumpWidget(
        ProviderScope(
          overrides: [
            agentApiFactoryProvider.overrideWithValue(
              () => AgentApi(baseUrl: 'http://s', token: 't', client: client),
            ),
            claudeLoginApiFactoryProvider.overrideWithValue(
              () => ClaudeLoginApi(
                baseUrl: 'http://s',
                token: 't',
                client: client,
              ),
            ),
          ],
          child: MaterialApp(
            theme: StormTheme.light(),
            home: Scaffold(
              body: NewSessionLauncher(
                hosts: [
                  AgentHost.fromJson(
                    hostJson('hst_1', 'build-vm', receives: receives),
                  ),
                ],
              ),
            ),
          ),
        ),
      );
      await tester.pumpAndSettle();
    }

    testWidgets('a ticked host uses the synced login', (tester) async {
      await pumpLauncher(tester, synced(hosts: ['hst_1']));
      expect(find.text('Claude: your synced login'), findsOneWidget);
    });

    testWidgets("an unticked host uses its own", (tester) async {
      await pumpLauncher(tester, synced());
      expect(find.text("Claude: this host's own login"), findsOneWidget);
    });

    testWidgets('no token: its own', (tester) async {
      await pumpLauncher(tester, none());
      expect(find.byKey(const Key('launcher-claude-login-own')), findsOne);
    });

    testWidgets('an old host, ticked, uses its own and says why', (
      tester,
    ) async {
      await pumpLauncher(tester, synced(hosts: ['hst_1']), receives: false);
      expect(find.byKey(const Key('launcher-claude-login-oldHost')), findsOne);
      expect(find.textContaining('update storm-runtime'), findsOneWidget);
    });

    testWidgets('expired is not delivered', (tester) async {
      await pumpLauncher(
        tester,
        synced(hosts: ['hst_1'], warn: 'expired', days: 0),
      );
      expect(find.byKey(const Key('launcher-claude-login-expired')), findsOne);
    });

    testWidgets('seven days left is said on a synced launch', (tester) async {
      await pumpLauncher(tester, synced(hosts: ['hst_1'], warn: '7d', days: 3));
      expect(find.byKey(const Key('launcher-claude-login-synced')), findsOne);
      expect(find.textContaining('expires in 3 days'), findsOneWidget);
    });

    testWidgets('a shell has no Claude line', (tester) async {
      await pumpLauncher(tester, synced(hosts: ['hst_1']));
      await tester.tap(find.byKey(const Key('launcher-agent')));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Shell').last);
      await tester.pumpAndSettle();
      expect(find.textContaining('Claude:'), findsNothing);
    });
  });

  test('the launch answer says when the synced login was not used', () {
    AgentSession launched(String credential, {String? mcp}) =>
        AgentSession.fromJson({
          'id': 'ags_1',
          'host_id': 'hst_1',
          'workspace': 'storm',
          'provider': 'claude-code',
          'status': 'starting',
          'credential': credential,
          'mcp': {'notice': mcp},
        });
    expect(launched('synced').launchNotice, isNull);
    expect(launched('native').launchNotice, isNull);
    expect(launched('old_host').launchNotice, contains('update storm-runtime'));
    expect(
      launched('expired').launchNotice,
      contains('re-sync it in Settings'),
    );
    expect(
      launched('old_host', mcp: "build-vm can't use integrations").launchNotice,
      allOf(contains('integrations'), contains('synced Claude login')),
    );
    expect(launched('expired').credential, 'expired');
  });
}
