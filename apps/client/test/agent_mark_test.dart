import 'package:flutter/material.dart';
import 'package:flutter_svg/flutter_svg.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:flutter_lucide/flutter_lucide.dart';

import 'package:storm/agent/agent_models.dart';
import 'package:storm/agent/agent_widgets.dart';
import 'package:storm/ui/theme.dart';
import 'package:storm/ui/tokens.dart';

/// D15 AM44: the agent's mark, and its status as a badge on it.
void main() {
  AgentSession session({
    String provider = 'claude-code',
    String status = 'running',
    String? activity,
  }) => AgentSession.fromJson({
    'id': 'ags_1',
    'host_id': 'hst_1',
    'workspace': 'storm',
    'provider': provider,
    'status': status,
    'end_reason': null,
    'exit_code': null,
    'signal': null,
    'cols': 80,
    'rows': 24,
    'created_at': '2026-10-03T12:00:00Z',
    'provider_fallback': null,
    'activity': activity,
  });

  Future<StormTokens> pump(
    WidgetTester tester,
    AgentSession s, {
    bool reduceMotion = false,
  }) async {
    await tester.pumpWidget(
      MaterialApp(
        theme: StormTheme.dark(),
        home: MediaQuery(
          data: MediaQueryData(disableAnimations: reduceMotion),
          child: Scaffold(
            body: Center(child: AgentMark(session: s)),
          ),
        ),
      ),
    );
    await tester.pump();
    return tester.element(find.byType(AgentMark)).tokens;
  }

  Color badgeColor(WidgetTester tester) {
    final box = tester.widget<Container>(
      find.descendant(
        of: find.byKey(const Key('badge-ags_1')),
        matching: find.byType(Container),
      ),
    );
    return (box.decoration! as BoxDecoration).color!;
  }

  double opacity(WidgetTester tester) => tester
      .widget<Opacity>(
        find.descendant(
          of: find.byType(AgentMark),
          matching: find.byType(Opacity),
        ),
      )
      .opacity;

  testWidgets('each agent shows its own mark', (tester) async {
    await pump(tester, session());
    expect(find.byType(SvgPicture), findsOneWidget);
    await pump(tester, session(provider: 'opencode'));
    expect(find.byType(SvgPicture), findsOneWidget);
    await pump(tester, session(provider: 'shell'));
    expect(find.byIcon(LucideIcons.square_terminal), findsOneWidget);
    await pump(tester, session(provider: 'mystery'));
    expect(find.byIcon(LucideIcons.bot), findsOneWidget);
  });

  testWidgets('working pulses in accent; waiting shows no badge', (
    tester,
  ) async {
    final t = await pump(tester, session(activity: 'working'));
    expect(badgeColor(tester), t.accent);
    await tester.pump(const Duration(milliseconds: 550));
    expect(badgeColor(tester), isNot(t.accent), reason: 'it pulses');

    await pump(tester, session(activity: 'idle'));
    expect(find.byKey(const Key('badge-ags_1')), findsNothing);
    await pump(tester, session());
    expect(find.byKey(const Key('badge-ags_1')), findsNothing);
  });

  testWidgets('reduced motion holds the badge still', (tester) async {
    final t = await pump(
      tester,
      session(activity: 'working'),
      reduceMotion: true,
    );
    await tester.pump(const Duration(milliseconds: 550));
    expect(badgeColor(tester), t.accent);
  });

  testWidgets('unknown, failed and ended', (tester) async {
    var t = await pump(tester, session(status: 'unknown'));
    expect(badgeColor(tester), t.amber);
    t = await pump(tester, session(status: 'failed'));
    expect(badgeColor(tester), t.danger);
    expect(opacity(tester), 0.5);
    await pump(tester, session(status: 'completed'));
    expect(find.byKey(const Key('badge-ags_1')), findsNothing);
    expect(opacity(tester), 0.5);
    await pump(tester, session());
    expect(opacity(tester), 1);
  });
}
