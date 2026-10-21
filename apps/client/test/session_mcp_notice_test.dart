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

  test('the skipped and local plugin servers are About rows', () {
    final s = session(
      skipped: [
        {
          'name': 'slack',
          'source': 'plugin slack',
          'reason': 'remote: add it in Settings ▸ Integrations',
        },
        {
          'name': 'proxmox',
          'source': 'your own',
          'reason': "your own MCP servers don't load in Storm sessions",
        },
      ],
      local: ['plugin_playwright_playwright'],
    );
    final rows = Map.fromEntries(
      aboutRows(
        s,
        host: 'work-mac',
        writeVault: null,
        integrations: const [],
      ).map((r) => MapEntry(r.$1, r.$2)),
    );
    expect(
      rows['Plugin servers'],
      'plugin_playwright_playwright. They run on work-mac, outside Storm’s '
      'integrations.',
    );
    expect(rows['Not loaded'], contains('slack (plugin slack): remote'));
    expect(rows['Not loaded'], contains('proxmox (your own)'));
    expect(s.mcpSkipped.first.remote, isTrue);
    expect(s.mcpSkipped.last.remote, isFalse);
  });

  test('a session from an older server has neither row', () {
    final rows = aboutRows(
      session(),
      host: 'h',
      writeVault: null,
      integrations: null,
    ).map((r) => r.$1);
    expect(rows, isNot(contains('Plugin servers')));
    expect(rows, isNot(contains('Not loaded')));
  });

  testWidgets('Add to Integrations is offered only for a remote server', (
    tester,
  ) async {
    Future<void> show(AgentSession s) => tester.pumpWidget(
      MaterialApp(
        theme: StormTheme.light(),
        home: Scaffold(body: AddSkippedToIntegrations(session: s)),
      ),
    );
    await show(
      session(
        skipped: [
          {'name': 'slack', 'source': 'plugin slack', 'reason': 'remote: …'},
        ],
      ),
    );
    expect(find.text('Add to Integrations'), findsOneWidget);
    await show(
      session(
        skipped: [
          {'name': 'mine', 'source': 'your own', 'reason': 'your own …'},
        ],
      ),
    );
    expect(find.text('Add to Integrations'), findsNothing);
  });
}
