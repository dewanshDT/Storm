import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:shared_preferences/shared_preferences.dart';

import 'package:storm/agent/agent_models.dart';
import 'package:storm/agent/agents_settings_screen.dart';
import 'package:storm/router.dart';
import 'package:storm/ui/tokens.dart';

import 'agent_fakes.dart';
import 'shell_harness.dart';

/// Settings › Agents & hosts (153-7): hosts as rows, agents as columns, on
/// the real router at both sides of 900. Installs keep decision 133's
/// behaviour, moved into each cell's popover.
void main() {
  const phone = Size(390, 844);
  const desk = Size(1280, 800);
  final dark = StormTokens.from(StormPreset.stormDark);

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
    String? login,
    bool? signedIn,
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
    'login_command': ?login,
    'signed_in': ?signedIn,
  };

  Map<String, dynamic> host(
    String id,
    String name,
    List<Map<String, dynamic>> providers, {
    bool agentInstall = true,
    bool typed = true,
    String status = 'online',
    String account = 'dewansh',
    String isolation = 'off',
    String? lastSeen,
  }) => {
    'id': id,
    'name': name,
    'status': status,
    'last_seen': ?lastSeen,
    'execution': {'account': account, 'isolation': isolation},
    'limits': {
      'max_children': 3,
      'max_tree': 6,
      'max_sessions': null,
      'host_max_sessions': 8,
      'effective_max_sessions': 8,
      'depth': 1,
      'hard_max_children': 10,
      'hard_max_tree': 24,
    },
    'capabilities': status == 'online'
        ? {
            'providers': providers,
            'workspaces': ['storm'],
            'max_sessions': 8,
            if (agentInstall) 'agent_install': true,
            if (typed) 'typed_input': true,
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
    c.read(routerProvider).go(Routes.settingsPage('hosts'));
    await tester.pumpAndSettle();
    return c;
  }

  Future<void> tap(WidgetTester tester, Finder f) async {
    await tester.ensureVisible(f);
    await tester.pumpAndSettle();
    await tester.tap(f);
    await tester.pumpAndSettle();
  }

  Text textOf(WidgetTester tester, String key) =>
      tester.widget<Text>(find.byKey(Key(key)));

  String word(WidgetTester tester, String h, String a) =>
      textOf(tester, 'cell-word-$h-$a').data!;

  String? subOf(WidgetTester tester, String h, String a) {
    final f = find.byKey(Key('cell-sub-$h-$a'));
    return f.evaluate().isEmpty ? null : tester.widget<Text>(f).data;
  }

  Future<void> openCell(WidgetTester tester, String h, String a) =>
      tap(tester, find.byKey(Key('cell-$h-$a')));

  String meaning(WidgetTester tester) => textOf(tester, 'cell-meaning').data!;

  test('an older server parses as before', () {
    final p = ProviderCap.fromJson({'id': 'codex', 'available': true});
    expect(
      (p.version, p.source, p.installable, p.checksumVerified, p.latest),
      (null, null, false, true, null),
    );
    expect((p.loginCommand, p.signedIn), (null, null));
    final h = AgentHost.fromJson({
      'id': 'h',
      'name': 'h',
      'status': 'online',
      'capabilities': {'providers': <Object>[]},
    });
    expect((h.agentInstall, h.typedInput), (false, false));
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

  FakeAgentServer everyState() => FakeAgentServer(
    hosts: [
      host('h1', 'build-vm', [
        provider(
          'claude-code',
          available: true,
          version: '2.0.31',
          source: 'storm',
          latest: '2.0.31',
          login: 'claude auth login',
        ),
        provider(
          'opencode',
          available: true,
          version: '0.15.2',
          source: 'system',
          login: 'opencode auth login',
          signedIn: false,
        ),
        provider('codex', installable: true, login: 'codex login'),
        provider('cursor', blocked: 'no_node'),
        provider(
          'command-code',
          available: true,
          version: '0.15',
          source: 'storm',
          latest: '0.16',
        ),
      ]),
      host(
        'h2',
        'mac-mini',
        [],
        status: 'offline',
        lastSeen: DateTime.now()
            .subtract(const Duration(minutes: 9))
            .toUtc()
            .toIso8601String(),
      ),
    ],
  );

  testWidgets('five agent columns, named, with the default tagged', (
    tester,
  ) async {
    final c = await open(tester, everyState());
    expect(find.byKey(const Key('agents-grid')), findsOneWidget);
    double left(String a) =>
        tester.getTopLeft(find.byKey(Key('agent-head-$a'))).dx;
    for (var i = 1; i < kInstallableAgents.length; i++) {
      expect(
        left(kInstallableAgents[i - 1]),
        lessThan(left(kInstallableAgents[i])),
      );
    }
    expect(kInstallableAgents, hasLength(5));
    expect(find.byKey(const Key('agent-head-shell')), findsNothing);
    for (final a in kInstallableAgents) {
      expect(
        find.descendant(
          of: find.byKey(Key('agent-head-$a')),
          matching: find.text(providerLabel(a)),
        ),
        findsOneWidget,
      );
    }
    // The host column is about a quarter of the card, never under 170, and
    // the five agent columns are equal.
    final widths = [
      for (final a in kInstallableAgents)
        tester.getSize(find.byKey(Key('cell-h1-$a'))).width,
    ];
    expect(widths.toSet(), hasLength(1));
    final hostColumn =
        tester.getTopLeft(find.byKey(const Key('cell-h1-claude-code'))).dx -
        tester.getTopLeft(find.byKey(const Key('host-cell-h1'))).dx;
    final card = tester.getSize(find.byKey(const Key('agents-grid'))).width;
    expect(hostColumn, greaterThanOrEqualTo(170));
    expect(hostColumn / card, inInclusiveRange(0.22, 0.28));
    final tag = find.byKey(const Key('default-tag-claude-code'));
    expect(tag, findsOneWidget);
    expect(
      tester
          .widget<Text>(find.descendant(of: tag, matching: find.byType(Text)))
          .style!
          .color,
      dark.accent,
    );
    await disposeShell(tester, c);
  });

  testWidgets('clicking another header makes it the default', (tester) async {
    final agents = everyState();
    final puts = <Map<String, dynamic>>[];
    agents.extra = (req) async {
      if (req.url.path == '/v1/config/agent' && req.method == 'PUT') {
        puts.add(jsonDecode(req.body) as Map<String, dynamic>);
        return http.Response('{}', 200);
      }
      return null;
    };
    final c = await open(tester, agents);
    await tap(tester, find.byKey(const Key('agent-head-codex')));
    expect(puts, [
      {'default_provider': 'codex'},
    ]);
    expect(find.byKey(const Key('default-tag-codex')), findsOneWidget);
    expect(find.byKey(const Key('default-tag-claude-code')), findsNothing);
    await disposeShell(tester, c);
  });

  testWidgets('each cell is one word, and only actionable words are accent', (
    tester,
  ) async {
    final c = await open(tester, everyState());
    expect(word(tester, 'h1', 'claude-code'), 'Ready');
    expect(subOf(tester, 'h1', 'claude-code'), '2.0.31');
    expect(word(tester, 'h1', 'opencode'), 'Sign in');
    expect(subOf(tester, 'h1', 'opencode'), '0.15.2');
    expect(word(tester, 'h1', 'codex'), 'Install');
    expect(word(tester, 'h1', 'cursor'), "Can't install");
    expect(subOf(tester, 'h1', 'cursor'), 'needs Node.js');
    expect(word(tester, 'h1', 'command-code'), 'Update');
    expect(subOf(tester, 'h1', 'command-code'), '0.15 → 0.16');
    for (final a in kInstallableAgents) {
      expect(word(tester, 'h2', a), '—', reason: 'offline');
    }
    Color colour(String h, String a) =>
        textOf(tester, 'cell-word-$h-$a').style!.color!;
    expect(colour('h1', 'claude-code'), dark.text);
    for (final a in ['opencode', 'codex', 'command-code']) {
      expect(colour('h1', a), dark.accent, reason: a);
    }
    expect(colour('h1', 'cursor'), dark.text3);
    expect(colour('h2', 'codex'), dark.text3);
    // The sub-line: mono, `text3`, at the smallest step (10.5 is below it).
    final sub = textOf(tester, 'cell-sub-h1-claude-code').style!;
    expect(sub.fontFamily, StormTokens.monoFamily);
    expect(sub.color, dark.text3);
    expect(sub.fontSize, StormTokens.minStep);
    expect(
      agentCell(
        AgentHost.fromJson(
          host('x', 'x', [provider('command-code', blocked: 'node_too_old')]),
        ),
        'command-code',
        null,
      ).sub,
      'needs Node 22',
    );
    await disposeShell(tester, c);
  });

  testWidgets('a host cell is never green and always says who it runs as', (
    tester,
  ) async {
    final c = await open(tester, everyState());
    expect(textOf(tester, 'host-sub-h1').data, 'Runs as dewansh');
    expect(
      textOf(tester, 'host-sub-h2').data,
      'Offline · last seen 9m ago\nRuns as dewansh',
    );
    BoxDecoration dot(String h) =>
        tester
                .widget<Container>(
                  find.descendant(
                    of: find.byKey(Key('host-dot-$h')),
                    matching: find.byType(Container),
                  ),
                )
                .decoration!
            as BoxDecoration;
    expect(dot('h1').color, dark.text2);
    expect(dot('h1').border, isNull);
    expect(dot('h2').color, isNull);
    expect((dot('h2').border! as Border).top.color, dark.text3);
    for (final h in ['h1', 'h2']) {
      expect(dot(h).color, isNot(dark.green));
    }
    final said = [
      for (final w in tester.widgetList<Text>(find.byType(Text)))
        w.data ?? w.textSpan?.toPlainText() ?? '',
    ].join('\n');
    expect(
      RegExp(
        r'trusted|secure|protected|sandboxed',
        caseSensitive: false,
      ).hasMatch(said),
      isFalse,
    );
    await disposeShell(tester, c);
  });

  group('the popover', () {
    testWidgets('says what each state means and offers one fix', (
      tester,
    ) async {
      final agents = everyState();
      final c = await open(tester, agents);

      await openCell(tester, 'h1', 'claude-code');
      expect(find.byKey(const Key('cell-popover')), findsOneWidget);
      expect(find.text('Claude Code on build-vm'), findsOneWidget);
      expect(
        textOf(tester, 'cell-meta').data,
        '2.0.31 · installed by Storm · up to date',
      );
      expect(meaning(tester), contains("Storm can't see whether"));
      expect(find.byKey(const Key('cell-fix')), findsNothing);
      expect(find.byKey(const Key('cell-remove')), findsOneWidget);
      await tester.tapAt(const Offset(1270, 790));
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('cell-popover')), findsNothing);

      await openCell(tester, 'h1', 'opencode');
      expect(textOf(tester, 'cell-meta').data, '0.15.2 · your own install');
      expect(
        meaning(tester),
        "Not signed in. Sessions can't use OpenCode on build-vm until it is.",
      );
      expect(find.byKey(const Key('cell-signin')), findsOneWidget);
      expect(
        textOf(tester, 'outside-storm').data,
        "Installed outside Storm, so Storm won't update or remove it.",
      );
      expect(find.byKey(const Key('cell-remove')), findsNothing);
      await tester.tapAt(const Offset(1270, 790));
      await tester.pumpAndSettle();

      await openCell(tester, 'h1', 'cursor');
      expect(
        meaning(tester),
        "Needs Node.js on this host. Sessions can't use Cursor there.",
      );
      expect(find.byKey(const Key('cell-fix')), findsNothing);
      await tester.tapAt(const Offset(1270, 790));
      await tester.pumpAndSettle();

      await openCell(tester, 'h2', 'codex');
      expect(meaning(tester), contains('mac-mini is offline.'));
      expect(find.byKey(const Key('cell-fix')), findsNothing);
      await tester.tapAt(const Offset(1270, 790));
      await tester.pumpAndSettle();

      await openCell(tester, 'h1', 'command-code');
      expect(meaning(tester), contains('0.16 is available.'));
      await tap(tester, find.byKey(const Key('cell-fix')));
      expect(agents.agentJobStarts, ['h1/command-code:update']);
      await disposeShell(tester, c);
    });

    testWidgets('Install shows each step and a bar, then the host as it is', (
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
      agents.onAgentJobDone = (hostId, agent, op) =>
          agents.hosts[0] = host('h1', 'codebox', [
            provider(
              'codex',
              available: true,
              version: '0.170.0',
              source: 'storm',
            ),
          ]);
      final c = await open(tester, agents);
      await openCell(tester, 'h1', 'codex');
      expect(meaning(tester), startsWith('Not installed.'));
      await tap(tester, find.byKey(const Key('cell-fix')));
      expect(agents.agentJobStarts, ['h1/codex:install']);
      expect(word(tester, 'h1', 'codex'), 'Installing');
      expect(find.byKey(const Key('cell-bar-h1-codex')), findsOneWidget);
      expect(
        tester.getSize(find.byKey(const Key('cell-bar-h1-codex'))).height,
        3,
      );
      for (final n in [1, 2, 3]) {
        await tester.pump(const Duration(seconds: 1));
        await tester.pump();
        expect(subOf(tester, 'h1', 'codex'), 'step $n of 3');
      }
      await tester.pump(const Duration(seconds: 1));
      await tester.pumpAndSettle();
      expect(word(tester, 'h1', 'codex'), 'Ready');
      expect(subOf(tester, 'h1', 'codex'), '0.170.0');
      expect(find.byKey(const Key('cell-remove')), findsOneWidget);
      await disposeShell(tester, c);
    });

    testWidgets('a failed or refused job says why, in one line', (
      tester,
    ) async {
      final agents =
          FakeAgentServer(
              hosts: [
                host('h1', 'codebox', [
                  provider(
                    'cursor',
                    installable: true,
                    checksumVerified: false,
                  ),
                  provider('claude-code', available: true, source: 'storm'),
                ]),
              ],
            )
            ..agentJobs['cursor'] = [
              {'state': 'running', 'step': 'Downloading'},
              {'state': 'failed', 'error': 'checksum_mismatch'},
            ]
            ..agentRefusals['claude-code'] = (409, 'agent_in_use');
      final c = await open(tester, agents);
      await openCell(tester, 'h1', 'cursor');
      expect(find.text('Not checksum-verified by Cursor.'), findsOneWidget);
      await tap(tester, find.byKey(const Key('cell-fix')));
      await tester.pump(const Duration(seconds: 1));
      await tester.pumpAndSettle();
      expect(
        textOf(tester, 'agent-error').data,
        "The download didn't match its checksum, so nothing was installed.",
      );
      await tester.tapAt(const Offset(1270, 790));
      await tester.pumpAndSettle();

      await openCell(tester, 'h1', 'claude-code');
      await tap(tester, find.byKey(const Key('cell-remove')));
      await tap(tester, find.byKey(const Key('agent-remove-confirm')));
      expect(
        textOf(tester, 'agent-error').data,
        'Sessions are using it. End them and try again.',
      );
      await disposeShell(tester, c);
    });

    testWidgets('Remove confirms in the popover, not a dialog', (tester) async {
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
      await openCell(tester, 'h1', 'codex');
      await tap(tester, find.byKey(const Key('cell-remove')));
      expect(find.byType(AlertDialog), findsNothing);
      expect(find.textContaining('Remove Codex from codebox?'), findsOneWidget);
      await tap(tester, find.text('Cancel'));
      expect(agents.agentJobStarts, isEmpty);
      await tap(tester, find.byKey(const Key('cell-remove')));
      await tap(tester, find.byKey(const Key('agent-remove-confirm')));
      expect(agents.agentJobStarts, ['h1/codex:remove']);
      expect(word(tester, 'h1', 'codex'), 'Removing');
      await tester.pump(const Duration(seconds: 1));
      await tester.pumpAndSettle();
      await disposeShell(tester, c);
    });
  });

  group('Sign in in a Shell (153-S3)', () {
    FakeAgentServer signIn({bool typed = true, String isolation = 'off'}) =>
        FakeAgentServer(
          hosts: [
            host(
              'h1',
              'build-vm',
              [
                provider(
                  'opencode',
                  available: true,
                  version: '0.15.2',
                  source: 'system',
                  login: 'opencode auth login',
                  signedIn: false,
                ),
                provider('codex', available: true, version: '1'),
                provider('shell', available: true),
              ],
              typed: typed,
              isolation: isolation,
            ),
          ],
        );

    testWidgets('launches a Shell with the login command typed in', (
      tester,
    ) async {
      final agents = signIn();
      final c = await open(tester, agents);
      await openCell(tester, 'h1', 'opencode');
      expect(
        find.textContaining('opencode auth login typed in', findRichText: true),
        findsOneWidget,
      );
      await tap(tester, find.byKey(const Key('cell-signin')));
      final launch = agents.launches.single;
      expect(launch['provider'], 'shell');
      expect(launch['host_id'], 'h1');
      expect(launch['workspace'], 'storm');
      expect(launch['typed_input'], 'opencode auth login');
      expect(
        c.read(routerProvider).state.uri.path,
        startsWith('/agents/'),
        reason: 'it goes to the new session',
      );
      await disposeShell(tester, c);
    });

    testWidgets('a host that cannot type gets the command to copy and a '
        'plain Shell', (tester) async {
      final agents = signIn(typed: false);
      final c = await open(tester, agents);
      await openCell(tester, 'h1', 'opencode');
      expect(find.byKey(const Key('cell-signin')), findsNothing);
      expect(find.byKey(const Key('signin-copy')), findsOneWidget);
      expect(find.text('opencode auth login'), findsOneWidget);
      await tap(tester, find.byKey(const Key('open-shell')));
      expect(agents.launches.single['provider'], 'shell');
      expect(agents.launches.single.containsKey('typed_input'), isFalse);
      await disposeShell(tester, c);
    });

    testWidgets("a 409 host_cannot_type falls back the same way", (
      tester,
    ) async {
      final agents = signIn()..refuseLaunch = (409, 'host_cannot_type');
      final c = await open(tester, agents);
      await openCell(tester, 'h1', 'opencode');
      await tap(tester, find.byKey(const Key('cell-signin')));
      expect(agents.launches.single['typed_input'], 'opencode auth login');
      expect(find.byKey(const Key('cell-signin')), findsNothing);
      expect(find.byKey(const Key('signin-fallback')), findsOneWidget);
      expect(find.byKey(const Key('open-shell')), findsOneWidget);
      await disposeShell(tester, c);
    });

    testWidgets('a per-session host says to sign in on the host itself', (
      tester,
    ) async {
      final agents = signIn(isolation: 'per-session');
      final c = await open(tester, agents);
      await openCell(tester, 'h1', 'opencode');
      expect(find.byKey(const Key('cell-signin')), findsNothing);
      expect(find.byKey(const Key('open-shell')), findsNothing);
      expect(
        textOf(tester, 'signin-on-host').data,
        'Each session on this host runs as its own user, so sign in on the '
        'host itself: opencode auth login',
      );
      await disposeShell(tester, c);
    });

    testWidgets('no login command, no Sign in', (tester) async {
      final c = await open(tester, signIn());
      await openCell(tester, 'h1', 'codex');
      expect(find.byKey(const Key('cell-signin')), findsNothing);
      expect(find.byKey(const Key('signin-command')), findsNothing);
      await disposeShell(tester, c);
    });
  });

  group('a revoked host', () {
    FakeAgentServer revoked({int status = 200}) {
      final agents = FakeAgentServer(
        hosts: [
          host('h1', 'build-vm', []),
          host('h9', 'old-mini', [], status: 'revoked'),
        ],
      );
      agents.extra = (req) async {
        final path = req.url.path;
        if (path.endsWith('/remove') || path.endsWith('/reenroll')) {
          if (status != 200) return http.Response('', status);
          if (path.endsWith('/remove')) {
            agents.hosts.removeWhere((h) => h['id'] == 'h9');
            return http.Response('', 204);
          }
          return http.Response(
            jsonEncode({
              'enrollment': 'storm-enroll:v1:http://s:srv:pk:sen_X.secret',
              'expires': '2026-11-12T12:10:00Z',
            }),
            200,
          );
        }
        return null;
      };
      return agents;
    }

    testWidgets('is one dimmed row; Re-enroll shows the string', (
      tester,
    ) async {
      final agents = revoked();
      final c = await open(tester, agents);
      expect(find.text("It can't connect again."), findsOneWidget);
      expect(find.byKey(const Key('cell-h9-codex')), findsNothing);
      expect(textOf(tester, 'host-sub-h9').data, 'Revoked\nRuns as dewansh');
      await tap(tester, find.byKey(const Key('reenroll-host-h9')));
      expect(agents.count('POST', '/v1/agent/hosts/h9/reenroll'), 1);
      expect(find.text('Re-enroll old-mini'), findsOneWidget);
      expect(find.byKey(const Key('enrollment-string')), findsOneWidget);
      await tap(tester, find.text('Done'));
      await disposeShell(tester, c);
    });

    testWidgets('Remove confirms in the row, then removes', (tester) async {
      final agents = revoked();
      final c = await open(tester, agents);
      await tap(tester, find.byKey(const Key('remove-host-h9')));
      expect(find.byType(AlertDialog), findsNothing);
      expect(agents.count('POST', '/v1/agent/hosts/h9/remove'), 0);
      await tap(tester, find.byKey(const Key('remove-host-confirm')));
      expect(agents.count('POST', '/v1/agent/hosts/h9/remove'), 1);
      expect(find.byKey(const Key('host-row-h9')), findsNothing);
      await disposeShell(tester, c);
    });

    for (final status in [404, 405]) {
      testWidgets('an older server ($status) hides both', (tester) async {
        final agents = revoked(status: status);
        final c = await open(tester, agents);
        await tap(tester, find.byKey(const Key('remove-host-h9')));
        await tap(tester, find.byKey(const Key('remove-host-confirm')));
        expect(find.byKey(const Key('remove-host-h9')), findsNothing);
        expect(find.byKey(const Key('reenroll-host-h9')), findsNothing);
        expect(find.text("It can't connect again."), findsOneWidget);
        await disposeShell(tester, c);
      });
    }
  });

  group('the selected host', () {
    FakeAgentServer two() {
      final agents = FakeAgentServer(
        hosts: [
          host('h1', 'build-vm', []),
          host('h2', 'pi-lab', [], account: 'storm-runtime'),
        ],
      );
      agents.extra = (req) async {
        final seg = req.url.pathSegments;
        if (seg.length == 4 && seg[2] == 'hosts') {
          if (req.method == 'PATCH') {
            final name = (jsonDecode(req.body) as Map)['name'];
            agents.hosts.firstWhere((h) => h['id'] == seg[3])['name'] = name;
            return http.Response('{}', 200);
          }
          if (req.method == 'DELETE') {
            agents.hosts.firstWhere((h) => h['id'] == seg[3])
              ..['status'] = 'revoked'
              ..['capabilities'] = null;
            return http.Response('', 204);
          }
        }
        return null;
      };
      return agents;
    }

    testWidgets('is the first, on surface2, and a click selects another', (
      tester,
    ) async {
      final c = await open(tester, two());
      expect(find.byKey(const Key('host-details-h1')), findsOneWidget);
      Color fill(String h) =>
          (tester.widget<Container>(find.byKey(Key('host-row-$h'))).decoration!
                  as BoxDecoration)
              .color!;
      expect(fill('h1'), dark.surface2);
      expect(fill('h2'), dark.surface);
      expect(textOf(tester, 'runs-as-h1').data, 'dewansh');
      await tap(tester, find.byKey(const Key('host-cell-h2')));
      expect(find.byKey(const Key('host-details-h2')), findsOneWidget);
      expect(textOf(tester, 'runs-as-h2').data, 'storm-runtime');
      expect(fill('h2'), dark.surface2);
      await disposeShell(tester, c);
    });

    testWidgets('Rename edits in place, with no dialog', (tester) async {
      final agents = two();
      final c = await open(tester, agents);
      await tap(tester, find.byKey(const Key('rename-host-h1')));
      expect(find.byType(AlertDialog), findsNothing);
      await tester.enterText(
        find.byKey(const Key('rename-host-field')),
        'build-box',
      );
      await tap(tester, find.byKey(const Key('rename-host-save')));
      expect(agents.count('PATCH', '/v1/agent/hosts/h1'), 1);
      expect(find.text('build-box'), findsWidgets);
      await disposeShell(tester, c);
    });

    testWidgets('Revoke… confirms in a strip, with no dialog', (tester) async {
      final agents = two();
      final c = await open(tester, agents);
      await tap(tester, find.byKey(const Key('revoke-host-h1')));
      expect(find.byType(AlertDialog), findsNothing);
      expect(find.byKey(const Key('revoke-strip')), findsOneWidget);
      await tap(tester, find.byKey(const Key('revoke-host-cancel')));
      expect(agents.count('DELETE', '/v1/agent/hosts/h1'), 0);
      await tap(tester, find.byKey(const Key('revoke-host-h1')));
      await tap(tester, find.byKey(const Key('revoke-host-confirm')));
      expect(agents.count('DELETE', '/v1/agent/hosts/h1'), 1);
      expect(find.text("It can't connect again."), findsOneWidget);
      expect(find.byKey(const Key('host-details-h2')), findsOneWidget);
      await disposeShell(tester, c);
    });

    testWidgets('limit labels never wrap', (tester) async {
      final c = await open(tester, two());
      for (final id in ['children', 'tree', 'sessions']) {
        final label = find.descendant(
          of: find.byKey(Key('limit-$id-stepper')),
          matching: find.byType(Text),
        );
        final t = tester.widget<Text>(label.first);
        expect((t.maxLines, t.softWrap), (1, false), reason: id);
      }
      expect(find.text('Agents per session'), findsOneWidget);
      expect(find.text('Per tree'), findsOneWidget);
      expect(find.text('Sessions at once'), findsOneWidget);
      await disposeShell(tester, c);
    });
  });

  group('Remove on a live host', () {
    FakeAgentServer live({int removeStatus = 204, bool sessions = false}) {
      final agents = FakeAgentServer(
        hosts: [
          host('h1', 'build-vm', []),
          host(
            'h2',
            'mac-mini',
            [],
            status: 'offline',
            lastSeen: DateTime.now().toUtc().toIso8601String(),
          ),
          host('h9', 'old-mini', [], status: 'revoked'),
        ],
        sessions: [
          if (sessions) ...[
            agentSession('s1', hostId: 'h1'),
            agentSession('s2', hostId: 'h1', status: 'starting'),
            agentSession('s3', hostId: 'h1', status: 'completed'),
            agentSession('s4', hostId: 'h2'),
          ],
        ],
      );
      agents.extra = (req) async {
        final seg = req.url.pathSegments;
        if (seg.length < 4 || seg[2] != 'hosts') return null;
        final id = seg[3];
        if (seg.length == 4 && req.method == 'DELETE') {
          agents.hosts.firstWhere((h) => h['id'] == id)
            ..['status'] = 'revoked'
            ..['capabilities'] = null;
          return http.Response('', 204);
        }
        if (seg.length == 5 && seg[4] == 'remove') {
          if (removeStatus != 204) {
            return http.Response(
              jsonEncode({'error': 'the disk is full', 'code': 'io'}),
              removeStatus,
            );
          }
          agents.hosts.removeWhere((h) => h['id'] == id);
          return http.Response('', 204);
        }
        return null;
      };
      return agents;
    }

    List<String> hostCalls(FakeAgentServer agents) => [
      for (final r in agents.requests)
        if (r.method != 'GET' && r.url.path.startsWith('/v1/agent/hosts/'))
          '${r.method} ${r.url.path}',
    ];

    testWidgets('shows for online, offline and revoked hosts', (tester) async {
      final c = await open(tester, live());
      expect(find.byKey(const Key('remove-host-h1')), findsOneWidget);
      expect(find.byKey(const Key('remove-host-h9')), findsOneWidget);
      expect(find.byKey(const Key('reenroll-host-h9')), findsOneWidget);
      await tap(tester, find.byKey(const Key('host-cell-h2')));
      expect(find.byKey(const Key('remove-host-h2')), findsOneWidget);
      final revoke = tester.getTopLeft(find.byKey(const Key('revoke-host-h2')));
      final remove = tester.getTopLeft(find.byKey(const Key('remove-host-h2')));
      expect(remove.dx, greaterThan(revoke.dx), reason: 'after Revoke…');
      await disposeShell(tester, c);
    });

    testWidgets('confirms in place with the copy, and no dialog', (
      tester,
    ) async {
      final agents = live();
      final c = await open(tester, agents);
      await tap(tester, find.byKey(const Key('remove-host-h1')));
      expect(find.byType(AlertDialog), findsNothing);
      expect(find.byKey(const Key('remove-live-strip')), findsOneWidget);
      expect(textOf(tester, 'remove-live-title').data, 'Remove build-vm?');
      expect(
        textOf(tester, 'remove-live-body').data,
        "It's revoked if it isn't already, then taken off this list. Its "
        'ended sessions keep its name. To use the machine again, enroll it '
        'as a new host.',
      );
      expect(find.byKey(const Key('remove-live-running')), findsNothing);
      expect(find.text('Remove host'), findsOneWidget);
      await tap(tester, find.byKey(const Key('remove-live-cancel')));
      expect(hostCalls(agents), isEmpty);
      await disposeShell(tester, c);
    });

    testWidgets('counts the running sessions it disconnects', (tester) async {
      final c = await open(tester, live(sessions: true));
      await tap(tester, find.byKey(const Key('remove-host-h1')));
      expect(
        textOf(tester, 'remove-live-running').data,
        '2 sessions on it are running; they stop being reachable.',
      );
      await disposeShell(tester, c);
    });

    testWidgets('revokes, then removes', (tester) async {
      final agents = live();
      final c = await open(tester, agents);
      await tap(tester, find.byKey(const Key('remove-host-h1')));
      await tap(tester, find.byKey(const Key('remove-live-confirm')));
      expect(hostCalls(agents), [
        'DELETE /v1/agent/hosts/h1',
        'POST /v1/agent/hosts/h1/remove',
      ]);
      expect(find.byKey(const Key('host-row-h1')), findsNothing);
      await disposeShell(tester, c);
    });

    testWidgets('a revoked host is only removed', (tester) async {
      final agents = live();
      final c = await open(tester, agents);
      await tap(tester, find.byKey(const Key('remove-host-h9')));
      await tap(tester, find.byKey(const Key('remove-host-confirm')));
      expect(hostCalls(agents), ['POST /v1/agent/hosts/h9/remove']);
      expect(find.byKey(const Key('host-row-h9')), findsNothing);
      await disposeShell(tester, c);
    });

    testWidgets('a refused remove leaves it revoked, with a toast', (
      tester,
    ) async {
      final agents = live(removeStatus: 500);
      final c = await open(tester, agents);
      await tap(tester, find.byKey(const Key('remove-host-h1')));
      await tap(tester, find.byKey(const Key('remove-live-confirm')));
      expect(hostCalls(agents), [
        'DELETE /v1/agent/hosts/h1',
        'POST /v1/agent/hosts/h1/remove',
      ]);
      expect(
        find.textContaining("Revoked build-vm, but couldn't remove it: "),
        findsOneWidget,
      );
      expect(find.byKey(const Key('host-row-h1')), findsOneWidget);
      expect(find.byKey(const Key('remove-host-h1')), findsOneWidget);
      expect(find.byKey(const Key('reenroll-host-h1')), findsOneWidget);
      await disposeShell(tester, c);
    });

    for (final status in [404, 405]) {
      testWidgets('an older server ($status) hides Remove after', (
        tester,
      ) async {
        final agents = live(removeStatus: status);
        final c = await open(tester, agents);
        await tap(tester, find.byKey(const Key('remove-host-h1')));
        await tap(tester, find.byKey(const Key('remove-live-confirm')));
        expect(find.byKey(const Key('remove-host-h1')), findsNothing);
        expect(find.byKey(const Key('remove-host-h9')), findsNothing);
        expect(find.byKey(const Key('remove-host-h2')), findsNothing);
        expect(find.textContaining('too old to remove hosts'), findsOneWidget);
        await disposeShell(tester, c);
      });
    }
  });

  testWidgets('no hosts is one dashed card with Enroll a host', (tester) async {
    final c = await open(tester, FakeAgentServer());
    expect(find.byKey(const Key('hosts-empty')), findsOneWidget);
    expect(find.byKey(const Key('agents-grid')), findsNothing);
    expect(find.text('No hosts yet'), findsOneWidget);
    expect(find.byKey(const Key('enroll-host-empty')), findsOneWidget);
    expect(find.byKey(const Key('enroll-host')), findsOneWidget);
    await disposeShell(tester, c);
  });

  testWidgets('at 900 the host column stays and the agents scroll', (
    tester,
  ) async {
    final c = await open(tester, everyState(), size: const Size(900, 800));
    final scroll = tester.widget<SingleChildScrollView>(
      find.byKey(const Key('agents-grid-scroll')),
    );
    final position = scroll.controller!.position;
    expect(position.maxScrollExtent, greaterThan(0));
    // The type is the designed type, not shrunk to fit.
    expect(textOf(tester, 'cell-word-h1-codex').style!.fontSize, dark.codeSize);
    final hostAt = tester.getTopLeft(find.byKey(const Key('host-cell-h1')));
    final cellAt = tester.getTopLeft(
      find.byKey(const Key('cell-h1-command-code')),
    );
    await tester.drag(
      find.byKey(const Key('agents-grid-scroll')),
      const Offset(-300, 0),
    );
    await tester.pumpAndSettle();
    expect(position.pixels, greaterThan(0));
    expect(
      tester.getTopLeft(find.byKey(const Key('host-cell-h1'))),
      hostAt,
      reason: 'pinned',
    );
    expect(
      tester.getTopLeft(find.byKey(const Key('cell-h1-command-code'))).dx,
      lessThan(cellAt.dx),
    );
    await disposeShell(tester, c);
  });

  testWidgets('at desk width nothing scrolls', (tester) async {
    final c = await open(tester, everyState());
    final scroll = tester.widget<SingleChildScrollView>(
      find.byKey(const Key('agents-grid-scroll')),
    );
    expect(scroll.controller!.position.maxScrollExtent, 0);
    await disposeShell(tester, c);
  });

  testWidgets('phone: a list per host, and a row opens its sheet', (
    tester,
  ) async {
    final agents = everyState();
    final c = await open(tester, agents, size: phone);
    expect(find.byKey(const Key('agents-grid')), findsNothing);
    expect(find.byKey(const Key('host-section-h1')), findsOneWidget);
    for (final a in kInstallableAgents) {
      expect(find.byKey(Key('agent-row-h1-$a')), findsOneWidget);
    }
    expect(word(tester, 'h1', 'opencode'), 'Sign in');
    expect(subOf(tester, 'h1', 'opencode'), '0.15.2');
    await tap(tester, find.byKey(const Key('agent-row-h1-codex')));
    expect(find.byKey(const Key('cell-panel')), findsOneWidget);
    expect(meaning(tester), startsWith('Not installed.'));
    await tap(tester, find.byKey(const Key('cell-fix')));
    expect(agents.agentJobStarts, ['h1/codex:install']);
    expect(find.byKey(const Key('make-default')), findsOneWidget);
    expect(tester.takeException(), isNull);
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
    await openCell(tester, 'h1', 'codex');
    await tap(tester, find.byKey(const Key('cell-fix')));
    await tester.tapAt(const Offset(1270, 790));
    await tester.pumpAndSettle();
    c.read(routerProvider).go(Routes.settingsPage('storage'));
    await tester.pumpAndSettle();
    final polls = agents.requests.where((r) => r.url.path.contains('/jobs/'));
    final before = polls.length;
    await tester.pump(const Duration(seconds: 3));
    expect(polls.length, before);
    await disposeShell(tester, c);
  });
}
