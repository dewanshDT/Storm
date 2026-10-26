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
import 'package:storm/agent/hosts_screen.dart';
import 'package:storm/agent/session_screen.dart';
import 'package:storm/ui/theme.dart';

/// Decision 135d: a host or session whose identity is not verified says so,
/// on the host row and in the session's Details.
void main() {
  Map<String, dynamic> identity(String state, [String? reason]) => {
    'state': state,
    'mechanism': state == 'verified' ? 'so_peerpidfd' : null,
    'reason': reason,
  };

  Map<String, dynamic> host(String id, Map<String, dynamic>? identity) => {
    'id': id,
    'name': id,
    'status': 'online',
    'egress': 'host',
    'capabilities': {
      'providers': [
        {'id': 'shell', 'kind': 'cli', 'available': true},
      ],
      'workspaces': ['storm'],
      'max_sessions': 8,
    },
    'identity': ?identity,
  };

  AgentSession session(Map<String, dynamic>? identity) =>
      AgentSession.fromJson({
        'id': 'ags_1',
        'host_id': 'hst_1',
        'workspace': 'storm',
        'provider': 'claude-code',
        'status': 'running',
        'identity': ?identity,
      });

  List<String> about(AgentSession s) => [
    for (final r in aboutRows(
      s,
      host: 'h',
      writeVault: null,
      integrations: null,
    ))
      if (r.$1 == 'Identity') r.$2,
  ];

  test('only a verified session, or one from an older server, has no row', () {
    const label =
        'Session identity not verified — sessions on this host can act as '
        'each other.';
    expect(about(session(identity('unavailable', 'old_host'))), [label]);
    expect(about(session(identity('unsupported', 'windows'))), [label]);
    expect(about(session(identity('unavailable', 'ptrace_scope'))), [label]);
    expect(about(session(identity('verified'))), isEmpty);
    expect(about(session(null)), isEmpty);
    // The label survives the live record merging in.
    final s = session(identity('unsupported', 'windows'));
    expect(s.withLive(session(null)).identityUnverified, isTrue);
  });

  testWidgets('an unverified host row says so; a verified one does not', (
    tester,
  ) async {
    SharedPreferences.setMockInitialValues({});
    final client = MockClient((req) async {
      if (req.url.path == '/v1/agent/hosts') {
        return http.Response(
          jsonEncode([
            host('hst_old', identity('unavailable', 'old_host')),
            host('hst_ok', identity('verified')),
            host('hst_unknown', null),
          ]),
          200,
        );
      }
      if (req.url.path == '/v1/config/agent') {
        return http.Response('{"default_provider":"shell"}', 200);
      }
      if (req.url.path == '/v1/agent/sessions') {
        return http.Response('[]', 200);
      }
      return http.Response('{"error":"nope"}', 404);
    });
    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          agentApiFactoryProvider.overrideWithValue(
            () => AgentApi(baseUrl: 'http://s', token: 't', client: client),
          ),
        ],
        child: MaterialApp(
          theme: StormTheme.light(),
          home: Builder(
            builder: (context) => MediaQuery(
              data: MediaQuery.of(
                context,
              ).copyWith(size: const Size(1280, 800)),
              child: const Scaffold(body: HostsScreen()),
            ),
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();
    final line = 'online now · Shell · network: host policy';
    expect(find.text('$line\n$kIdentityNotVerified'), findsOneWidget);
    expect(find.text(line), findsNWidgets(2));
  });
}
