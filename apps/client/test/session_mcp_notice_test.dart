import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:storm/agent/agent_models.dart';
import 'package:storm/agent/session_screen.dart';
import 'package:storm/ui/theme.dart';

/// A session's MCP servers it did not get, in its details (decision 131a).
void main() {
  AgentSession session({List<Object>? skipped, List<String>? local}) =>
      AgentSession.fromJson({
        'id': 'ags_1',
        'host_id': 'hst_1',
        'workspace': 'storm',
        'provider': 'claude-code',
        'status': 'running',
        'mcp_skipped': ?skipped,
        'mcp_plugin_local': ?local,
      });

  test('a plugin problem is one line in About (153j)', () {
    final s = session(
      skipped: [
        {
          'name': 'figma',
          'source': 'plugin figma',
          'reason': 'it needs its address',
        },
        {
          'name': 'proxmox',
          'source': 'your own',
          'reason': "your own MCP servers don't load in Storm sessions",
        },
      ],
      local: ['plugin_playwright_playwright'],
    );
    expect(pluginProblemLine(s), 'figma isn’t loaded: it needs its address.');
    expect(s.mcpSkipped.last.remote, isFalse);
    // About's rows are who it ran as and who started it, nothing more.
    expect([for (final r in aboutRows(s)) r.$1], ['Ran as', 'Started by']);
  });

  test('a remote plugin server reaches a session only as an integration', () {
    final s = session(
      skipped: [
        {
          'name': 'slack',
          'source': 'plugin slack',
          'reason': 'remote: add it in Settings ▸ Integrations',
        },
      ],
    );
    expect(s.mcpSkipped.single.remote, isTrue);
    expect(
      pluginProblemLine(s),
      'slack isn’t loaded: it reaches a session only as an integration.',
    );
  });

  test('several are counted; none, or only your own, says nothing', () {
    expect(
      pluginProblemLine(
        session(
          skipped: [
            {'name': 'a', 'source': 'plugin a', 'reason': 'x'},
            {'name': 'b', 'source': 'plugin b', 'reason': 'y'},
          ],
        ),
      ),
      '2 plugin servers aren’t loaded: a, b.',
    );
    expect(pluginProblemLine(session()), isNull);
    expect(
      pluginProblemLine(
        session(
          skipped: [
            {'name': 'mine', 'source': 'your own', 'reason': 'your own …'},
          ],
        ),
      ),
      isNull,
    );
  });

  testWidgets('Fix in Integrations is offered with the problem', (
    tester,
  ) async {
    Future<void> show(AgentSession s) => tester.pumpWidget(
      MaterialApp(
        theme: StormTheme.light(),
        home: Scaffold(body: PluginProblem(session: s)),
      ),
    );
    await show(
      session(
        skipped: [
          {'name': 'figma', 'source': 'plugin figma', 'reason': 'remote: …'},
        ],
      ),
    );
    expect(find.text('Fix in Integrations ›'), findsOneWidget);
    await show(
      session(
        skipped: [
          {'name': 'mine', 'source': 'your own', 'reason': 'your own …'},
        ],
      ),
    );
    expect(find.text('Fix in Integrations ›'), findsNothing);
  });
}
