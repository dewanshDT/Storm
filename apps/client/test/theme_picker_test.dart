import 'dart:ui' show Tristate;

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:storm/router.dart';
import 'package:storm/state/app_state.dart';
import 'package:storm/ui/settings/theme_picker.dart';
import 'package:storm/ui/theme.dart';
import 'package:storm/ui/themes/theme_config.dart';
import 'package:storm/ui/themes/theme_library.dart';
import 'package:storm/ui/tokens.dart';

import 'shell_harness.dart';

/// Decision 160: the inspired families, the picker and the credits.
void main() {
  const desk = Size(1280, 900);

  setUpAll(() async {
    TestWidgetsFlutterBinding.ensureInitialized();
    await ThemeLibrary.load(rootBundle);
  });
  tearDownAll(() => ThemeLibrary.families.value = ThemeLibrary.builtIn);

  List<ThemeVariant> variants() => [
    for (final f in ThemeLibrary.families.value) ...[?f.light, ?f.dark],
  ];

  Future<void> open(WidgetTester tester, c, String page) async {
    c.read(routerProvider).go(Routes.settingsPage(page));
    await tester.pumpAndSettle();
  }

  Future<void> tapKey(WidgetTester tester, String key) async {
    final f = find.byKey(Key(key));
    await tester.ensureVisible(f);
    await tester.pumpAndSettle();
    await tester.tap(f);
    await tester.pumpAndSettle();
  }

  test('all eight families and fifteen variants load', () {
    expect(ThemeLibrary.families.value.map((f) => f.id), [
      'storm',
      'slowflow',
      'catppuccin',
      'gruvbox',
      'nord',
      'rosepine',
      'solarized',
      'tokyonight',
    ]);
    expect(variants(), hasLength(15));
    for (final f in ThemeLibrary.families.value.skip(2)) {
      expect(f.inspiredBy?.licence, 'MIT', reason: f.id);
      expect(f.light, isNotNull, reason: f.id);
      expect(f.dark, isNotNull, reason: f.id);
    }
  });

  testWidgets('every variant builds the shell without an exception', (
    tester,
  ) async {
    for (final v in variants()) {
      final c = shellContainer();
      await pumpShell(tester, c, size: desk, theme: StormTheme.from(v));
      await openVault(tester, c);
      expect(tester.takeException(), isNull, reason: v.label);
      await open(tester, c, 'device');
      expect(tester.takeException(), isNull, reason: v.label);
      await disposeShell(tester, c);
    }
  });

  testWidgets('the picker switches family and mode', (tester) async {
    final c = shellContainer();
    await pumpShell(tester, c, size: desk);
    await open(tester, c, 'device');
    Settings s() => c.read(settingsProvider).value!;

    await tapKey(tester, 'theme-family-catppuccin');
    expect(s().theme, const ThemeChoice('catppuccin', ThemeMode.dark));
    expect(find.text('Catppuccin'), findsOneWidget);
    expect(find.text('Catppuccin (inspired)'), findsNothing);
    expect(find.text('Mocha'), findsOneWidget);
    expect(find.text('Inspired by Catppuccin'), findsOneWidget);

    await tapKey(tester, 'theme-mode-light');
    expect(s().theme, const ThemeChoice('catppuccin', ThemeMode.light));
    expect(find.text('Latte'), findsOneWidget);
    expect(find.text('Light only'), findsNothing);

    await tapKey(tester, 'theme-mode-system');
    expect(s().theme.mode, ThemeMode.system);

    await tapKey(tester, 'theme-mode-dark');
    expect(find.text('SlowFlow earth'), findsOneWidget);
    expect(find.text('Light only'), findsOneWidget);
    await tapKey(tester, 'theme-family-slowflow');
    expect(s().theme.resolve(Brightness.dark).label, 'SlowFlow earth');

    final mode = tester.getSemantics(find.byKey(const Key('theme-mode-dark')));
    expect(mode.flagsCollection.isSelected, Tristate.isTrue);

    final card = tester.getSemantics(
      find.byKey(const Key('theme-family-storm')),
    );
    expect(card.label, contains('Storm'));
    expect(card.flagsCollection.isSelected, Tristate.isFalse);
    final picked = tester.getSemantics(
      find.byKey(const Key('theme-family-slowflow')),
    );
    expect(picked.label, 'SlowFlow, SlowFlow earth, Light only');
    expect(picked.flagsCollection.isSelected, Tristate.isTrue);
    await disposeShell(tester, c);
  });

  testWidgets('the cards fill the page in equal columns of equal height', (
    tester,
  ) async {
    final c = shellContainer();
    await pumpShell(tester, c, size: desk);
    await open(tester, c, 'device');
    final cards = [
      for (final f in ThemeLibrary.families.value)
        tester.getRect(find.byKey(Key('theme-family-${f.id}'))),
    ];
    final widths = cards.map((r) => r.width.round()).toSet();
    final heights = cards.map((r) => r.height.round()).toSet();
    expect(widths, hasLength(1));
    expect(heights, hasLength(1));
    expect(widths.single, greaterThanOrEqualTo(220));
    final picker = tester.getRect(find.byType(ThemePicker));
    final rowOne = cards.where((r) => r.top == cards.first.top).toList();
    expect(rowOne.length, greaterThan(1));
    expect(rowOne.last.right, moreOrLessEquals(picker.right, epsilon: 1));
    await disposeShell(tester, c);
  });

  testWidgets('the low-contrast note shows only for a theme that fails', (
    tester,
  ) async {
    final weak = themeFamilyFromJson({
      'id': 'weak',
      'label': 'Weak',
      'variants': {
        'dark': {
          'bgL': 40,
          'hue': 0,
          'chroma': 0,
          'accentH': 250,
          'accentC': 0.1,
          'accentL': 50,
        },
      },
    });
    expect(lowContrastNote(weak.dark!), startsWith('Low contrast: '));
    for (final v in variants()) {
      expect(lowContrastNote(v), isNull, reason: v.label);
    }

    final before = ThemeLibrary.families.value;
    ThemeLibrary.families.value = ThemeLibrary.merge([...before, weak]);
    addTearDown(() => ThemeLibrary.families.value = before);
    final c = shellContainer();
    await pumpShell(tester, c, size: desk);
    await open(tester, c, 'device');
    expect(find.byKey(const Key('theme-low-weak')), findsOneWidget);
    expect(find.byKey(const Key('theme-low-storm')), findsNothing);
    expect(
      find.textContaining('Low contrast: '),
      findsOneWidget,
      reason: 'Storm and the inspired themes pass',
    );
    await disposeShell(tester, c);
  });

  testWidgets('About lists the theme credits', (tester) async {
    final c = shellContainer();
    await pumpShell(tester, c, size: desk);
    await open(tester, c, 'health');
    await tester.ensureVisible(find.byKey(const Key('theme-credits')));
    expect(
      find.text(
        'Catppuccin (inspired): inspired by Catppuccin '
        '(https://catppuccin.com, MIT)',
      ),
      findsOneWidget,
    );
    expect(find.textContaining('inspired by '), findsNWidgets(6));
    await disposeShell(tester, c);
  });
}
