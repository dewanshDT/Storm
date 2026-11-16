import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'package:storm/router.dart';
import 'package:storm/state/terminal_prefs.dart';
import 'package:storm/ui/theme.dart';

import 'agent_fakes.dart';
import 'shell_harness.dart';

/// 171-8: a launch says the brightness its terminal will answer with
/// (decision 157c), so the host can set COLORFGBG.
void main() {
  setUp(() => SharedPreferences.setMockInitialValues({}));

  for (final (name, theme, colours, sent) in [
    ('a light app', StormTheme.light(), TerminalColours.matchApp, 'light'),
    ('a dark app', StormTheme.dark(), TerminalColours.matchApp, 'dark'),
    ('Always dark', StormTheme.light(), TerminalColours.alwaysDark, 'dark'),
  ]) {
    testWidgets('$name launches with $sent', (t) async {
      final agents = FakeAgentServer(
        hosts: [
          agentHost(providers: ['claude-code', 'shell']),
        ],
        sessions: [agentSession('ags_1')],
      );
      final c = shellContainer(agentClient: agents.client);
      c
          .read(terminalPrefsProvider.notifier)
          .set(TerminalPrefs(colours: colours));
      await pumpShell(t, c, size: const Size(1400, 900), theme: theme);
      c.read(routerProvider).go(Routes.agents);
      await t.pumpAndSettle();
      await t.tap(find.byKey(const Key('new-session')));
      await t.pumpAndSettle();
      await t.ensureVisible(find.byKey(const Key('launch')));
      await t.pumpAndSettle();
      await t.tap(find.byKey(const Key('launch')));
      await t.pumpAndSettle();
      expect(agents.launches.single['terminal_brightness'], sent);
      await disposeShell(t, c);
    });
  }
}
