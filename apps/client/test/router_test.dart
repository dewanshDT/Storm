import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:storm/router.dart';
import 'package:storm/state/app_state.dart';
import 'package:storm/ui/browse_screen.dart';

import 'shell_harness.dart';
import 'fake_server.dart';

/// Routing and the shell.
///
/// Written before the screens were wired to real state, because the last
/// build's worst bugs were UI wiring with no test coverage sitting on top of
/// an exhaustively tested sync layer.
void main() {
  group('redirects', () {
    testWidgets('an unconfigured app lands on connect', (tester) async {
      final c = shellContainer(configured: false);
      await pumpShell(tester, c);

      expect(find.text('Connect to your homelab vault'), findsOneWidget);
      await disposeShell(tester, c);
    });

    testWidgets('a configured app lands on the dashboard', (tester) async {
      final c = shellContainer();
      await pumpShell(tester, c);

      expect(find.text('Storm'), findsOneWidget);
      expect(find.text('Recently opened'), findsOneWidget);
      await disposeShell(tester, c);
    });

    testWidgets('disconnecting sends an open app back to connect', (
      tester,
    ) async {
      // The regression that motivated `refreshListenable`: watching settings
      // rebuilt the provider into a *new* GoRouter while the MaterialApp kept
      // the old one, so this redirect never fired.
      final c = shellContainer();
      await pumpShell(tester, c);
      expect(find.text('Recently opened'), findsOneWidget);

      await c.read(settingsProvider.notifier).save(const Settings());
      await tester.pumpAndSettle();

      expect(find.text('Connect to your homelab vault'), findsOneWidget);
      await disposeShell(tester, c);
    });

    testWidgets('the router survives a settings change', (tester) async {
      final c = shellContainer();
      await pumpShell(tester, c);
      final before = c.read(routerProvider);

      await c
          .read(settingsProvider.notifier)
          .save(const Settings(baseUrl: 'http://other', token: 'u'));
      await tester.pumpAndSettle();

      expect(
        identical(c.read(routerProvider), before),
        isTrue,
        reason: 'a replaced router silently stops navigating',
      );
      await disposeShell(tester, c);
    });
  });

  group('navigation', () {
    testWidgets('a note route opens that note', (tester) async {
      final c = shellContainer();
      await pumpShell(tester, c);

      c.read(routerProvider).go(Routes.note(FakeServer.primaryVault, 'n0'));
      await tester.pumpAndSettle();

      expect(c.read(openNoteIdProvider), 'n0');
      await disposeShell(tester, c);
    });

    testWidgets('a deep link into a nested folder shows its breadcrumb', (
      tester,
    ) async {
      // The reason for a real router: this location has to be reachable
      // directly, not only by tapping through.
      final c = shellContainer();
      await pumpShell(tester, c);

      c
          .read(routerProvider)
          .go(Routes.folder(FakeServer.primaryVault, 'Projects/Storm'));
      await tester.pumpAndSettle();

      expect(find.text('Vault'), findsOneWidget);
      expect(find.text('Projects'), findsWidgets);
      expect(find.text('Storm'), findsWidgets);
      expect(find.text('Design'), findsOneWidget);
      await disposeShell(tester, c);
    });

    testWidgets('drilling into a folder and back up', (tester) async {
      final c = shellContainer();
      await pumpShell(tester, c);

      c.read(routerProvider).go(Routes.browse(FakeServer.primaryVault));
      await tester.pumpAndSettle();
      expect(find.text('Daily'), findsOneWidget);

      await tester.tap(find.text('Daily'));
      await tester.pumpAndSettle();
      expect(find.text('2026-08-05'), findsOneWidget);

      await tester.tap(find.byIcon(Icons.arrow_back));
      await tester.pumpAndSettle();
      expect(find.text('Projects'), findsOneWidget);
      await disposeShell(tester, c);
    });
  });

  group('the folder listing', () {
    test('shows one level, folders before notes', () {
      final notes = [
        for (final p in [
          'Welcome.md',
          'Daily/a.md',
          'Daily/b.md',
          'Projects/Storm/Design.md',
        ])
          noteMeta(p),
      ];

      final root = childrenOfFolder(notes, '');
      expect(root.map((e) => e.name), ['Daily', 'Projects', 'Welcome']);
      expect(root.first.isFolder, isTrue);
      expect(root.first.childCount, 2, reason: 'Daily holds two notes');

      // A nested folder shows only its own children, not the whole subtree.
      expect(childrenOfFolder(notes, 'Projects').map((e) => e.name), ['Storm']);
      expect(childrenOfFolder(notes, 'Projects/Storm').map((e) => e.name), [
        'Design',
      ]);
    });

    test('an empty or unknown folder yields nothing', () {
      expect(childrenOfFolder([], ''), isEmpty);
      expect(childrenOfFolder([noteMeta('A.md')], 'Nope'), isEmpty);
    });
  });

  group('layout', () {
    for (final (name, size) in [
      ('phone', Size(411, 900)),
      ('desktop', Size(1400, 900)),
    ]) {
      testWidgets('the dashboard does not overflow on $name', (tester) async {
        // The check that caught an AppBar silently dropping the attach button.
        final c = shellContainer();
        await pumpShell(tester, c, size: size);
        expect(tester.takeException(), isNull);
        await disposeShell(tester, c);
      });

      testWidgets('the directory does not overflow on $name', (tester) async {
        final c = shellContainer();
        await pumpShell(tester, c, size: size);
        c
            .read(routerProvider)
            .go(Routes.folder(FakeServer.primaryVault, 'Projects/Storm'));
        await tester.pumpAndSettle();
        expect(tester.takeException(), isNull);
        await disposeShell(tester, c);
      });

      testWidgets('an open note does not overflow on $name', (tester) async {
        final c = shellContainer();
        await pumpShell(tester, c, size: size);
        c.read(routerProvider).go(Routes.note(FakeServer.primaryVault, 'n0'));
        await tester.pumpAndSettle();
        expect(tester.takeException(), isNull);
        await disposeShell(tester, c);
      });
    }
  });
}
