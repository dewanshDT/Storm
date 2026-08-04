import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../api/models.dart';
import '../state/app_state.dart';

/// "Linked mentions" — the notes that link *to* the one being edited.
///
/// Collapsed by default and shown as a strip under the editor rather than a
/// third column: it is a reference, not something to keep in view while
/// writing, and a third column doesn't fit on a phone.
///
/// The server resolves these by wikilink target title, so a note only appears
/// once its `[[Link]]` has been indexed — which happens on write.
class BacklinksPanel extends ConsumerStatefulWidget {
  const BacklinksPanel({
    super.key,
    required this.noteId,
    required this.onOpen,
  });

  final String noteId;
  final void Function(NoteMeta) onOpen;

  @override
  ConsumerState<BacklinksPanel> createState() => _BacklinksPanelState();
}

class _BacklinksPanelState extends ConsumerState<BacklinksPanel> {
  bool _expanded = false;

  @override
  Widget build(BuildContext context) {
    final engine = ref.watch(syncEngineProvider);
    final theme = Theme.of(context);

    // Offline the link index is unavailable, and an empty panel would read as
    // "nothing links here", which is a different and wrong claim.
    if (!engine.isOnline) return const SizedBox.shrink();

    final backlinks = ref.watch(backlinksProvider(widget.noteId));
    final count = backlinks.value?.length ?? 0;

    if (backlinks.hasValue && count == 0 && !_expanded) {
      return const SizedBox.shrink();
    }

    return Container(
      decoration: BoxDecoration(
        border: Border(top: BorderSide(color: theme.dividerColor)),
        color: theme.colorScheme.surfaceContainerLowest,
      ),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          InkWell(
            onTap: () => setState(() => _expanded = !_expanded),
            child: Padding(
              padding: const EdgeInsets.symmetric(horizontal: 20, vertical: 9),
              child: Row(
                children: [
                  Icon(
                    _expanded ? Icons.expand_more : Icons.chevron_right,
                    size: 16,
                    color: theme.colorScheme.onSurfaceVariant,
                  ),
                  const SizedBox(width: 6),
                  Text(
                    count == 1 ? '1 linked mention' : '$count linked mentions',
                    style: theme.textTheme.bodySmall?.copyWith(
                      fontWeight: FontWeight.w600,
                      color: theme.colorScheme.onSurfaceVariant,
                    ),
                  ),
                ],
              ),
            ),
          ),
          if (_expanded)
            ConstrainedBox(
              constraints: const BoxConstraints(maxHeight: 180),
              child: backlinks.when(
                loading: () => const Padding(
                  padding: EdgeInsets.all(12),
                  child: Center(
                    child: SizedBox(
                      height: 16,
                      width: 16,
                      child: CircularProgressIndicator(strokeWidth: 2),
                    ),
                  ),
                ),
                error: (e, _) => Padding(
                  padding: const EdgeInsets.all(12),
                  child: Text('$e', style: theme.textTheme.bodySmall),
                ),
                data: (notes) => notes.isEmpty
                    ? Padding(
                        padding: const EdgeInsets.fromLTRB(42, 0, 20, 12),
                        child: Text(
                          'Nothing links here yet.',
                          style: theme.textTheme.bodySmall?.copyWith(
                            color: theme.colorScheme.onSurfaceVariant,
                          ),
                        ),
                      )
                    : ListView.builder(
                        shrinkWrap: true,
                        itemCount: notes.length,
                        itemBuilder: (c, i) => InkWell(
                          onTap: () => widget.onOpen(notes[i]),
                          child: Padding(
                            padding:
                                const EdgeInsets.fromLTRB(42, 6, 20, 6),
                            child: Row(
                              children: [
                                Icon(Icons.north_east,
                                    size: 13,
                                    color: theme.colorScheme.primary),
                                const SizedBox(width: 8),
                                Expanded(
                                  child: Text(
                                    notes[i].title.isEmpty
                                        ? notes[i].path
                                        : notes[i].title,
                                    style: theme.textTheme.bodySmall,
                                    overflow: TextOverflow.ellipsis,
                                  ),
                                ),
                                Text(
                                  notes[i].folder,
                                  style: theme.textTheme.bodySmall?.copyWith(
                                    color: theme.colorScheme.onSurfaceVariant,
                                    fontSize: 11,
                                  ),
                                ),
                              ],
                            ),
                          ),
                        ),
                      ),
              ),
            ),
        ],
      ),
    );
  }
}
