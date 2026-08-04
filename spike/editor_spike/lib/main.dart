import 'package:flutter/material.dart';
import 'package:flutter/scheduler.dart';

import 'editor/markdown_theme.dart';
import 'editor/storm_markdown_controller.dart';
import 'sample_note.dart';

/// M0 editor spike.
///
/// Throwaway harness for the one question that gates the whole project: can a
/// Flutter `TextField` with a custom controller give an acceptable markdown
/// editing experience on macOS, Android and web? The HUD reports what the
/// automated gate in `test/editor_perf_test.dart` measures, so the two can be
/// cross-checked by hand on a real device.
void main() => runApp(const SpikeApp());

class SpikeApp extends StatelessWidget {
  const SpikeApp({super.key});

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'Storm — editor spike',
      debugShowCheckedModeBanner: false,
      theme: ThemeData(brightness: Brightness.light, useMaterial3: true),
      darkTheme: ThemeData(brightness: Brightness.dark, useMaterial3: true),
      home: const SpikeHome(),
    );
  }
}

class SpikeHome extends StatefulWidget {
  const SpikeHome({super.key});

  @override
  State<SpikeHome> createState() => _SpikeHomeState();
}

class _SpikeHomeState extends State<SpikeHome> {
  late StormMarkdownController _controller;
  final _focus = FocusNode();

  Brightness _brightness = Brightness.light;
  double _fontSize = 16;
  int _lineCount = 200;

  // Rolling frame-time sample, which is what "does it feel laggy" actually
  // means. Build-span timing alone understates cost because it ignores layout.
  final List<double> _frameMs = [];

  @override
  void initState() {
    super.initState();
    _controller = StormMarkdownController(
      theme: _themeFor(_brightness),
      text: sampleNote(_lineCount),
    );
    SchedulerBinding.instance.addTimingsCallback(_onTimings);
  }

  @override
  void dispose() {
    SchedulerBinding.instance.removeTimingsCallback(_onTimings);
    _controller.dispose();
    _focus.dispose();
    super.dispose();
  }

  void _onTimings(List<FrameTiming> timings) {
    for (final t in timings) {
      _frameMs.add(t.totalSpan.inMicroseconds / 1000.0);
    }
    if (_frameMs.length > 240) {
      _frameMs.removeRange(0, _frameMs.length - 240);
    }
  }

  MarkdownTheme _themeFor(Brightness b) {
    final base = TextStyle(
      fontSize: _fontSize,
      height: 1.55,
      color: b == Brightness.dark
          ? const Color(0xFFE6E9EF)
          : const Color(0xFF1F2430),
      fontFamilyFallback: const ['SF Pro Text', 'Inter', 'Roboto'],
    );
    return b == Brightness.dark
        ? MarkdownTheme.dark(base)
        : MarkdownTheme.light(base);
  }

  double get _p95 {
    if (_frameMs.isEmpty) return 0;
    final sorted = [..._frameMs]..sort();
    return sorted[(sorted.length * 0.95).floor().clamp(0, sorted.length - 1)];
  }

  void _reload(int lines) {
    setState(() {
      _lineCount = lines;
      _controller.text = sampleNote(lines);
      _frameMs.clear();
    });
  }

  @override
  Widget build(BuildContext context) {
    _controller.theme = _themeFor(_brightness);
    final dark = _brightness == Brightness.dark;

    return Theme(
      data: dark ? ThemeData.dark(useMaterial3: true) : ThemeData.light(useMaterial3: true),
      child: Scaffold(
        backgroundColor: dark ? const Color(0xFF14171D) : const Color(0xFFFCFCFD),
        appBar: AppBar(
          title: const Text('Storm — editor spike'),
          actions: [
            IconButton(
              tooltip: 'Toggle theme',
              icon: Icon(dark ? Icons.light_mode : Icons.dark_mode),
              onPressed: () => setState(() {
                _brightness = dark ? Brightness.light : Brightness.dark;
              }),
            ),
          ],
        ),
        body: Column(
          children: [
            _StatsBar(
              lineCount: _lineCount,
              p95: _p95,
              controller: _controller,
              onReload: _reload,
              fontSize: _fontSize,
              onFontSize: (v) => setState(() {
                _fontSize = v;
                _frameMs.clear();
              }),
            ),
            Expanded(
              child: Scrollbar(
                child: SingleChildScrollView(
                  padding: const EdgeInsets.fromLTRB(24, 16, 24, 96),
                  child: Center(
                    child: ConstrainedBox(
                      constraints: const BoxConstraints(maxWidth: 780),
                      child: TextField(
                        controller: _controller,
                        focusNode: _focus,
                        maxLines: null,
                        autofocus: true,
                        cursorWidth: 2,
                        keyboardType: TextInputType.multiline,
                        textCapitalization: TextCapitalization.sentences,
                        decoration: const InputDecoration(
                          border: InputBorder.none,
                          isDense: true,
                        ),
                      ),
                    ),
                  ),
                ),
              ),
            ),
          ],
        ),
      ),
    );
  }
}

class _StatsBar extends StatelessWidget {
  const _StatsBar({
    required this.lineCount,
    required this.p95,
    required this.controller,
    required this.onReload,
    required this.fontSize,
    required this.onFontSize,
  });

  final int lineCount;
  final double p95;
  final StormMarkdownController controller;
  final void Function(int) onReload;
  final double fontSize;
  final void Function(double) onFontSize;

  @override
  Widget build(BuildContext context) {
    final pass = p95 > 0 && p95 < 16.0;
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 8),
      decoration: BoxDecoration(
        border: Border(
          bottom: BorderSide(color: Theme.of(context).dividerColor),
        ),
      ),
      child: Wrap(
        spacing: 16,
        runSpacing: 8,
        crossAxisAlignment: WrapCrossAlignment.center,
        children: [
          _chip(
            context,
            'frame p95  ${p95.toStringAsFixed(1)}ms',
            pass ? Colors.green : (p95 == 0 ? Colors.grey : Colors.red),
          ),
          _chip(context, 'lines  ${controller.lastLinesTotal}', Colors.blueGrey),
          _chip(
            context,
            'retokenized  ${controller.lastLinesTokenized}',
            Colors.blueGrey,
          ),
          if (controller.lastDegraded)
            _chip(context, 'DEGRADED (unstyled)', Colors.orange),
          const SizedBox(width: 8),
          for (final n in [200, 1000, 5000, 8000])
            OutlinedButton(
              onPressed: () => onReload(n),
              style: OutlinedButton.styleFrom(
                visualDensity: VisualDensity.compact,
                backgroundColor:
                    lineCount == n ? Theme.of(context).colorScheme.primaryContainer : null,
              ),
              child: Text('$n'),
            ),
          SizedBox(
            width: 160,
            child: Slider(
              value: fontSize,
              min: 12,
              max: 24,
              divisions: 12,
              label: '${fontSize.round()}px',
              onChanged: onFontSize,
            ),
          ),
        ],
      ),
    );
  }

  Widget _chip(BuildContext context, String label, Color color) => Container(
        padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 4),
        decoration: BoxDecoration(
          color: color.withValues(alpha: 0.14),
          borderRadius: BorderRadius.circular(6),
        ),
        child: Text(
          label,
          style: TextStyle(
            fontSize: 12,
            fontWeight: FontWeight.w600,
            color: color,
            fontFamily: 'monospace',
          ),
        ),
      );
}
