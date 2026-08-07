import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:storm/editor/markdown_theme.dart';
import 'package:storm/editor/storm_markdown_controller.dart';

/// The controller's editing methods.
///
/// These are the only sanctioned way for the toolbar to change text, so they
/// carry the whole burden of not corrupting the buffer. Every case asserts the
/// caret as well as the text: a correct string with the caret in the wrong
/// place is still a bug, and the kind that only shows up under your fingers.
void main() {
  StormMarkdownController make(String text, {int? base, int? extent}) {
    final c = StormMarkdownController(
      theme: MarkdownTheme.dark(const TextStyle(fontSize: 16)),
      text: text,
    );
    c.selection = TextSelection(
      baseOffset: base ?? text.length,
      extentOffset: extent ?? base ?? text.length,
    );
    return c;
  }

  group('toggleInline', () {
    test('wraps a selection and keeps it selected', () {
      final c = make('hello world', base: 6, extent: 11);
      c.toggleInline('**');

      expect(c.text, 'hello **world**');
      expect(c.selection.start, 8);
      expect(c.selection.end, 13);
      expect(c.text.substring(c.selection.start, c.selection.end), 'world');
    });

    test('unwraps when the markers sit just outside the selection', () {
      // Double-tapping a bold word selects the word, not the asterisks.
      final c = make('hello **world**', base: 8, extent: 13);
      c.toggleInline('**');

      expect(c.text, 'hello world');
      expect(c.text.substring(c.selection.start, c.selection.end), 'world');
    });

    test('unwraps when the markers sit inside the selection', () {
      final c = make('hello **world**', base: 6, extent: 15);
      c.toggleInline('**');

      expect(c.text, 'hello world');
      expect(c.text.substring(c.selection.start, c.selection.end), 'world');
    });

    test('a collapsed caret inserts the pair and sits between them', () {
      final c = make('hello ', base: 6);
      c.toggleInline('**');

      expect(c.text, 'hello ****');
      expect(c.selection.isCollapsed, isTrue);
      expect(c.selection.baseOffset, 8);
    });

    test('italic and code use the same path', () {
      final c = make('a word', base: 2, extent: 6);
      c.toggleInline('*');
      expect(c.text, 'a *word*');

      c.toggleInline('`');
      expect(c.text, 'a *`word`*');
    });

    test('does nothing without a valid selection', () {
      final c = StormMarkdownController(
        theme: MarkdownTheme.dark(const TextStyle()),
        text: 'untouched',
      );
      c.selection = const TextSelection.collapsed(offset: -1);
      c.toggleInline('**');
      expect(c.text, 'untouched');
    });
  });

  group('setBlockPrefix', () {
    test('adds a heading and moves the caret with its content', () {
      final c = make('title', base: 2);
      c.setBlockPrefix('## ');

      expect(c.text, '## title');
      expect(
        c.selection.baseOffset,
        5,
        reason: 'still before the third letter',
      );
    });

    test('replaces one prefix with another rather than stacking', () {
      final c = make('# title', base: 4);
      c.setBlockPrefix('### ');
      expect(c.text, '### title');
    });

    test('applying the prefix a line already has removes it', () {
      final c = make('- item', base: 4);
      c.setBlockPrefix('- ');
      expect(c.text, 'item');
    });

    test('null strips whatever is there', () {
      final c = make('> quoted', base: 4);
      c.setBlockPrefix(null);
      expect(c.text, 'quoted');
    });

    test('a task marker is not mistaken for a bullet', () {
      final c = make('- [ ] task', base: 8);
      c.setBlockPrefix('## ');
      expect(c.text, '## task');
    });

    test('applies to every line the selection touches', () {
      final c = make('one\ntwo\nthree', base: 1, extent: 9);
      c.setBlockPrefix('- ');

      expect(c.text, '- one\n- two\n- three');
      expect(
        c.text.substring(c.selection.start, c.selection.end),
        '- one\n- two\n- three',
      );
    });

    test('preserves indentation', () {
      final c = make('    nested', base: 6);
      c.setBlockPrefix('- ');
      expect(c.text, '    - nested');
    });

    test('leaves the rest of the document alone', () {
      final c = make('before\ntarget\nafter', base: 9);
      c.setBlockPrefix('# ');
      expect(c.text, 'before\n# target\nafter');
    });
  });

  group('insertWikilink', () {
    test('inserts empty brackets with the caret inside', () {
      final c = make('see ', base: 4);
      c.insertWikilink();

      expect(c.text, 'see [[]]');
      expect(c.selection.baseOffset, 6);
      expect(c.selection.isCollapsed, isTrue);
    });

    test('wraps a selection and selects the target', () {
      final c = make('see Design Notes', base: 4, extent: 16);
      c.insertWikilink();

      expect(c.text, 'see [[Design Notes]]');
      expect(
        c.text.substring(c.selection.start, c.selection.end),
        'Design Notes',
      );
    });
  });

  group('wikilinkAt', () {
    const text = 'intro [[Design]] and [[Daily/Today]] end\nsecond [[Other]]';

    test('finds the link the caret is inside', () {
      expect(wikilinkAt(text, 10)?.target, 'Design');
      expect(wikilinkAt(text, 25)?.target, 'Daily/Today');
      expect(wikilinkAt(text, 50)?.target, 'Other');
    });

    test('includes both edges, so a tap on a bracket counts', () {
      expect(wikilinkAt(text, 6)?.target, 'Design');
      expect(wikilinkAt(text, 16)?.target, 'Design');
    });

    test('returns null outside a link', () {
      expect(wikilinkAt(text, 3), isNull);
      expect(wikilinkAt(text, 18), isNull);
      expect(wikilinkAt('no links here', 5), isNull);
    });

    test('does not run past the end of a line', () {
      // Offset 40 is the newline; the link on the next line must not match.
      expect(wikilinkAt(text, 40), isNull);
    });

    test('reports offsets that address the link in the buffer', () {
      final hit = wikilinkAt(text, 10)!;
      expect(text.substring(hit.start, hit.end), '[[Design]]');
    });

    test('tolerates out-of-range offsets', () {
      expect(wikilinkAt(text, -1), isNull);
      expect(wikilinkAt(text, text.length + 5), isNull);
    });
  });

  group('the buffer invariant holds after every operation', () {
    // The load-bearing assertion of the whole editor: buildTextSpan must
    // flatten to the controller buffer character for character, or every caret
    // offset past the divergence is wrong.
    String flatten(InlineSpan span) {
      final buffer = StringBuffer();
      span.visitChildren((s) {
        if (s is TextSpan && s.text != null) buffer.write(s.text);
        return true;
      });
      return buffer.toString();
    }

    testWidgets('after formatting a real document', (tester) async {
      late BuildContext ctx;
      await tester.pumpWidget(
        MaterialApp(
          home: Builder(
            builder: (c) {
              ctx = c;
              return const SizedBox();
            },
          ),
        ),
      );

      final c = make(
        '# Notes\n\nSome text with a word here.\n- [ ] a task\n> quoted\n',
        base: 25,
        extent: 29,
      );

      void check(String label) {
        final span = c.buildTextSpan(
          context: ctx,
          style: const TextStyle(fontSize: 16),
          withComposing: false,
        );
        expect(
          flatten(span),
          c.text,
          reason: 'span diverged from buffer: $label',
        );
        expect(
          c.selection.end,
          lessThanOrEqualTo(c.text.length),
          reason: 'caret past the end of the buffer: $label',
        );
      }

      check('initial');
      c.toggleInline('**');
      check('bold');
      c.toggleInline('*');
      check('italic');
      c.setBlockPrefix('## ');
      check('heading');
      c.insertWikilink();
      check('wikilink');
      c.setBlockPrefix(null);
      check('strip prefix');
    });
  });
}
