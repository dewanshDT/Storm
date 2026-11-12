import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'package:storm/agent/agent_state.dart';
import 'package:storm/agent/spawn_widgets.dart' show LineageIndent;
import 'package:storm/router.dart';
import 'package:storm/ui/shell/corner_bubbles.dart' show PlacesBubble;
import 'package:storm/ui/tokens.dart';

import 'agent_fakes.dart';
import 'fake_server.dart';
import 'shell_harness.dart';

/// Decision 153o (R2 §7): the needs-you count on the place bubble and in the
/// place picker, and the phone Agents list in the sidebar's order, held
/// while it is touched or scrolled, with ENDED folded and no drag.
void main() {
  const phone = Size(411, 900);

  setUp(() => SharedPreferences.setMockInitialValues({}));

  Map<String, dynamic> session(
    String id, {
    String status = 'running',
    String? attention,
    String? parent,
    (String, int)? crew,
  }) => {
    ...agentSession(id, name: id, status: status, crew: crew),
    'needs_attention': ?attention,
    if (parent != null) ...{'parent_session': parent, 'depth': 1},
  };

  Future<ProviderContainer> open(
    WidgetTester t,
    FakeAgentServer agents, {
    String at = Routes.agents,
  }) async {
    final c = shellContainer(agentClient: agents.client);
    await pumpShell(t, c, size: phone);
    c.read(routerProvider).go(at);
    await t.pumpAndSettle();
    return c;
  }

  Finder row(String id) => find.byKey(Key('session-$id'));
  double top(WidgetTester t, String id) => t.getTopLeft(row(id)).dy;
  StormTokens tokens(WidgetTester t) =>
      t.element(find.byType(PlacesBubble)).tokens;

  group('the place bubble counts what needs you', () {
    testWidgets('in danger, on Agents and in a vault', (t) async {
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        sessions: [
          session('ags_a', attention: 'permission_prompt'),
          session('ags_b', attention: 'delete_approval'),
          session('ags_c', status: 'failed'),
          session('ags_d', attention: 'blocked'),
        ],
      );
      final c = await open(t, agents);
      final badge = find.byKey(const Key('places-needs-count'));
      expect(find.descendant(of: badge, matching: find.text('2')), findsOne);
      final box =
          t
                  .widget<Container>(
                    find.descendant(
                      of: badge,
                      matching: find.byType(Container),
                    ),
                  )
                  .decoration!
              as BoxDecoration;
      expect(box.color, tokens(t).danger);
      expect(find.byTooltip('Places, 2 need you'), findsOneWidget);

      // In a vault too: it counts sessions, wherever you are.
      c.read(routerProvider).go(Routes.browse(FakeServer.primaryVault));
      await t.pumpAndSettle();
      expect(badge, findsOneWidget);
      await disposeShell(t, c);
    });

    testWidgets('none: the live dot stays, and no count', (t) async {
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        sessions: [session('ags_a')],
      );
      final c = await open(t, agents);
      expect(find.byKey(const Key('places-needs-count')), findsNothing);
      expect(find.byTooltip('Places'), findsOneWidget);
      final dot = t.widget<Container>(
        find
            .descendant(
              of: find.byType(PlacesBubble),
              matching: find.byType(Container),
            )
            .last,
      );
      expect((dot.decoration! as BoxDecoration).color, tokens(t).accent);

      agents.session('ags_a')!['needs_attention'] = 'question';
      c.invalidate(agentOverviewProvider);
      await t.pumpAndSettle();
      expect(find.byKey(const Key('places-needs-count')), findsOneWidget);
      await disposeShell(t, c);
    });
  });

  group('the place picker', () {
    Text metaText(WidgetTester t) =>
        t.widget<Text>(find.byKey(const Key('place-agents-meta')));

    late ProviderContainer c;

    Future<TextSpan> meta(WidgetTester t, FakeAgentServer agents) async {
      c = await open(t, agents);
      await t.tap(find.byKey(const Key('places-bubble')));
      await t.pumpAndSettle();
      return t
              .widget<Text>(find.byKey(const Key('place-agents-meta')))
              .textSpan!
          as TextSpan;
    }

    testWidgets('"N running · M need you", M in danger', (t) async {
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        sessions: [
          session('ags_a', attention: 'permission_prompt'),
          session('ags_b', attention: 'question'),
          session('ags_c'),
        ],
      );
      final span = await meta(t, agents);
      expect(span.toPlainText(), '3 running · 2 need you');
      final tk = tokens(t);
      final needs = span.children!.last as TextSpan;
      expect(needs.text, '2 need you');
      expect(needs.style!.color, tk.danger);
      expect((span.children!.first as TextSpan).style?.color, isNull);
      expect(metaText(t).style!.color, tk.accent);
      await disposeShell(t, c);
    });

    testWidgets('one needs you', (t) async {
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        sessions: [session('ags_a', attention: 'permission_prompt')],
      );
      expect((await meta(t, agents)).toPlainText(), '1 running · 1 needs you');
      await disposeShell(t, c);
    });

    testWidgets('none: running only', (t) async {
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        sessions: [session('ags_a'), session('ags_b')],
      );
      expect((await meta(t, agents)).toPlainText(), '2 running');
      await disposeShell(t, c);
    });
  });

  group('the phone list sorts as the sidebar does', () {
    testWidgets('urgent rows lead RUNNING; the rest keep their order', (
      t,
    ) async {
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        sessions: [
          session('ags_a'),
          session('ags_b', attention: 'usage_limit'),
          session('ags_c', attention: 'blocked'),
          session('ags_d', attention: 'delete_approval'),
        ],
      );
      final c = await open(t, agents);
      final natural = [
        for (final s in c.read(agentOverviewProvider).value!.looseLive)
          if (s.id == 'ags_a' || s.id == 'ags_b') s.id,
      ];
      expect(top(t, 'ags_d'), lessThan(top(t, 'ags_c')));
      expect(top(t, 'ags_c'), lessThan(top(t, natural.first)));
      expect(top(t, natural.first), lessThan(top(t, natural.last)));
      await disposeShell(t, c);
    });

    testWidgets('crews by their most urgent member, rolled up when folded', (
      t,
    ) async {
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        crews: [
          agentCrew('crw_1', name: 'Relay'),
          agentCrew('crw_2', name: 'Gateway', order: 1),
        ],
        sessions: [
          session('ags_r', crew: ('crw_1', 0)),
          session('ags_g1', crew: ('crw_2', 0)),
          session('ags_g2', crew: ('crw_2', 1), attention: 'permission_prompt'),
        ],
      );
      final c = await open(t, agents);
      double header(String id) => t.getTopLeft(find.byKey(Key('crew-$id'))).dy;
      expect(header('crw_2'), lessThan(header('crw_1')));
      expect(top(t, 'ags_g2'), lessThan(top(t, 'ags_g1')));

      await t.tap(find.byKey(const Key('crew-chevron-crw_2')));
      await t.pumpAndSettle();
      final count = t.widget<Text>(find.byKey(const Key('crew-count-crw_2')));
      expect(count.data, '1 needs you');
      expect(count.style!.color, tokens(t).danger);
      final edge =
          (t
                      .widget<Material>(
                        find
                            .ancestor(
                              of: find.byKey(const Key('crew-crw_2')),
                              matching: find.byType(Material),
                            )
                            .first,
                      )
                      .shape!
                  as RoundedRectangleBorder)
              .side;
      expect(edge.color, tokens(t).danger);
      await disposeShell(t, c);
    });

    testWidgets('a tree moves as one and folds with a roll-up', (t) async {
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        sessions: [
          session('ags_x'),
          session('ags_p'),
          session('ags_c', parent: 'ags_p', attention: 'permission_prompt'),
          session('ags_y'),
        ],
      );
      final c = await open(t, agents);
      expect(top(t, 'ags_p'), lessThan(top(t, 'ags_c')));
      for (final other in ['ags_x', 'ags_y']) {
        expect(top(t, 'ags_c'), lessThan(top(t, other)));
      }
      expect(
        find.ancestor(of: row('ags_c'), matching: find.byType(LineageIndent)),
        findsOneWidget,
      );
      // No hover on a phone: an open tree shows its chevron.
      await t.tap(find.byKey(const Key('tree-chevron-ags_p')));
      await t.pumpAndSettle();
      expect(row('ags_c'), findsNothing);
      expect(
        find.descendant(
          of: find.byKey(const Key('sub-ags_p')),
          matching: find.textContaining('1 needs you'),
        ),
        findsOneWidget,
      );
      await disposeShell(t, c);
    });

    testWidgets('ENDED is folded to its count, with Dismiss all', (t) async {
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        sessions: [
          session('ags_live'),
          session('ags_done', status: 'completed'),
        ],
      );
      final c = await open(t, agents);
      expect(find.text('ENDED · 1'), findsOneWidget);
      expect(row('ags_done'), findsNothing);
      expect(find.byKey(const Key('ended-dismiss-all')), findsOneWidget);
      await t.tap(find.byKey(const Key('ended-toggle')));
      await t.pumpAndSettle();
      expect(row('ags_done'), findsOneWidget);
      await disposeShell(t, c);
    });

    testWidgets('no drag on a phone', (t) async {
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        crews: [agentCrew('crw_1', name: 'Relay')],
        sessions: [
          session('ags_a', crew: ('crw_1', 0)),
          session('ags_b'),
        ],
      );
      final c = await open(t, agents);
      expect(find.byType(Draggable<Object>), findsNothing);
      expect(find.byType(LongPressDraggable<Object>), findsNothing);
      await disposeShell(t, c);
    });
  });

  group('the order holds while the list is touched or scrolled', () {
    FakeAgentServer many() => FakeAgentServer(
      hosts: [agentHost()],
      sessions: [for (var i = 0; i < 16; i++) session('ags_$i')],
    );

    List<String> order(WidgetTester t, List<String> ids) =>
        [...ids]..sort((x, y) => top(t, x).compareTo(top(t, y)));

    testWidgets('a finger down holds it; lifting it applies it', (t) async {
      final agents = many();
      final c = await open(t, agents);
      final shownIds = [
        for (var i = 0; i < 16; i++)
          if (row('ags_$i').evaluate().isNotEmpty) 'ags_$i',
      ];
      final ids = order(t, shownIds);
      final last = ids.last;

      final finger = await t.startGesture(t.getCenter(row(ids.first)));
      await t.pump();
      agents.session(last)!['needs_attention'] = 'permission_prompt';
      c.invalidate(agentOverviewProvider);
      await t.pump(const Duration(milliseconds: 100));
      await t.pump(const Duration(milliseconds: 100));
      expect(order(t, ids).last, last, reason: 'held under the finger');

      await finger.cancel();
      await t.pumpAndSettle();
      expect(order(t, ids).first, last, reason: 'applied on settle');
      await disposeShell(t, c);
    });

    testWidgets('a fling holds it until the list comes to rest', (t) async {
      final agents = many();
      final c = await open(t, agents);
      final ids = order(t, [
        for (var i = 0; i < 16; i++)
          if (row('ags_$i').evaluate().isNotEmpty) 'ags_$i',
      ]);
      final first = ids.first;
      final second = ids[1];

      await t.fling(row(first), const Offset(0, -60), 600);
      await t.pump(const Duration(milliseconds: 16));
      agents.session(second)!['needs_attention'] = 'permission_prompt';
      c.invalidate(agentOverviewProvider);
      await t.pump(const Duration(milliseconds: 16));
      await t.pump(const Duration(milliseconds: 16));
      expect(
        top(t, first),
        lessThan(top(t, second)),
        reason: 'held while it moves',
      );
      await t.pumpAndSettle();
      await t.drag(find.byType(ListView).first, const Offset(0, 600));
      await t.pumpAndSettle();
      expect(top(t, second), lessThan(top(t, first)), reason: 'at rest');
      await disposeShell(t, c);
    });
  });
}
