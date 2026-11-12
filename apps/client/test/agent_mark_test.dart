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
    for (final p in [
      'claude-code',
      'opencode',
      'codex',
      'cursor',
      'command-code',
    ]) {
      await pump(tester, session(provider: p));
      final svg = tester.widget<SvgPicture>(find.byType(SvgPicture));
      expect(
        (svg.bytesLoader as SvgAssetLoader).assetName,
        'assets/agents/$p.svg',
      );
    }
    await pump(tester, session(provider: 'shell'));
    expect(find.byIcon(LucideIcons.square_terminal), findsOneWidget);
    await pump(tester, session(provider: 'mystery'));
    expect(find.byIcon(LucideIcons.bot), findsOneWidget);
  });

  BoxDecoration badgeBox(WidgetTester tester) =>
      tester
              .widget<Container>(
                find.descendant(
                  of: find.byKey(const Key('badge-ags_1')),
                  matching: find.byType(Container),
                ),
              )
              .decoration!
          as BoxDecoration;

  testWidgets('working pulses in accent; idle is an accent ring (153e)', (
    tester,
  ) async {
    final t = await pump(tester, session(activity: 'working'));
    expect(badgeColor(tester), t.accent);
    await tester.pump(const Duration(milliseconds: 550));
    expect(badgeColor(tester), isNot(t.accent), reason: 'it pulses');

    for (final idle in [session(activity: 'idle'), session()]) {
      await pump(tester, idle);
      final box = badgeBox(tester);
      expect((box.border! as Border).top.color, t.accent, reason: 'a ring');
      expect(box.color, isNot(t.accent), reason: 'not filled');
      await tester.pump(const Duration(milliseconds: 550));
      expect((badgeBox(tester).border! as Border).top.color, t.accent);
    }
  });

  testWidgets('starting is filled text3', (tester) async {
    final t = await pump(tester, session(status: 'starting'));
    expect(badgeColor(tester), t.text3);
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

  testWidgets('parked, out of usage and unknown have none; never amber', (
    tester,
  ) async {
    await pump(tester, session(status: 'unknown'));
    expect(find.byKey(const Key('badge-ags_1')), findsNothing);
    await pump(tester, session(status: 'parked'));
    expect(find.byKey(const Key('badge-ags_1')), findsNothing);
    final usage = AgentSession.fromJson({
      ...{
        'id': 'ags_1',
        'host_id': 'hst_1',
        'workspace': 'storm',
        'provider': 'claude-code',
        'status': 'running',
        'cols': 80,
        'rows': 24,
        'created_at': '2026-10-03T12:00:00Z',
      },
      'needs_attention': 'usage_limit',
    });
    await pump(tester, usage);
    expect(find.byKey(const Key('badge-ags_1')), findsNothing);
  });

  testWidgets('failed and ended', (tester) async {
    final t = await pump(tester, session(status: 'failed'));
    expect(badgeColor(tester), t.danger);
    expect(opacity(tester), 0.5);
    await pump(tester, session(status: 'completed'));
    expect(find.byKey(const Key('badge-ags_1')), findsNothing);
    expect(opacity(tester), 0.5);
    await pump(tester, session());
    expect(opacity(tester), 1);
  });
}
