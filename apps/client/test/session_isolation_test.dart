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

/// Decision 136j: a host's session isolation is said on its row and in its
/// sessions' Details once isolation is on; with it off, nothing changes.
void main() {
  Map<String, dynamic> isolation(String state, [String? reason]) => {
    'state': state,
    'mechanism': state == 'unsupported' ? null : 'systemd_units',
    'reason': reason,
  };

  Map<String, dynamic> host(String id, Map<String, dynamic>? isolation) => {
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
    'identity': {'state': 'verified', 'mechanism': 'so_peerpidfd'},
    'isolation': ?isolation,
  };

  AgentSession session(Map<String, dynamic>? isolation) =>
      AgentSession.fromJson({
        'id': 'ags_1',
        'host_id': 'hst_1',
        'workspace': 'storm',
        'provider': 'claude-code',
        'status': 'running',
        'isolation': ?isolation,
      });

  List<String> about(AgentSession s) => [
    for (final r in aboutRows(
      s,
      host: 'h',
      writeVault: null,
      integrations: null,
    ))
      if (r.$1 == 'Isolation') r.$2,
  ];

  test('off, an older host or server, and macOS or Windows say nothing', () {
    expect(about(session(isolation('unavailable', 'off'))), isEmpty);
    expect(about(session(isolation('unavailable', 'old_host'))), isEmpty);
    expect(about(session(isolation('unsupported', 'windows'))), isEmpty);
    expect(about(session(null)), isEmpty);
  });

  test('a verified host says so; any other state with it on refuses', () {
    expect(about(session(isolation('verified'))), ['$kIsolationVerified.']);
    for (final (reason, why) in [
      ('pending', 'self-test running'),
      ('self_test', 'self-test failed'),
      ('no_passt', 'pasta missing'),
      ('helper', 'helper not answering'),
      ('no_helper', 'no isolation helper'),
      ('unrecognised', 'unrecognised'),
    ]) {
      expect(about(session(isolation('unavailable', reason))), [
        'Session isolation unavailable ($why) — new sessions are refused.',
      ], reason: reason);
    }
    // The label survives the live record merging in.
    final s = session(isolation('unavailable', 'no_passt'));
    expect(s.withLive(session(null)).isolationLabel, isNotNull);
  });

  test('a start refused for it says why', () {
    final s = AgentSession.fromJson({
      'id': 'ags_1',
      'host_id': 'hst_1',
      'workspace': 'storm',
      'provider': 'shell',
      'status': 'failed',
      'end_reason': 'isolation_unavailable',
    });
    expect(s.endDetail, 'isolation unavailable on this host');
  });

  testWidgets('the host row says what isolation is, only when it is on', (
    tester,
  ) async {
    SharedPreferences.setMockInitialValues({});
    final client = MockClient((req) async {
      if (req.url.path == '/v1/agent/hosts') {
        return http.Response(
          jsonEncode([
            host('hst_off', isolation('unavailable', 'off')),
            host('hst_ok', isolation('verified')),
            host('hst_down', isolation('unavailable', 'self_test')),
            host('hst_old', null),
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
    const line = 'online now · Shell · network: host policy';
    expect(find.text(line), findsNWidgets(2));
    expect(find.text('$line\n$kIsolationVerified'), findsOneWidget);
    expect(
      find.text(
        '$line\nSession isolation unavailable (self-test failed) — new '
        'sessions are refused',
      ),
      findsOneWidget,
    );
  });
}
