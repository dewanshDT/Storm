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

  Map<String, dynamic> host(
    String id,
    Map<String, dynamic>? isolation, {
    Map<String, dynamic>? execution,
    String identity = 'verified',
  }) => {
    'id': id,
    'name': id,
    'status': 'online',
    'egress': 'host',
    'execution': ?execution,
    'capabilities': {
      'providers': [
        {'id': 'shell', 'kind': 'cli', 'available': true},
      ],
      'workspaces': ['storm'],
      'max_sessions': 8,
    },
    'identity': {'state': identity, 'mechanism': 'so_peerpidfd'},
    'isolation': ?isolation,
  };

  AgentSession session(
    Map<String, dynamic>? isolation, {
    Map<String, dynamic>? ranAs,
  }) => AgentSession.fromJson({
    'id': 'ags_1',
    'host_id': 'hst_1',
    'workspace': 'storm',
    'provider': 'claude-code',
    'status': 'running',
    'isolation': ?isolation,
    'ran_as': ?ranAs,
  });

  List<String> rows(AgentSession s, String label) => [
    for (final r in aboutRows(s))
      if (r.$1 == label) r.$2,
  ];

  List<String> about(AgentSession s) => rows(s, 'Isolation');

  test('off, an older host or server, and macOS or Windows say nothing', () {
    expect(about(session(isolation('unavailable', 'off'))), isEmpty);
    expect(about(session(isolation('unavailable', 'old_host'))), isEmpty);
    expect(about(session(isolation('unsupported', 'windows'))), isEmpty);
    expect(about(session(null)), isEmpty);
  });

  test('About has no Isolation row; the session keeps its label (153j)', () {
    expect(about(session(isolation('verified'))), isEmpty);
    for (final reason in ['pending', 'self_test', 'no_passt', 'helper']) {
      final s = session(isolation('unavailable', reason));
      expect(about(s), isEmpty, reason: reason);
      expect(s.isolationLabel, isNotNull, reason: reason);
    }
    // The label survives the live record merging in.
    final s = session(isolation('unavailable', 'no_passt'));
    expect(s.withLive(session(null)).isolationLabel, isNotNull);
  });

  test('Ran as reads the snapshot, and an older session is Not recorded', () {
    Map<String, dynamic> ranAs(String? account, String state) => {
      'account': account,
      'isolation': isolation(state, state == 'verified' ? null : 'off'),
      'identity': {'state': 'verified'},
    };
    String ranAsOf(AgentSession s) => rows(s, 'Ran as').single;
    expect(
      ranAsOf(session(null, ranAs: ranAs('dewansh', 'unavailable'))),
      'dewansh',
    );
    expect(
      ranAsOf(session(null, ranAs: ranAs(null, 'unavailable'))),
      'An unknown account',
    );
    // The snapshot wins over what the session's host says now.
    expect(
      ranAsOf(
        session(
          isolation('unavailable', 'off'),
          ranAs: ranAs('storm-runtime', 'verified'),
        ),
      ),
      'Its own session user (isolated) · the host runs as storm-runtime',
    );
    expect(ranAsOf(session(isolation('verified'))), 'Not recorded');
    expect(ranAsOf(session(null)), 'Not recorded');
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

  Future<void> pumpHosts(
    WidgetTester tester,
    List<Map<String, dynamic>> hosts,
  ) async {
    SharedPreferences.setMockInitialValues({});
    final client = MockClient((req) async {
      if (req.url.path == '/v1/agent/hosts') {
        return http.Response(jsonEncode(hosts), 200);
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
  }

  const line = 'online now · Shell · network: host policy';

  testWidgets('the host row says what isolation is, only when it is on', (
    tester,
  ) async {
    await pumpHosts(tester, [
      host('hst_off', isolation('unavailable', 'off')),
      host('hst_ok', isolation('verified')),
      host('hst_down', isolation('unavailable', 'self_test')),
      host('hst_old', null),
    ]);
    expect(find.text('$line\n$kRunsAsUnknown'), findsNWidgets(2));
    expect(
      find.text('$line\n$kRunsAsUnknown\n$kIsolationVerified'),
      findsOneWidget,
    );
    expect(
      find.text(
        '$line\n$kRunsAsUnknown\nSession isolation unavailable (self-test '
        'failed) — new sessions are refused',
      ),
      findsOneWidget,
    );
  });

  // Decision 151q: every row says who its host runs as, and identity only
  // as a fix-it line when it is not verified.
  final userRun = {'account': 'dewansh', 'isolation': 'off'};
  final service = {'account': 'storm-runtime', 'isolation': 'off'};
  final sandbox = {'account': 'storm-runtime', 'isolation': 'per-session'};

  testWidgets('every host row says who it runs as', (tester) async {
    await pumpHosts(tester, [
      host('hst_me', isolation('unavailable', 'off'), execution: userRun),
      host('hst_svc', isolation('unavailable', 'off'), execution: service),
      host('hst_box', isolation('verified'), execution: sandbox),
      host(
        'hst_nameless',
        null,
        execution: {'account': null, 'isolation': 'off'},
      ),
      host('hst_old', null, identity: 'unavailable'),
    ]);
    expect(find.text('$line\nRuns as dewansh'), findsOneWidget);
    expect(find.text('$line\nRuns as storm-runtime'), findsOneWidget);
    expect(
      find.text('$line\nRuns as storm-runtime · each session isolated'),
      findsOneWidget,
    );
    expect(find.text('$line\n$kRunsAsUnknown'), findsOneWidget);
    expect(
      find.text('$line\n$kRunsAsUnknown\n$kIdentityNotVerified'),
      findsOneWidget,
    );
    expect(find.textContaining(kIdentityNotVerified), findsOneWidget);
  });

  testWidgets('no non-sandbox host is called isolated, trusted, secure or '
      'protected', (tester) async {
    await pumpHosts(tester, [
      host('hst_me', isolation('unavailable', 'off'), execution: userRun),
      host('hst_svc', isolation('unavailable', 'off'), execution: service),
      host('hst_old', null),
      host('hst_unverified', null, execution: userRun, identity: 'unsupported'),
    ]);
    final forbidden = RegExp(
      r'isolated|trusted|secure|protected',
      caseSensitive: false,
    );
    final said = [
      for (final w in tester.widgetList<Text>(find.byType(Text)))
        w.data ?? w.textSpan?.toPlainText() ?? '',
    ];
    expect(said.where((x) => x.contains('\nRuns as ')), hasLength(4));
    expect(said.where(forbidden.hasMatch), isEmpty);
    for (final r in [
      ranAsLine(null),
      ranAsLine(const RanAs(account: 'dewansh')),
      ranAsLine(const RanAs(isolationState: 'unavailable')),
      executionLineFor(const HostExecution(account: 'dewansh')),
      executionLineFor(null),
    ]) {
      expect(forbidden.hasMatch(r), isFalse, reason: r);
    }
  });
}
