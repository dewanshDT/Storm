import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:storm/agent/agent_models.dart';
import 'package:storm/agent/agent_state.dart';
import 'package:storm/agent/crew_widgets.dart';
import 'package:storm/ui/accents.dart';
import 'package:storm/ui/theme.dart';
import 'package:storm/ui/tokens.dart';

void main() {
  const crew = Crew(id: 'crw_1', name: 'Docs', color: 'sky', order: 0);

  Future<void> pump(WidgetTester t, Widget child) async {
    await t.pumpWidget(
      ProviderScope(
        child: MaterialApp(
          theme: StormTheme.from(StormPreset.stormDark),
          home: Scaffold(
            body: Center(child: SizedBox(width: 400, child: child)),
          ),
        ),
      ),
    );
    await t.pumpAndSettle();
  }

  testWidgets('a crew card, header and swatches render', (t) async {
    await pump(
      t,
      Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          CrewCard(
            crew: crew,
            overview: const AgentOverview(
              sessions: [],
              hosts: [],
              crews: [crew],
            ),
            collapsed: false,
            onOpenSession: (_) {},
          ),
          CrewHeader(
            crew: crew,
            members: const [],
            collapsed: false,
            onToggle: () {},
            onOpen: () {},
            onMenu: (_) {},
          ),
          CrewSwatches(selected: Accent.sky, onSelected: (_) {}),
        ],
      ),
    );
    expect(t.takeException(), isNull);
    expect(find.byKey(const Key('crew-card-crw_1')), findsOneWidget);
    expect(find.byKey(const Key('crew-crw_1')), findsOneWidget);
  });

  testWidgets('Enter on a focused crew header opens the crew', (t) async {
    var opened = 0;
    await pump(
      t,
      CrewHeader(
        crew: crew,
        members: const [],
        collapsed: false,
        onToggle: () {},
        onOpen: () => opened++,
        onMenu: (_) {},
      ),
    );
    await t.sendKeyEvent(LogicalKeyboardKey.tab);
    await t.pump();
    await t.sendKeyEvent(LogicalKeyboardKey.enter);
    await t.pump();
    expect(opened, 1);
  });
}
