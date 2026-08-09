import 'package:flutter/material.dart';
import 'package:flutter_lucide/flutter_lucide.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:storm/api/models.dart';
import 'package:storm/editor/markdown_theme.dart';
import 'package:storm/editor/storm_markdown_controller.dart';
import 'package:storm/state/wikilinks.dart';
import 'package:storm/ui/editor_toolbar.dart';

void main() {
  NoteMeta meta(String path, {String title = ''}) => NoteMeta(
    id: path,
    path: path,
    title: title,
    version: 1,
    modified: '2026-08-05T10:00:00Z',
    size: 0,
  );

  group('resolveWikilink', () {
    final notes = [
      meta('Welcome.md', title: 'Welcome to Storm'),
      meta('Daily/2026-08-05.md'),
      meta('Projects/Storm/Design.md', title: 'Storm Design'),
      meta('Projects/Ideas.md'),
    ];

    test('matches a bare filename', () {
      expect(resolveWikilink(notes, 'Ideas')?.path, 'Projects/Ideas.md');
      expect(
        resolveWikilink(notes, 'Design')?.path,
        'Projects/Storm/Design.md',
      );
    });

    test('matches a full path, with or without the extension', () {
      expect(
        resolveWikilink(notes, 'Daily/2026-08-05')?.path,
        'Daily/2026-08-05.md',
      );
      expect(
        resolveWikilink(notes, 'Daily/2026-08-05.md')?.path,
        'Daily/2026-08-05.md',
      );
    });

    test('matches a frontmatter title', () {
      expect(resolveWikilink(notes, 'Welcome to Storm')?.path, 'Welcome.md');
      expect(
        resolveWikilink(notes, 'Storm Design')?.path,
        'Projects/Storm/Design.md',
      );
    });

    test('falls back to a case-insensitive match', () {
      expect(resolveWikilink(notes, 'ideas')?.path, 'Projects/Ideas.md');
      expect(resolveWikilink(notes, 'welcome to storm')?.path, 'Welcome.md');
    });

    test('prefers an exact match over a caseless one', () {
      final both = [meta('note.md'), meta('Note.md')];
      expect(resolveWikilink(both, 'Note')?.path, 'Note.md');
    });

    test('ignores a heading anchor or an alias', () {
      expect(resolveWikilink(notes, 'Ideas#Later')?.path, 'Projects/Ideas.md');
      expect(
        resolveWikilink(notes, 'Ideas|other things')?.path,
        'Projects/Ideas.md',
      );
    });

    test('returns null rather than inventing a note', () {
      expect(resolveWikilink(notes, 'Nothing Here'), isNull);
      expect(resolveWikilink(notes, ''), isNull);
      expect(resolveWikilink(notes, '   '), isNull);
      expect(resolveWikilink(const [], 'Ideas'), isNull);
    });
  });

  group('the toolbar goes through the controller and nowhere else', () {
    testWidgets('every button lands on an editing method', (tester) async {
      // The rule the whole stage rests on. A button that assigned
      // `controller.text` directly would leave the caret pointing into a
      // buffer that no longer matches it.
      final controller = _RecordingController();

      await tester.pumpWidget(
        MaterialApp(
          home: Scaffold(
            body: Align(
              alignment: Alignment.bottomCenter,
              child: EditorToolbar(controller: controller),
            ),
          ),
        ),
      );

      for (final (icon, expected) in [
        (LucideIcons.bold, 'toggleInline(**)'),
        (LucideIcons.italic, 'toggleInline(*)'),
        (LucideIcons.code, 'toggleInline(`)'),
        (LucideIcons.strikethrough, 'toggleInline(~~)'),
        (LucideIcons.highlighter, 'toggleInline(==)'),
        (LucideIcons.list, 'setBlockPrefix(- )'),
        (LucideIcons.list_ordered, 'setBlockPrefix(1. )'),
        (LucideIcons.square_check, 'setBlockPrefix(- [ ] )'),
        (LucideIcons.text_quote, 'setBlockPrefix(> )'),
        (LucideIcons.link, 'insertWikilink()'),
      ]) {
        controller.calls.clear();
        await tester.tap(find.byIcon(icon), warnIfMissed: false);
        await tester.pump();
        expect(controller.calls, [expected], reason: 'button $icon');
      }

      expect(
        controller.directWrites,
        0,
        reason: 'the toolbar assigned text or value itself',
      );
    });

    testWidgets('dismissing the heading menu changes nothing', (tester) async {
      // "Paragraph" and a dismissed menu both come back as null from showMenu;
      // conflating them would strip the heading of anyone who tapped away.
      final controller = _RecordingController();

      await tester.pumpWidget(
        MaterialApp(
          home: Scaffold(
            body: Align(
              alignment: Alignment.bottomCenter,
              child: EditorToolbar(controller: controller),
            ),
          ),
        ),
      );

      await tester.tap(find.byIcon(LucideIcons.heading));
      await tester.pumpAndSettle();
      expect(find.text('Heading 2'), findsOneWidget);

      // Tap outside the menu to dismiss it.
      await tester.tapAt(const Offset(5, 5));
      await tester.pumpAndSettle();

      expect(controller.calls, isEmpty);
    });

    testWidgets('picking a heading level sets that prefix', (tester) async {
      final controller = _RecordingController();

      await tester.pumpWidget(
        MaterialApp(
          home: Scaffold(
            body: Align(
              alignment: Alignment.bottomCenter,
              child: EditorToolbar(controller: controller),
            ),
          ),
        ),
      );

      await tester.tap(find.byIcon(LucideIcons.heading));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Heading 3'));
      await tester.pumpAndSettle();

      expect(controller.calls, ['setBlockPrefix(### , explicit)']);
    });

    testWidgets('Paragraph strips the prefix', (tester) async {
      final controller = _RecordingController();

      await tester.pumpWidget(
        MaterialApp(
          home: Scaffold(
            body: Align(
              alignment: Alignment.bottomCenter,
              child: EditorToolbar(controller: controller),
            ),
          ),
        ),
      );

      await tester.tap(find.byIcon(LucideIcons.heading));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Paragraph'));
      await tester.pumpAndSettle();

      expect(controller.calls, ['setBlockPrefix(null, explicit)']);
    });
  });
}

/// Records which editing methods the toolbar reaches for, and notices if it
/// reaches past them to the raw value.
class _RecordingController extends StormMarkdownController {
  _RecordingController()
    : super(theme: MarkdownTheme.dark(const TextStyle()), text: 'x');

  final List<String> calls = [];
  int directWrites = 0;

  @override
  void toggleInline(String marker) => calls.add('toggleInline($marker)');

  @override
  void setBlockPrefix(String? prefix, {bool toggle = true}) =>
      calls.add('setBlockPrefix($prefix${toggle ? '' : ', explicit'})');

  @override
  void insertWikilink() => calls.add('insertWikilink()');

  @override
  set value(TextEditingValue newValue) {
    directWrites++;
    super.value = newValue;
  }
}
