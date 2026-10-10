import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shared_preferences/shared_preferences.dart';
import 'package:xterm2/xterm.dart' show TerminalStyle;

import 'package:storm/agent/terminal_surface.dart';
import 'package:storm/router.dart';
import 'package:storm/state/terminal_prefs.dart';
import 'package:storm/ui/theme.dart';
import 'package:storm/ui/tokens.dart';

import 'shell_harness.dart';

/// D15 AM47: even terminal metrics and Settings › Terminal.
void main() {
  group('evenTerminalPadding', () {
    const base = EdgeInsets.all(10);
    for (final cell in [16.25, 17.0, 19.5]) {
      for (final height in [301.0, 444.4, 812.0]) {
        test('cell $cell in $height centres whole rows', () {
          final pad = evenTerminalPadding(base, height, cell);
          final inner = height - pad.vertical;
          final rows = inner ~/ cell;
          expect(rows, (height - base.vertical) ~/ cell, reason: 'no row lost');
          expect(rows * cell + pad.vertical, closeTo(height, 0.5));
          expect(pad.top, closeTo(pad.bottom, 1e-9));
          expect(pad.left, base.left);
        });
      }
    }

    test('a box shorter than a row keeps the base', () {
      expect(evenTerminalPadding(base, 25, 16), base);
      expect(evenTerminalPadding(base, double.infinity, 16), base);
    });
  });

  group('TerminalPrefs', () {
    test('round-trips through prefs, and a default size is no key', () async {
      SharedPreferences.setMockInitialValues({});
      final prefs = await SharedPreferences.getInstance();
      const custom = TerminalPrefs(
        fontSize: 15,
        spacing: TerminalSpacing.relaxed,
        padding: TerminalPadding.tight,
      );
      await custom.write(prefs);
      expect(TerminalPrefs.read(prefs), custom);

      await const TerminalPrefs().write(prefs);
      expect(prefs.containsKey('storm.terminal.fontSize'), isFalse);
      expect(TerminalPrefs.read(prefs).isDefault, isTrue);
    });

    test('unknown or out-of-range values fall back', () async {
      SharedPreferences.setMockInitialValues({
        'storm.terminal.fontSize': 99.0,
        'storm.terminal.spacing': 'cosy',
      });
      final p = TerminalPrefs.read(await SharedPreferences.getInstance());
      expect(p.fontSize, TerminalPrefs.maxFontSize);
      expect(p.spacing, TerminalSpacing.normal);
    });
  });

  group('StormTerminalView', () {
    Future<StormTerminal> pump(
      WidgetTester tester,
      double height,
      TerminalPrefs prefs,
    ) async {
      final t = StormTerminal();
      addTearDown(t.dispose);
      await tester.pumpWidget(
        MaterialApp(
          theme: StormTheme.dark(),
          home: Align(
            alignment: Alignment.topLeft,
            child: SizedBox(
              width: 600,
              height: height,
              child: StormTerminalView(terminal: t, prefs: prefs),
            ),
          ),
        ),
      );
      await tester.pump();
      return t;
    }

    testWidgets('fits every row the box can hold, at every spacing', (
      tester,
    ) async {
      for (final spacing in TerminalSpacing.values) {
        for (final height in [300.0, 433.0, 517.5]) {
          final prefs = TerminalPrefs(spacing: spacing);
          final t = await pump(tester, height, prefs);
          final ctx = tester.element(find.byType(StormTerminalView));
          final tokens = ctx.tokens;
          final cell = terminalCellHeight(
            TerminalStyle(
              fontFamily: StormTokens.monoFamily,
              fontSize: tokens.codeSize,
              height: spacing.lineHeight,
            ),
            MediaQuery.textScalerOf(ctx),
          );
          final inner = height - tokens.sp * 1.25 * 2;
          expect(t.rows, inner ~/ cell, reason: '$spacing at $height');
        }
      }
    });

    testWidgets('relaxed spacing fits fewer rows than compact', (tester) async {
      final compact = await pump(
        tester,
        500,
        const TerminalPrefs(spacing: TerminalSpacing.compact),
      );
      final compactRows = compact.rows;
      final relaxed = await pump(
        tester,
        500,
        const TerminalPrefs(spacing: TerminalSpacing.relaxed),
      );
      expect(relaxed.rows, lessThan(compactRows));
    });
  });

  group('Settings › Terminal', () {
    const desk = Size(1280, 800);

    Future<ProviderContainer> open(WidgetTester tester) async {
      SharedPreferences.setMockInitialValues({});
      final c = shellContainer();
      await pumpShell(tester, c, size: desk);
      c.read(routerProvider).go(Routes.settingsPage('terminal'));
      await tester.pumpAndSettle();
      return c;
    }

    Future<void> tap(WidgetTester tester, Finder f) async {
      await tester.ensureVisible(f);
      await tester.pumpAndSettle();
      await tester.tap(f);
      await tester.pumpAndSettle();
    }

    testWidgets('choices change the prefs, and Reset restores them', (
      tester,
    ) async {
      final c = await open(tester);
      expect(find.byKey(const Key('terminal-preview')), findsOneWidget);
      expect(find.byKey(const Key('terminal-reset')), findsNothing);
      expect(find.text('Default'), findsWidgets);

      await tap(tester, find.text('Relaxed'));
      await tap(tester, find.text('Roomy'));
      await tap(tester, find.byKey(const Key('terminal-larger')));
      final tokens = tester
          .element(find.byKey(const Key('terminal-preview')))
          .tokens;
      expect(
        c.read(terminalPrefsProvider),
        TerminalPrefs(
          fontSize: tokens.codeSize.roundToDouble() + 1,
          spacing: TerminalSpacing.relaxed,
          padding: TerminalPadding.roomy,
        ),
      );
      final stored = TerminalPrefs.read(await SharedPreferences.getInstance());
      expect(stored, c.read(terminalPrefsProvider));

      await tap(tester, find.byKey(const Key('terminal-reset')));
      expect(c.read(terminalPrefsProvider).isDefault, isTrue);
      expect(find.byKey(const Key('terminal-reset')), findsNothing);
      await disposeShell(tester, c);
    });
  });
}
