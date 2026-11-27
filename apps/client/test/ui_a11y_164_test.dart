import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:storm/ui/buttons.dart';
import 'package:storm/ui/theme.dart';
import 'package:storm/ui/widgets.dart';

Widget _host(Widget child) => MaterialApp(
  theme: StormTheme.dark(),
  home: Scaffold(
    body: Align(alignment: Alignment.topLeft, child: child),
  ),
);

void main() {
  group('StormToggle', () {
    Future<void> pump(
      WidgetTester tester,
      bool Function() get,
      void Function(bool) set,
    ) => tester.pumpWidget(
      _host(
        StatefulBuilder(
          builder: (c, setState) => StormToggle(
            value: get(),
            onChanged: (v) => setState(() => set(v)),
          ),
        ),
      ),
    );

    testWidgets('Tab focuses, Space and Enter toggle, ring shows', (
      tester,
    ) async {
      var on = false;
      await pump(tester, () => on, (v) => on = v);
      await tester.sendKeyEvent(LogicalKeyboardKey.tab);
      await tester.pump();
      expect(find.byKey(const Key('tappable-focus-ring')), findsOneWidget);
      await tester.sendKeyEvent(LogicalKeyboardKey.space);
      await tester.pump();
      expect(on, isTrue);
      await tester.sendKeyEvent(LogicalKeyboardKey.enter);
      await tester.pump();
      expect(on, isFalse);
    });

    testWidgets('announces toggled state', (tester) async {
      final h = tester.ensureSemantics();
      await tester.pumpWidget(
        _host(
          StormToggle(value: true, onChanged: (_) {}, semanticLabel: 'Sync'),
        ),
      );
      expect(
        tester.getSemantics(find.bySemanticsLabel('Sync')),
        matchesSemantics(
          label: 'Sync',
          hasToggledState: true,
          isToggled: true,
          hasEnabledState: true,
          isEnabled: true,
          hasTapAction: true,
        ),
      );
      h.dispose();
    });

    testWidgets('visual size is unchanged but the hit area reaches 40', (
      tester,
    ) async {
      var on = false;
      await pump(tester, () => on, (v) => on = v);
      expect(tester.getSize(find.byType(StormToggle)).height, lessThan(30));
      final c = tester.getCenter(find.byType(StormToggle));
      await tester.tapAt(c + const Offset(0, 19));
      await tester.pump();
      expect(on, isTrue);
    });
  });

  group('StormCheckbox', () {
    testWidgets('Tab focus, Space/Enter, checked semantics, hit area', (
      tester,
    ) async {
      final h = tester.ensureSemantics();
      var on = false;
      await tester.pumpWidget(
        _host(
          StatefulBuilder(
            builder: (c, setState) => StormCheckbox(
              value: on,
              semanticLabel: 'Agree',
              onChanged: (v) => setState(() => on = v),
            ),
          ),
        ),
      );
      await tester.sendKeyEvent(LogicalKeyboardKey.tab);
      await tester.pump();
      expect(find.byKey(const Key('tappable-focus-ring')), findsOneWidget);
      await tester.sendKeyEvent(LogicalKeyboardKey.space);
      await tester.pump();
      expect(on, isTrue);
      expect(
        tester.getSemantics(find.bySemanticsLabel('Agree')),
        matchesSemantics(
          label: 'Agree',
          hasCheckedState: true,
          isChecked: true,
          hasEnabledState: true,
          isEnabled: true,
          hasTapAction: true,
        ),
      );
      await tester.sendKeyEvent(LogicalKeyboardKey.enter);
      await tester.pump();
      expect(on, isFalse);
      expect(tester.getSize(find.byType(StormCheckbox)).height, lessThan(30));
      await tester.tapAt(
        tester.getCenter(find.byType(StormCheckbox)) + const Offset(0, 19),
      );
      await tester.pump();
      expect(on, isTrue);
      h.dispose();
    });

    testWidgets(
      'StormCheckboxRow is one focus stop and toggles from the label',
      (tester) async {
        var on = false;
        await tester.pumpWidget(
          _host(
            SizedBox(
              width: 300,
              child: StatefulBuilder(
                builder: (c, setState) => StormCheckboxRow(
                  value: on,
                  onChanged: (v) => setState(() => on = v),
                  child: const Text('Include diagnostics'),
                ),
              ),
            ),
          ),
        );
        await tester.tap(find.text('Include diagnostics'));
        await tester.pump();
        expect(on, isTrue);
        await tester.sendKeyEvent(LogicalKeyboardKey.tab);
        await tester.sendKeyEvent(LogicalKeyboardKey.space);
        await tester.pump();
        expect(on, isFalse);
      },
    );
  });

  testWidgets('Breadcrumb crumbs are focusable and labelled', (tester) async {
    var taps = 0;
    await tester.pumpWidget(
      _host(
        Breadcrumb(
          crumbs: [
            Crumb('Home', onTap: () => taps++),
            const Crumb('Here'),
          ],
        ),
      ),
    );
    await tester.sendKeyEvent(LogicalKeyboardKey.tab);
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    expect(taps, 1);
  });
}
