import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../api/models.dart';
import '../state/app_state.dart';
import 'states.dart';
import 'tokens.dart';
import 'widgets.dart';

/// Tag browser.
///
/// Tags are hierarchical in Obsidian (`proj/sub`), so this groups on the first
/// segment: a flat list of forty `proj/*` tags is unusable, and collapsing them
/// under `proj` matches what a vault actually looks like.
///
/// Server-only — resolving tags needs the whole vault's index, which the client
/// deliberately doesn't hold. Offline it says so rather than showing an empty
/// list that looks like "you have no tags".
class TagsPanel extends ConsumerWidget {
  const TagsPanel({super.key, required this.onOpen});

  final void Function(NoteMeta) onOpen;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final t = context.tokens;
    final engine = ref.watch(syncEngineProvider);
    final tags = ref.watch(tagsProvider);
    final selected = ref.watch(selectedTagProvider);

    if (!engine.isOnline) {
      return const EmptyState(
        icon: Icons.cloud_off,
        title: 'Tags need the server',
        detail: 'Reconnect to browse them. Your notes are still here to read.',
      );
    }

    if (selected != null) {
      return _TaggedNotes(tag: selected, onOpen: onOpen);
    }

    return tags.when(
      loading: () => const SkeletonRows(rows: 3),
      error: (e, _) => EmptyState(
        icon: Icons.cloud_off,
        title: 'Could not load tags',
        detail: describeFailure(e),
      ),
      data: (all) {
        if (all.isEmpty) {
          return const EmptyState(
            icon: Icons.label_outline,
            title: 'No tags in this vault yet',
            detail: 'Add #a-tag to a note, or tags: in its frontmatter.',
          );
        }

        final groups = <String, List<TagCount>>{};
        for (final tag in all) {
          groups.putIfAbsent(tag.topLevel, () => []).add(tag);
        }
        final roots = groups.keys.toList()..sort();

        void select(String tag) =>
            ref.read(selectedTagProvider.notifier).state = tag;

        return Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          mainAxisSize: MainAxisSize.min,
          children: [
            for (final root in roots)
              Padding(
                padding: EdgeInsets.only(bottom: t.sp * 1.75),
                child: TagGroup(
                  name: root,
                  // Amber, because amber means tags. They read accent-purple
                  // here before, which is the colour of "interactive" — and a
                  // colour used for a second purpose stops working as a signal.
                  tags: [for (final m in groups[root]!) m.tag],
                  onTagTap: select,
                ),
              ),
          ],
        );
      },
    );
  }
}

/// Notes carrying one tag, with a way back to the tag list.
class _TaggedNotes extends ConsumerWidget {
  const _TaggedNotes({required this.tag, required this.onOpen});

  final String tag;
  final void Function(NoteMeta) onOpen;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final t = context.tokens;
    final notes = ref.watch(notesWithTagProvider(tag));

    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      mainAxisSize: MainAxisSize.min,
      children: [
        InkWell(
          onTap: () => ref.read(selectedTagProvider.notifier).state = null,
          borderRadius: BorderRadius.circular(t.rControl),
          child: Padding(
            padding: EdgeInsets.symmetric(vertical: t.sp * 0.75),
            child: Row(
              children: [
                Icon(Icons.arrow_back, size: t.bodySize, color: t.text3),
                SizedBox(width: t.sp * 0.75),
                TagChip(label: tag),
              ],
            ),
          ),
        ),
        SizedBox(height: t.sp * 0.5),
        notes.when(
          loading: () => const SkeletonRows(rows: 3),
          error: (e, _) => EmptyState(
            icon: Icons.cloud_off,
            title: 'Could not load these notes',
            detail: describeFailure(e),
          ),
          data: (list) => list.isEmpty
              ? const EmptyState(
                  icon: Icons.label_off_outlined,
                  title: 'No notes carry this tag',
                )
              : Column(
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    for (final note in list)
                      NoteRow(
                        title: note.title.isEmpty ? note.path : note.title,
                        meta: note.folder.isEmpty ? null : note.folder,
                        padding: EdgeInsets.symmetric(vertical: t.sp * 1.25),
                        onTap: () => onOpen(note),
                      ),
                  ],
                ),
        ),
      ],
    );
  }
}
