import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:storm/ui/decision_strip.dart';
import 'package:storm/ui/note_header.dart';
import 'package:storm/ui/note_properties.dart';
import 'package:storm/ui/panels.dart';
import 'package:storm/ui/tags.dart';
import 'package:storm/ui/theme.dart';

Widget _host(Widget child) => MaterialApp(
  theme: StormTheme.dark(),
  home: Scaffold(
    body: Align(alignment: Alignment.topLeft, child: child),
  ),
);

Future<void> _activate(WidgetTester tester) async {
  await tester.sendKeyEvent(LogicalKeyboardKey.tab);
  await tester.sendKeyEvent(LogicalKeyboardKey.enter);
  await tester.pump();
}

void main() {
  testWidgets('KeyValueList.widgets lays out text and widget values', (
    tester,
  ) async {
    await tester.pumpWidget(
      _host(
        KeyValueList.widgets(
          bodies: [
            ('Crew', const Text('Core')),
            ('Role', TextButton(onPressed: () {}, child: const Text('Edit'))),
          ],
        ),
      ),
    );
    expect(find.text('CREW'), findsOneWidget);
    expect(find.text('ROLE'), findsOneWidget);
    expect(find.text('Edit'), findsOneWidget);
  });

  testWidgets('a value chip is an OutlineTag whose remove is keyboard-ready', (
    tester,
  ) async {
    var removed = 0;
    await tester.pumpWidget(
      _host(ValueChip(label: 'alpha', onRemove: () => removed++)),
    );
    expect(find.byType(OutlineTag), findsOneWidget);
    await _activate(tester);
    expect(removed, 1);
  });

  testWidgets('the drawer toggle opens from the keyboard', (tester) async {
    var taps = 0;
    await tester.pumpWidget(
      _host(DrawerToggle(open: false, onTap: () => taps++)),
    );
    await _activate(tester);
    expect(taps, 1);
  });

  testWidgets('the phone strip reviews from the keyboard', (tester) async {
    var reviewed = 0;
    await tester.pumpWidget(
      _host(
        CompactDecisionStrip(
          label: 'NEEDS YOU',
          title: 'Delete a note',
          onReview: () => reviewed++,
        ),
      ),
    );
    await _activate(tester);
    expect(reviewed, 1);
  });
}
