import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:storm/ui/settings/settings_widgets.dart' show TextAction;
import 'package:storm/ui/theme.dart';
import 'package:storm/ui/widgets.dart';

Widget _host(Widget child, {TargetPlatform? platform}) {
  final base = StormTheme.dark();
  return MaterialApp(
    theme: platform == null ? base : base.copyWith(platform: platform),
    home: Scaffold(body: Center(child: child)),
  );
}

void main() {
  group('Tappable', () {
    testWidgets('Enter and Space activate when focused', (tester) async {
      var taps = 0;
      await tester.pumpWidget(
        _host(
          Tappable(
            onTap: () => taps++,
            child: const SizedBox(width: 60, height: 60),
          ),
        ),
      );
      final focus = Focus.of(tester.element(find.byType(InkWell)));
      focus.requestFocus();
      await tester.pump();
      await tester.sendKeyEvent(LogicalKeyboardKey.enter);
      await tester.sendKeyEvent(LogicalKeyboardKey.space);
      expect(taps, 2);
    });

    testWidgets('exposes a button with its label', (tester) async {
      final handle = tester.ensureSemantics();
      await tester.pumpWidget(
        _host(
          Tappable(
            onTap: () {},
            semanticLabel: 'Close',
            child: const SizedBox(width: 60, height: 60),
          ),
        ),
      );
      expect(
        tester.getSemantics(find.bySemanticsLabel('Close')),
        matchesSemantics(
          label: 'Close',
          isButton: true,
          isEnabled: true,
          hasEnabledState: true,
          hasTapAction: true,
        ),
      );
      handle.dispose();
    });

    for (final (platform, min) in [
      (TargetPlatform.linux, 40.0),
      (TargetPlatform.android, 48.0),
    ]) {
      testWidgets('hit area reaches $min on $platform, layout unchanged', (
        tester,
      ) async {
        var taps = 0;
        await tester.pumpWidget(
          _host(
            Tappable(
              onTap: () => taps++,
              child: const SizedBox(width: 12, height: 12, key: Key('box')),
            ),
            platform: platform,
          ),
        );
        expect(
          tester.getSize(find.byKey(const Key('box'))),
          const Size(12, 12),
        );
        final c = tester.getCenter(find.byKey(const Key('box')));
        await tester.tapAt(c + Offset(min / 2 - 1, 0));
        expect(taps, 1);
        await tester.tapAt(c + Offset(min / 2 + 2, 0));
        expect(taps, 1);
      });
    }

    testWidgets('secondary tap and long press fire', (tester) async {
      var secondary = 0;
      var long = 0;
      await tester.pumpWidget(
        _host(
          Tappable(
            onTap: () {},
            onSecondaryTap: () => secondary++,
            onLongPress: () => long++,
            child: const SizedBox(width: 60, height: 60, key: Key('box')),
          ),
        ),
      );
      await tester.longPress(find.byKey(const Key('box')), warnIfMissed: false);
      expect(long, 1);
      await tester.tap(find.byKey(const Key('box')), buttons: kSecondaryButton);
      expect(secondary, 1);
    });
  });

  group('Hairline', () {
    testWidgets('draws only the chosen edges', (tester) async {
      await tester.pumpWidget(
        _host(
          const Hairline(top: true, left: true, child: SizedBox(width: 20)),
        ),
      );
      final box = tester.widget<Container>(find.byType(Container).last);
      final border = (box.decoration! as BoxDecoration).border! as Border;
      expect(border.top.style, BorderStyle.solid);
      expect(border.left.style, BorderStyle.solid);
      expect(border.bottom.style, BorderStyle.none);
      expect(border.right.style, BorderStyle.none);
    });
  });

  group('OutlineTag', () {
    testWidgets('ellipsises a long label inside a bounded width', (
      tester,
    ) async {
      await tester.pumpWidget(
        _host(
          const SizedBox(
            width: 80,
            child: OutlineTag(
              label: 'a-very-long-label-that-cannot-possibly-fit',
              maxWidth: 80,
            ),
          ),
        ),
      );
      final text = tester.widget<Text>(find.byType(Text));
      expect(text.overflow, TextOverflow.ellipsis);
      expect(text.maxLines, 1);
      expect(
        tester.getSize(find.byType(OutlineTag)).width,
        lessThanOrEqualTo(80),
      );
      expect(tester.takeException(), isNull);
    });
  });

  group('migrated agent pieces', () {
    testWidgets('banner, dots and tag overrides render', (tester) async {
      await tester.pumpWidget(
        _host(
          const Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              NoticeBanner(child: Text('notice')),
              StatusDot(color: Colors.grey, hollow: true),
              OutlineTag(
                label: 'Starts agents',
                radius: 12,
                trailing: Icon(Icons.expand_more),
                trailingGap: 4,
              ),
              TextActionProbe(),
            ],
          ),
        ),
      );
      expect(find.text('notice'), findsOneWidget);
      expect(tester.takeException(), isNull);
    });
  });
}

class TextActionProbe extends StatelessWidget {
  const TextActionProbe({super.key});

  @override
  Widget build(BuildContext context) =>
      TextAction(label: 'Open', accent: true, lineHeight: 1.4, onTap: () {});
}
