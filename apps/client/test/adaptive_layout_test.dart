import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:storm/router.dart';
import 'package:storm/ui/shell/nav_bubble.dart';
import 'package:storm/ui/shell/vault_sidebar.dart';

import 'fake_server.dart';
import 'shell_harness.dart';

/// The wide-screen layout.
///
/// Every test here comes in a pair, or asserts a width explicitly. The phone
/// layout is the default and the wide branch is additive, so "it works on
/// desktop" is only half a result — the other half is that nothing moved at
/// 411px, which is the width this project exists for.
void main() {
  const phone = Size(411, 900);
  const desk = Size(1280, 900);

  group('the sidebar appears only when there is room', () {
    testWidgets('present on a wide screen', (tester) async {
      final c = shellContainer();
      await pumpShell(tester, c, size: desk);
      await openVault(tester, c);

      expect(find.byType(VaultSidebar), findsOneWidget);
      expect(tester.takeException(), isNull);
      await disposeShell(tester, c);
    });

    testWidgets('absent on a phone', (tester) async {
      final c = shellContainer();
      await pumpShell(tester, c, size: phone);
      await openVault(tester, c);

      expect(find.byType(VaultSidebar), findsNothing);
      await disposeShell(tester, c);
    });

    testWidgets('absent on the dashboard even when wide', (tester) async {
      // There is no vault to show folders for.
      final c = shellContainer();
      await pumpShell(tester, c, size: desk);

      expect(find.byType(VaultSidebar), findsNothing);
      await disposeShell(tester, c);
    });
  });

  group('the floating pill is the phone half of the same decision', () {
    testWidgets('shown on a phone', (tester) async {
      final c = shellContainer();
      await pumpShell(tester, c, size: phone);
      await openVault(tester, c);

      expect(find.byTooltip('Directory'), findsOneWidget);
      expect(find.byIcon(Icons.more_horiz), findsNothing);
      await disposeShell(tester, c);
    });

    testWidgets('hidden on a wide screen, its actions in the sidebar', (
      tester,
    ) async {
      final c = shellContainer();
      await pumpShell(tester, c, size: desk);
      await openVault(tester, c);

      // The bubble renders nothing, but the actions are still reachable —
      // both draw the same list, so they cannot offer different things.
      expect(
        find.descendant(
          of: find.byType(NavBubble),
          matching: find.byTooltip('Directory'),
        ),
        findsNothing,
      );
      expect(
        find.descendant(
          of: find.byType(VaultSidebar),
          matching: find.byTooltip('Directory'),
        ),
        findsOneWidget,
      );
      expect(find.byTooltip('New note'), findsOneWidget);
      await disposeShell(tester, c);
    });
  });

  group('the folder tree', () {
    testWidgets('expands a folder in place rather than navigating', (
      tester,
    ) async {
      final c = shellContainer();
      await pumpShell(tester, c, size: desk);
      await openVault(tester, c);

      Finder inTree(String label) => find.descendant(
        of: find.byType(VaultSidebar),
        matching: find.text(label),
      );

      // `Projects/Ideas.md` and `Projects/Storm/Design.md` are in the harness.
      expect(inTree('Ideas'), findsNothing);
      await tester.tap(inTree('Projects'));
      await tester.pumpAndSettle();

      expect(inTree('Ideas'), findsOneWidget);
      expect(
        c.read(routerProvider).state.uri.path,
        Routes.browse(FakeServer.primaryVault),
        reason: 'expanding is not navigation',
      );
      await disposeShell(tester, c);
    });

    testWidgets('keeps its expansion when a note is opened', (tester) async {
      // The reason the vault routes live under a `ShellRoute`. Wrapping each
      // route's child individually rebuilds the whole subtree on every
      // navigation, so the tree would collapse the moment you opened a note.
      final c = shellContainer();
      await pumpShell(tester, c, size: desk);
      await openVault(tester, c);

      // Two traps this test fell into before it caught anything.
      //
      // First, a bare `find.text(...)` also matches the note's own AppBar
      // title once it opens, so everything is scoped to the sidebar.
      //
      // Second — and subtler — the folder expanded must be *unrelated* to the
      // note opened. `_revealOpenNote` re-expands the ancestors of whatever
      // note is in the URL, so opening a note from the folder under test
      // reopens that folder even from a freshly discarded tree, and the
      // assertion passes with no state preserved at all.
      Finder inTree(String label) => find.descendant(
        of: find.byType(VaultSidebar),
        matching: find.text(label),
      );

      await tester.tap(inTree('Daily'));
      await tester.pumpAndSettle();
      expect(inTree('2026-08-05'), findsOneWidget, reason: 'precondition');

      // `Welcome` is at the vault root, so nothing auto-expands `Daily`.
      await tester.tap(inTree('Welcome'));
      await tester.pumpAndSettle();

      expect(
        c.read(routerProvider).state.uri.path,
        contains('/note/'),
        reason: 'the note opened',
      );
      expect(
        inTree('2026-08-05'),
        findsOneWidget,
        reason: 'an unrelated branch must stay open across navigation',
      );
      await disposeShell(tester, c);
    });

    testWidgets('a deep link opens the folders leading to it', (tester) async {
      // Landing at a collapsed root with the note highlighted somewhere you
      // cannot see is worse than no tree at all.
      final c = shellContainer();
      final server = serverOf(c);
      final deep = server.notes.values.firstWhere(
        (n) => n.path == 'Projects/Storm/Design.md',
      );

      await pumpShell(tester, c, size: desk);
      c.read(routerProvider).go(Routes.note(FakeServer.primaryVault, deep.id));
      await tester.pumpAndSettle();

      Finder inTree(String label) => find.descendant(
        of: find.byType(VaultSidebar),
        matching: find.text(label),
      );
      expect(inTree('Projects'), findsOneWidget);
      expect(inTree('Storm'), findsOneWidget);
      expect(inTree('Design'), findsOneWidget);
      await disposeShell(tester, c);
    });

    testWidgets('selecting notes does not pile up a back stack', (
      tester,
    ) async {
      // The sidebar stays put and only the pane changes, so back should leave
      // the vault rather than replaying everything glanced at.
      final c = shellContainer();
      await pumpShell(tester, c, size: desk);
      await openVault(tester, c);

      await tester.tap(find.text('Welcome'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Projects'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Ideas'));
      await tester.pumpAndSettle();

      await tester.binding.handlePopRoute();
      await tester.pumpAndSettle();

      expect(c.read(routerProvider).state.uri.path, Routes.dashboard);
      await disposeShell(tester, c);
    });
  });

  group('the dashboard', () {
    /// The widest a vault card is allowed to get.
    ///
    /// The complaint that started this: at `crossAxisCount: 2` a 2000px window
    /// gave cards roughly 980px across.
    Future<double> cardWidth(WidgetTester tester) async {
      final card = find
          .ancestor(of: find.text('Primary'), matching: find.byType(Container))
          .first;
      return tester.getSize(card).width;
    }

    testWidgets('cards stay card-sized on a wide screen', (tester) async {
      final c = shellContainer();
      final server = serverOf(c);
      for (var i = 0; i < 6; i++) {
        server.addVault('v-extra-$i', 'Extra $i');
      }
      await pumpShell(tester, c, size: const Size(1600, 900));

      expect(await cardWidth(tester), lessThanOrEqualTo(220));
      expect(tester.takeException(), isNull);
      await disposeShell(tester, c);
    });

    testWidgets('the phone still gets two columns', (tester) async {
      final c = shellContainer();
      serverOf(c).addVault('v-second', 'Second');
      await pumpShell(tester, c, size: phone);

      // (411 - 32 padding - 12 gutter) / 2 ≈ 183.
      final width = await cardWidth(tester);
      expect(width, greaterThan(150));
      expect(width, lessThan(220));
      await disposeShell(tester, c);
    });

    testWidgets('recents move to a rail instead of stretching', (tester) async {
      final c = shellContainer();
      final server = serverOf(c);
      server.markOpened(FakeServer.primaryVault, 'n0', '2026-08-07T09:00:00Z');
      await pumpShell(tester, c, size: desk);

      final rail = tester.getSize(
        find
            .ancestor(
              of: find.text('Recently opened'),
              matching: find.byType(SizedBox),
            )
            .last,
      );
      expect(rail.width, lessThanOrEqualTo(340));
      await disposeShell(tester, c);
    });
  });

  group('nothing overflows at any width', () {
    for (final size in [phone, const Size(900, 900), const Size(1600, 900)]) {
      testWidgets('${size.width.toInt()}px', (tester) async {
        final c = shellContainer();
        await pumpShell(tester, c, size: size);
        expect(tester.takeException(), isNull, reason: 'dashboard');

        await openVault(tester, c);
        expect(tester.takeException(), isNull, reason: 'browser');

        c.read(routerProvider).go(Routes.note(FakeServer.primaryVault, 'n0'));
        await tester.pumpAndSettle();
        expect(tester.takeException(), isNull, reason: 'note');

        await disposeShell(tester, c);
      });
    }
  });
}
