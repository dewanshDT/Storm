import 'package:flutter/material.dart';
import 'package:flutter_lucide/flutter_lucide.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';

import '../api/models.dart';
import '../router.dart';
import '../state/app_state.dart';
import '../state/vault_config.dart';
import 'breakpoints.dart';
import 'tokens.dart';
import 'widgets.dart';
import 'shell/corner_bubbles.dart' show relativeTime;
import 'shell/nav_bubble.dart' show NewNoteRequest;
import 'states.dart';
import 'shell/storm_scaffold.dart';
import 'shell/vault_sidebar.dart' show NoNoteSelected;
import 'shell/vault_gate.dart';

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
    final vaultId = VaultGate.of(context);
    // Folders created on purpose. Without these an empty folder would be
    // invisible, because everything else here is derived from note paths.
    final known = ref.watch(vaultFoldersProvider);

    // On a wide screen the sidebar *is* the browser, so this route's pane is
    // the empty state rather than the same list twice.
    if (context.isExpanded) return const NoNoteSelected();

    final t = context.tokens;

    return StormScaffold(
      header: _Breadcrumbs(folder: folder, vaultId: vaultId),
      child: notes.when(
        loading: () => Padding(
          padding: EdgeInsets.symmetric(horizontal: t.sp * 2.5),
          child: const SkeletonRows(),
        ),
        error: (e, _) => EmptyState(
          icon: LucideIcons.cloud_off,
          title: 'Could not list this folder',
          detail: describeFailure(e),
          action: 'Try again',
          onAction: () => ref.invalidate(treeProvider),
        ),
        data: (list) {
          final entries = _childrenOf(list, folder, known);
          if (entries.isEmpty) {
            return EmptyState(
              icon: LucideIcons.folder_open,
              title: 'Nothing in this folder',
              detail: folder.isEmpty
                  ? 'New notes will land at the top of this vault.'
                  : 'New notes made here will land in '
                        '${folder.split('/').last}.',
              action: 'New note',
              onAction: () => NewNoteRequest.of(context)?.call(),
            );
          }
          return ListView.builder(
            padding: EdgeInsets.fromLTRB(
              t.cardPad,
              t.sp * 1.25,
              t.cardPad,
              StormChrome.navClearance(context),
            ),
            itemCount: entries.length,
            itemBuilder: (c, i) => EntryTile(entry: entries[i], divider: true),
          );
        },
      ),
    );
  }
}

/// A folder or a note directly inside the folder being viewed.
class BrowseEntry {
  const BrowseEntry.folder(this.name, this.path) : note = null, childCount = 0;
  const BrowseEntry.folderWith(this.name, this.path, this.childCount)
    : note = null;
  const BrowseEntry.note(this.name, this.note) : path = '', childCount = 0;

  final String name;
  final String path;
  final NoteMeta? note;
  final int childCount;

  bool get isFolder => note == null;
}

/// Everything directly inside [folder] — one level, not the whole subtree.
///
/// Derived from note paths, exactly as the vault's folders are: the server
/// has no separate folder record because the vault is a directory of files.
List<BrowseEntry> childrenOfFolder(
  List<NoteMeta> notes,
  String folder, [
  List<String> knownFolders = const [],
]) => _childrenOf(notes, folder, knownFolders);

List<BrowseEntry> _childrenOf(
  List<NoteMeta> notes,
  String folder,
  List<String> knownFolders,
) {
  final prefix = folder.isEmpty ? '' : '$folder/';
  final folders = <String, int>{};
  final direct = <BrowseEntry>[];

  for (final note in notes) {
    // `_storm/` is Storm's own configuration, not the user's notes.
    if (isVaultConfigPath(note.path)) continue;
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

  // Folders the server knows about that no note put here — the empty ones.
  for (final known in knownFolders) {
    if (isVaultConfigPath('$known/')) continue;
    if (!known.startsWith(prefix)) continue;
    final rest = known.substring(prefix.length);
    if (rest.isEmpty || rest.contains('/')) continue;
    folders.putIfAbsent(rest, () => 0);
  }

  final folderEntries =
      folders.entries
          .map((e) => BrowseEntry.folderWith(e.key, '$prefix${e.key}', e.value))
          .toList()
        ..sort((a, b) => a.name.toLowerCase().compareTo(b.name.toLowerCase()));

  direct.sort((a, b) => a.name.toLowerCase().compareTo(b.name.toLowerCase()));

  // Folders first, then notes — matching Obsidian.
  return [...folderEntries, ...direct];
}

String _stripExtension(String fileName) => fileName.endsWith('.md')
    ? fileName.substring(0, fileName.length - 3)
    : fileName;

class _Breadcrumbs extends ConsumerWidget {
  const _Breadcrumbs({required this.folder, required this.vaultId});

  final String folder;
  final String vaultId;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final vaults = ref.watch(vaultsProvider).value ?? const [];
    final name =
        vaults.where((v) => v.id == vaultId).firstOrNull?.name ?? 'Vault';
    final parts = folder.isEmpty ? <String>[] : folder.split('/');

    return Padding(
      padding: EdgeInsets.symmetric(
        horizontal: StormChrome.contentInset(context),
      ),
      child: Breadcrumb(
        crumbs: [
          Crumb('Vaults', onTap: () => context.go(Routes.dashboard)),
          Crumb(name, onTap: () => context.go(Routes.browse(vaultId))),
          for (var i = 0; i < parts.length; i++)
            Crumb(
              parts[i],
              onTap: () => context.go(
                Routes.folder(vaultId, parts.take(i + 1).join('/')),
              ),
            ),
        ],
      ),
    );
  }
}

/// One row in a folder listing.
///
/// Shared by the phone's drill-down browser and the wide screen's folder tree.
/// The two navigate differently — one replaces the screen, the other expands
/// in place — but a folder row has to look and behave the same in both, right
/// down to the long-press actions.
class EntryTile extends ConsumerWidget {
  const EntryTile({
    super.key,
    required this.entry,
    this.onFolderTap,
    this.leading,
    this.selected = false,
    this.replaceRoute = false,
    this.contentPadding,
    this.divider = false,
    this.inTree = false,
  });

  final BrowseEntry entry;

  /// What tapping a folder does. Null means the browser's behaviour: push the
  /// folder as a screen. The tree passes its own, to expand in place.
  final VoidCallback? onFolderTap;

  /// Replaces the folder icon, so the tree can show its twisty.
  final Widget? leading;

  /// Marks the note currently open, for the tree.
  final bool selected;

  /// Navigate with `go` rather than `push`.
  ///
  /// True in the sidebar: the tree stays put and only the pane beside it
  /// changes, so stacking a route per note would make the back gesture walk
  /// through everything you had merely glanced at.
  final bool replaceRoute;

  final EdgeInsetsGeometry? contentPadding;
  final bool divider;

  /// Drawn as a row of the sidebar's tree rather than of the phone's list.
  ///
  /// Explicit, not inferred. It used to be read off `leading != null`, which
  /// silently stopped being true for notes the day the leading spacer was
  /// removed — so every note in the tree started rendering at the full-width
  /// list's size, with a timestamp, and the hierarchy flattened.
  final bool inTree;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final t = context.tokens;
    final vaultId = VaultGate.of(context);

    if (entry.isFolder) {
      return FolderRow(
        name: entry.name,
        dense: inTree,
        leading: leading,
        // The tree shows counts too — the design's sidebar has them, and a
        // folder's weight is the reason to open it or not.
        count: entry.childCount,
        chevron: !inTree,
        divider: divider,
        padding: contentPadding as EdgeInsets?,
        onTap:
            onFolderTap ??
            () => context.push(Routes.folder(vaultId, entry.path)),
        // Long-press rather than a visible menu per row: renaming and deleting
        // folders are rare next to walking into them.
        onLongPress: () => _folderActions(context, ref, vaultId, entry),
      );
    }

    final pinned = ref.watch(pinnedNotesProvider).value ?? const <String>{};
    return NoteRow(
      title: entry.name,
      dense: inTree,
      meta: inTree ? null : _noteMeta(entry.note!),
      leading: leading,
      selected: selected,
      divider: divider,
      padding: contentPadding as EdgeInsets?,
      trailing: pinned.contains(entry.note!.id)
          ? Icon(LucideIcons.pin, size: t.labelSize, color: t.accent)
          : null,
      onTap: () => replaceRoute
          ? context.go(Routes.note(vaultId, entry.note!.id))
          : context.push(Routes.note(vaultId, entry.note!.id)),
    );
  }
}

String? _noteMeta(NoteMeta note) {
  final modified = DateTime.tryParse(note.modified)?.toLocal();
  return modified == null ? null : relativeTime(modified);
}

Future<void> _folderActions(
  BuildContext context,
  WidgetRef ref,
  String vaultId,
  BrowseEntry entry,
) async {
  final action = await showModalBottomSheet<String>(
    context: context,
    builder: (c) => SafeArea(
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          ListTile(
            leading: const Icon(LucideIcons.pencil_line),
            title: Text('Rename “${entry.name}”'),
            onTap: () => Navigator.pop(c, 'rename'),
          ),
          ListTile(
            leading: Icon(
              LucideIcons.trash_2,
              color: Theme.of(c).colorScheme.error,
            ),
            title: const Text('Delete folder'),
            onTap: () => Navigator.pop(c, 'delete'),
          ),
        ],
      ),
    ),
  );
  if (action == null || !context.mounted) return;

  final api = ref.read(apiProvider);
  if (api == null) return;

  if (action == 'rename') {
    final name = await promptForPath(
      context,
      title: 'Rename folder',
      initial: entry.name,
      isFolder: true,
    );
    if (name == null || !context.mounted) return;
    final parent = entry.path.contains('/')
        ? entry.path.substring(0, entry.path.lastIndexOf('/'))
        : '';
    final to = parent.isEmpty ? name : '$parent/$name';
    try {
      await api.renameFolder(vaultId, entry.path, to);
      ref.read(vaultRevisionProvider.notifier).state++;
      ref.invalidate(treeProvider);
    } catch (e) {
      if (context.mounted) _toast(context, describeFailure(e));
    }
    return;
  }

  try {
    await api.deleteFolder(vaultId, entry.path);
    ref.read(vaultRevisionProvider.notifier).state++;
    ref.invalidate(treeProvider);
  } catch (e) {
    // The server refuses a folder that still holds notes rather than taking
    // them with it. Surfacing its wording keeps the count accurate.
    if (context.mounted) _toast(context, describeFailure(e));
  }
}

/// Creates a folder inside [parent] (`''` at the vault root).
Future<void> createFolder(
  BuildContext context,
  WidgetRef ref,
  String vaultId,
  String parent,
) async {
  final name = await promptForPath(
    context,
    title: 'New folder',
    initial: '',
    isFolder: true,
  );
  if (name == null || name.trim().isEmpty || !context.mounted) return;

  final api = ref.read(apiProvider);
  if (api == null) return;
  final path = parent.isEmpty ? name : '$parent/$name';
  try {
    await api.createFolder(vaultId, path);
    ref.read(vaultRevisionProvider.notifier).state++;
    ref.invalidate(treeProvider);
  } catch (e) {
    if (context.mounted) _toast(context, describeFailure(e));
  }
}

void _toast(BuildContext context, String message) => ScaffoldMessenger.of(
  context,
).showSnackBar(SnackBar(content: Text(message)));
