import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import 'states.dart';

import '../api/models.dart';
import '../state/app_state.dart';

/// Full-text search over the vault, served by the server's FTS5 index.
class SearchPanel extends ConsumerStatefulWidget {
  const SearchPanel({super.key, required this.onOpen});

  final void Function(NoteMeta) onOpen;

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
    final results = ref.watch(searchResultsProvider);
    final query = ref.watch(searchQueryProvider);

    return Column(
      children: [
        Padding(
          padding: const EdgeInsets.all(10),
          child: TextField(
            controller: _controller,
            autofocus: true,
            onChanged: _onChanged,
            decoration: InputDecoration(
              hintText: 'Search notes',
              prefixIcon: const Icon(Icons.search, size: 18),
              isDense: true,
              border: const OutlineInputBorder(),
              suffixIcon: _controller.text.isEmpty
                  ? null
                  : IconButton(
                      icon: const Icon(Icons.clear, size: 16),
                      onPressed: () {
                        _controller.clear();
                        ref.read(searchQueryProvider.notifier).state = '';
                      },
                    ),
            ),
          ),
        ),
        Expanded(
          child: results.when(
            loading: () => const Padding(
              padding: EdgeInsets.symmetric(horizontal: 20),
              child: SkeletonRows(rows: 3),
            ),
            error: (e, _) => Center(
              child: Padding(
                padding: const EdgeInsets.all(16),
                child: Text('$e', style: Theme.of(context).textTheme.bodySmall),
              ),
            ),
            data: (hits) {
              if (query.trim().isEmpty) {
                return const EmptyState(
                  icon: Icons.search,
                  title: 'Search this vault',
                  detail: 'Every note, by its words.',
                );
              }
              if (hits.isEmpty) {
                return EmptyState(
                  icon: Icons.search_off,
                  title: 'No notes match “$query”',
                  detail: 'Try fewer words, or a word from the body.',
                );
              }
              return ListView.separated(
                itemCount: hits.length,
                separatorBuilder: (_, _) => const Divider(height: 1),
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
    final theme = Theme.of(context);
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
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 10),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text(
              hit.title.isEmpty ? hit.path : hit.title,
              style: theme.textTheme.bodyMedium?.copyWith(
                fontWeight: FontWeight.w600,
              ),
              overflow: TextOverflow.ellipsis,
            ),
            const SizedBox(height: 2),
            _Snippet(raw: hit.snippet),
          ],
        ),
      ),
    );
  }
}

/// Renders the server's `<<match>>` markers as highlighted spans.
class _Snippet extends StatelessWidget {
  const _Snippet({required this.raw});

  final String raw;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final base = theme.textTheme.bodySmall!.copyWith(
      color: theme.colorScheme.onSurfaceVariant,
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
          style: base.copyWith(
            color: theme.colorScheme.primary,
            fontWeight: FontWeight.w700,
          ),
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
