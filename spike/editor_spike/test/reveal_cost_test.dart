import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:editor_spike/editor/markdown_theme.dart';
import 'package:editor_spike/editor/storm_markdown_controller.dart';
import 'package:editor_spike/sample_note.dart';

/// What reveal-on-active-line actually costs.
///
/// Measured as an A/B inside one process and interleaved, because absolute
/// timings on a shared machine are worthless — a background build can move
/// them by 2x between runs. The ratio is the number that means something.
void main() {
  const style = TextStyle(fontSize: 16, height: 1.55);

  testWidgets('hiding markers is not materially more expensive', (tester) async {
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

    final note = sampleNote(4000);
    final lines = note.split('\n');
    final mid = lines.length ~/ 2;
    var caret = 0;
    for (var i = 0; i < mid; i++) {
      caret += lines[i].length + 1;
    }

    double run({required bool reveal}) {
      final c = StormMarkdownController(theme: MarkdownTheme.light(style))
        ..revealOnActiveLine = reveal
        ..text = note
        ..selection = TextSelection.collapsed(offset: caret);

      // Warm the caches.
      c.buildTextSpan(context: ctx, style: style, withComposing: false);

      final samples = <double>[];
      final buf = StringBuffer(lines[mid]);
      for (var i = 0; i < 120; i++) {
        buf.write('x');
        lines[mid] = buf.toString();
        c.text = lines.join('\n');
        c.selection = TextSelection.collapsed(offset: caret + buf.length);

        final sw = Stopwatch()..start();
        c.buildTextSpan(context: ctx, style: style, withComposing: false);
        sw.stop();
        samples.add(sw.elapsedMicroseconds / 1000.0);
      }
      c.dispose();
      samples.sort();
      return samples[samples.length ~/ 2]; // p50
    }

    // Interleaved so a load spike can't land on one arm only.
    final off = <double>[];
    final on = <double>[];
    for (var round = 0; round < 3; round++) {
      off.add(run(reveal: false));
      on.add(run(reveal: true));
    }
    off.sort();
    on.sort();
    final baseline = off[1];
    final revealed = on[1];
    final ratio = revealed / baseline;

    // ignore: avoid_print
    print('  4000 lines, typing p50 — '
        'dimmed ${baseline.toStringAsFixed(2)}ms, '
        'hidden ${revealed.toStringAsFixed(2)}ms, '
        'ratio ${ratio.toStringAsFixed(2)}x');

    expect(ratio, lessThan(1.5),
        reason: 'hiding markers should be roughly free; a real regression '
            'would show up here regardless of machine load');
  });

  testWidgets('caret movement within a line stays free', (tester) async {
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

    final c = StormMarkdownController(theme: MarkdownTheme.light(style))
      ..text = sampleNote(4000);
    addTearDown(c.dispose);

    // Somewhere in the middle of a long line.
    final firstProse = c.text.indexOf('Plain prose');
    c.selection = TextSelection.collapsed(offset: firstProse);
    c.buildTextSpan(context: ctx, style: style, withComposing: false);

    for (var i = 1; i < 20; i++) {
      c.selection = TextSelection.collapsed(offset: firstProse + i);
      c.buildTextSpan(context: ctx, style: style, withComposing: false);
      expect(c.lastServedFromMemo, isTrue,
          reason: 'moving inside one line must not rebuild the span tree');
    }
  });
}
