import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'package:storm/agent/agent_models.dart';
import 'package:storm/ui/settings/settings_widgets.dart';
import 'package:storm/router.dart';
import 'package:storm/ui/controls.dart';

import 'agent_fakes.dart';
import 'shell_harness.dart';

/// Decision 133: Settings › Agents installs, updates and removes the agent
/// CLIs on each host, on the real router at both sides of 900.
void main() {
  const phone = Size(390, 844);
  const desk = Size(1280, 800);

  setUp(() => SharedPreferences.setMockInitialValues({}));

  Map<String, dynamic> provider(
    String id, {
    bool available = false,
    String? version,
    String? source,
    bool? installable,
    String? blocked,
    bool? checksumVerified,
    String? latest,
  }) => {
    'id': id,
    'kind': 'cli',
    'available': available,
    'version': ?version,
    'source': ?source,
    'installable': ?installable,
    'install_blocked': ?blocked,
    'checksum_verified': ?checksumVerified,
    'latest': ?latest,
  };

  Map<String, dynamic> host(
    String id,
    String name,
    List<Map<String, dynamic>> providers, {
    bool agentInstall = true,
    String status = 'online',
  }) => {
    'id': id,
    'name': name,
    'status': status,
    'capabilities': status == 'online'
        ? {
            'providers': providers,
            'workspaces': ['storm'],
            'max_sessions': 8,
            if (agentInstall) 'agent_install': true,
          }
        : null,
  };

  Future<ProviderContainer> open(
    WidgetTester tester,
    FakeAgentServer agents, {
    Size size = desk,
  }) async {
    final c = shellContainer(agentClient: agents.client);
    await pumpShell(tester, c, size: size);
    c.read(routerProvider).go(Routes.settingsPage('agents'));
    await tester.pumpAndSettle();
    return c;
  }

  String sub(WidgetTester tester, String hostId, String agent) => tester
      .widget<SettingsRow>(
        find.descendant(
          of: find.byKey(Key('agent-row-$hostId-$agent')),
          matching: find.byType(SettingsRow),
        ),
      )
      .sub!;

  Finder action(String what, String hostId, String agent) =>
      find.byKey(Key('agent-$what-$hostId-$agent'));

  Future<void> tap(WidgetTester tester, Finder f) async {
    await tester.ensureVisible(f);
    await tester.pumpAndSettle();
    await tester.tap(f);
    await tester.pumpAndSettle();
  }

  test('an older server parses as before', () {
    final p = ProviderCap.fromJson({'id': 'codex', 'available': true});
    expect(
      (p.version, p.source, p.installable, p.checksumVerified, p.latest),
      (null, null, false, true, null),
    );
    final h = AgentHost.fromJson({
      'id': 'h',
      'name': 'h',
      'status': 'online',
      'capabilities': {'providers': <Object>[]},
    });
    expect(h.agentInstall, isFalse);
    final job = AgentJob.fromJson({
      'id': 'job_1',
      'host_id': 'h',
      'agent': 'codex',
      'op': 'install',
      'state': 'running',
      'step': 'Verifying',
      'created': 'x',
      'updated': 'y',
    });
    expect((job.state, job.step, job.done), ('running', 'Verifying', false));
  });

  for (final size in [phone, desk]) {
    testWidgets('every agent lists every host with its version and source '
        'at ${size.width}', (tester) async {
      final agents = FakeAgentServer(
        hosts: [
          host('h1', 'codebox', [
            provider(
              'claude-code',
              available: true,
              version: '2.1.310',
              source: 'storm',
              latest: '2.1.320',
            ),
            provider(
              'opencode',
              available: true,
              version: '1.0.4',
              source: 'system',
            ),
            provider('codex', installable: true),
            provider('cursor', installable: true, checksumVerified: false),
            provider('command-code', installable: true),
          ]),
          host('h2', 'laptop', [], status: 'offline'),
        ],
      );
      final c = await open(tester, agents, size: size);

      for (final a in kInstallableAgents) {
        expect(find.byKey(Key('agent-section-$a')), findsOneWidget);
        expect(find.byKey(Key('agent-row-h1-$a')), findsOneWidget);
        expect(find.byKey(Key('agent-row-h2-$a')), findsOneWidget);
      }
      expect(sub(tester, 'h1', 'claude-code'), '2.1.310 · installed by Storm');
      expect(sub(tester, 'h1', 'opencode'), '1.0.4 · your own install');
      expect(sub(tester, 'h1', 'codex'), 'Not installed');
      expect(sub(tester, 'h2', 'codex'), 'offline');

      expect(find.text('Update to 2.1.320'), findsOneWidget);
      expect(action('remove', 'h1', 'claude-code'), findsOneWidget);
      // Not Storm's copy: nothing to update or remove from here.
      expect(action('update', 'h1', 'opencode'), findsNothing);
      expect(action('remove', 'h1', 'opencode'), findsNothing);
      expect(action('install', 'h1', 'opencode'), findsNothing);
      for (final a in ['codex', 'cursor', 'command-code']) {
        expect(action('install', 'h1', a), findsOneWidget, reason: a);
        expect(action('install', 'h2', a), findsNothing, reason: 'offline');
      }
      expect(find.text('Not checksum-verified by Cursor'), findsOneWidget);
      expect(
        find.textContaining("It's offline."),
        findsNWidgets(kInstallableAgents.length),
      );
      expect(find.textContaining('codex login --device-auth'), findsOneWidget);
      expect(find.textContaining('opencode auth login'), findsOneWidget);
      expect(find.textContaining('cursor-agent login'), findsOneWidget);
      expect(tester.takeException(), isNull);

      await tap(tester, find.byKey(const Key('agents-claude-login')));
      expect(
        c.read(routerProvider).state.uri.toString(),
        Routes.settingsPage('claude-login'),
      );
      await disposeShell(tester, c);
    });
  }

  testWidgets('Update is plain when the newest version is unknown', (
    tester,
  ) async {
    final agents = FakeAgentServer(
      hosts: [
        host('h1', 'codebox', [
          provider('codex', available: true, version: '0.9', source: 'storm'),
        ]),
      ],
    );
    final c = await open(tester, agents);
    expect(
      find.descendant(
        of: action('update', 'h1', 'codex'),
        matching: find.text('Update'),
      ),
      findsOneWidget,
    );
    await disposeShell(tester, c);
  });

  testWidgets('a host that cannot install says why, in words', (tester) async {
    final agents = FakeAgentServer(
      hosts: [
        host('h1', 'codebox', [
          provider('claude-code', blocked: 'disabled'),
          provider('opencode', blocked: 'unsupported_platform'),
          provider('codex', blocked: 'no_node'),
          provider('command-code', blocked: 'node_too_old'),
        ]),
      ],
    );
    final c = await open(tester, agents);
    for (final a in ['claude-code', 'opencode', 'codex', 'command-code']) {
      expect(action('install', 'h1', a), findsNothing, reason: a);
    }
    String blocked(String a) => tester
        .widget<Text>(
          find.descendant(
            of: action('blocked', 'h1', a),
            matching: find.byType(Text),
          ),
        )
        .data!;
    expect(blocked('claude-code'), 'Installs are off on this host');
    expect(blocked('opencode'), "Not available on this host's system yet");
    expect(blocked('codex'), 'Needs Node.js on this host');
    expect(
      blocked('command-code'),
      'Needs a newer Node.js on this host (22 or later)',
    );
    await disposeShell(tester, c);
  });

  testWidgets('an older runtime is read only and says to update it', (
    tester,
  ) async {
    final agents = FakeAgentServer(
      hosts: [
        host('h1', 'old-box', [
          provider('claude-code', available: true),
          provider('codex', installable: true),
        ], agentInstall: false),
      ],
    );
    final c = await open(tester, agents);
    expect(action('install', 'h1', 'codex'), findsNothing);
    expect(sub(tester, 'h1', 'claude-code'), 'installed');
    expect(
      find.text('Update storm-runtime to manage agents here.'),
      findsNWidgets(kInstallableAgents.length),
    );
    await disposeShell(tester, c);
  });

  testWidgets('an install shows each step, then the host as it is now', (
    tester,
  ) async {
    final agents =
        FakeAgentServer(
            hosts: [
              host('h1', 'codebox', [provider('codex', installable: true)]),
            ],
          )
          ..agentJobs['codex'] = [
            {'state': 'queued'},
            {'state': 'running', 'step': 'Downloading'},
            {'state': 'running', 'step': 'Verifying'},
            {'state': 'running', 'step': 'Installing'},
            {'state': 'done', 'version': '0.170.0'},
          ];
    agents.onAgentJobDone = (hostId, agent, op) => agents.hosts[0] = host(
      'h1',
      'codebox',
      [provider('codex', available: true, version: '0.170.0', source: 'storm')],
    );
    final c = await open(tester, agents);

    await tap(tester, action('install', 'h1', 'codex'));
    expect(agents.agentJobStarts, ['h1/codex:install']);
    String progress() =>
        tester.widget<Text>(action('progress', 'h1', 'codex')).data!;
    expect(progress(), 'Starting…');
    expect(action('install', 'h1', 'codex'), findsNothing);
    for (final step in ['Downloading…', 'Verifying…', 'Installing…']) {
      await tester.pump(const Duration(seconds: 1));
      await tester.pump();
      expect(progress(), step);
    }
    await tester.pump(const Duration(seconds: 1));
    await tester.pumpAndSettle();
    expect(action('progress', 'h1', 'codex'), findsNothing);
    expect(sub(tester, 'h1', 'codex'), '0.170.0 · installed by Storm');
    expect(action('remove', 'h1', 'codex'), findsOneWidget);
    await disposeShell(tester, c);
  });

  testWidgets('a failed job says why in one line', (tester) async {
    final agents =
        FakeAgentServer(
            hosts: [
              host('h1', 'codebox', [
                provider('cursor', installable: true, checksumVerified: false),
                provider('codex', installable: true),
              ]),
            ],
          )
          ..agentJobs['cursor'] = [
            {'state': 'running', 'step': 'Downloading'},
            {'state': 'failed', 'error': 'checksum_mismatch'},
          ]
          ..agentJobs['codex'] = [
            {'state': 'running', 'step': 'Downloading'},
            {'state': 'failed', 'error': 'mystery_code'},
          ];
    final c = await open(tester, agents);

    await tap(tester, action('install', 'h1', 'cursor'));
    expect(
      tester.widget<Text>(action('progress', 'h1', 'cursor')).data,
      'Downloading…',
    );
    // The host does one at a time.
    expect(
      tester.widget<TextAction>(action('install', 'h1', 'codex')).onTap,
      isNull,
    );
    await tester.pump(const Duration(seconds: 1));
    await tester.pumpAndSettle();
    expect(
      tester.widget<Text>(action('error', 'h1', 'cursor')).data,
      "The download didn't match its checksum, so nothing was installed.",
    );
    expect(action('install', 'h1', 'cursor'), findsOneWidget);

    await tap(tester, action('install', 'h1', 'codex'));
    await tester.pump(const Duration(seconds: 1));
    await tester.pumpAndSettle();
    expect(
      tester.widget<Text>(action('error', 'h1', 'codex')).data,
      contains('mystery_code'),
    );
    await disposeShell(tester, c);
  });

  testWidgets('a refused start is said the same way', (tester) async {
    final agents =
        FakeAgentServer(
            hosts: [
              host('h1', 'codebox', [
                provider('claude-code', available: true, source: 'storm'),
                provider('codex', installable: true),
              ]),
            ],
          )
          ..agentRefusals['claude-code'] = (409, 'agent_in_use')
          ..agentRefusals['codex'] = (503, 'host_offline');
    final c = await open(tester, agents);
    await tap(tester, action('update', 'h1', 'claude-code'));
    expect(
      tester.widget<Text>(action('error', 'h1', 'claude-code')).data,
      'Sessions are using it. End them and try again.',
    );
    await tap(tester, action('install', 'h1', 'codex'));
    expect(
      tester.widget<Text>(action('error', 'h1', 'codex')).data,
      'The host is offline.',
    );
    await disposeShell(tester, c);
  });

  testWidgets('Remove asks first', (tester) async {
    final agents =
        FakeAgentServer(
            hosts: [
              host('h1', 'codebox', [
                provider(
                  'codex',
                  available: true,
                  version: '1',
                  source: 'storm',
                ),
              ]),
            ],
          )
          ..agentJobs['codex'] = [
            {'state': 'running'},
            {'state': 'done'},
          ];
    final c = await open(tester, agents);

    await tap(tester, action('remove', 'h1', 'codex'));
    expect(find.text('Remove Codex from codebox?'), findsOneWidget);
    await tester.tap(find.text('Cancel'));
    await tester.pumpAndSettle();
    expect(agents.agentJobStarts, isEmpty);

    await tap(tester, action('remove', 'h1', 'codex'));
    await tester.tap(find.byKey(const Key('agent-remove-confirm')));
    await tester.pumpAndSettle();
    expect(agents.agentJobStarts, ['h1/codex:remove']);
    expect(
      tester.widget<Text>(action('progress', 'h1', 'codex')).data,
      'Removing…',
    );
    await tester.pump(const Duration(seconds: 1));
    await tester.pumpAndSettle();
    expect(action('progress', 'h1', 'codex'), findsNothing);
    await disposeShell(tester, c);
  });

  testWidgets('leaving the page stops asking about the job', (tester) async {
    final agents =
        FakeAgentServer(
            hosts: [
              host('h1', 'codebox', [provider('codex', installable: true)]),
            ],
          )
          ..agentJobs['codex'] = [
            {'state': 'running', 'step': 'Downloading'},
          ];
    final c = await open(tester, agents);
    await tap(tester, action('install', 'h1', 'codex'));
    c.read(routerProvider).go(Routes.settingsPage('storage'));
    await tester.pumpAndSettle();
    final polls = agents.requests.where((r) => r.url.path.contains('/jobs/'));
    final before = polls.length;
    await tester.pump(const Duration(seconds: 3));
    expect(polls.length, before);
    await disposeShell(tester, c);
  });
}
