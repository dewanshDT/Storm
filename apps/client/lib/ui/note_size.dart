import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/scheduler.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_riverpod/legacy.dart';

import '../editor/token_estimate.dart';
import 'tokens.dart';
import 'widgets.dart';

/// The text selected in the open note's editor or reader, if any.
final noteSelectionProvider = StateProvider<String>((ref) => '');

/// Sets [noteSelectionProvider], after the frame when called mid-build or
/// mid-layout, where a provider may not change.
void reportNoteSelection(WidgetRef ref, String text) {
  final notifier = ref.read(noteSelectionProvider.notifier);
  void apply() {
    if (notifier.state != text) notifier.state = text;
  }

  final phase = SchedulerBinding.instance.schedulerPhase;
  if (phase == SchedulerPhase.idle ||
      phase == SchedulerPhase.postFrameCallbacks) {
    apply();
  } else {
    SchedulerBinding.instance.addPostFrameCallback((_) => apply());
  }
}

/// `~2.4k tokens · 1,812 words · 10.2k chars`.
String sizeLabel(TokenEstimate e) =>
    '${formatTokens(e.tokens)} tokens · ${_words(e.words)} · '
    '${compactCount(e.chars)} chars';

/// `Selection: ~240 tokens · 180 words`.
String selectionLabel(TokenEstimate e) =>
    'Selection: ${formatTokens(e.tokens)} tokens · ${_words(e.words)}';

String _words(int n) => '${groupedCount(n)} ${n == 1 ? 'word' : 'words'}';

/// The Properties panel's Size row (decision 158).
class NoteSizeRow extends ConsumerStatefulWidget {
  const NoteSizeRow({super.key, required this.content});

  /// The note's whole file, frontmatter included.
  final String content;

  static const debounce = Duration(milliseconds: 300);

  @override
  ConsumerState<NoteSizeRow> createState() => _NoteSizeRowState();
}

/// A note past [kLargeNoteChars] is estimated with `compute`, off the UI
/// thread; on the web `compute` runs on the same thread, which is the
/// fallback.
class _NoteSizeRowState extends ConsumerState<NoteSizeRow> {
  TokenEstimate? _total;
  Timer? _timer;
  int _generation = 0;
  String _selection = '';
  TokenEstimate? _selected;

  @override
  void initState() {
    super.initState();
    _recompute(initial: true);
  }

  @override
  void didUpdateWidget(NoteSizeRow old) {
    super.didUpdateWidget(old);
    if (old.content == widget.content) return;
    _timer?.cancel();
    _timer = Timer(NoteSizeRow.debounce, _recompute);
  }

  @override
  void dispose() {
    _timer?.cancel();
    super.dispose();
  }

  void _recompute({bool initial = false}) {
    final generation = ++_generation;
    final content = widget.content;
    if (content.length <= kLargeNoteChars) {
      final e = estimateNote(content);
      if (initial) {
        _total = e;
      } else if (mounted) {
        setState(() => _total = e);
      }
      return;
    }
    compute(estimateNote, content).then((e) {
      if (!mounted || generation != _generation) return;
      setState(() => _total = e);
    });
  }

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final selection = ref.watch(noteSelectionProvider);
    if (selection != _selection) {
      _selection = selection;
      _selected = selection.trim().isEmpty ? null : estimateText(selection);
    }
    final style = TextStyle(
      fontFamily: StormTokens.sansFamily,
      fontSize: t.labelSize,
      color: t.text3,
    );
    final total = _total;
    return Tooltip(
      message: kSizeTooltip,
      child: Padding(
        padding: EdgeInsets.symmetric(vertical: t.sp * 0.75),
        child: Row(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            const Padding(
              padding: EdgeInsets.only(top: 2),
              child: KeyChip(label: 'Size'),
            ),
            SizedBox(width: t.sp * 1.25),
            Expanded(
              child: Padding(
                padding: EdgeInsets.symmetric(vertical: t.sp * 0.5),
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(
                      total == null ? '—' : sizeLabel(total),
                      key: const Key('note-size'),
                      style: style,
                    ),
                    if (_selected != null)
                      Padding(
                        padding: EdgeInsets.only(top: t.sp * 0.5),
                        child: Text(
                          selectionLabel(_selected!),
                          key: const Key('note-size-selection'),
                          style: style.copyWith(color: t.text2),
                        ),
                      ),
                  ],
                ),
              ),
            ),
          ],
        ),
      ),
    );
  }
}
