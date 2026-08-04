import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../api/models.dart';
import '../state/app_state.dart';

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
    final engine = ref.watch(syncEngineProvider);
    final tags = ref.watch(tagsProvider);
    final selected = ref.watch(selectedTagProvider);

    if (!engine.isOnline) {
      return const _Hint(
        icon: Icons.cloud_off,
        text: 'Tags need the server.\nReconnect to browse them.',
      );
    }

    if (selected != null) {
      return _TaggedNotes(tag: selected, onOpen: onOpen);
    }

    return tags.when(
      loading: () => const Center(child: CircularProgressIndicator()),
      error: (e, _) => _Hint(icon: Icons.error_outline, text: '$e'),
      data: (all) {
        if (all.isEmpty) {
          return const _Hint(
            icon: Icons.label_outline,
            text:
                'No tags in this vault yet.\n'
                'Add #a-tag to a note, or tags: in its frontmatter.',
          );
        }

        final groups = <String, List<TagCount>>{};
        for (final t in all) {
          groups.putIfAbsent(t.topLevel, () => []).add(t);
        }
        final roots = groups.keys.toList()..sort();

        return ListView.builder(
          padding: const EdgeInsets.symmetric(vertical: 6),
          itemCount: roots.length,
          itemBuilder: (c, i) => _TagGroup(
            root: roots[i],
            members: groups[roots[i]]!,
            onSelect: (tag) =>
                ref.read(selectedTagProvider.notifier).state = tag,
          ),
        );
      },
    );
  }
}

class _TagGroup extends StatefulWidget {
  const _TagGroup({
    required this.root,
    required this.members,
    required this.onSelect,
  });

  final String root;
  final List<TagCount> members;
  final void Function(String) onSelect;

  @override
  State<_TagGroup> createState() => _TagGroupState();
}

class _TagGroupState extends State<_TagGroup> {
  bool _expanded = false;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);

    // A tag with no children is just a row — no disclosure triangle for
    // something that can't expand.
    final exact = widget.members.where((m) => m.tag == widget.root).firstOrNull;
    final children = widget.members.where((m) => m.tag != widget.root).toList();
    final total = widget.members.fold<int>(0, (sum, m) => sum + m.count);

    if (children.isEmpty) {
      return _TagRow(
        label: widget.root,
        count: exact?.count ?? total,
        indent: 10,
        onTap: () => widget.onSelect(widget.root),
      );
    }

    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        InkWell(
          onTap: () => setState(() => _expanded = !_expanded),
          child: Padding(
            padding: const EdgeInsets.fromLTRB(6, 6, 12, 6),
            child: Row(
              children: [
                Icon(
                  _expanded
                      ? Icons.keyboard_arrow_down
                      : Icons.keyboard_arrow_right,
                  size: 16,
                  color: theme.colorScheme.onSurfaceVariant,
                ),
                Expanded(
                  child: Text(
                    '#${widget.root}',
                    style: theme.textTheme.bodyMedium?.copyWith(
                      fontWeight: FontWeight.w600,
                    ),
                    overflow: TextOverflow.ellipsis,
                  ),
                ),
                Text('$total', style: theme.textTheme.bodySmall),
              ],
            ),
          ),
        ),
        if (_expanded) ...[
          if (exact != null)
            _TagRow(
              label: exact.tag,
              count: exact.count,
              indent: 30,
              onTap: () => widget.onSelect(exact.tag),
            ),
          for (final child in children)
            _TagRow(
              label: child.tag,
              count: child.count,
              indent: 30,
              onTap: () => widget.onSelect(child.tag),
            ),
        ],
      ],
    );
  }
}

class _TagRow extends StatelessWidget {
  const _TagRow({
    required this.label,
    required this.count,
    required this.indent,
    required this.onTap,
  });

  final String label;
  final int count;
  final double indent;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return InkWell(
      onTap: onTap,
      child: Padding(
        padding: EdgeInsets.fromLTRB(indent, 5, 12, 5),
        child: Row(
          children: [
            Expanded(
              child: Text(
                '#$label',
                style: theme.textTheme.bodyMedium?.copyWith(
                  color: theme.colorScheme.primary,
                ),
                overflow: TextOverflow.ellipsis,
              ),
            ),
            Text('$count', style: theme.textTheme.bodySmall),
          ],
        ),
      ),
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
    final notes = ref.watch(notesWithTagProvider(tag));
    final theme = Theme.of(context);

    return Column(
      children: [
        InkWell(
          onTap: () => ref.read(selectedTagProvider.notifier).state = null,
          child: Padding(
            padding: const EdgeInsets.fromLTRB(8, 10, 12, 10),
            child: Row(
              children: [
                const Icon(Icons.arrow_back, size: 16),
                const SizedBox(width: 6),
                Expanded(
                  child: Text(
                    '#$tag',
                    style: theme.textTheme.bodyMedium?.copyWith(
                      fontWeight: FontWeight.w600,
                      color: theme.colorScheme.primary,
                    ),
                    overflow: TextOverflow.ellipsis,
                  ),
                ),
              ],
            ),
          ),
        ),
        const Divider(height: 1),
        Expanded(
          child: notes.when(
            loading: () => const Center(child: CircularProgressIndicator()),
            error: (e, _) => _Hint(icon: Icons.error_outline, text: '$e'),
            data: (list) => list.isEmpty
                ? const _Hint(
                    icon: Icons.label_off_outlined,
                    text: 'No notes carry this tag.',
                  )
                : ListView.separated(
                    itemCount: list.length,
                    separatorBuilder: (_, _) => const Divider(height: 1),
                    itemBuilder: (c, i) => ListTile(
                      dense: true,
                      title: Text(
                        list[i].title.isEmpty ? list[i].path : list[i].title,
                        overflow: TextOverflow.ellipsis,
                      ),
                      subtitle: Text(
                        list[i].path,
                        style: theme.textTheme.bodySmall,
                        overflow: TextOverflow.ellipsis,
                      ),
                      onTap: () => onOpen(list[i]),
                    ),
                  ),
          ),
        ),
      ],
    );
  }
}

class _Hint extends StatelessWidget {
  const _Hint({required this.icon, required this.text});

  final IconData icon;
  final String text;

  @override
  Widget build(BuildContext context) {
    final muted = Theme.of(context).colorScheme.onSurfaceVariant;
    return Center(
      child: Padding(
        padding: const EdgeInsets.all(24),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Icon(icon, size: 28, color: muted),
            const SizedBox(height: 10),
            Text(
              text,
              textAlign: TextAlign.center,
              style: Theme.of(
                context,
              ).textTheme.bodySmall?.copyWith(color: muted),
            ),
          ],
        ),
      ),
    );
  }
}
