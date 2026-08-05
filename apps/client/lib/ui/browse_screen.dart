import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';

import '../api/models.dart';
import '../router.dart';
import '../state/app_state.dart';
import 'shell/storm_scaffold.dart';

/// One folder at a time, with a breadcrumb back up.
///
/// A drill-down rather than an indented tree: trees compress badly at phone
/// width, where every level costs horizontal space that a title needs, while
/// breadcrumbs stay readable at any depth.
class BrowseScreen extends ConsumerWidget {
  const BrowseScreen({super.key, required this.folder});

  /// Vault-relative, `''` at the root.
  final String folder;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final notes = ref.watch(treeProvider);

    return StormScaffold(
      leading: folder.isEmpty
          ? null
          : IconButton(
              icon: const Icon(Icons.arrow_back),
              onPressed: () => context.go(Routes.folder(_parentOf(folder))),
            ),
      title: const Text('Directory'),
      child: notes.when(
        loading: () => const Center(child: CircularProgressIndicator()),
        error: (e, _) => Center(child: Text('$e')),
        data: (list) {
          final entries = _childrenOf(list, folder);
          return Column(
            children: [
              _Breadcrumbs(folder: folder),
              const Divider(height: 1),
              Expanded(
                child: entries.isEmpty
                    ? const Center(child: Text('This folder is empty'))
                    : ListView.builder(
                        padding: const EdgeInsets.only(bottom: 110),
                        itemCount: entries.length,
                        itemBuilder: (c, i) => _EntryTile(entry: entries[i]),
                      ),
              ),
            ],
          );
        },
      ),
    );
  }
}

/// A folder or a note directly inside the folder being viewed.
class BrowseEntry {
  const BrowseEntry.folder(this.name, this.path)
      : note = null,
        childCount = 0;
  const BrowseEntry.folderWith(this.name, this.path, this.childCount)
      : note = null;
  const BrowseEntry.note(this.name, this.note)
      : path = '',
        childCount = 0;

  final String name;
  final String path;
  final NoteMeta? note;
  final int childCount;

  bool get isFolder => note == null;
}

String _parentOf(String folder) {
  final i = folder.lastIndexOf('/');
  return i < 0 ? '' : folder.substring(0, i);
}

/// Everything directly inside [folder] — one level, not the whole subtree.
///
/// Derived from note paths, exactly as the vault's folders are: the server
/// has no separate folder record because the vault is a directory of files.
List<BrowseEntry> childrenOfFolder(List<NoteMeta> notes, String folder) =>
    _childrenOf(notes, folder);

List<BrowseEntry> _childrenOf(List<NoteMeta> notes, String folder) {
  final prefix = folder.isEmpty ? '' : '$folder/';
  final folders = <String, int>{};
  final direct = <BrowseEntry>[];

  for (final note in notes) {
    if (!note.path.startsWith(prefix)) continue;
    final rest = note.path.substring(prefix.length);
    if (rest.isEmpty) continue;

    final slash = rest.indexOf('/');
    if (slash < 0) {
      direct.add(BrowseEntry.note(_stripExtension(rest), note));
    } else {
      final name = rest.substring(0, slash);
      folders[name] = (folders[name] ?? 0) + 1;
    }
  }

  final folderEntries = folders.entries
      .map((e) => BrowseEntry.folderWith(e.key, '$prefix${e.key}', e.value))
      .toList()
    ..sort((a, b) => a.name.toLowerCase().compareTo(b.name.toLowerCase()));

  direct.sort((a, b) => a.name.toLowerCase().compareTo(b.name.toLowerCase()));

  // Folders first, then notes — matching Obsidian.
  return [...folderEntries, ...direct];
}

String _stripExtension(String fileName) =>
    fileName.endsWith('.md')
        ? fileName.substring(0, fileName.length - 3)
        : fileName;

class _Breadcrumbs extends StatelessWidget {
  const _Breadcrumbs({required this.folder});

  final String folder;

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    final parts = folder.isEmpty ? <String>[] : folder.split('/');

    final crumbs = <Widget>[
      _Crumb(
        label: 'Vault',
        onTap: () => context.go(Routes.browse),
        muted: parts.isNotEmpty,
      ),
    ];
    for (var i = 0; i < parts.length; i++) {
      final path = parts.take(i + 1).join('/');
      crumbs
        ..add(Icon(Icons.chevron_right, size: 16, color: scheme.outline))
        ..add(
          _Crumb(
            label: parts[i],
            onTap: () => context.go(Routes.folder(path)),
            muted: i != parts.length - 1,
          ),
        );
    }

    return SizedBox(
      height: 44,
      child: ListView(
        scrollDirection: Axis.horizontal,
        padding: const EdgeInsets.symmetric(horizontal: 14),
        children: [
          for (final c in crumbs) Center(child: c),
        ],
      ),
    );
  }
}

class _Crumb extends StatelessWidget {
  const _Crumb({required this.label, required this.onTap, required this.muted});

  final String label;
  final VoidCallback onTap;
  final bool muted;

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    return InkWell(
      onTap: onTap,
      borderRadius: BorderRadius.circular(6),
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 4),
        child: Text(
          label,
          style: TextStyle(
            fontSize: 13,
            color: muted ? scheme.onSurfaceVariant : scheme.onSurface,
            fontWeight: muted ? FontWeight.w400 : FontWeight.w600,
          ),
        ),
      ),
    );
  }
}

class _EntryTile extends ConsumerWidget {
  const _EntryTile({required this.entry});

  final BrowseEntry entry;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final scheme = Theme.of(context).colorScheme;

    if (entry.isFolder) {
      return ListTile(
        leading: Icon(Icons.folder_outlined, color: scheme.primary),
        title: Text(entry.name),
        subtitle: Text(
          entry.childCount == 1 ? '1 note' : '${entry.childCount} notes',
        ),
        trailing: const Icon(Icons.chevron_right, size: 18),
        onTap: () => context.go(Routes.folder(entry.path)),
      );
    }

    final pinned = ref.watch(pinnedNotesProvider).value ?? const <String>{};
    return ListTile(
      leading: Icon(Icons.description_outlined, color: scheme.onSurfaceVariant),
      title: Text(entry.name),
      trailing: pinned.contains(entry.note!.id)
          ? Icon(Icons.push_pin, size: 13, color: scheme.primary)
          : null,
      onTap: () => context.go(Routes.note(entry.note!.id)),
    );
  }
}
