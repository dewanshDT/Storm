import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:storm/router.dart';
import 'package:storm/ui/accents.dart';
import 'package:storm/ui/browse_screen.dart';
import 'package:storm/ui/note_header.dart';
import 'package:storm/ui/properties_panel.dart';
import 'package:storm/ui/shell/sidebar_rows.dart';
import 'package:storm/ui/shell/vault_sidebar.dart';
import 'package:storm/ui/tokens.dart';

import 'fake_server.dart';
import 'shell_harness.dart';

/// Storm v2 Notes (handoff §2.2–§2.4): every place on both sides of 900.
void main() {
  const phone = Size(411, 900);
  const desk = Size(1280, 900);
  const primary = FakeServer.primaryVault;

  String locationOf(ProviderContainer c) =>
      c.read(routerProvider).state.uri.path;

  /// A second vault with one note, and recents across both vaults: newest
  /// first `n2` (Primary), `w1` (work), `n1`, `n0`, `n3` (Primary).
  ProviderContainer withRecents() {
    final c = shellContainer();
    final s = serverOf(c);
    s.addVault('v-work', 'work')['w1'] = ServerNote(
      id: 'w1',
      path: 'Sprint notes.md',
      content: 'Ship it.\n',
      version: 1,
    );
    s.markOpened(primary, 'n3', '2026-08-07T08:00:00Z');
    s.markOpened(primary, 'n0', '2026-08-07T09:00:00Z');
    s.markOpened(primary, 'n1', '2026-08-07T10:00:00Z');
    s.markOpened('v-work', 'w1', '2026-08-07T11:00:00Z');
    s.markOpened(primary, 'n2', '2026-08-07T12:00:00Z');
    return c;
  }

  group('desktop sidebar', () {
    testWidgets('vault header, search for this vault, and no gear or '
        'mentions in the footer', (tester) async {
      final c = shellContainer();
      await pumpShell(tester, c, size: desk);
      await openVault(tester, c);

      final sidebar = find.byType(VaultSidebar);
      Finder inSidebar(Finder f) => find.descendant(of: sidebar, matching: f);
      expect(inSidebar(find.text('Primary')), findsOneWidget);
      expect(inSidebar(find.byKey(const Key('vault-sync-line'))), findsOne);
      expect(inSidebar(find.text('Search Primary')), findsOneWidget);
      for (final action in ['New note', 'New folder', 'Tags']) {
        expect(inSidebar(find.byTooltip(action)), findsOneWidget);
      }
      expect(inSidebar(find.byTooltip('Mentions')), findsNothing);
      expect(inSidebar(find.byTooltip('Settings')), findsNothing);

      await tester.tap(find.byKey(const Key('sidebar-search')));
      await tester.pumpAndSettle();
      expect(locationOf(c), Routes.search(primary));
      await disposeShell(tester, c);
    });

    testWidgets('RECENT is the four newest notes from every vault, each '
        'tagged with its vault', (tester) async {
      final c = withRecents();
      await pumpShell(tester, c, size: desk);
      await openVault(tester, c);

      final rows = find.byWidgetPredicate(
        (w) =>
            w.key is ValueKey<String> &&
            (w.key! as ValueKey<String>).value.startsWith('recent:'),
      );
      expect(rows, findsNWidgets(4));
      double y(String id) => tester.getTopLeft(find.byKey(Key(id))).dy;
      expect(y('recent:n2'), lessThan(y('recent:w1')));
      expect(y('recent:w1'), lessThan(y('recent:n1')));
      expect(y('recent:n1'), lessThan(y('recent:n0')));
      expect(find.byKey(const Key('recent:n3')), findsNothing);
      expect(
        find.descendant(
          of: find.byKey(const Key('recent:w1')),
          matching: find.widgetWithText(VaultTag, 'work'),
        ),
        findsOneWidget,
      );
      await disposeShell(tester, c);
    });

    testWidgets('a recent row in another vault opens it there', (tester) async {
      final c = withRecents();
      await pumpShell(tester, c, size: desk);
      await openVault(tester, c);

      await tester.tap(find.byKey(const Key('recent:w1')));
      await tester.pumpAndSettle();
      expect(locationOf(c), Routes.note('v-work', 'w1'));
      expect(find.text('Search work'), findsOneWidget);
      await disposeShell(tester, c);
    });

    testWidgets('the open note is selected: text on surface2', (tester) async {
      final c = withRecents();
      await pumpShell(tester, c, size: desk);
      c.read(routerProvider).go(Routes.note(primary, 'n1'));
      await tester.pumpAndSettle();

      final t = StormTokens.from(StormPreset.stormDark);
      Color fill(Finder row) => tester
          .widget<Material>(
            find.descendant(of: row, matching: find.byType(Material)).first,
          )
          .color!;
      final recent = find.byKey(const Key('recent:n1'));
      expect(tester.widget<SidebarRow>(recent).selected, isTrue);
      expect(fill(recent), t.surface2);
      expect(
        tester.widget<SidebarRow>(find.byKey(const Key('recent:n0'))).selected,
        isFalse,
      );
      await disposeShell(tester, c);
    });

    testWidgets('RECENT is absent until something has been opened', (
      tester,
    ) async {
      final c = shellContainer();
      await pumpShell(tester, c, size: desk);
      await openVault(tester, c);
      expect(find.text('RECENT'), findsNothing);
      expect(find.text('FOLDERS'), findsOneWidget);
      await disposeShell(tester, c);
    });

    testWidgets('the folder tree carries no counts and indents by depth', (
      tester,
    ) async {
      final c = shellContainer();
      await pumpShell(tester, c, size: desk);
      c.read(routerProvider).go(Routes.note(primary, 'n3'));
      await tester.pumpAndSettle();

      final projects = find.byKey(const ValueKey('folder:Projects'));
      final storm = find.byKey(const ValueKey('folder:Projects/Storm'));
      final design = find.byKey(const ValueKey('note:n3'));
      expect(projects, findsOneWidget);
      expect(storm, findsOneWidget, reason: 'opened to reveal the note');
      double textX(Finder row) => tester
          .getTopLeft(find.descendant(of: row, matching: find.byType(Text)))
          .dx;
      expect(textX(storm), greaterThan(textX(projects)));
      expect(textX(design), greaterThan(textX(storm)));
      expect(
        find.descendant(of: projects, matching: find.text('2')),
        findsNothing,
      );
      await disposeShell(tester, c);
    });

    testWidgets('an empty pane says how to find a note', (tester) async {
      final c = shellContainer();
      await pumpShell(tester, c, size: desk);
      await openVault(tester, c);
      expect(find.textContaining('Select a note, or press'), findsOneWidget);
      await disposeShell(tester, c);
    });
  });

  group('vault switcher', () {
    testWidgets('lists vaults with counts and a check, and Manage vaults', (
      tester,
    ) async {
      final c = shellContainer();
      serverOf(c).addVault('v-second', 'Second');
      await pumpShell(tester, c, size: desk);
      await openVault(tester, c);

      await tester.tap(find.text('Primary'));
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('switcher-current')), findsOneWidget);
      expect(
        find.descendant(
          of: find.byKey(const Key('switcher-vault-$primary')),
          matching: find.byKey(const Key('switcher-current')),
        ),
        findsOneWidget,
      );
      expect(find.text('Server settings ›'), findsNothing);
      expect(find.text('Manage vaults ›'), findsOneWidget);

      await tester.tap(find.byKey(const Key('switcher-vault-v-second')));
      await tester.pumpAndSettle();
      expect(locationOf(c), Routes.browse('v-second'));
      await disposeShell(tester, c);
    });

    testWidgets('a long-press colours that vault (Q7)', (tester) async {
      final c = shellContainer();
      serverOf(c).addVault('v-second', 'Second');
      await pumpShell(tester, c, size: desk);
      await openVault(tester, c);

      await tester.tap(find.text('Primary'));
      await tester.pumpAndSettle();
      await tester.longPress(find.byKey(const Key('switcher-vault-v-second')));
      await tester.pumpAndSettle();
      expect(find.byType(AccentPicker), findsOneWidget);

      await tester.tap(find.byTooltip(Accent.mint.label));
      await tester.pumpAndSettle();
      final config = serverOf(c).byVault['v-second']!.values
          .where((n) => n.path == '_storm/vault.md')
          .single;
      expect(config.content, contains('storm.color: mint'));
      await disposeShell(tester, c);
    });

    testWidgets('and so does a long-press in the phone place picker', (
      tester,
    ) async {
      final c = shellContainer();
      serverOf(c).addVault('v-second', 'Second');
      await pumpShell(tester, c, size: phone);
      await openVault(tester, c);

      await tester.tap(find.byKey(const Key('places-bubble')));
      await tester.pumpAndSettle();
      await tester.longPress(find.byKey(const Key('place-vault-v-second')));
      await tester.pumpAndSettle();
      expect(find.byType(AccentPicker), findsOneWidget);
      expect(locationOf(c), Routes.browse(primary), reason: 'not a switch');
      await disposeShell(tester, c);
    });
  });

  group('note header', () {
    ProviderContainer withPlainNote() {
      final c = shellContainer();
      serverOf(c).notes['p1'] = ServerNote(
        id: 'p1',
        path: 'Projects/Storm/BOARD.md',
        content: '---\nstatus: active\n---\n\n## Now\n\n- one\n',
        version: 51,
      );
      return c;
    }

    testWidgets('desktop: crumb, Read | Edit, Start session, drawer toggle, '
        'title and version line', (tester) async {
      final c = withPlainNote();
      await pumpShell(tester, c, size: desk);
      c.read(routerProvider).go(Routes.note(primary, 'p1'));
      await tester.pumpAndSettle();

      expect(find.text('Primary / Projects / Storm'), findsOneWidget);
      expect(find.byKey(const Key('mode-read')), findsOneWidget);
      expect(find.text('Start session'), findsOneWidget);
      expect(find.byKey(const Key('drawer-toggle')), findsOneWidget);
      expect(find.byKey(const Key('back-link')), findsNothing);
      expect(find.byTooltip('Note actions'), findsNothing);
      expect(find.text('BOARD'), findsWidgets);
      expect(find.byKey(const Key('note-title')), findsOneWidget);
      final line = tester.widget<Text>(find.byKey(const Key('version-line')));
      expect(line.textSpan!.toPlainText(), 'v51 · Saved');
      expect(find.byTooltip('Directory'), findsNothing);
      await disposeShell(tester, c);
    });

    testWidgets('phone: "‹ folder", Session, actions and properties; no '
        'crumb, no drawer toggle, no pill', (tester) async {
      final c = withPlainNote();
      await pumpShell(tester, c, size: phone);
      await openVault(tester, c);
      c.read(routerProvider).go(Routes.note(primary, 'p1'));
      await tester.pumpAndSettle();

      expect(find.text('‹ Storm'), findsOneWidget);
      expect(find.text('Session'), findsOneWidget);
      expect(find.byKey(const Key('note-crumb')), findsNothing);
      expect(find.byKey(const Key('drawer-toggle')), findsNothing);
      expect(find.byTooltip('Directory'), findsNothing);

      await tester.tap(find.byTooltip('Properties'));
      await tester.pumpAndSettle();
      expect(find.byType(PropertiesPanel), findsOneWidget, reason: 'Q6');
      await disposeShell(tester, c);
    });

    testWidgets('phone: the back link leads to the note\'s folder', (
      tester,
    ) async {
      final c = withPlainNote();
      await pumpShell(tester, c, size: phone);
      c.read(routerProvider).go(Routes.note(primary, 'p1'));
      await tester.pumpAndSettle();

      await tester.tap(find.byKey(const Key('back-link')));
      await tester.pumpAndSettle();
      expect(locationOf(c), Routes.folder(primary, 'Projects/Storm'));
      await disposeShell(tester, c);
    });

    for (final size in [phone, desk]) {
      testWidgets('Start session goes to Agents when no host is online at '
          '${size.width.toInt()}px', (tester) async {
        final c = withPlainNote();
        await pumpShell(tester, c, size: size);
        c.read(routerProvider).go(Routes.note(primary, 'p1'));
        await tester.pumpAndSettle();

        await tester.tap(find.byKey(const Key('start-session')));
        await tester.pumpAndSettle();
        expect(locationOf(c), Routes.agents);
        await disposeShell(tester, c);
      });
    }

    test('the title is the file name unless the body opens with a heading', () {
      expect(displayTitleFor('a/BOARD.md', '## Now\n'), 'BOARD');
      expect(
        displayTitleFor('Gateway spec.md', '\n\nStorm holds'),
        'Gateway spec',
      );
      expect(displayTitleFor('Welcome.md', '\n# Welcome\n\nbody'), isNull);
    });

    testWidgets('a note with its own # heading gets no second title', (
      tester,
    ) async {
      final c = shellContainer();
      await pumpShell(tester, c, size: desk);
      c.read(routerProvider).go(Routes.note(primary, 'n0'));
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('note-title')), findsNothing);
      await disposeShell(tester, c);
    });
  });

  group('phone notes list', () {
    testWidgets('vault root: the vault as title, RECENT with where each '
        'lives, then FOLDERS', (tester) async {
      final c = withRecents();
      await pumpShell(tester, c, size: phone);
      await openVault(tester, c);

      expect(find.byKey(const Key('browse-title')), findsOneWidget);
      expect(
        tester.widget<Text>(find.byKey(const Key('browse-title'))).data,
        'Primary',
      );
      expect(find.text('RECENT'), findsOneWidget);
      expect(find.text('FOLDERS'), findsOneWidget);
      expect(find.byKey(const Key('recent:n3')), findsNothing, reason: 'four');
      expect(find.text('Primary · Daily'), findsNWidgets(2), reason: 'n1, n2');
      expect(find.text('work'), findsOneWidget, reason: 'w1, at its root');
      expect(
        tester.getTopLeft(find.text('RECENT')).dy,
        lessThan(tester.getTopLeft(find.text('FOLDERS')).dy),
      );
      final daily = find.byKey(const ValueKey('folder:Daily'));
      expect(
        find.descendant(of: daily, matching: find.text('2')),
        findsOneWidget,
      );
      expect(find.text('‹ Primary'), findsNothing, reason: 'the root');
      await disposeShell(tester, c);
    });

    testWidgets('a folder: "‹ parent" and its name, and back up', (
      tester,
    ) async {
      final c = shellContainer();
      await pumpShell(tester, c, size: phone);
      c.read(routerProvider).go(Routes.folder(primary, 'Projects/Storm'));
      await tester.pumpAndSettle();

      expect(find.text('‹ Projects'), findsOneWidget);
      expect(
        tester.widget<Text>(find.byKey(const Key('browse-title'))).data,
        'Storm',
      );
      expect(find.text('RECENT'), findsNothing);
      await tester.tap(find.byKey(const Key('back-link')));
      await tester.pumpAndSettle();
      expect(locationOf(c), Routes.folder(primary, 'Projects'));
      await disposeShell(tester, c);
    });

    testWidgets('at desk width the browse pane is not the list', (
      tester,
    ) async {
      final c = withRecents();
      await pumpShell(tester, c, size: desk);
      await openVault(tester, c);
      expect(find.byKey(const Key('browse-title')), findsNothing);
      expect(find.byType(EntryTile), findsNothing);
      await disposeShell(tester, c);
    });

    test('ages are short', () {
      final now = DateTime.now();
      expect(shortAge(now.subtract(const Duration(hours: 2))), '2h');
      expect(shortAge(now.subtract(const Duration(days: 4))), '4d');
      expect(shortAge(now.subtract(const Duration(minutes: 5))), '5m');
      expect(shortAge(null), '');
    });
  });

  testWidgets('a note opened in another vault is recorded for RECENT', (
    tester,
  ) async {
    final c = withRecents();
    await pumpShell(tester, c, size: desk);
    c.read(routerProvider).go(Routes.note(primary, 'n0'));
    await tester.pumpAndSettle();
    c.read(routerProvider).go(Routes.note('v-work', 'w1'));
    await tester.pumpAndSettle();

    expect(serverOf(c).opened['v-work']?['w1'], '2026-08-07T12:00:00Z');
    await disposeShell(tester, c);
  });

  testWidgets('a window resized across 900 with a popover open stays sound', (
    tester,
  ) async {
    final c = shellContainer();
    await pumpShell(tester, c, size: desk);
    c.read(routerProvider).go(Routes.note(primary, 'n0'));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('vault-sync-line')));
    await tester.pumpAndSettle();

    tester.view.physicalSize = phone;
    await tester.pumpAndSettle();
    c.read(routerProvider).go(Routes.browse(primary));
    await tester.pumpAndSettle();
    expect(tester.takeException(), isNull);
    expect(find.byKey(const Key('browse-title')), findsOneWidget);
    await disposeShell(tester, c);
  });

  testWidgets('an unseen note shows the accent dot with its tooltip', (
    tester,
  ) async {
    await tester.pumpWidget(
      MaterialApp(
        home: Scaffold(body: const SidebarRow(label: 'BOARD', unseen: true)),
      ),
    );
    expect(find.byType(UnseenDot), findsOneWidget);
    expect(
      find.byTooltip('Changed by an agent since you last opened it'),
      findsOneWidget,
    );
  });
}
