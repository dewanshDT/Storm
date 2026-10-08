import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:storm/ui/controls.dart';
import 'package:storm/ui/session_status.dart';
import 'package:storm/ui/shell/sidebar_frame.dart';
import 'package:storm/ui/states.dart';
import 'package:storm/ui/surfaces.dart';
import 'package:storm/ui/theme.dart';
import 'package:storm/ui/tokens.dart';
import 'package:flutter_lucide/flutter_lucide.dart';

void main() {
  final t = StormTokens.from(StormPreset.stormDark);

  Future<void> pump(WidgetTester tester, Widget child, {Size? size}) async {
    if (size != null) {
      tester.view.physicalSize = size;
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.reset);
    }
    await tester.pumpWidget(
      MaterialApp(
        theme: StormTheme.dark(),
        home: Scaffold(body: Center(child: child)),
      ),
    );
  }

  BoxDecoration decorationOf(WidgetTester tester, Finder f) =>
      tester.widget<AnimatedContainer>(f).decoration! as BoxDecoration;

  group('StormToggle', () {
    testWidgets('is 38 × 22 at the default text size and flips on tap', (
      tester,
    ) async {
      var value = false;
      await pump(
        tester,
        StatefulBuilder(
          builder: (context, setState) => StormToggle(
            value: value,
            onChanged: (v) => setState(() => value = v),
          ),
        ),
      );
      expect(
        tester.getSize(find.byType(AnimatedContainer)),
        const Size(38, 22),
      );
      var d = decorationOf(tester, find.byType(AnimatedContainer));
      expect(d.color, t.surface2);

      await tester.tap(find.byType(StormToggle));
      await tester.pumpAndSettle();
      expect(value, isTrue);
      d = decorationOf(tester, find.byType(AnimatedContainer));
      expect(d.color, t.accent);
    });

    testWidgets('does nothing when disabled', (tester) async {
      var taps = 0;
      await pump(
        tester,
        StormToggle(value: false, enabled: false, onChanged: (_) => taps++),
      );
      await tester.tap(find.byType(StormToggle));
      expect(taps, 0);
    });
  });

  group('StormButton', () {
    testWidgets('each kind takes its fill from the tokens', (tester) async {
      Color? fillOf(StormButtonKind kind) {
        final m = tester.widget<Material>(
          find.descendant(
            of: find.byKey(ValueKey(kind)),
            matching: find.byType(Material),
          ),
        );
        return m.color;
      }

      await pump(
        tester,
        Column(
          children: [
            for (final k in StormButtonKind.values)
              StormButton(
                key: ValueKey(k),
                label: k.name,
                kind: k,
                onPressed: () {},
              ),
          ],
        ),
      );
      expect(fillOf(StormButtonKind.primary), t.accent);
      expect(fillOf(StormButtonKind.soft), t.accentSoft);
      expect(fillOf(StormButtonKind.danger), t.danger);
      expect(fillOf(StormButtonKind.outline), Colors.transparent);
    });

    testWidgets('a null onPressed is disabled', (tester) async {
      await pump(
        tester,
        const StormButton.primary(label: 'Go', onPressed: null),
      );
      expect(tester.widget<Opacity>(find.byType(Opacity)).opacity, lessThan(1));
    });
  });

  testWidgets('ChoiceChips selects by value and marks the selection', (
    tester,
  ) async {
    String? picked;
    await pump(
      tester,
      ChoiceChips<int>(
        options: const [ChoiceOption(1, 'One'), ChoiceOption(2, 'Two')],
        selected: 1,
        onSelected: (v) => picked = '$v',
      ),
    );
    await tester.tap(find.text('Two'));
    expect(picked, '2');
    expect(tester.widget<Text>(find.text('One')).style!.color, t.accent);
    expect(tester.widget<Text>(find.text('Two')).style!.color, t.text);
  });

  testWidgets('SettingsRow shows label, sub-line and trailing control', (
    tester,
  ) async {
    var tapped = false;
    await pump(
      tester,
      SettingsRow(
        label: 'Allow writes',
        sub: 'One vault per session.',
        trailing: StormToggle(value: true, onChanged: (_) {}),
        onTap: () => tapped = true,
      ),
    );
    expect(find.text('Allow writes'), findsOneWidget);
    expect(find.text('One vault per session.'), findsOneWidget);
    expect(find.byType(StormToggle), findsOneWidget);
    expect(
      tester.widget<Text>(find.text('Allow writes')).style!.fontSize,
      t.uiSize,
    );
    await tester.tap(find.text('Allow writes'));
    expect(tapped, isTrue);
  });

  group('NumberedSteps', () {
    testWidgets('only the current step is accent, and only it has its action', (
      tester,
    ) async {
      await pump(
        tester,
        NumberedSteps(
          current: 1,
          steps: [
            const NumberedStep('First', action: Text('act-1')),
            const NumberedStep('Second', action: Text('act-2')),
            const NumberedStep('Third'),
          ],
        ),
      );
      Color border(int n) =>
          ((tester.widget<Container>(find.byKey(Key('step-$n'))).decoration!
                          as BoxDecoration)
                      .border!
                  as Border)
              .top
              .color;
      expect(border(1), t.border);
      expect(border(2), t.accent);
      expect(border(3), t.border);
      expect(
        tester.getSize(find.byKey(const Key('step-1'))),
        const Size(26, 26),
      );
      expect(find.text('act-1'), findsNothing);
      expect(find.text('act-2'), findsOneWidget);
    });

    testWidgets('EmptyState carries them', (tester) async {
      await pump(
        tester,
        const EmptyState(
          icon: LucideIcons.server,
          title: 'No host yet',
          steps: [NumberedStep('Enroll a host'), NumberedStep('Start')],
        ),
      );
      expect(find.byType(NumberedSteps), findsOneWidget);
    });
  });

  group('session status', () {
    test('draws the handoff vocabulary', () {
      final cases = {
        'creating': ('Starting', t.accent, true, true),
        'starting': ('Starting', t.accent, true, true),
        'running': ('Running', t.accent, true, false),
        'unknown': ('Unknown', t.text3, true, false),
        'completed': ('Completed', t.text3, false, false),
        'stopped': ('Stopped', t.text3, false, false),
        'failed': ('Failed', t.danger, false, false),
      };
      cases.forEach((status, want) {
        final look = sessionStatusLook(status, t);
        expect(
          (look.label, look.color, look.live, look.ring),
          want,
          reason: status,
        );
      });
    });

    testWidgets('starting is a ring, running a fill', (tester) async {
      await pump(
        tester,
        const Row(
          children: [
            SessionStatusDot(key: Key('s'), status: 'starting'),
            SessionStatusDot(key: Key('r'), status: 'running'),
          ],
        ),
      );
      BoxDecoration d(String k) =>
          tester
                  .widget<Container>(
                    find.descendant(
                      of: find.byKey(Key(k)),
                      matching: find.byType(Container),
                    ),
                  )
                  .decoration!
              as BoxDecoration;
      expect(d('s').color, isNull);
      expect(d('s').border, isNotNull);
      expect(d('r').color, t.accent);
      expect(tester.getSize(find.byKey(const Key('r'))), const Size(8, 8));
    });

    testWidgets('the chip says the label in mono', (tester) async {
      await pump(tester, const SessionStatusChip(status: 'failed'));
      final text = tester.widget<Text>(find.text('Failed'));
      expect(text.style!.fontFamily, StormTokens.monoFamily);
      expect(text.style!.color, t.danger);
    });
  });

  group('placePopover', () {
    const overlay = Size(1280, 800);

    test('drops below an anchor near the top', () {
      final p = placePopover(
        anchor: const Rect.fromLTWH(100, 20, 200, 40),
        overlay: overlay,
        gap: 8,
      );
      expect((p.top, p.bottom, p.left, p.right), (68.0, null, 100.0, null));
    });

    test('grows upward from an anchor near the foot', () {
      final p = placePopover(
        anchor: const Rect.fromLTWH(100, 700, 200, 40),
        overlay: overlay,
        gap: 8,
      );
      expect((p.top, p.bottom), (null, 108.0));
    });

    test('aligns right edges when asked', () {
      final p = placePopover(
        anchor: const Rect.fromLTWH(1000, 20, 200, 40),
        overlay: overlay,
        gap: 8,
        alignRight: true,
      );
      expect((p.left, p.right), (null, 80.0));
    });

    test('opens beside the rail, bottoms aligned when the anchor is low', () {
      final low = placePopover(
        anchor: const Rect.fromLTWH(16, 700, 24, 24),
        overlay: overlay,
        gap: 8,
        side: PopoverSide.right,
      );
      expect((low.top, low.bottom, low.left), (null, 76.0, 48.0));
      final high = placePopover(
        anchor: const Rect.fromLTWH(16, 40, 24, 24),
        overlay: overlay,
        gap: 8,
        side: PopoverSide.right,
      );
      expect((high.top, high.bottom, high.left), (40.0, null, 48.0));
    });
  });

  testWidgets('SidebarFrame is surface, at least 260 wide, footer last', (
    tester,
  ) async {
    await pump(
      tester,
      const SizedBox(
        height: 600,
        child: SidebarFrame(body: Text('body'), footer: Text('footer')),
      ),
      size: const Size(1280, 800),
    );
    final material = tester.widget<Material>(
      find.descendant(
        of: find.byType(SidebarFrame),
        matching: find.byType(Material),
      ),
    );
    expect(material.color, t.surface);
    expect(
      tester.getSize(find.byType(SidebarFrame)).width,
      greaterThanOrEqualTo(260),
    );
    expect(
      tester.getTopLeft(find.text('footer')).dy,
      greaterThan(tester.getTopLeft(find.text('body')).dy),
    );
  });
}
