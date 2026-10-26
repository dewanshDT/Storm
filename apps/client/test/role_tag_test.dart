import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart' show RenderParagraph;
import 'package:flutter_test/flutter_test.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'package:storm/agent/agent_models.dart';
import 'package:storm/agent/agent_state.dart';
import 'package:storm/agent/crew_widgets.dart';
import 'package:storm/router.dart';
import 'package:storm/ui/theme.dart';

import 'agent_fakes.dart';
import 'real_fonts.dart';
import 'shell_harness.dart';

/// The role tag (decision 138d, 138l; handoff §3): one widget everywhere,
/// neutral, about 14ch at most, giving way before the name, struck through
/// once its note is deleted, and following a rename.
void main() {
  setUpAll(loadRealFonts);
  setUp(() => SharedPreferences.setMockInitialValues({}));

  const style = TextStyle(fontFamily: 'IBMPlexSans', fontSize: 13);
  const reviewer = RoleRef(
    vaultId: 'v-kit',
    noteId: 'k_rev',
    title: 'Storm Reviewer',
  );

  Future<void> pumpIn(WidgetTester t, Widget child, {double width = 400}) =>
      t.pumpWidget(
        MaterialApp(
          theme: StormTheme.dark(),
          home: Scaffold(
            body: Align(
              alignment: Alignment.topLeft,
              child: SizedBox(
                width: width,
                child: Align(alignment: Alignment.topLeft, child: child),
              ),
            ),
          ),
        ),
      );

  double width(WidgetTester t, Finder f) => t.getSize(f).width;
  final tag = find.byKey(const Key('role-tag-k_rev'));
  final mono = find.descendant(of: tag, matching: find.byType(Text));

  /// One `ch` of the tag's face, as drawn (with the ambient spacing).
  double ch(WidgetTester t) {
    final drawn = DefaultTextStyle.of(
      t.element(mono),
    ).style.merge(t.widget<Text>(mono).style);
    final p = TextPainter(
      text: TextSpan(text: '0', style: drawn),
      textDirection: TextDirection.ltr,
    )..layout();
    return p.width;
  }

  testWidgets('lower case, outlined, never coloured by its note', (t) async {
    await pumpIn(t, const RoleTag(role: reviewer));
    expect(find.text('storm reviewer'), findsOneWidget);
    final box = t.widget<Container>(tag).decoration! as BoxDecoration;
    expect(box.color, isNull);
    expect(box.border, isNotNull);
  });

  testWidgets('caps at 15ch and keeps 5ch', (t) async {
    const long = RoleRef(
      vaultId: 'v-kit',
      noteId: 'k_rev',
      title: 'Storm Reviewer For Every Release Candidate',
    );
    await pumpIn(t, const RoleTag(role: long));
    final c = ch(t);
    // 15ch of text, plus 5px padding a side and the outline.
    expect(width(t, tag), lessThanOrEqualTo(c * 15 + 12.5));
    expect(width(t, tag), greaterThan(c * 14));
    expect(t.renderObject<RenderParagraph>(mono).didExceedMaxLines, isTrue);

    const qa = RoleRef(vaultId: 'v-kit', noteId: 'k_rev', title: 'qa');
    await pumpIn(t, const RoleTag(role: qa));
    expect(width(t, tag), greaterThanOrEqualTo(c * 5));
  });

  testWidgets('shrinks before the name, and the name only after', (t) async {
    final name = find.text('gateway-impl');
    Widget row() =>
        const RoleTagRow(name: 'gateway-impl', role: reviewer, style: style);
    await pumpIn(t, row());
    final natural = width(t, tag);
    final nameNatural = width(t, name);

    await pumpIn(t, row(), width: nameNatural + 6 + natural - 30);
    expect(width(t, name), closeTo(nameNatural, 1), reason: 'name kept');
    expect(width(t, tag), lessThan(natural - 20));
    expect(t.renderObject<RenderParagraph>(mono).didExceedMaxLines, isTrue);

    final c = ch(t);
    await pumpIn(t, row(), width: nameNatural * 0.5 + 6 + c * 5 + 12);
    expect(width(t, tag), closeTo(c * 5 + 12, 1), reason: 'at its 5ch floor');
    expect(
      t.renderObject<RenderParagraph>(name).didExceedMaxLines,
      isTrue,
      reason: 'then the name gives way',
    );
  });

  testWidgets('a deleted note is struck through at .6', (t) async {
    const gone = RoleRef(vaultId: 'v-kit', noteId: 'k_rev');
    await pumpIn(t, const RoleTag(role: gone, launchRole: reviewer));
    expect(find.text('storm reviewer'), findsOneWidget, reason: 'last title');
    expect(t.widget<Text>(mono).style!.decoration, TextDecoration.lineThrough);
    expect(
      t
          .widget<Opacity>(
            find.ancestor(of: tag, matching: find.byType(Opacity)).first,
          )
          .opacity,
      0.6,
    );

    await pumpIn(t, const RoleTag(role: gone));
    expect(find.text('deleted role'), findsOneWidget);
  });

  testWidgets('a renamed note renames the tag at the next poll', (t) async {
    final agents = FakeAgentServer(
      hosts: [agentHost()],
      sessions: [
        agentSession('ags_1', role: roleRef('k_rev', 'Storm Reviewer')),
      ],
    );
    final c = shellContainer(agentClient: agents.client);
    await pumpShell(t, c, size: const Size(1400, 900));
    c.read(routerProvider).go(Routes.agents);
    await t.pumpAndSettle();
    expect(find.text('storm reviewer'), findsNWidgets(2), reason: 'both lists');

    (agents.session('ags_1')!['role'] as Map)['title'] = 'SOP Reviewer';
    c.invalidate(agentOverviewProvider);
    await t.pumpAndSettle();
    expect(find.text('storm reviewer'), findsNothing);
    expect(find.text('sop reviewer'), findsNWidgets(2));

    (agents.session('ags_1')!['role'] as Map)['title'] = null;
    c.invalidate(agentOverviewProvider);
    await t.pumpAndSettle();
    expect(find.text('deleted role'), findsNWidgets(2));
    await disposeShell(t, c);
  });

  testWidgets('a loose session carries a role', (t) async {
    final agents = FakeAgentServer(
      hosts: [agentHost()],
      sessions: [
        agentSession('ags_1', role: roleRef('k_rev', 'Storm Reviewer')),
      ],
    );
    final c = shellContainer(agentClient: agents.client);
    await pumpShell(t, c, size: const Size(1400, 900));
    c.read(routerProvider).go(Routes.agents);
    await t.pumpAndSettle();
    expect(
      find.descendant(
        of: find.byKey(const Key('work-ags_1')),
        matching: find.text('storm reviewer'),
      ),
      findsOneWidget,
    );
    expect(find.byKey(const Key('crews-section')), findsNothing);
    await disposeShell(t, c);
  });
}
