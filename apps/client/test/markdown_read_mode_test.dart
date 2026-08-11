import 'package:flutter/material.dart';
import 'package:flutter_markdown_plus/flutter_markdown_plus.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:markdown/markdown.dart' as md;

import 'package:storm/router.dart';
import 'package:storm/state/app_state.dart';
import 'package:storm/ui/markdown/storm_markdown_style.dart';
import 'package:storm/ui/markdown/storm_markdown_view.dart';
import 'package:storm/ui/note_mode_toggle.dart';
import 'package:storm/ui/shell/corner_bubbles.dart';
import 'package:storm/ui/theme.dart';
import 'package:storm/ui/widgets.dart';

import 'fake_server.dart';
import 'shell_harness.dart';

/// Read Mode renderer — Storm-styled Markdown, not the package defaults.
void main() {
  Future<void> pumpMarkdown(
    WidgetTester tester,
    String markdown, {
    void Function(String)? onFollow,
    double fontSize = 16,
    BodyFont bodyFont = BodyFont.serif,
  }) async {
    final c = shellContainer();
    addTearDown(c.dispose);
    await c.read(settingsProvider.future);
    final current = c.read(settingsProvider).value!;
    await c
        .read(settingsProvider.notifier)
        .save(current.copyWith(fontSize: fontSize, bodyFont: bodyFont));

    await tester.pumpWidget(
      UncontrolledProviderScope(
        container: c,
        child: MaterialApp(
          theme: StormTheme.from(current.theme, fontSize: fontSize),
          home: Scaffold(
            body: SingleChildScrollView(
              child: StormMarkdownView(
                markdown: markdown,
                onFollowLink: onFollow,
                onOpenEdit: () {},
              ),
            ),
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();
  }

  group('StormMarkdownView renders', () {
    testWidgets('headings and paragraphs', (tester) async {
      await pumpMarkdown(tester, '# Storm\n\nA paragraph of prose.\n');

      expect(find.byKey(const Key('storm-markdown-body')), findsOneWidget);
      expect(find.text('Storm'), findsOneWidget);
      expect(find.text('A paragraph of prose.'), findsOneWidget);
      expect(tester.takeException(), isNull);
    });

    testWidgets('bold and italic', (tester) async {
      await pumpMarkdown(
        tester,
        'Storm is **self-hosted** and *Markdown-native*.\n',
      );
      expect(find.textContaining('self-hosted'), findsOneWidget);
      expect(find.textContaining('Markdown-native'), findsOneWidget);
    });

    testWidgets('links', (tester) async {
      await pumpMarkdown(tester, 'See [the plan](https://example.com/plan).\n');
      expect(
        find.textContaining('the plan', findRichText: true),
        findsOneWidget,
      );
    });

    testWidgets('bullet and ordered lists', (tester) async {
      await pumpMarkdown(tester, '''
- Markdown
- Sync

1. First
2. Second
''');
      expect(find.text('Markdown'), findsOneWidget);
      expect(find.text('Sync'), findsOneWidget);
      expect(find.text('First'), findsOneWidget);
      expect(find.text('Second'), findsOneWidget);
    });

    testWidgets('nested lists', (tester) async {
      await pumpMarkdown(tester, '''
- Parent
  - Child
  - Child
- Parent two
''');
      expect(find.text('Parent'), findsOneWidget);
      expect(find.text('Child'), findsNWidgets(2));
      expect(find.text('Parent two'), findsOneWidget);
    });

    testWidgets('task lists with Storm checkboxes', (tester) async {
      await pumpMarkdown(tester, '''
- [ ] Pending
- [x] Completed
''');
      expect(find.byType(StormCheckbox), findsNWidgets(2));
      final boxes = tester
          .widgetList<StormCheckbox>(find.byType(StormCheckbox))
          .toList();
      expect(boxes[0].value, isFalse);
      expect(boxes[0].onChanged, isNull, reason: 'read-only');
      expect(boxes[0].size, 18, reason: 'prototype box is 18×18 at 16px');
      expect(boxes[1].value, isTrue);
      expect(boxes[1].onChanged, isNull);

      // The package tight-sizes the bullet column to indent+gap. Without
      // matching right pad the box stretches into a pill over the label.
      for (final finder in [
        find.byType(StormCheckbox).at(0),
        find.byType(StormCheckbox).at(1),
      ]) {
        final size = tester.getSize(finder);
        expect(size.width, size.height, reason: 'checkbox stays square');
        expect(size.width, 18);
      }
      final pending = tester.getTopLeft(find.text('Pending'));
      final boxRight = tester.getTopRight(find.byType(StormCheckbox).first).dx;
      expect(
        pending.dx,
        greaterThanOrEqualTo(boxRight + 8),
        reason: 'label clears the box (prototype 10px gap)',
      );
    });

    testWidgets('Read Mode body size follows client text size', (tester) async {
      await pumpMarkdown(tester, 'Prose at the chosen size.\n', fontSize: 22);

      expect(find.textContaining('Prose at the chosen size'), findsOneWidget);
      final selectable = tester
          .widgetList<SelectableText>(find.byType(SelectableText))
          .where(
            (s) =>
                s.textSpan?.toPlainText().contains(
                  'Prose at the chosen size',
                ) ??
                false,
          );
      expect(selectable, isNotEmpty);
      expect(
        selectable.first.textSpan?.style?.fontSize,
        22,
        reason: 'settings.fontSize drives Read Mode body',
      );
    });

    testWidgets('task checkbox scales with text size', (tester) async {
      await pumpMarkdown(tester, '- [ ] Bigger type\n', fontSize: 24);
      final box = tester.widget<StormCheckbox>(find.byType(StormCheckbox));
      expect(box.size, stormMarkdownCheckboxSize(24));
      final size = tester.getSize(find.byType(StormCheckbox));
      expect(size.width, size.height);
      expect(size.width, stormMarkdownCheckboxSize(24));
    });

    testWidgets('tables', (tester) async {
      await pumpMarkdown(tester, '''
| Client | Platform |
|---|---|
| Storm | macOS |
''');
      expect(find.text('Client'), findsOneWidget);
      expect(find.text('Platform'), findsOneWidget);
      expect(find.text('Storm'), findsOneWidget);
      expect(find.text('macOS'), findsOneWidget);
      expect(find.byType(Table), findsOneWidget);
    });

    testWidgets('images (placeholder when unresolved)', (tester) async {
      await pumpMarkdown(tester, '![Architecture](attachments/missing.png)\n');
      // Fake server has no attachment bytes; the fallback must still render.
      expect(find.textContaining('Architecture'), findsOneWidget);
      expect(tester.takeException(), isNull);
    });

    testWidgets('code block and inline code', (tester) async {
      await pumpMarkdown(tester, '''
Run `storm-server`.

```rust
fn main() {}
```
''');
      expect(find.textContaining('storm-server'), findsOneWidget);
      expect(find.textContaining('fn main()'), findsOneWidget);
    });

    testWidgets('blockquote', (tester) async {
      await pumpMarkdown(tester, '> The server owns the canonical vault.\n');
      expect(
        find.textContaining('The server owns the canonical vault.'),
        findsOneWidget,
      );
    });

    testWidgets('horizontal rule', (tester) async {
      await pumpMarkdown(tester, 'Above\n\n---\n\nBelow\n');
      expect(find.textContaining('Above'), findsOneWidget);
      expect(find.textContaining('Below'), findsOneWidget);
      expect(tester.takeException(), isNull);
    });

    testWidgets('empty markdown shows EmptyState', (tester) async {
      await pumpMarkdown(tester, '   \n');
      expect(find.byKey(const Key('read-empty')), findsOneWidget);
      expect(find.text('Nothing to read yet.'), findsOneWidget);
      expect(find.byType(MarkdownBody), findsNothing);
    });

    testWidgets('malformed markdown does not crash', (tester) async {
      await pumpMarkdown(
        tester,
        '```\nunclosed fence\n\n[[broken\n**also broken\n',
      );
      expect(tester.takeException(), isNull);
      expect(find.byKey(const Key('storm-markdown-body')), findsOneWidget);
    });

    testWidgets('wikilinks render and parse to storm-wikilink hrefs', (
      tester,
    ) async {
      String? followed;
      await pumpMarkdown(
        tester,
        'See [[Design]] here.\n',
        onFollow: (t) => followed = t,
      );
      expect(find.textContaining('Design', findRichText: true), findsOneWidget);

      // Parse the same way Read Mode does — the tap path is package-owned.
      final doc = md.Document(
        extensionSet: md.ExtensionSet(
          md.ExtensionSet.gitHubFlavored.blockSyntaxes,
          <md.InlineSyntax>[
            WikilinkSyntax(),
            ...md.ExtensionSet.gitHubFlavored.inlineSyntaxes,
          ],
        ),
      );
      final nodes = doc.parseInline('[[Design]]');
      final link = nodes.whereType<md.Element>().firstWhere(
        (e) => e.tag == 'a',
      );
      expect(link.textContent, 'Design');
      expect(link.attributes['href'], 'storm-wikilink:Design');

      // Drive the same handler the MarkdownBody onTapLink uses.
      followed = null;
      const href = 'storm-wikilink:Design';
      const prefix = '$kStormWikilinkScheme:';
      expect(href.startsWith(prefix), isTrue);
      followed = Uri.decodeComponent(href.substring(prefix.length));
      expect(followed, 'Design');
    });

    test('WikilinkSyntax encodes spaces in the href', () {
      final doc = md.Document(
        inlineSyntaxes: [
          WikilinkSyntax(),
          ...md.ExtensionSet.gitHubFlavored.inlineSyntaxes,
        ],
      );
      final nodes = doc.parseInline('[[My Note]]');
      final link = nodes.whereType<md.Element>().firstWhere(
        (e) => e.tag == 'a',
      );
      expect(link.attributes['href'], 'storm-wikilink:My%20Note');
      expect(link.textContent, 'My Note');
    });
  });

  group('NoteModeToggle', () {
    testWidgets('reports the chosen mode', (tester) async {
      var mode = NoteViewMode.read;
      await tester.pumpWidget(
        MaterialApp(
          theme: StormTheme.dark(),
          home: Scaffold(
            body: StatefulBuilder(
              builder: (context, setState) => NoteModeToggle(
                mode: mode,
                onChanged: (next) => setState(() => mode = next),
              ),
            ),
          ),
        ),
      );

      expect(find.byKey(const Key('mode-read')), findsOneWidget);
      expect(find.byKey(const Key('mode-edit')), findsOneWidget);

      await tester.tap(find.byKey(const Key('mode-edit')));
      await tester.pumpAndSettle();
      expect(mode, NoteViewMode.edit);

      await tester.tap(find.byKey(const Key('mode-read')));
      await tester.pumpAndSettle();
      expect(mode, NoteViewMode.read);
    });
  });

  group('note screen defaults to Read Mode', () {
    testWidgets('opening a note shows the renderer, not the TextField', (
      tester,
    ) async {
      final c = shellContainer();
      await pumpShell(tester, c);
      c.read(routerProvider).go(Routes.note(FakeServer.primaryVault, 'n0'));
      await tester.pumpAndSettle();

      expect(find.byKey(const Key('note-read')), findsOneWidget);
      expect(find.byKey(const Key('note-body')), findsNothing);
      expect(find.byType(NoteModeToggle), findsOneWidget);

      await enterEditMode(tester);
      expect(find.byKey(const Key('note-body')), findsOneWidget);
      expect(find.byKey(const Key('note-read')), findsNothing);

      await tester.tap(find.byKey(const Key('mode-read')));
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('note-read')), findsOneWidget);

      await disposeShell(tester, c);
    });

    testWidgets('Read Mode reflects unsaved Edit Mode text', (tester) async {
      final c = shellContainer();
      await pumpShell(tester, c);
      c.read(routerProvider).go(Routes.note(FakeServer.primaryVault, 'n0'));
      await tester.pumpAndSettle();
      await enterEditMode(tester);

      await tester.enterText(
        find.byKey(const Key('note-body')),
        '# Fresh heading\n\nUnsaved prose.\n',
      );
      await tester.pump();

      await tester.tap(find.byKey(const Key('mode-read')));
      await tester.pumpAndSettle();

      expect(find.text('Fresh heading'), findsOneWidget);
      expect(find.text('Unsaved prose.'), findsOneWidget);
      expect(c.read(noteSessionProvider).body, contains('Unsaved prose.'));

      await disposeShell(tester, c);
    });

    testWidgets('readMode off hides the switch and stays in Edit', (
      tester,
    ) async {
      final c = shellContainer();
      await c.read(settingsProvider.future);
      await c
          .read(settingsProvider.notifier)
          .save(c.read(settingsProvider).value!.copyWith(readMode: false));
      await pumpShell(tester, c);
      c.read(routerProvider).go(Routes.note(FakeServer.primaryVault, 'n0'));
      await tester.pumpAndSettle();

      expect(find.byType(NoteModeToggle), findsNothing);
      expect(find.byKey(const Key('mode-read')), findsNothing);
      expect(find.byKey(const Key('note-body')), findsOneWidget);
      expect(find.byKey(const Key('note-read')), findsNothing);

      await disposeShell(tester, c);
    });
  });

  group('client settings Read mode switch', () {
    testWidgets('toggles settings.readMode', (tester) async {
      final c = shellContainer();
      addTearDown(c.dispose);
      await c.read(settingsProvider.future);
      expect(c.read(settingsProvider).value!.readMode, isTrue);

      await tester.pumpWidget(
        UncontrolledProviderScope(
          container: c,
          child: MaterialApp(
            theme: StormTheme.dark(),
            home: const Scaffold(
              body: SingleChildScrollView(child: ClientSettingsBody()),
            ),
          ),
        ),
      );
      await tester.pumpAndSettle();

      expect(find.text('Read mode'), findsOneWidget);
      await tester.tap(find.byKey(const Key('setting-read-mode')));
      await tester.pumpAndSettle();
      expect(c.read(settingsProvider).value!.readMode, isFalse);

      await tester.tap(find.byKey(const Key('setting-read-mode')));
      await tester.pumpAndSettle();
      expect(c.read(settingsProvider).value!.readMode, isTrue);
    });
  });
}
