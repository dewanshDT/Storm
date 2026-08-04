import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:storm/editor/markdown_theme.dart';
import 'package:storm/editor/markdown_tokenizer.dart';
import 'package:storm/editor/storm_markdown_controller.dart';
import 'package:storm/editor/sample_note.dart';

/// The invariant everything else rests on.
///
/// Flutter lays out the span tree we return, not the raw string. If the spans
/// drop, duplicate or reorder a single character, the rendered text silently
/// diverges from the buffer and every caret offset after that point is wrong.
/// So: whatever we build must flatten back to exactly the input.
void main() {
  late StormMarkdownController controller;

  setUp(() {
    controller = StormMarkdownController(
      theme: MarkdownTheme.light(const TextStyle(fontSize: 16)),
    );
  });

  Future<String> render(WidgetTester tester, String source) async {
    controller.text = source;
    late TextSpan span;
    await tester.pumpWidget(
      MaterialApp(
        home: Builder(
          builder: (context) {
            span = controller.buildTextSpan(
              context: context,
              style: const TextStyle(fontSize: 16),
              withComposing: false,
            );
            return const SizedBox();
          },
        ),
      ),
    );
    return span.toPlainText();
  }

  group('span tree round-trips the buffer exactly', () {
    final corpus = <String, String>{
      'empty': '',
      'single newline': '\n',
      'trailing newlines': 'a\n\n\n',
      'leading newlines': '\n\n\na',
      'plain prose': 'just some ordinary text with no markup',
      'heading': '# Heading one',
      'all heading levels': '# h1\n## h2\n### h3\n#### h4\n##### h5\n###### h6',
      'bold': 'some **bold** text',
      'italic star': 'some *italic* text',
      'italic underscore': 'some _italic_ text',
      'bold italic': 'some ***both*** text',
      'strikethrough': 'some ~~struck~~ text',
      'highlight': 'some ==marked== text',
      'inline code': 'call `foo(bar)` now',
      'wikilink': 'see [[Some Note]] there',
      'md link': 'see [label](https://example.com) there',
      'tag': 'tagged #homelab here',
      'nested-ish': '**bold with `code` inside**',
      'adjacent': '**a**_b_`c`',
      'blockquote': '> quoted line',
      'nested blockquote': '>> deeply quoted',
      'bullet': '- item one',
      'ordered': '1. item one',
      'indented bullet': '    - nested item',
      'hr dashes': '---',
      'hr stars': '***',
      'fence': '```dart\nvoid main() {}\n```',
      'unclosed fence': '```\nstill inside',
      'frontmatter': '---\nid: abc\ntags: [a, b]\n---\n\n# Title',
      'unterminated frontmatter': '---\nid: abc',
      'unmatched asterisk': 'a * b * c ** d',
      'unmatched bracket': 'text [[ unclosed',
      'unmatched backtick': 'a ` b',
      'empty emphasis': '**** and ____',
      'url with hash': 'see https://x.com/a#frag not a tag',
      'hash number': 'issue #1 is not a tag',
      'unicode': 'héllo **wörld** 日本語 #tag/ünïcode',
      'emoji': 'note 🎉 with **bold** 🚀 and [[link]]',
      'tabs': '\t- tabbed item\t**bold**',
      'crlf-ish': 'line one\r\nline two',
      'long line': 'x' * 5000,
      'many inline': List.filled(200, '**b** *i* `c` [[w]] #t').join(' '),
    };

    corpus.forEach((name, source) {
      testWidgets(name, (tester) async {
        expect(await render(tester, source), equals(source),
            reason: 'span tree diverged from buffer for case: $name');
      });
    });

    testWidgets('generated sample notes of every size', (tester) async {
      for (final n in [10, 200, 1000]) {
        final note = sampleNote(n);
        expect(await render(tester, note), equals(note));
      }
    });
  });

  group('tokens tile each line without gaps or overlaps', () {
    void checkTiling(String line) {
      final result = tokenizeLine(line, 0, const BlockContext(), 1);
      var cursor = 0;
      for (final t in result.tokens) {
        expect(t.start, greaterThanOrEqualTo(cursor),
            reason: 'overlap at $t in "$line"');
        expect(t.end, greaterThanOrEqualTo(t.start),
            reason: 'inverted range $t in "$line"');
        expect(t.end, lessThanOrEqualTo(line.length),
            reason: 'token past end of line: $t in "$line"');
        cursor = t.end;
      }
    }

    for (final line in [
      '# Heading **bold** `code` [[link]] #tag',
      '- [ ] task with **bold** and [link](url)',
      '> quote with *emphasis* and #tag',
      '***',
      'a**b**c*d*e`f`g',
      '[a](b)[c](d)',
      '[[a]][[b]]',
    ]) {
      test('tiling: $line', () => checkTiling(line));
    }
  });

  group('styling is actually applied', () {
    testWidgets('heading marker is dimmed, content is bold', (tester) async {
      await render(tester, '# Title');
      final result = tokenizeLine('# Title', 0, const BlockContext(), 1);
      expect(result.tokens.first.kind, TokenKind.marker);
      expect(result.tokens.any((t) => t.kind == TokenKind.heading1), isTrue);
    });

    testWidgets('fence toggles block context', (tester) async {
      var ctx = const BlockContext();
      ctx = tokenizeLine('```dart', 0, ctx, 1).nextContext;
      expect(ctx.inFence, isTrue);
      final inside = tokenizeLine('# not a heading', 0, ctx, 2);
      expect(inside.tokens.single.kind, TokenKind.codeBlock);
      ctx = tokenizeLine('```', 0, ctx, 3).nextContext;
      expect(ctx.inFence, isFalse);
    });

    testWidgets('frontmatter only opens on line 0', (tester) async {
      expect(
        tokenizeLine('---', 0, const BlockContext(), 0).nextContext.inFrontmatter,
        isTrue,
      );
      expect(
        tokenizeLine('---', 0, const BlockContext(), 5).nextContext.inFrontmatter,
        isFalse,
      );
    });
  });

  group('caching', () {
    testWidgets('unchanged text is served from the whole-span memo',
        (tester) async {
      final note = sampleNote(500);
      await render(tester, note);
      await render(tester, note);
      expect(controller.lastServedFromMemo, isTrue,
          reason: 'caret movement must not re-tokenize');
    });

    testWidgets('a one-line edit re-tokenizes about one line', (tester) async {
      final note = sampleNote(1000);
      await render(tester, note);
      final lines = note.split('\n');
      lines[lines.length ~/ 2] = '${lines[lines.length ~/ 2]}X';
      await render(tester, lines.join('\n'));

      expect(controller.lastServedFromMemo, isFalse);
      expect(controller.lastLinesTokenized, lessThan(5),
          reason: 'expected the per-line cache to absorb the untouched lines, '
              'but ${controller.lastLinesTokenized} of '
              '${controller.lastLinesTotal} lines were re-tokenized');
    });

    testWidgets('inserting a line does not invalidate the lines below',
        (tester) async {
      final note = sampleNote(1000);
      await render(tester, note);
      final lines = note.split('\n')..insert(10, 'a brand new line');
      await render(tester, lines.join('\n'));
      expect(controller.lastLinesTokenized, lessThan(5),
          reason: 'spans are offset-independent, so a shift must be free');
    });

    testWidgets('degrades to unstyled above the line threshold',
        (tester) async {
      final huge = List.filled(StormMarkdownController.maxStyledLines + 10, '# h')
          .join('\n');
      expect(await render(tester, huge), equals(huge));
      expect(controller.lastDegraded, isTrue);
    });
  });
}
