import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:editor_spike/editor/markdown_theme.dart';
import 'package:editor_spike/editor/storm_markdown_controller.dart';
import 'package:editor_spike/sample_note.dart';

/// The M0 perf gate.
///
/// Measures `buildTextSpan` in isolation — the part of the frame this project
/// actually controls, and the part flutter#114158 says will bite. It does not
/// measure text layout or raster, so these numbers are a floor, not a
/// prediction of frame time. Real frame timing has to come from the HUD in
/// `main.dart` on a real device.
///
/// Budget: a 60fps frame is 16.7ms and layout will want most of it, so span
/// building gets 4ms p95. Cache hits should be far below that.
const double kSpanBudgetMs = 4.0;

void main() {
  late StormMarkdownController controller;
  const style = TextStyle(fontSize: 16, height: 1.55);

  setUp(() {
    controller = StormMarkdownController(theme: MarkdownTheme.light(style));
  });

  /// Runs [body] against a live BuildContext and returns per-iteration millis.
  Future<List<double>> measure(
    WidgetTester tester,
    int iterations,
    void Function(int i) mutate,
  ) async {
    final samples = <double>[];
    late BuildContext ctx;
    await tester.pumpWidget(
      MaterialApp(
        home: Builder(builder: (c) {
          ctx = c;
          return const SizedBox();
        }),
      ),
    );

    // Warm up: first build pays for regex compilation and a cold cache.
    controller.buildTextSpan(
        context: ctx, style: style, withComposing: false);

    for (var i = 0; i < iterations; i++) {
      mutate(i);
      final sw = Stopwatch()..start();
      controller.buildTextSpan(
          context: ctx, style: style, withComposing: false);
      sw.stop();
      samples.add(sw.elapsedMicroseconds / 1000.0);
    }
    return samples;
  }

  ({double p50, double p95, double max}) stats(List<double> xs) {
    final s = [...xs]..sort();
    double at(double q) => s[(s.length * q).floor().clamp(0, s.length - 1)];
    return (p50: at(0.50), p95: at(0.95), max: s.last);
  }

  void report(String label, List<double> samples) {
    final r = stats(samples);
    // ignore: avoid_print
    print('  $label: p50=${r.p50.toStringAsFixed(3)}ms  '
        'p95=${r.p95.toStringAsFixed(3)}ms  max=${r.max.toStringAsFixed(3)}ms  '
        '(n=${samples.length})');
  }

  // 4800 rather than 5000: the "enter" case inserts 100 lines as it runs, and
  // crossing maxStyledLines mid-test would silently switch the controller to
  // the unstyled path and report a meaninglessly fast result.
  for (final lineTarget in [1000, 4800]) {
    group('$lineTarget-line document', () {
      testWidgets('typing at the end of the document', (tester) async {
        final base = sampleNote(lineTarget);
        controller.text = base;
        final buf = StringBuffer(base);

        final samples = await measure(tester, 200, (i) {
          buf.write('x');
          controller.text = buf.toString();
        });

        report('typing (end)', samples);
        expect(controller.lastDegraded, isFalse,
            reason: 'benchmark fell back to the unstyled path — the numbers '
                'would be meaningless');
        expect(stats(samples).p95, lessThan(kSpanBudgetMs));
      });

      testWidgets('typing in the middle of the document', (tester) async {
        // The harder case: everything after the caret shifts offset.
        final lines = sampleNote(lineTarget).split('\n');
        final mid = lines.length ~/ 2;
        controller.text = lines.join('\n');

        final samples = await measure(tester, 200, (i) {
          lines[mid] = '${lines[mid]}x';
          controller.text = lines.join('\n');
        });

        report('typing (middle)', samples);
        expect(controller.lastDegraded, isFalse,
            reason: 'benchmark fell back to the unstyled path — the numbers '
                'would be meaningless');
        expect(stats(samples).p95, lessThan(kSpanBudgetMs));
      });

      testWidgets('pressing enter repeatedly (line insertion)', (tester) async {
        final lines = sampleNote(lineTarget).split('\n');
        controller.text = lines.join('\n');

        final samples = await measure(tester, 100, (i) {
          lines.insert(lines.length ~/ 3, 'new line $i with **bold** and #tag');
          controller.text = lines.join('\n');
        });

        report('enter (insert line)', samples);
        expect(controller.lastDegraded, isFalse,
            reason: 'benchmark fell back to the unstyled path — the numbers '
                'would be meaningless');
        expect(stats(samples).p95, lessThan(kSpanBudgetMs));
      });

      testWidgets('caret movement within a line must hit the memo',
          (tester) async {
        // The flutter#114158 case. Since reveal-on-active-line, the rendered
        // output depends on which line the caret is in, so the memo covers
        // movement *within* a line — which is the overwhelmingly common case:
        // typing, arrow-left/right, click-to-position. Crossing a line
        // boundary legitimately rebuilds, because the syntax that shows
        // changes.
        controller.text = sampleNote(lineTarget);
        final start = controller.text.indexOf('Plain prose');
        var offset = start;

        final samples = await measure(tester, 40, (i) {
          offset = start + (i % 30);
          controller.selection = TextSelection.collapsed(offset: offset);
        });

        report('caret within a line', samples);
        expect(controller.lastServedFromMemo, isTrue);
        expect(stats(samples).p95, lessThan(0.1),
            reason: 'must be served from the memo, not re-tokenized');
      });
    });
  }

  testWidgets('cold build of a 4800-line document', (tester) async {
    late BuildContext ctx;
    await tester.pumpWidget(MaterialApp(
      home: Builder(builder: (c) {
        ctx = c;
        return const SizedBox();
      }),
    ));

    controller.text = sampleNote(4800);
    final sw = Stopwatch()..start();
    controller.buildTextSpan(context: ctx, style: style, withComposing: false);
    sw.stop();

    final distinct = controller.text.split('\n').toSet().length;
    // ignore: avoid_print
    print('  cold build (4800 lines): '
        '${(sw.elapsedMicroseconds / 1000).toStringAsFixed(1)}ms  '
        'total=${controller.lastLinesTotal}  '
        'tokenized=${controller.lastLinesTokenized}  '
        'distinct=$distinct');

    expect(controller.lastDegraded, isFalse);
    // Sanity: most lines must genuinely have been tokenized. If this collapses
    // the corpus is too repetitive and the whole benchmark is measuring cache
    // hits rather than work.
    expect(controller.lastLinesTokenized, greaterThan(2000),
        reason: 'corpus too repetitive to be a meaningful cold-build measure');

    // Paid once when opening a note, so the budget is a frame or two, not 4ms.
    expect(sw.elapsedMilliseconds, lessThan(400));
  });

  testWidgets('degrade threshold protects very large documents',
      (tester) async {
    late BuildContext ctx;
    await tester.pumpWidget(MaterialApp(
      home: Builder(builder: (c) {
        ctx = c;
        return const SizedBox();
      }),
    ));

    controller.text = sampleNote(12000);
    final sw = Stopwatch()..start();
    final span = controller.buildTextSpan(
        context: ctx, style: style, withComposing: false);
    sw.stop();

    // ignore: avoid_print
    print('  degraded build (12000 lines): '
        '${(sw.elapsedMicroseconds / 1000).toStringAsFixed(1)}ms  '
        'degraded=${controller.lastDegraded}');

    expect(controller.lastDegraded, isTrue);
    expect(span.toPlainText(), equals(controller.text));
  });
}
