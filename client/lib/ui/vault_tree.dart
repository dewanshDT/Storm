import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../api/models.dart';
import '../state/app_state.dart';

/// Builds a nested folder tree from the flat note list the server returns.
///
/// The server has no separate folder record — folders are implied by note
/// paths, exactly as they are in an Obsidian vault on disk — so the hierarchy
/// is derived here.
class TreeNode {
  TreeNode(this.name, {this.note});

  final String name;
  final NoteMeta? note;
  final Map<String, TreeNode> children = {};

  bool get isFolder => note == null;

  /// Folders first, then notes, each alphabetically — matching Obsidian.
  List<TreeNode> get sortedChildren {
    final list = children.values.toList()
      ..sort((a, b) {
        if (a.isFolder != b.isFolder) return a.isFolder ? -1 : 1;
        return a.name.toLowerCase().compareTo(b.name.toLowerCase());
      });
    return list;
  }

  static TreeNode build(List<NoteMeta> notes) {
    final root = TreeNode('');
    for (final note in notes) {
      final parts = note.path.split('/');
      var cursor = root;
      for (var i = 0; i < parts.length - 1; i++) {
        cursor = cursor.children.putIfAbsent(parts[i], () => TreeNode(parts[i]));
      }
      final leaf = parts.last;
      cursor.children[leaf] = TreeNode(leaf, note: note);
    }
    return root;
  }
}

class VaultTreePanel extends ConsumerWidget {
  const VaultTreePanel({super.key, required this.onOpen});

  final void Function(NoteMeta) onOpen;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final tree = ref.watch(treeProvider);
    final openId = ref.watch(openNoteIdProvider);

    return tree.when(
      loading: () => const Center(child: CircularProgressIndicator()),
      error: (e, _) => _Error(message: '$e', onRetry: () => ref.invalidate(treeProvider)),
      data: (vault) {
        if (vault.notes.isEmpty) {
          return const _Empty();
        }
        final root = TreeNode.build(vault.notes);
        return ListView(
          padding: const EdgeInsets.symmetric(vertical: 8),
          children: [
            for (final child in root.sortedChildren)
              _TreeRow(node: child, depth: 0, openId: openId, onOpen: onOpen),
          ],
        );
      },
    );
  }
}

class _TreeRow extends StatefulWidget {
  const _TreeRow({
    required this.node,
    required this.depth,
    required this.openId,
    required this.onOpen,
  });

  final TreeNode node;
  final int depth;
  final String? openId;
  final void Function(NoteMeta) onOpen;

  @override
  State<_TreeRow> createState() => _TreeRowState();
}

class _TreeRowState extends State<_TreeRow> {
  bool _expanded = true;

  @override
  Widget build(BuildContext context) {
    final node = widget.node;
    final theme = Theme.of(context);
    final indent = 8.0 + widget.depth * 14;

    if (node.isFolder) {
      return Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          InkWell(
            onTap: () => setState(() => _expanded = !_expanded),
            child: Padding(
              padding: EdgeInsets.fromLTRB(indent, 5, 8, 5),
              child: Row(
                children: [
                  Icon(
                    _expanded ? Icons.keyboard_arrow_down : Icons.keyboard_arrow_right,
                    size: 16,
                    color: theme.colorScheme.onSurfaceVariant,
                  ),
                  const SizedBox(width: 2),
                  Expanded(
                    child: Text(
                      node.name,
                      style: theme.textTheme.bodyMedium?.copyWith(
                        fontWeight: FontWeight.w600,
                      ),
                      overflow: TextOverflow.ellipsis,
                    ),
                  ),
                ],
              ),
            ),
          ),
          if (_expanded)
            for (final child in node.sortedChildren)
              _TreeRow(
                node: child,
                depth: widget.depth + 1,
                openId: widget.openId,
                onOpen: widget.onOpen,
              ),
        ],
      );
    }

    final note = node.note!;
    final selected = note.id == widget.openId;
    return InkWell(
      onTap: () => widget.onOpen(note),
      child: Container(
        color: selected ? theme.colorScheme.primaryContainer.withValues(alpha: 0.5) : null,
        padding: EdgeInsets.fromLTRB(indent + 18, 5, 8, 5),
        child: Text(
          // Strip the extension: the vault shows note names, not filenames.
          note.fileName.endsWith('.md')
              ? note.fileName.substring(0, note.fileName.length - 3)
              : note.fileName,
          style: theme.textTheme.bodyMedium?.copyWith(
            color: selected ? theme.colorScheme.onPrimaryContainer : null,
            fontWeight: selected ? FontWeight.w600 : null,
          ),
          overflow: TextOverflow.ellipsis,
        ),
      ),
    );
  }
}

class _Empty extends StatelessWidget {
  const _Empty();

  @override
  Widget build(BuildContext context) => Center(
        child: Padding(
          padding: const EdgeInsets.all(24),
          child: Text(
            'This vault is empty.\nCreate a note to get started.',
            textAlign: TextAlign.center,
            style: Theme.of(context).textTheme.bodyMedium?.copyWith(
                  color: Theme.of(context).colorScheme.onSurfaceVariant,
                ),
          ),
        ),
      );
}

class _Error extends StatelessWidget {
  const _Error({required this.message, required this.onRetry});

  final String message;
  final VoidCallback onRetry;

  @override
  Widget build(BuildContext context) => Center(
        child: Padding(
          padding: const EdgeInsets.all(20),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              Icon(Icons.cloud_off, color: Theme.of(context).colorScheme.error),
              const SizedBox(height: 10),
              Text(
                message,
                textAlign: TextAlign.center,
                style: Theme.of(context).textTheme.bodySmall,
              ),
              const SizedBox(height: 12),
              OutlinedButton(onPressed: onRetry, child: const Text('Retry')),
            ],
          ),
        ),
      );
}
