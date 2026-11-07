import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'package:storm/agent/agent_models.dart';
import 'package:storm/agent/terminal_surface.dart';
import 'package:storm/router.dart';

import 'agent_fakes.dart';
import 'shell_harness.dart';

/// Decision 147 in the client: Tab and Shift+Tab reach the agent from
/// Storm's terminal, with Escape-then-Tab as the way out (147b), so the
/// person switches modes there; the touch key bar's Tab and ⇧Tab; and the
/// launcher's default, the agent's own setting (151 N5).
void main() {
  setUp(() => SharedPreferences.setMockInitialValues({}));

  group('A-147-2: the touch key bar', () {
    testWidgets('the touch key bar has Tab and ⇧Tab', (t) async {
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        sessions: [agentSession('ags_live')],
      );
      final c = shellContainer(agentClient: agents.client);
      await pumpShell(t, c, size: const Size(411, 900), keyboard: 300);
      c.read(routerProvider).go('/agents/s/ags_live');
      await t.pumpAndSettle();
      expect(find.byKey(const Key('key-tab')), findsOneWidget);
      await t.tap(find.byKey(const Key('key-tab')));
      final row = find.descendant(
        of: find.ancestor(
          of: find.byKey(const Key('key-esc')),
          matching: find.byType(ListView),
        ),
        matching: find.byType(Scrollable),
      );
      await t.scrollUntilVisible(
        find.byKey(const Key('key-backtab')),
        80,
        scrollable: row.first,
      );
      await t.ensureVisible(find.byKey(const Key('key-backtab')));
      await t.pumpAndSettle();
      expect(find.byKey(const Key('key-backtab')), findsOneWidget);
      expect(find.text('⇧Tab'), findsOneWidget);
      await t.tap(find.byKey(const Key('key-backtab')));
      await t.pump(const Duration(milliseconds: 200));
      final typed = [
        for (final r in agents.requests)
          if (r.url.path.endsWith('/terminal/input'))
            latin1.decode(r.bodyBytes),
      ].join();
      expect(typed, '\t\x1b[Z');
      await disposeShell(t, c);
    });

    test('the bar’s sticky Shift makes its Tab a Shift+Tab', () {
      final term = StormTerminal();
      final sent = <String>[];
      term.onInput = (b) => sent.add(latin1.decode(b));
      term.stickyShift = true;
      term.tab();
      term.tab();
      expect(sent, ['\x1b[Z', '\t']);
      term.dispose();
    });
  });

  group('151 N5: the agent\'s own setting is the first and default choice', () {
    testWidgets(
      'the launcher shows the agent\'s own setting first and sends nothing for it',
      (t) async {
        final agents = FakeAgentServer(
          hosts: [
            agentHost(providers: ['claude-code', 'opencode']),
          ],
          sessions: [agentSession('ags_1')],
        );
        final c = shellContainer(agentClient: agents.client);
        await pumpShell(t, c, size: const Size(1400, 900));
        c.read(routerProvider).go(Routes.agents);
        await t.pumpAndSettle();
        await t.tap(find.byKey(const Key('new-session')));
        await t.pumpAndSettle();
        expect(
          find.descendant(
            of: find.byKey(const Key('launcher-permission-mode')),
            matching: find.text("The agent's own setting"),
          ),
          findsOneWidget,
        );
        expect(permissionModesOf('claude-code').first, 'default');
        expect(permissionModeLabel('default'), "The agent's own setting");
        await t.tap(find.byKey(const Key('launch')));
        await t.pumpAndSettle();
        expect(agents.launches.single.containsKey('permission_mode'), isFalse);
        await disposeShell(t, c);
      },
    );
  });
}
