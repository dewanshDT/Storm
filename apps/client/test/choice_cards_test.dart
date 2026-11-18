import 'dart:ui' show Tristate;

import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:storm/ui/controls.dart';
import 'package:storm/ui/theme.dart';
import 'package:storm/ui/tokens.dart';

void main() {
  final t = StormTokens.from(StormPreset.stormDark);

  Future<void> pump(WidgetTester tester, Widget child, {double width = 1000}) {
    tester.view.physicalSize = Size(width, 800);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    return tester.pumpWidget(
      MaterialApp(
        theme: StormTheme.dark(),
        home: Scaffold(
          body: Padding(padding: const EdgeInsets.all(20), child: child),
        ),
      ),
    );
  }

  BoxDecoration face(WidgetTester tester, String key) =>
      tester
              .widget<AnimatedContainer>(
                find.descendant(
                  of: find.byKey(Key(key)),
                  matching: find.byType(AnimatedContainer),
                ),
              )
              .decoration!
          as BoxDecoration;

  Widget cards({
    int? selected = 1,
    ValueChanged<int>? onSelected,
    int count = 3,
    double minCardWidth = 220,
  }) => ChoiceCards<int>(
    options: [
      for (var i = 1; i <= count; i++)
        ChoiceCardOption(
          i,
          'Card $i',
          key: Key('card-$i'),
          subtitle: i == 2
              ? 'A subtitle long enough that it would wrap onto a second line'
              : 'Sub $i',
          note: i == 3 ? 'Note' : null,
          enabled: i != 4,
        ),
    ],
    selected: selected,
    onSelected: onSelected,
    minCardWidth: minCardWidth,
  );

  testWidgets('selects by value and marks the selection as ChoiceChips do', (
    tester,
  ) async {
    int? picked;
    await pump(tester, cards(onSelected: (v) => picked = v));
    await tester.tap(find.text('Card 2'));
    expect(picked, 2);

    final on = face(tester, 'card-1');
    expect(on.color, t.accentSoft);
    expect((on.border! as Border).top.color, t.accent);
    expect((on.border! as Border).top.width, t.bw);
    expect(tester.widget<Text>(find.text('Card 1')).style!.color, t.accent);

    final off = face(tester, 'card-2');
    expect(off.color, Colors.transparent);
    expect((off.border! as Border).top.color, t.border);
    expect(tester.widget<Text>(find.text('Card 2')).style!.color, t.text);
    expect(on.borderRadius, BorderRadius.circular(t.rCard));
  });

  testWidgets('hover and press change the fill', (tester) async {
    await pump(tester, cards(onSelected: (_) {}));
    final mouse = await tester.createGesture(kind: PointerDeviceKind.mouse);
    addTearDown(mouse.removePointer);
    await mouse.addPointer(location: Offset.zero);
    await mouse.moveTo(tester.getCenter(find.byKey(const Key('card-2'))));
    await tester.pumpAndSettle();
    expect(face(tester, 'card-2').color, t.surface);

    final press = await tester.startGesture(
      tester.getCenter(find.byKey(const Key('card-2'))),
    );
    await tester.pumpAndSettle();
    expect(face(tester, 'card-2').color, t.surface2);
    await press.up();
    await tester.pumpAndSettle();
    expect(face(tester, 'card-2').color, t.surface);
  });

  testWidgets('keyboard focus shows a ring and Enter or Space activates', (
    tester,
  ) async {
    final picked = <int>[];
    await pump(tester, cards(onSelected: picked.add));
    final ring = find.byKey(const Key('choice-card-focus-ring'));
    expect(ring, findsNothing);

    await tester.sendKeyEvent(LogicalKeyboardKey.tab);
    await tester.pumpAndSettle();
    expect(ring, findsOneWidget);
    expect(
      find.descendant(of: find.byKey(const Key('card-1')), matching: ring),
      findsOneWidget,
    );

    await tester.sendKeyEvent(LogicalKeyboardKey.tab);
    await tester.pumpAndSettle();
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.sendKeyEvent(LogicalKeyboardKey.tab);
    await tester.pumpAndSettle();
    await tester.sendKeyEvent(LogicalKeyboardKey.space);
    expect(picked, [2, 3]);
  });

  testWidgets('a disabled card is dimmed, skipped and does nothing', (
    tester,
  ) async {
    final picked = <int>[];
    await pump(tester, cards(onSelected: picked.add, count: 4));
    await tester.tap(find.text('Card 4'), warnIfMissed: false);
    expect(picked, isEmpty);
    final dim = tester.widget<Opacity>(
      find.descendant(
        of: find.byKey(const Key('card-4')),
        matching: find.byType(Opacity),
      ),
    );
    expect(dim.opacity, lessThan(1));
    final s = tester.getSemantics(find.byKey(const Key('card-4')));
    expect(s.flagsCollection.isEnabled, Tristate.isFalse);
  });

  testWidgets('semantics: a selectable button named by its lines', (
    tester,
  ) async {
    final handle = tester.ensureSemantics();
    await pump(tester, cards(onSelected: (_) {}));
    final one = tester.getSemantics(find.byKey(const Key('card-1')));
    expect(one.label, 'Card 1, Sub 1');
    expect(one.flagsCollection.isButton, isTrue);
    expect(one.flagsCollection.isSelected, Tristate.isTrue);
    final three = tester.getSemantics(find.byKey(const Key('card-3')));
    expect(three.label, 'Card 3, Sub 3, Note');
    expect(three.flagsCollection.isSelected, Tristate.isFalse);
    handle.dispose();
  });

  testWidgets('fills the width in equal columns with equal heights', (
    tester,
  ) async {
    await pump(tester, cards(count: 5), width: 1000);
    final rects = [
      for (var i = 1; i <= 5; i++) tester.getRect(find.byKey(Key('card-$i'))),
    ];
    // (960 + 16) / (220 + 16) = 4 per row.
    expect(rects[4].top, greaterThan(rects[3].bottom));
    expect(rects.map((r) => r.width).toSet(), hasLength(1));
    expect(rects.take(4).map((r) => r.height).toSet(), hasLength(1));
    expect(rects[0].left, 20);
    expect(rects[3].right, moreOrLessEquals(980, epsilon: 0.01));
    expect(tester.widget<Text>(find.textContaining('A subtitle')).maxLines, 1);

    await pump(tester, cards(count: 5), width: 300);
    expect(
      tester.getRect(find.byKey(const Key('card-1'))).width,
      moreOrLessEquals(260, epsilon: 0.01),
    );
  });
}
