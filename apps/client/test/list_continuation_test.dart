import 'package:flutter/material.dart';
import 'package:flutter_lucide/flutter_lucide.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:storm/editor/list_continuation.dart';
import 'package:storm/editor/markdown_theme.dart';
import 'package:storm/editor/storm_markdown_controller.dart';
import 'package:storm/ui/editor_toolbar.dart';

/// Reported from the phone, all three at once: headings did nothing, numbered
/// lists inserted `1. ` on every line, and Enter did not carry a list on.
void main() {
  StormMarkdownController controllerOn(
    String text, {
    required int base,
    int? extent,
  }) {
    final c = StormMarkdownController(
      theme: MarkdownTheme.dark(const TextStyle()),
      text: text,
    );
    c.selection = TextSelection(baseOffset: base, extentOffset: extent ?? base);
    return c;
  }

  group('numbered lists count', () {
    test('a multi-line selection numbers 1, 2, 3', () {
      final c = controllerOn('one\ntwo\nthree', base: 0, extent: 13);
      c.setBlockPrefix('1. ');
      expect(c.text, '1. one\n2. two\n3. three');
    });

    test('numbering continues the list above the selection', () {
      // Extending a list must pick up where it left off, not restart.
      final c = controllerOn('1. one\n2. two\nthree', base: 14);
      c.setBlockPrefix('1. ');
      expect(c.text, '1. one\n2. two\n3. three');
    });

    test('a blank line ends the run, so a new list starts at 1', () {
      final c = controllerOn('1. one\n\nfresh', base: 9);
      c.setBlockPrefix('1. ');
      expect(c.text, '1. one\n\n1. fresh');
    });

    test('re-applying to an already numbered block removes it', () {
      final c = controllerOn('1. one\n2. two', base: 0, extent: 13);
      c.setBlockPrefix('1. ');
      expect(c.text, 'one\ntwo');
    });

    test('renumbers a block that was numbered wrongly', () {
      final c = controllerOn('1. one\n1. two\n1. three', base: 0, extent: 22);
      c.setBlockPrefix('1. ', toggle: false);
      expect(c.text, '1. one\n2. two\n3. three');
    });

    test('bullets are still a repeated marker, not a sequence', () {
      final c = controllerOn('one\ntwo', base: 0, extent: 7);
      c.setBlockPrefix('- ');
      expect(c.text, '- one\n- two');
    });
  });

  group('Enter carries a list on', () {
    /// What the formatter produces for Enter pressed at the end of [text].
    TextEditingValue? enterAtEndOf(String text) =>
        listContinuation('$text\n', text.length + 1);

    test('a bullet', () {
      expect(enterAtEndOf('- item')?.text, '- item\n- ');
    });

    test('a numbered item advances the number', () {
      expect(enterAtEndOf('1. one')?.text, '1. one\n2. ');
      expect(enterAtEndOf('9. nine')?.text, '9. nine\n10. ');
    });

    test('keeps the delimiter it found', () {
      expect(enterAtEndOf('1) one')?.text, '1) one\n2) ');
    });

    test('a task continues as an unticked one', () {
      expect(enterAtEndOf('- [ ] todo')?.text, '- [ ] todo\n- [ ] ');
      expect(
        enterAtEndOf('- [x] done')?.text,
        '- [x] done\n- [ ] ',
        reason: 'the new item has not been done yet',
      );
    });

    test('keeps indentation, so nested lists stay nested', () {
      expect(enterAtEndOf('    - item')?.text, '    - item\n    - ');
    });

    test('the caret lands after the new marker', () {
      final v = enterAtEndOf('- item')!;
      expect(v.selection.baseOffset, '- item\n- '.length);
      expect(v.selection.isCollapsed, isTrue);
    });

    test('Enter on an empty item ends the list', () {
      // Otherwise there is no way out of a list except backspacing.
      final v = listContinuation('- one\n- \n', 9)!;
      expect(v.text, '- one\n');
      expect(v.selection.baseOffset, 6);
    });

    test('an ordinary line is left alone', () {
      expect(enterAtEndOf('just prose'), isNull);
      expect(enterAtEndOf(''), isNull);
      expect(enterAtEndOf('# A heading'), isNull);
    });

    test('only a plain Enter triggers it', () {
      const formatter = ListContinuationFormatter();
      const old = TextEditingValue(
        text: '- item',
        selection: TextSelection.collapsed(offset: 6),
      );

      // A paste of several characters is not an Enter.
      const pasted = TextEditingValue(
        text: '- item pasted\n',
        selection: TextSelection.collapsed(offset: 14),
      );
      expect(formatter.formatEditUpdate(old, pasted).text, pasted.text);

      // Nor is a deletion.
      const deleted = TextEditingValue(
        text: '- ite',
        selection: TextSelection.collapsed(offset: 5),
      );
      expect(formatter.formatEditUpdate(old, deleted).text, deleted.text);
    });
  });

  group('the heading picker survives its own toolbar unmounting', () {
    testWidgets('a choice still reaches the controller', (tester) async {
      // Opening the menu closes the keyboard, which makes keyboardIsOpen false,
      // which unmounts the toolbar. Guarding the result on `context.mounted`
      // meant headings *never* applied, while every other button worked,
      // because the rest apply synchronously.
      final controller = controllerOn('title', base: 2);
      var visible = true;
      late StateSetter setVisible;

      await tester.pumpWidget(
        MaterialApp(
          home: Scaffold(
            body: StatefulBuilder(
              builder: (context, setState) {
                setVisible = setState;
                return Column(
                  children: [
                    const Spacer(),
                    if (visible) EditorToolbar(controller: controller),
                  ],
                );
              },
            ),
          ),
        ),
      );

      await tester.tap(find.byIcon(LucideIcons.heading));
      await tester.pumpAndSettle();

      // The keyboard would close here, taking the toolbar with it.
      setVisible(() => visible = false);
      await tester.pump();

      await tester.tap(find.text('Heading 2'));
      await tester.pumpAndSettle();

      expect(controller.text, '## title');
    });
  });
}
