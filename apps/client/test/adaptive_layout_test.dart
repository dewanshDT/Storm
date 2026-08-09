import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:storm/router.dart';
import 'package:storm/ui/note_editor.dart';
import 'package:storm/ui/note_properties.dart';
import 'package:storm/ui/properties_panel.dart';
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

    testWidgets('absent on the dashboard, which only a phone reaches', (
      tester,
    ) async {
      // There is no vault to show folders for — and no dashboard at all at
      // desk width once a vault exists, so this is the phone's case.
      final c = shellContainer();
      await pumpShell(tester, c, size: phone);

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

      // The bubble renders nothing, and the actions are still reachable —
      // both draw the same list, so they cannot offer different things.
      expect(
        find.descendant(
          of: find.byType(NavBubble),
          matching: find.byTooltip('Tags'),
        ),
        findsNothing,
      );
      expect(
        find.descendant(
          of: find.byType(VaultSidebar),
          matching: find.byTooltip('Tags'),
        ),
        findsOneWidget,
      );
      // Except the two the rail already *is*: a Directory button beside the
      // folder tree, or a Search button beside the search field, would be a
      // control that does what is already on screen.
      expect(
        find.descendant(
          of: find.byType(VaultSidebar),
          matching: find.byTooltip('Directory'),
        ),
        findsNothing,
      );
      expect(find.byTooltip('New note'), findsOneWidget);
      await disposeShell(tester, c);
    });
  });

  group('the rail is ordered the way the design orders it', () {
    testWidgets('vault, then search, then folders, then actions', (
      tester,
    ) async {
      // The order is the design decision, not the contents. Actions used to be
      // on top, where they were the first thing the eye hit and the last thing
      // anyone wanted; the vault you are in and the way to a note come first.
      final c = shellContainer();
      await pumpShell(tester, c, size: desk);
      await openVault(tester, c);

      double topOf(Finder f) => tester.getTopLeft(f).dy;
      final sidebar = find.byType(VaultSidebar);
      Finder inRail(Finder f) => find.descendant(of: sidebar, matching: f);

      final search = inRail(find.text('Search…'));
      final actions = inRail(find.byTooltip('Tags'));
      expect(search, findsOneWidget, reason: 'the rail offers search');
      expect(actions, findsOneWidget);
      expect(
        topOf(search),
        lessThan(topOf(actions)),
        reason: 'actions belong at the bottom of the rail',
      );
      await disposeShell(tester, c);
    });
  });

  group('the columns keep the design\'s proportions', () {
    // The mockup is a 1200px frame with a 260 sidebar and a 280 drawer. Those
    // were pinned, so at 1200 the layout matched exactly and at 1700 the side
    // columns stayed put while the note pane swallowed every extra pixel.
    testWidgets('the sidebar is the design width at the design frame', (
      tester,
    ) async {
      final c = shellContainer();
      await pumpShell(tester, c, size: const Size(1200, 900));
      await openVault(tester, c);

      expect(tester.getSize(find.byType(VaultSidebar)).width, 260);
      await disposeShell(tester, c);
    });

    testWidgets('and grows with the window rather than staying put', (
      tester,
    ) async {
      final c = shellContainer();
      await pumpShell(tester, c, size: const Size(1800, 900));
      await openVault(tester, c);

      final width = tester.getSize(find.byType(VaultSidebar)).width;
      expect(width, greaterThan(260));
      // Capped: a 4K display must not hand a third of the screen to a list of
      // folder names.
      expect(width, lessThanOrEqualTo(400));
      await disposeShell(tester, c);
    });

    testWidgets('never below the design width, however narrow the pane', (
      tester,
    ) async {
      final c = shellContainer();
      await pumpShell(tester, c, size: const Size(901, 900));
      await openVault(tester, c);

      expect(tester.getSize(find.byType(VaultSidebar)).width, 260);
      await disposeShell(tester, c);
    });
  });

  String locationOf(ProviderContainer c) =>
      c.read(routerProvider).state.uri.path;

  group('the vault switcher', () {
    testWidgets('opens the switcher rather than navigating home', (
      tester,
    ) async {
      // It used to `go(dashboard)` on tap, so the one control labelled with
      // the current vault navigated away from every vault.
      final c = shellContainer();
      serverOf(c).addVault('v-second', 'Second');
      await pumpShell(tester, c, size: desk);
      await openVault(tester, c);
      final before = locationOf(c);

      await tester.tap(find.text('Primary'));
      await tester.pumpAndSettle();

      expect(locationOf(c), before, reason: 'opening a menu is not navigation');
      expect(find.text('Second'), findsOneWidget, reason: 'the other vault');
      await disposeShell(tester, c);
    });

    testWidgets('and server settings are the entry beneath the vaults', (
      tester,
    ) async {
      // Not "All vaults": there is no dashboard at this width, and an entry
      // that navigates to a screen the layout does not have is worse than no
      // entry at all.
      final c = shellContainer();
      await pumpShell(tester, c, size: desk);
      await openVault(tester, c);

      await tester.tap(find.text('Primary'));
      await tester.pumpAndSettle();
      expect(find.text('All vaults'), findsNothing);

      await tester.tap(find.text('Server settings ›'));
      await tester.pumpAndSettle();

      expect(locationOf(c), Routes.serverSettings);
      await disposeShell(tester, c);
    });
  });

  group('the folder tree', () {
    testWidgets('indents a note under the folder that holds it', (
      tester,
    ) async {
      // `inTree` used to be inferred from `leading != null`, which silently
      // stopped being true for notes the day the leading spacer was removed —
      // so every note in the tree rendered at the full-width list's size and
      // the hierarchy flattened.
      final c = shellContainer();
      await pumpShell(tester, c, size: desk);
      await openVault(tester, c);

      final sidebar = find.byType(VaultSidebar);
      Finder inRail(Finder f) => find.descendant(of: sidebar, matching: f);

      await tester.tap(inRail(find.text('Daily')));
      await tester.pumpAndSettle();

      final folder = tester.getTopLeft(inRail(find.text('Daily'))).dx;
      final note = tester.getTopLeft(inRail(find.text('2026-08-05'))).dx;
      expect(
        note,
        greaterThan(folder),
        reason: 'a child sits in from the folder that holds it',
      );
      await disposeShell(tester, c);
    });

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

  group('properties are a surface of their own', () {
    // They used to sit inline above the prose, which cost a phone's first
    // screen permanently: a note with a dozen keys pushed the writing off the
    // bottom before a word of it was visible.
    testWidgets('never inside the prose column', (tester) async {
      final c = shellContainer();
      await pumpShell(tester, c, size: phone);
      c.read(routerProvider).go(Routes.note(FakeServer.primaryVault, 'n0'));
      await tester.pumpAndSettle();

      expect(
        find.descendant(
          of: find.byType(NoteEditor),
          matching: find.byType(NoteProperties),
        ),
        findsNothing,
      );
      await disposeShell(tester, c);
    });

    testWidgets('open as a drawer beside the note on a wide screen', (
      tester,
    ) async {
      final c = shellContainer();
      await pumpShell(tester, c, size: desk);
      c.read(routerProvider).go(Routes.note(FakeServer.primaryVault, 'n0'));
      await tester.pumpAndSettle();

      expect(find.byType(PropertiesDrawer), findsNothing, reason: 'closed');
      await tester.tap(find.byTooltip('Properties'));
      await tester.pumpAndSettle();
      expect(find.byType(PropertiesDrawer), findsOneWidget);

      // And close again, because a drawer that cannot be dismissed is a
      // column. The rail's toggle is what does it: the design draws the
      // drawer's header as the word alone, with no close of its own.
      await tester.tap(find.byTooltip('Properties'));
      await tester.pumpAndSettle();
      expect(find.byType(PropertiesDrawer), findsNothing);
      await disposeShell(tester, c);
    });

    testWidgets('its rule runs the full height, level with the sidebar\'s', (
      tester,
    ) async {
      // The shell used to put an 8px spacer above the pane's Row, so the
      // vertical rules on the rail and the drawer started below the top of
      // the pane with the page showing above them.
      final c = shellContainer();
      await pumpShell(tester, c, size: desk);
      c.read(routerProvider).go(Routes.note(FakeServer.primaryVault, 'n0'));
      await tester.pumpAndSettle();
      await tester.tap(find.byTooltip('Properties'));
      await tester.pumpAndSettle();

      expect(
        tester.getTopLeft(find.byType(PropertiesDrawer)).dy,
        tester.getTopLeft(find.byType(VaultSidebar)).dy,
        reason: 'the columns are the same height, so their rules line up',
      );
      await disposeShell(tester, c);
    });

    testWidgets('open as a sheet on a phone, never a drawer', (tester) async {
      final c = shellContainer();
      await pumpShell(tester, c, size: phone);
      c.read(routerProvider).go(Routes.note(FakeServer.primaryVault, 'n0'));
      await tester.pumpAndSettle();

      await tester.tap(find.byTooltip('Properties'));
      await tester.pumpAndSettle();

      expect(find.byType(PropertiesDrawer), findsNothing);
      expect(find.byType(PropertiesPanel), findsOneWidget);
      expect(find.text('PROPERTIES'), findsOneWidget);
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

    testWidgets('hands off to a vault at desk width instead of showing', (
      tester,
    ) async {
      // Everything the dashboard offers is already in the sidebar at this
      // width — the switcher lists the vaults, the tree is the browser — so a
      // whole screen for it is a page you pass through on the way to the only
      // thing you came for.
      final c = shellContainer();
      await pumpShell(tester, c, size: desk);

      expect(locationOf(c), Routes.browse(FakeServer.primaryVault));
      expect(find.byType(VaultSidebar), findsOneWidget);
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

    testWidgets('but stays when there is no vault to hand off to', (
      tester,
    ) async {
      // It is the only screen that can make one.
      final c = shellContainer();
      serverOf(c).vaults.clear();
      await pumpShell(tester, c, size: desk);

      expect(locationOf(c), Routes.dashboard);
      expect(find.text('No vaults yet'), findsOneWidget);
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
