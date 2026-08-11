import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:storm/api/models.dart';
import 'package:storm/editor/markdown_theme.dart';
import 'package:storm/editor/storm_markdown_controller.dart';
import 'package:storm/router.dart';
import 'package:storm/state/wikilinks.dart';
import 'package:storm/ui/wikilink_suggestions.dart';

import 'shell_harness.dart';
import 'fake_server.dart';

/// Completing a `[[` as you type it.
///
/// The last piece `docs/storm-ui-refactor.md` §2.6 assumed already existed.
/// Typing a full note name by hand on a phone is the difference between links
/// being usable and being theoretical.
void main() {
  NoteMeta meta(
    String path, {
    String title = '',
    String modified = '2026-08-01',
  }) => NoteMeta(
    id: path,
    path: path,
    title: title,
    version: 1,
    modified: modified,
    size: 0,
  );

  StormMarkdownController on(String text, {required int caret}) {
    final c = StormMarkdownController(
      theme: MarkdownTheme.dark(const TextStyle()),
      text: text,
    );
    c.selection = TextSelection.collapsed(offset: caret);
    return c;
  }

  group('spotting the query', () {
    test('an open link is a query', () {
      final q = activeWikilinkQuery('see [[desi', 10);
      expect(q?.query, 'desi');
      expect(q?.start, 6);
      expect(q?.end, 10);
    });

    test('empty brackets are a query with an empty term', () {
      expect(activeWikilinkQuery('see [[', 6)?.query, '');
    });

    test('the caret inside the toolbar-inserted brackets counts', () {
      // The link button produces `[[]]` with the caret in the middle.
      expect(activeWikilinkQuery('see [[]]', 6)?.query, '');
    });

    test('a finished link is not a query', () {
      expect(activeWikilinkQuery('see [[Design]] more', 19), isNull);
      expect(activeWikilinkQuery('see [[Design]]', 14), isNull);
    });

    test('plain text is not a query', () {
      expect(activeWikilinkQuery('nothing here', 6), isNull);
      expect(activeWikilinkQuery('', 0), isNull);
    });

    test('a link does not span lines', () {
      expect(activeWikilinkQuery('[[open\nnext line', 16), isNull);
    });

    test('tolerates out-of-range offsets', () {
      expect(activeWikilinkQuery('see [[a', -1), isNull);
      expect(activeWikilinkQuery('see [[a', 99), isNull);
    });
  });

  group('ranking', () {
    final notes = [
      meta('Design.md'),
      meta('Redesign Notes.md'),
      meta('Projects/Design System.md'),
      meta('Daily/2026-08-05.md', modified: '2026-08-05'),
    ];

    test('prefix matches beat contained ones', () {
      final names = suggestWikilinks(notes, 'des').map((n) => n.path);
      expect(names.first, 'Design.md');
      expect(names, contains('Redesign Notes.md'));
    });

    test('shorter names win ties, so the exact thing is first', () {
      final names = suggestWikilinks(
        notes,
        'design',
      ).map((n) => n.path).toList();
      expect(names.first, 'Design.md');
      expect(names[1], 'Projects/Design System.md');
    });

    test('matches a folder in the path too', () {
      expect(suggestWikilinks(notes, 'projects').map((n) => n.path), [
        'Projects/Design System.md',
      ]);
    });

    test('an empty query offers recent notes rather than nothing', () {
      // Opening `[[` on a phone should show somewhere to go.
      final first = suggestWikilinks(notes, '').first;
      expect(first.path, 'Daily/2026-08-05.md', reason: 'most recently edited');
    });

    test('no match yields nothing', () {
      expect(suggestWikilinks(notes, 'zzzz'), isEmpty);
    });

    test('the limit is honoured', () {
      final many = [for (var i = 0; i < 40; i++) meta('note$i.md')];
      expect(suggestWikilinks(many, 'note').length, 8);
    });
  });

  group('what gets written', () {
    test('a bare name when it is unambiguous', () {
      final notes = [meta('Projects/Design.md'), meta('Other.md')];
      expect(wikilinkTargetFor(notes, notes.first), 'Design');
    });

    test('the full path when two notes share a name', () {
      // An ambiguous link would resolve to whichever the resolver saw first.
      final notes = [meta('Projects/Design.md'), meta('Archive/Design.md')];
      expect(wikilinkTargetFor(notes, notes.first), 'Projects/Design');
    });
  });

  group('completion', () {
    test('replaces the typed query and steps past the brackets', () {
      final c = on('see [[desi]]', caret: 10);
      c.completeWikilink('Design');

      expect(c.text, 'see [[Design]]');
      expect(c.selection.baseOffset, 14, reason: 'past the closing brackets');
      expect(c.selection.isCollapsed, isTrue);
    });

    test('supplies the closing brackets when they are missing', () {
      final c = on('see [[desi', caret: 10);
      c.completeWikilink('Design');

      expect(c.text, 'see [[Design]]');
      expect(c.selection.baseOffset, 14);
    });

    test('leaves the rest of the line alone', () {
      final c = on('a [[d]] and more text', caret: 5);
      c.completeWikilink('Design');
      expect(c.text, 'a [[Design]] and more text');
    });

    test('does nothing when the caret is not in a link', () {
      final c = on('plain text', caret: 5);
      c.completeWikilink('Design');
      expect(c.text, 'plain text');
    });

    test('the span tree still flattens to the buffer', () {
      // The invariant every editing path has to hold.
      final c = on('see [[desi]]', caret: 10);
      c.completeWikilink('Design');
      expect(c.selection.baseOffset, lessThanOrEqualTo(c.text.length));
    });
  });

  group('in the editor', () {
    testWidgets('typing [[ offers notes, and tapping one completes the link', (
      tester,
    ) async {
      final c = shellContainer();
      await pumpShell(tester, c, size: const Size(411, 900));
      c.read(routerProvider).go(Routes.note(FakeServer.primaryVault, 'n0'));
      await tester.pumpAndSettle();
      await enterEditMode(tester);
      await pumpShell(tester, c, size: const Size(411, 900), keyboard: 320);
      await tester.pumpAndSettle();

      final field = find.byType(TextField);
      await tester.tap(field);
      await tester.pumpAndSettle();
      final ctrl = tester.widget<TextField>(field).controller!;

      expect(
        find.byType(ActionChip),
        findsNothing,
        reason: 'nothing typed yet',
      );

      // Open a link at the end of the note.
      ctrl.value = TextEditingValue(
        text: '${ctrl.text}[[Des',
        selection: TextSelection.collapsed(offset: ctrl.text.length + 5),
      );
      await tester.pumpAndSettle();

      // Only Projects/Storm/Design.md matches "Des".
      expect(find.byType(ActionChip), findsOneWidget);

      await tester.tap(find.byType(ActionChip));
      await tester.pumpAndSettle();

      expect(ctrl.text, endsWith('[[Design]]'));
      expect(
        find.byType(ActionChip),
        findsNothing,
        reason: 'a finished link is not a query',
      );

      await disposeShell(tester, c);
    });

    testWidgets('the strip is absent when the keyboard is down', (
      tester,
    ) async {
      final c = shellContainer();
      await pumpShell(tester, c, size: const Size(411, 900));
      c.read(routerProvider).go(Routes.note(FakeServer.primaryVault, 'n0'));
      await tester.pumpAndSettle();

      expect(find.byType(WikilinkSuggestions), findsNothing);
      await disposeShell(tester, c);
    });
  });
}
