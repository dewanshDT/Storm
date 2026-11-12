import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:storm/agent/agents_shell.dart' show AgentsSidebar;
import 'package:storm/router.dart';
import 'package:storm/ui/settings/settings_shell.dart' show SettingsNav;
import 'package:storm/ui/shell/activity_rail.dart';
import 'package:storm/ui/shell/nav_history.dart';
import 'package:storm/ui/shell/vault_header.dart';
import 'package:storm/ui/shell/window_buttons.dart';
import 'package:storm/ui/tokens.dart';

import 'fake_server.dart';
import 'shell_harness.dart';

/// The app bar and window chrome (decision 153a–d).
void main() {
  const v = FakeServer.primaryVault;
  const phone = Size(411, 900);
  const wide = Size(1280, 800);

  String location(ProviderContainer c) =>
      c.read(routerProvider).state.uri.toString();

  final bar = find.byKey(const Key('app-bar'));
  final back = find.byKey(const Key('app-bar-back'));
  final forward = find.byKey(const Key('app-bar-forward'));

  bool enabled(WidgetTester tester, Finder button) =>
      tester
          .widget<InkWell>(
            find.descendant(of: button, matching: find.byType(InkWell)),
          )
          .onTap !=
      null;

  group('NavHistory', () {
    test('back and forward walk the visits; a new visit drops forward', () {
      var h = const NavHistory().visit('/a').visit('/b').visit('/c');
      expect(h.canBack, isTrue);
      expect(h.canForward, isFalse);
      h = h.back();
      expect(h.current, '/b');
      expect(h.canForward, isTrue);
      h = h.visit('/b');
      expect(h.current, '/b', reason: 'arriving where back went is no visit');
      expect(h.canForward, isTrue);
      h = h.visit('/d');
      expect(h.entries, ['/a', '/b', '/d']);
      expect(h.canForward, isFalse);
      h = h.back().back();
      expect(h.current, '/a');
      expect(h.canBack, isFalse);
      expect(identical(h.back(), h), isTrue);
    });

    test('a browser step either side moves the index rather than pushing', () {
      var h = const NavHistory().visit('/a').visit('/b');
      h = h.visit('/a', browser: true);
      expect(h.entries, ['/a', '/b']);
      expect(h.current, '/a');
      h = h.visit('/b', browser: true);
      expect(h.current, '/b');
    });

    test('keeps the newest ${NavHistory.limit}', () {
      var h = const NavHistory();
      for (var i = 0; i < NavHistory.limit + 5; i++) {
        h = h.visit('/$i');
      }
      expect(h.entries.length, NavHistory.limit);
      expect(h.entries.first, '/5');
      expect(h.current, '/${NavHistory.limit + 4}');
    });
  });

  test('gtk-decoration-layout picks each end and its order', () {
    const min = CaptionButton.minimize;
    const max = CaptionButton.maximize;
    const close = CaptionButton.close;
    void layout(String gtk, List<CaptionButton> l, List<CaptionButton> r) {
      final parsed = parseDecorationLayout(gtk);
      expect(parsed.left, l, reason: gtk);
      expect(parsed.right, r, reason: gtk);
    }

    layout('appmenu:minimize,maximize,close', [], [min, max, close]);
    layout('close,minimize,maximize:', [close, min, max], []);
    layout('icon:close', [], [close]);
    layout('close', [close], []);
  });

  group('the bar', () {
    testWidgets('44 high and full width at desk width; none on a phone', (
      tester,
    ) async {
      final p = shellContainer();
      await pumpShell(tester, p, size: phone);
      expect(bar, findsNothing);
      await disposeShell(tester, p);

      final w = shellContainer();
      await pumpShell(tester, w, size: wide);
      expect(tester.getRect(bar), const Rect.fromLTWH(0, 0, 1280, 45));
      final rail = tester.getRect(find.byType(ActivityRail));
      expect(rail.top, 45, reason: 'the rail starts under the 44 and its line');
      for (final route in [Routes.agents, Routes.settingsPage('device')]) {
        w.read(routerProvider).go(route);
        await tester.pumpAndSettle();
        expect(bar, findsOneWidget, reason: route);
      }
      await disposeShell(tester, w);
    });

    testWidgets('holds the health dot; the rail no longer does', (
      tester,
    ) async {
      final c = shellContainer();
      await pumpShell(tester, c, size: wide);
      final dot = find.byKey(const Key('bar-status'));
      expect(find.descendant(of: bar, matching: dot), findsOneWidget);
      expect(
        find.descendant(
          of: find.byType(ActivityRail),
          matching: find.bySemanticsLabel('Status'),
        ),
        findsNothing,
      );
      expect(find.byKey(const Key('rail-status')), findsNothing);
      // 20 from the right edge where no caption buttons sit.
      expect(tester.getRect(dot).center.dx, closeTo(1280 - 20, 0.5));
      await tester.tap(dot);
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('health-about')), findsOneWidget);
      await disposeShell(tester, c);
    });

    testWidgets('has no search field (153b); Notes keeps its own', (
      tester,
    ) async {
      final c = shellContainer();
      await pumpShell(tester, c, size: wide);
      c.read(routerProvider).go(Routes.browse(v));
      await tester.pumpAndSettle();
      expect(
        find.descendant(of: bar, matching: find.byType(EditableText)),
        findsNothing,
      );
      expect(
        find.descendant(of: bar, matching: find.textContaining('Search')),
        findsNothing,
      );
      expect(find.textContaining('Search'), findsWidgets);
      await disposeShell(tester, c);
    });
  });

  group('back and forward', () {
    testWidgets('retrace the router; forward dims at the end, back at the '
        'start', (tester) async {
      final c = shellContainer();
      await pumpShell(tester, c, size: wide);
      final router = c.read(routerProvider);
      router.go(Routes.browse(v));
      await tester.pumpAndSettle();
      final start = location(c);
      router.go(Routes.agents);
      await tester.pumpAndSettle();
      router.go(Routes.settingsPage('device'));
      await tester.pumpAndSettle();

      expect(enabled(tester, forward), isFalse);
      expect(enabled(tester, back), isTrue);

      await tester.tap(back);
      await tester.pumpAndSettle();
      expect(location(c), Routes.agents);
      expect(enabled(tester, forward), isTrue);

      await tester.tap(forward);
      await tester.pumpAndSettle();
      expect(location(c), Routes.settingsPage('device'));

      await tester.tap(back);
      await tester.pumpAndSettle();
      await tester.tap(back);
      await tester.pumpAndSettle();
      // The launch location first, then the browse route this test opened.
      while (enabled(tester, back)) {
        await tester.tap(back);
        await tester.pumpAndSettle();
      }
      expect(c.read(navHistoryProvider).index, 0);
      expect(location(c), c.read(navHistoryProvider).entries.first);
      expect(c.read(navHistoryProvider).entries, contains(start));

      // A new place drops what was forward of here.
      router.go(Routes.settingsPage('ai'));
      await tester.pumpAndSettle();
      expect(enabled(tester, forward), isFalse);
      await disposeShell(tester, c);
    });

    testWidgets('Alt+← and Alt+→ on Linux', (tester) async {
      debugDefaultTargetPlatformOverride = TargetPlatform.linux;
      final c = shellContainer();
      await pumpShell(tester, c, size: wide);
      final router = c.read(routerProvider);
      router.go(Routes.agents);
      await tester.pumpAndSettle();
      router.go(Routes.settingsPage('device'));
      await tester.pumpAndSettle();

      await tester.sendKeyDownEvent(LogicalKeyboardKey.altLeft);
      await tester.sendKeyEvent(LogicalKeyboardKey.arrowLeft);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.altLeft);
      await tester.pumpAndSettle();
      expect(location(c), Routes.agents);

      await tester.sendKeyDownEvent(LogicalKeyboardKey.altLeft);
      await tester.sendKeyEvent(LogicalKeyboardKey.arrowRight);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.altLeft);
      await tester.pumpAndSettle();
      expect(location(c), Routes.settingsPage('device'));
      await disposeShell(tester, c);
      debugDefaultTargetPlatformOverride = null;
    });

    testWidgets('⌘[ and ⌘] on macOS', (tester) async {
      debugDefaultTargetPlatformOverride = TargetPlatform.macOS;
      final c = shellContainer();
      await pumpShell(tester, c, size: wide);
      final router = c.read(routerProvider);
      router.go(Routes.agents);
      await tester.pumpAndSettle();
      router.go(Routes.settingsPage('device'));
      await tester.pumpAndSettle();

      await tester.sendKeyDownEvent(LogicalKeyboardKey.metaLeft);
      await tester.sendKeyEvent(LogicalKeyboardKey.bracketLeft);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.metaLeft);
      await tester.pumpAndSettle();
      expect(location(c), Routes.agents);
      await disposeShell(tester, c);
      debugDefaultTargetPlatformOverride = null;
    });
  });

  group('window buttons per platform', () {
    Future<void> check(
      WidgetTester tester,
      TargetPlatform platform, {
      required bool captions,
      required double backLeft,
    }) async {
      debugDefaultTargetPlatformOverride = platform;
      final c = shellContainer();
      await pumpShell(tester, c, size: wide);
      for (final key in ['minimize', 'maximize', 'close']) {
        expect(
          find.byKey(Key('caption-$key')),
          captions ? findsOneWidget : findsNothing,
          reason: '$platform $key',
        );
      }
      if (captions) {
        final close = tester.getRect(find.byKey(const Key('caption-close')));
        expect(close.size, const Size(46, 44));
        expect(close.right, 1280);
        final dot = tester.getRect(find.byKey(const Key('bar-status')));
        final minimize = tester.getRect(
          find.byKey(const Key('caption-minimize')),
        );
        expect(dot.right, lessThanOrEqualTo(minimize.left));
      }
      expect(tester.getRect(back).left, backLeft, reason: '$platform');
      await disposeShell(tester, c);
      debugDefaultTargetPlatformOverride = null;
    }

    testWidgets('Linux: Storm draws them', (tester) async {
      await check(tester, TargetPlatform.linux, captions: true, backLeft: 20);
    });

    testWidgets('Windows: Storm draws them', (tester) async {
      await check(tester, TargetPlatform.windows, captions: true, backLeft: 20);
    });

    testWidgets('macOS: room for the traffic lights, 18 after them', (
      tester,
    ) async {
      await check(tester, TargetPlatform.macOS, captions: false, backLeft: 92);
    });

    testWidgets('web and tablets: none, back at 20', (tester) async {
      await check(
        tester,
        TargetPlatform.android,
        captions: false,
        backLeft: 20,
      );
    });

    testWidgets('a narrow desktop window keeps a frame, without back', (
      tester,
    ) async {
      debugDefaultTargetPlatformOverride = TargetPlatform.linux;
      final c = shellContainer();
      await pumpShell(tester, c, size: phone);
      expect(bar, findsOneWidget);
      expect(find.byKey(const Key('caption-close')), findsOneWidget);
      expect(back, findsNothing);
      expect(find.byKey(const Key('bar-status')), findsNothing);
      await disposeShell(tester, c);
      debugDefaultTargetPlatformOverride = null;
    });

    testWidgets('unfocused, the buttons go text3', (tester) async {
      debugDefaultTargetPlatformOverride = TargetPlatform.linux;
      final c = shellContainer();
      await pumpShell(tester, c, size: wide);
      Color ink() => tester
          .widget<Icon>(
            find.descendant(
              of: find.byKey(const Key('caption-close')),
              matching: find.byType(Icon),
            ),
          )
          .color!;
      final t = tester.element(bar).tokens;
      expect(ink(), t.text2);
      c.read(windowStateProvider.notifier).onWindowBlur();
      await tester.pump();
      expect(ink(), t.text3);
      await disposeShell(tester, c);
      debugDefaultTargetPlatformOverride = null;
    });
  });

  group('one top line under the bar (153a)', () {
    testWidgets('rail 8, sidebar 12, page title 14', (tester) async {
      final c = shellContainer();
      await pumpShell(tester, c, size: wide);
      final router = c.read(routerProvider);
      router.go(Routes.browse(v));
      await tester.pumpAndSettle();

      final railIcon = tester.getRect(
        find.descendant(
          of: find.byKey(const Key('rail-notes')),
          matching: find.byType(InkWell),
        ),
      );
      expect(railIcon.top, 45 + 8);
      expect(tester.getRect(find.byType(VaultHeader)).top, 45 + 12);

      router.go(Routes.agents);
      await tester.pumpAndSettle();
      final agentsTitle = find.descendant(
        of: find.byType(AgentsSidebar),
        matching: find.text('Agents'),
      );
      final agentsRow = tester.getRect(
        find.ancestor(of: agentsTitle, matching: find.byType(Padding)).first,
      );
      expect(agentsRow.top, 45 + 12);

      router.go(Routes.settingsPage('device'));
      await tester.pumpAndSettle();
      final settingsRow = tester.getRect(
        find
            .ancestor(
              of: find.descendant(
                of: find.byType(SettingsNav),
                matching: find.text('Settings'),
              ),
              matching: find.byType(Padding),
            )
            .first,
      );
      expect(settingsRow.top, 45 + 12);
      expect(tester.getRect(find.text('This device').last).top, 45 + 14);
      await disposeShell(tester, c);
    });
  });
}
