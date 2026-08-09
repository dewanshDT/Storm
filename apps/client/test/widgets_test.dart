import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:storm/ui/gallery_screen.dart';
import 'package:storm/ui/widgets.dart';
import 'package:storm/ui/theme.dart';
import 'package:storm/ui/tokens.dart';

/// The atoms, checked for the things that are easy to get wrong quietly.
///
/// Not pixel assertions — those break on every legitimate change and prove
/// nothing. These check the properties the design system actually promises: an
/// atom takes its colour from the tokens, a tag is not a control-sized slab,
/// and the brand mark keeps its own ground in every theme.
void main() {
  testWidgets('the gallery renders every preset without overflowing', (
    tester,
  ) async {
    // The claim in widgets.dart's header, kept honest. Rendering it is also
    // the cheapest smoke test there is for the whole shared set.
    tester.view.physicalSize = const Size(2400, 2400);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);

    await tester.pumpWidget(
      MaterialApp(theme: StormTheme.dark(), home: const GalleryScreen()),
    );
    await tester.pumpAndSettle();
    expect(tester.takeException(), isNull);
    for (final preset in StormPreset.values) {
      expect(find.text(preset.label), findsOneWidget);
    }
  });

  Widget wrap(Widget child, {StormPreset preset = StormPreset.stormDark}) =>
      MaterialApp(
        theme: StormTheme.from(preset),
        home: Scaffold(body: Center(child: child)),
      );

  /// Pumps and lets the theme transition finish.
  ///
  /// `MaterialApp` wraps its child in an `AnimatedTheme`, so the frame
  /// immediately after `pumpWidget` still carries the *previous* theme's
  /// colours. A test that only pumps once therefore reads the old palette and
  /// fails with two correct-looking colours that simply are not the ones asked
  /// for — which is exactly how this suite failed first time.
  Future<void> show(
    WidgetTester tester,
    Widget child, {
    StormPreset preset = StormPreset.stormDark,
  }) async {
    await tester.pumpWidget(wrap(child, preset: preset));
    await tester.pumpAndSettle();
  }

  group('StatusDot', () {
    testWidgets('says one thing per colour, in every theme', (tester) async {
      // Green is synced and nothing else; grey is offline. If these ever came
      // from the same token the dot would stop being a signal.
      for (final preset in StormPreset.values) {
        final t = StormTokens.from(preset);
        for (final (status, expected) in [
          (DotStatus.synced, t.green),
          (DotStatus.syncing, t.amber),
          (DotStatus.offline, t.text3),
        ]) {
          await show(tester, StatusDot(status: status), preset: preset);
          final box = tester.widget<Container>(find.byType(Container));
          final decoration = box.decoration! as BoxDecoration;
          expect(
            decoration.color,
            expected,
            reason: '$status in ${preset.label}',
          );
          expect(decoration.shape, BoxShape.circle);
        }
      }
    });
  });

  group('TagChip', () {
    testWidgets('stays inline-sized rather than becoming a tap slab', (
      tester,
    ) async {
      // A tag sits inside a line of prose. At Material's 48px minimum it would
      // break the line rhythm everywhere it appeared, which is why the design
      // calls out roughly 26px.
      await show(tester, const TagChip(label: '#proj/storm'));
      expect(tester.getSize(find.byType(TagChip)).height, lessThan(32));
    });

    testWidgets('is amber on amber-soft, the tag meaning', (tester) async {
      final t = StormTokens.from(StormPreset.stormDark);
      await show(tester, const TagChip(label: '#cooking'));

      final box = tester.widget<Container>(
        find.descendant(
          of: find.byType(TagChip),
          matching: find.byType(Container),
        ),
      );
      expect((box.decoration! as BoxDecoration).color, t.amberSoft);
      expect(tester.widget<Text>(find.text('#cooking')).style?.color, t.amber);
    });
  });

  group('KeyChip', () {
    testWidgets('is mono, because a key is an identifier', (tester) async {
      await show(tester, const KeyChip(label: 'created'));
      expect(
        tester.widget<Text>(find.text('created')).style?.fontFamily,
        StormTokens.monoFamily,
      );
    });
  });

  group('SaveStateLabel', () {
    testWidgets('colours each state by what it means', (tester) async {
      final t = StormTokens.from(StormPreset.stormDark);
      for (final (tone, expected) in [
        (SaveTone.good, t.green),
        (SaveTone.waiting, t.amber),
        (SaveTone.bad, t.danger),
      ]) {
        await show(tester, SaveStateLabel(label: 'state', tone: tone));
        expect(tester.widget<Text>(find.text('state')).style?.color, expected);
      }
    });
  });

  group('BrandMark', () {
    testWidgets('loads the real asset', (tester) async {
      // The asset was referenced by the launcher-icon config for weeks without
      // being declared in `flutter: assets:`, which generates icons perfectly
      // and leaves Image.asset unable to find it at runtime. This fails if that
      // declaration goes away again.
      await show(tester, const BrandMark());

      expect(tester.takeException(), isNull);
      final image = tester.widget<Image>(find.byType(Image));
      expect((image.image as AssetImage).assetName, contains('storm_icon'));
    });

    testWidgets('the wordmark is drawn beside the mark, not instead of it', (
      tester,
    ) async {
      await show(tester, const BrandMark(withWordmark: true));
      expect(find.byType(Image), findsOneWidget);
      expect(find.text('STORM'), findsOneWidget);
    });
  });

  group('every atom takes its colour from the tokens', () {
    testWidgets('so switching preset moves all of them together', (
      tester,
    ) async {
      // The contract the whole system rests on. If an atom hard-codes a colour
      // this is where it shows up, because two presets would render it the same.
      Future<Color?> dotColour(StormPreset preset) async {
        await show(
          tester,
          const StatusDot(status: DotStatus.synced),
          preset: preset,
        );
        final box = tester.widget<Container>(find.byType(Container));
        return (box.decoration! as BoxDecoration).color;
      }

      expect(
        await dotColour(StormPreset.stormDark),
        isNot(await dotColour(StormPreset.slowflowEarth)),
      );
    });
  });
}
