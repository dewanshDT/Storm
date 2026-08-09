import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_lucide/flutter_lucide.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import 'states.dart';
import 'shell/storm_scaffold.dart';
import 'tokens.dart';
import 'widgets.dart';

import '../api/models.dart';
import '../state/app_state.dart';

/// Full-text search over the vault, served by the server's FTS5 index.
class SearchPanel extends ConsumerStatefulWidget {
  const SearchPanel({super.key, required this.onOpen, this.onClose});

  final void Function(NoteMeta) onOpen;

  /// Shown as the trailing `×` when search owns the whole screen.
  final VoidCallback? onClose;

  @override
  ConsumerState<SearchPanel> createState() => _SearchPanelState();
}

class _SearchPanelState extends ConsumerState<SearchPanel> {
  final _controller = TextEditingController();
  Timer? _debounce;

  @override
  void initState() {
    super.initState();
    _controller.text = ref.read(searchQueryProvider);
  }

  @override
  void dispose() {
    _debounce?.cancel();
    _controller.dispose();
    super.dispose();
  }

  void _onChanged(String value) {
    // Debounced so typing doesn't fire a query per keystroke.
    _debounce?.cancel();
    _debounce = Timer(const Duration(milliseconds: 250), () {
      if (mounted) ref.read(searchQueryProvider.notifier).state = value;
    });
  }

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final results = ref.watch(searchResultsProvider);
    final query = ref.watch(searchQueryProvider);
    final online = ref.watch(syncEngineProvider).isOnline;

    return Column(
      children: [
        Padding(
          padding: EdgeInsets.fromLTRB(t.sp * 2, t.sp * 0.5, t.sp, t.sp),
          child: Row(
            children: [
              Expanded(
                child: StormInput(
                  controller: _controller,
                  autofocus: true,
                  onChanged: _onChanged,
                  hintText: 'Search notes',
                  prefixIcon: LucideIcons.search,
                  suffix: _controller.text.isEmpty
                      ? null
                      : IconButton(
                          icon: Icon(LucideIcons.x, size: t.codeSize),
                          onPressed: () {
                            _controller.clear();
                            ref.read(searchQueryProvider.notifier).state = '';
                          },
                        ),
                ),
              ),
              if (widget.onClose != null)
                IconButton(
                  icon: Icon(LucideIcons.x, size: t.bodySize, color: t.text3),
                  onPressed: widget.onClose,
                  tooltip: 'Close',
                ),
            ],
          ),
        ),
        Expanded(
          child: results.when(
            loading: () => Padding(
              padding: EdgeInsets.symmetric(horizontal: t.sp * 2.5),
              child: const SkeletonRows(rows: 3),
            ),
            error: (e, _) => EmptyState(
              icon: LucideIcons.cloud_off,
              title: 'Search needs the server',
              detail: describeFailure(e),
            ),
            data: (hits) {
              // Search is served by the index, so offline it can only ever
              // answer "nothing" — which is not the same as "no matches".
              if (!online) {
                return const EmptyState(
                  icon: LucideIcons.cloud_off,
                  title: 'Search needs the server',
                  detail:
                      'Reconnect to search this vault. '
                      'Your notes are still here to read.',
                );
              }
              if (query.trim().isEmpty) {
                return const EmptyState(
                  icon: LucideIcons.search,
                  title: 'Search this vault',
                  detail: 'Every note, by its words.',
                );
              }
              if (hits.isEmpty) {
                return EmptyState(
                  icon: LucideIcons.search_x,
                  title: 'No notes match “$query”',
                  detail: 'Try fewer words, or a word from the body.',
                );
              }
              return ListView.builder(
                padding: EdgeInsets.fromLTRB(
                  t.sp,
                  0,
                  t.sp,
                  StormChrome.navClearance(context),
                ),
                itemCount: hits.length,
                itemBuilder: (c, i) =>
                    _HitRow(hit: hits[i], onOpen: widget.onOpen),
              );
            },
          ),
        ),
      ],
    );
  }
}

class _HitRow extends StatelessWidget {
  const _HitRow({required this.hit, required this.onOpen});

  final SearchHit hit;
  final void Function(NoteMeta) onOpen;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final tags = tagsIn(hit.snippet);

    return InkWell(
      onTap: () => onOpen(
        NoteMeta(
          id: hit.id,
          path: hit.path,
          title: hit.title,
          // The search index doesn't carry a version; opening the note fetches
          // the real one before any edit can be saved.
          version: 0,
          modified: '',
          size: 0,
        ),
      ),
      borderRadius: BorderRadius.circular(t.rControl),
      child: Padding(
        padding: EdgeInsets.symmetric(
          horizontal: t.sp * 1.5,
          vertical: t.sp * 1.5,
        ),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text(
              hit.title.isEmpty ? hit.path : hit.title,
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: TextStyle(
                fontFamily: StormTokens.sansFamily,
                fontSize: t.codeSize,
                fontWeight: FontWeight.w500,
                color: t.text,
              ),
            ),
            SizedBox(height: t.sp * 0.25),
            _Snippet(raw: hit.snippet),
            if (tags.isNotEmpty) ...[
              SizedBox(height: t.sp * 0.75),
              Wrap(
                spacing: t.sp * 0.75,
                runSpacing: t.sp * 0.5,
                children: [for (final tag in tags) TagChip(label: tag)],
              ),
            ],
          ],
        ),
      ),
    );
  }
}

/// The `#tags` a snippet happens to contain, de-duplicated and in order.
List<String> tagsIn(String snippet) {
  final found = <String>{};
  for (final m in RegExp(
    r'#[\w/-]+',
  ).allMatches(snippet.replaceAll('<<', '').replaceAll('>>', ''))) {
    found.add(m.group(0)!);
  }
  return found.toList(growable: false);
}

/// Renders the server's `<<match>>` markers as highlighted spans.
class _Snippet extends StatelessWidget {
  const _Snippet({required this.raw});

  final String raw;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final base = TextStyle(
      fontFamily: StormTokens.sansFamily,
      fontSize: t.labelSize,
      color: t.text3,
      height: 1.4,
    );
    final spans = <TextSpan>[];

    // Collapse newlines: a snippet spanning lines would blow up the row.
    var text = raw.replaceAll('\n', ' ');
    while (true) {
      final open = text.indexOf('<<');
      final close = text.indexOf('>>', open + 2);
      if (open < 0 || close < 0) {
        spans.add(TextSpan(text: text, style: base));
        break;
      }
      spans.add(TextSpan(text: text.substring(0, open), style: base));
      spans.add(
        TextSpan(
          text: text.substring(open + 2, close),
          style: base.copyWith(color: t.accent, fontWeight: FontWeight.w600),
        ),
      );
      text = text.substring(close + 2);
    }

    return Text.rich(
      TextSpan(children: spans),
      maxLines: 2,
      overflow: TextOverflow.ellipsis,
    );
  }
}
