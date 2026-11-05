import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:shared_preferences/shared_preferences.dart';

import 'package:storm/agent/agent_models.dart';
import 'package:storm/agent/terminal_surface.dart';
import 'package:storm/router.dart';

import 'agent_fakes.dart';
import 'shell_harness.dart';

/// Decision 147 in the client: Tab and Shift+Tab reach the agent from
/// Storm's terminal, with Escape-then-Tab as the way out (147b); the touch
/// key bar's Tab and ⇧Tab; the launcher's Manual (147a); and the request
/// card for auto (147c).
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

  group('A-147-1: Manual is the first and default choice', () {
    testWidgets(
      'the launcher shows Manual (asks) first and sends nothing for it',
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
            matching: find.text('Manual (asks)'),
          ),
          findsOneWidget,
        );
        expect(permissionModesOf('claude-code').first, 'default');
        expect(permissionModeLabel('default'), 'Manual (asks)');
        await t.tap(find.byKey(const Key('launch')));
        await t.pumpAndSettle();
        expect(agents.launches.single.containsKey('permission_mode'), isFalse);
        await disposeShell(t, c);
      },
    );
  });

  group('A-147-4: the request card', () {
    Map<String, dynamic> asking(String id, {String state = 'pending'}) => {
      ...agentSession(id, name: 'fixer'),
      'mode_request': {
        'id': 'amr_1',
        'session_id': id,
        'mode': 'auto',
        'reason': 'Running the <b>test</b> suite asks for every command.',
        'created_at': DateTime.now().toUtc().toIso8601String(),
        'expires_at': DateTime.now()
            .toUtc()
            .add(const Duration(minutes: 10))
            .toIso8601String(),
        'state': state,
      },
    };

    Future<(ProviderContainer, FakeAgentServer, List<Map>)> open(
      WidgetTester t,
      Map<String, dynamic> session, {
      String? switchTo,
    }) async {
      final decisions = <Map>[];
      final agents = FakeAgentServer(hosts: [agentHost()], sessions: [session]);
      agents.extra = (req) async {
        final seg = req.url.pathSegments;
        if (req.method == 'POST' &&
            seg.length == 6 &&
            seg[4] == 'mode-requests') {
          final body = jsonDecode(req.body) as Map;
          decisions.add(body);
          final approve = body['decision'] == 'approve';
          final s = agents.sessions.firstWhere((s) => s['id'] == seg[3]);
          final r = Map<String, dynamic>.from(s['mode_request'] as Map)
            ..['state'] = approve ? 'approved' : 'denied';
          if (approve && switchTo != null) {
            r['switched_to'] = switchTo;
            agents.sessions.add(agentSession(switchTo, name: 'fixer'));
            s['status'] = 'stopped';
          }
          s['mode_request'] = approve ? r : null;
          return http.Response(jsonEncode(r), 200);
        }
        return null;
      };
      final c = shellContainer(agentClient: agents.client);
      await pumpShell(t, c, size: const Size(1400, 900));
      c.read(routerProvider).go('/agents/s/${session['id']}');
      await t.pumpAndSettle();
      return (c, agents, decisions);
    }

    testWidgets('one card, the reason as plain data, and the row says so', (
      t,
    ) async {
      final (c, _, _) = await open(t, asking('ags_a'));
      expect(find.byKey(const Key('mode-request-card')), findsOneWidget);
      expect(find.text('fixer asks to switch to Auto'), findsOneWidget);
      expect(
        find.text('Running the <b>test</b> suite asks for every command.'),
        findsOneWidget,
        reason: 'the agent’s words, unrendered',
      );
      expect(find.textContaining('Expires in 10 minutes'), findsOneWidget);
      expect(
        find.textContaining(RegExp('bypass|skip', caseSensitive: false)),
        findsNothing,
      );
      c.read(routerProvider).go(Routes.agents);
      await t.pumpAndSettle();
      expect(find.byKey(const Key('mode-ask-tag')), findsWidgets);
      await disposeShell(t, c);
    });

    testWidgets('Deny answers it and the card goes', (t) async {
      final (c, _, decisions) = await open(t, asking('ags_d'));
      await t.tap(find.byKey(const Key('mode-request-deny')));
      await t.pumpAndSettle();
      expect(decisions, [
        {'decision': 'deny'},
      ]);
      expect(find.byKey(const Key('mode-request-card')), findsNothing);
      await disposeShell(t, c);
    });

    testWidgets('Approve answers it and follows the switch', (t) async {
      final (c, _, decisions) = await open(
        t,
        asking('ags_old'),
        switchTo: 'ags_new',
      );
      await t.tap(find.byKey(const Key('mode-request-approve')));
      await t.pump(const Duration(seconds: 2));
      await t.pumpAndSettle();
      expect(decisions, [
        {'decision': 'approve'},
      ]);
      expect(
        c.read(routerProvider).state.uri.path,
        '/agents/s/ags_new',
        reason: 'the approving device follows the session into auto',
      );
      await disposeShell(t, c);
    });

    testWidgets('an ended session shows no card to answer', (t) async {
      final ended = asking('ags_e')..['status'] = 'stopped';
      final (c, _, _) = await open(t, ended);
      expect(find.byKey(const Key('mode-request-approve')), findsNothing);
      await disposeShell(t, c);
    });
  });
}
