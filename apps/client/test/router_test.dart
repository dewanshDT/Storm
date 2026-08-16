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
    testWidgets('an unconfigured app lands on pairing', (tester) async {
      final c = shellContainer(configured: false);
      await pumpShell(tester, c);

      expect(find.text('Pair with your Storm server'), findsOneWidget);
      await disposeShell(tester, c);
    });

    testWidgets('a configured app lands on the dashboard', (tester) async {
      final c = shellContainer();
      await pumpShell(tester, c);

      // The wordmark is gone — the app's own name is the least useful thing
      // on the screen you opened the app to see.
      expect(find.text('STORM'), findsNothing);
      expect(find.text('RECENTLY OPENED'), findsOneWidget);
      expect(find.text('VAULTS'), findsOneWidget);
      await disposeShell(tester, c);
    });

    testWidgets('a legacy token install is never sent to pairing', (
      tester,
    ) async {
      // The harness is exactly an install that predates auth: a URL and a
      // token, no device. Requiring pairing here would lock it out of a vault
      // it can already read, which is what "auth stays additive" forbids.
      final c = shellContainer();
      await pumpShell(tester, c);

      expect(c.read(settingsProvider).value?.isPaired, isFalse);
      expect(find.text('Pair with your Storm server'), findsNothing);
      expect(find.text('VAULTS'), findsOneWidget);
      await disposeShell(tester, c);
    });

    testWidgets('pairing stays reachable before anything is configured', (
      tester,
    ) async {
      // /connect is not dead: a server with no auth still needs it.
      final c = shellContainer(configured: false);
      await pumpShell(tester, c);

      c.read(routerProvider).go(Routes.connect);
      await tester.pumpAndSettle();
      expect(find.text('Connect to your homelab vault'), findsOneWidget);

      c.read(routerProvider).go(Routes.pairing);
      await tester.pumpAndSettle();
      expect(find.text('Pair with your Storm server'), findsOneWidget);
      await disposeShell(tester, c);
    });

    testWidgets('disconnecting sends an open app back to pairing', (
      tester,
    ) async {
      // The regression that motivated `refreshListenable`: watching settings
      // rebuilt the provider into a *new* GoRouter while the MaterialApp kept
      // the old one, so this redirect never fired.
      final c = shellContainer();
      await pumpShell(tester, c);
      expect(find.text('RECENTLY OPENED'), findsOneWidget);

      await c.read(settingsProvider.notifier).save(const Settings());
      await tester.pumpAndSettle();

      expect(find.text('Pair with your Storm server'), findsOneWidget);
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

      expect(find.text('Vaults'), findsOneWidget);
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

      // Up is the breadcrumb now: there is no back button in the chrome.
      await tester.tap(find.text('Primary'));
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
