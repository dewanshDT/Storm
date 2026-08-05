import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:storm/editor/markdown_theme.dart';
import 'package:storm/editor/storm_markdown_controller.dart';
import 'package:storm/editor/sample_note.dart';

/// Hiding markdown syntax on lines the caret isn't on.
///
/// The characters must stay in the buffer — Flutter maps caret offsets and hit
/// testing through the span tree by index, so omitting a `#` would put every
/// later offset one character out. They are rendered at effectively zero size
/// instead.
void main() {
  const base = TextStyle(fontSize: 16, height: 1.5);
  late StormMarkdownController controller;

  setUp(() {
    controller = StormMarkdownController(theme: MarkdownTheme.light(base));
  });
  tearDown(() => controller.dispose());

  /// Builds the span tree with the caret at [offset].
  Future<TextSpan> render(
    WidgetTester tester,
    String source, {
    int? caret,
    TextSelection? selection,
  }) async {
    controller.text = source;
    if (selection != null) {
      controller.selection = selection;
    } else if (caret != null) {
      controller.selection = TextSelection.collapsed(offset: caret);
    }

    late TextSpan span;
    await tester.pumpWidget(
      MaterialApp(
        home: Builder(
          builder: (context) {
            span = controller.buildTextSpan(
              context: context,
              style: base,
              withComposing: false,
            );
            return const SizedBox();
          },
        ),
      ),
    );
    return span;
  }

  /// Every styled run, flattened, with its font size.
  List<(String, double?)> runs(TextSpan span) {
    final out = <(String, double?)>[];
    void walk(InlineSpan s) {
      if (s is TextSpan) {
        if (s.text != null) out.add((s.text!, s.style?.fontSize));
        for (final c in s.children ?? const <InlineSpan>[]) {
          walk(c);
        }
      }
    }

    walk(span);
    return out;
  }

  bool isHidden(double? size) => size != null && size < 1;

  group('the buffer is never altered', () {
    const doc =
        '# Heading\n\nSome **bold** and `code` here.\n\n- a [l](u) item\n';

    testWidgets('with the caret on the first line', (tester) async {
      final span = await render(tester, doc, caret: 2);
      expect(span.toPlainText(), doc);
    });

    testWidgets('with the caret elsewhere', (tester) async {
      final span = await render(tester, doc, caret: doc.length - 2);
      expect(span.toPlainText(), doc);
    });

    testWidgets('with no selection at all', (tester) async {
      final span = await render(tester, doc);
      expect(span.toPlainText(), doc);
    });

    testWidgets('across the whole generated sample', (tester) async {
      final note = sampleNote(300);
      final span = await render(tester, note, caret: note.length ~/ 2);
      expect(span.toPlainText(), note);
    });
  });

  group('syntax hides and reveals', () {
    const doc = '# Alpha\n\n**bold** text\n';

    testWidgets('the caret line shows its markers full size', (tester) async {
      final span = await render(tester, doc, caret: 3); // inside "# Alpha"
      final hash = runs(span).firstWhere((r) => r.$1 == '# ');
      expect(isHidden(hash.$2), isFalse, reason: 'the active line reveals');
    });

    testWidgets('other lines hide theirs', (tester) async {
      // Caret on the "**bold**" line, so the heading's hash should vanish.
      final span = await render(tester, doc, caret: doc.indexOf('bold'));
      final hash = runs(span).firstWhere((r) => r.$1 == '# ');
      expect(isHidden(hash.$2), isTrue);

      // …and the markers on the caret's own line stay.
      final stars = runs(span).where((r) => r.$1 == '**').toList();
      expect(stars, isNotEmpty);
      expect(stars.every((r) => !isHidden(r.$2)), isTrue);
    });

    testWidgets('a link URL hides so it reads as its label', (tester) async {
      const src = 'see [label](https://example.com) here\nother line\n';
      final span = await render(tester, src, caret: src.indexOf('other'));
      final url = runs(span).firstWhere((r) => r.$1.contains('example.com'));
      expect(isHidden(url.$2), isTrue);
      final label = runs(span).firstWhere((r) => r.$1 == 'label');
      expect(isHidden(label.$2), isFalse);
    });

    testWidgets('list bullets stay visible — they are not syntax', (
      tester,
    ) async {
      const src = '- first item\n- second item\n';
      final span = await render(tester, src, caret: src.indexOf('second'));
      final bullets = runs(span).where((r) => r.$1 == '- ').toList();
      expect(bullets, isNotEmpty);
      expect(bullets.every((r) => !isHidden(r.$2)), isTrue);
    });

    testWidgets('a selection reveals every line it spans', (tester) async {
      const src = '# One\n\n## Two\n\n### Three\n';
      final span = await render(
        tester,
        src,
        selection: TextSelection(baseOffset: 0, extentOffset: src.length - 1),
      );
      final hashes = runs(span).where((r) => r.$1.trim().startsWith('#'));
      expect(hashes.every((r) => !isHidden(r.$2)), isTrue);
    });

    testWidgets('a line of only punctuation stays visible', (tester) async {
      // Hiding every run would collapse the line and leave the caret nowhere
      // to land.
      const src = 'text\n***\nmore\n';
      final span = await render(tester, src, caret: 0);
      expect(span.toPlainText(), src);
    });

    testWidgets('the feature can be switched off', (tester) async {
      controller.revealOnActiveLine = false;
      final span = await render(tester, doc, caret: doc.indexOf('bold'));
      final hash = runs(span).firstWhere((r) => r.$1 == '# ');
      expect(isHidden(hash.$2), isFalse);
    });
  });

  group('caching still holds up', () {
    testWidgets('moving within a line is free', (tester) async {
      // The memo is what made caret movement cost nothing; adding the revealed
      // range to its key must not have broken that.
      final note = sampleNote(400);
      await render(tester, note, caret: 10);
      await render(tester, note, caret: 12);
      expect(controller.lastServedFromMemo, isTrue);
    });

    testWidgets('crossing to another line re-renders but re-tokenizes little', (
      tester,
    ) async {
      final note = sampleNote(400);
      final lines = note.split('\n');
      await render(tester, note, caret: 5);

      // Jump the caret far down the document.
      var offset = 0;
      for (var i = 0; i < 200; i++) {
        offset += lines[i].length + 1;
      }
      await render(tester, note, caret: offset + 1);

      expect(controller.lastServedFromMemo, isFalse);
      expect(
        controller.lastLinesTokenized,
        lessThan(6),
        reason: 'only the two lines that changed state should re-tokenize',
      );
    });
  });
}
