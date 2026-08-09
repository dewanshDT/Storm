import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';

import '../../api/models.dart';
import '../../state/app_state.dart';
import '../../router.dart';
import '../accents.dart';
import '../tokens.dart';
import '../browse_screen.dart' show EntryTile, childrenOfFolder;
import 'vault_actions.dart';

/// How wide the sidebar is. Narrow enough to leave the note its column, wide
/// enough for two levels of indent and a filename.
const kSidebarWidth = 260.0;

/// The wide-screen navigation rail.
///
/// Ordered as the design has it, and the order is the point: **which vault**
/// you are in, **search** as the fastest way to a note, then the folders, and
/// only then the actions. The actions sit at the *bottom* because they are the
/// least-used thing on the rail — the previous version put them on top, where
/// they were the first thing the eye hit and the last thing anyone wanted.
class VaultSidebar extends ConsumerWidget {
  const VaultSidebar({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final t = context.tokens;
    final theme = Theme.of(context);
    final uri = GoRouterState.of(context).uri;
    final notes = ref.watch(treeProvider);
    final known = ref.watch(vaultFoldersProvider);
    final vaultId = ref.watch(activeVaultProvider);
    final vault = ref
        .watch(vaultsProvider)
        .value
        ?.where((v) => v.id == vaultId)
        .firstOrNull;
    final accent =
        ref.watch(vaultAccentsProvider).value?[vaultId] ?? Accent.none;

    // `Material`, not a coloured `Container`: ListTile paints its background
    // and ink splashes onto the nearest Material ancestor, so a plain
    // ColoredBox here would swallow every tap ripple in the tree.
    return Material(
      color: t.bg,
      child: SizedBox(
        width: kSidebarWidth,
        child: SafeArea(
          right: false,
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              _VaultSwitcher(vault: vault, accent: accent),
              Padding(
                padding: EdgeInsets.fromLTRB(t.sp * 1.5, 0, t.sp * 1.5, t.sp),
                child: _SearchField(vaultId: vaultId),
              ),
              Expanded(
                child: notes.when(
                  loading: () =>
                      const Center(child: CircularProgressIndicator()),
                  error: (e, _) => Padding(
                    padding: EdgeInsets.all(t.cardPad),
                    child: Text('$e', style: theme.textTheme.bodySmall),
                  ),
                  data: (list) => FolderTree(notes: list, knownFolders: known),
                ),
              ),
              Divider(height: t.bw, color: t.border),
              // The same actions the floating pill carries on a phone, drawn
              // from the same list so the two can never offer different things.
              Padding(
                padding: EdgeInsets.symmetric(
                  horizontal: t.sp,
                  vertical: t.sp * 0.75,
                ),
                child: Row(
                  children: [
                    for (final action in vaultActions(context, ref, uri))
                      IconButton(
                        icon: Icon(action.icon, size: 18),
                        tooltip: action.tooltip,
                        color: action.selected ? t.accent : t.text3,
                        visualDensity: VisualDensity.compact,
                        onPressed: action.onTap,
                      ),
                  ],
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

/// Which vault you are in, and the way out of it.
class _VaultSwitcher extends StatelessWidget {
  const _VaultSwitcher({required this.vault, required this.accent});

  final VaultInfo? vault;
  final Accent accent;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return InkWell(
      onTap: () => context.go(Routes.dashboard),
      child: Padding(
        padding: EdgeInsets.all(t.sp * 1.5),
        child: Row(
          children: [
            Container(
              width: 30,
              height: 30,
              alignment: Alignment.center,
              decoration: BoxDecoration(
                color: accent.tile(t),
                borderRadius: BorderRadius.circular(t.rControl),
              ),
              child: Text(
                (vault?.name.trim().isNotEmpty ?? false)
                    ? vault!.name.trim()[0].toUpperCase()
                    : 'S',
                style: TextStyle(
                  fontFamily: StormTokens.sansFamily,
                  fontSize: t.codeSize,
                  fontWeight: FontWeight.w600,
                  color: const Color(0xFF1A1626),
                ),
              ),
            ),
            SizedBox(width: t.sp * 1.25),
            Expanded(
              child: Text(
                vault?.name ?? 'Storm',
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: TextStyle(
                  fontFamily: StormTokens.sansFamily,
                  fontSize: t.bodySize,
                  fontWeight: FontWeight.w600,
                  color: t.text,
                ),
              ),
            ),
            Icon(Icons.expand_more, size: 16, color: t.text3),
          ],
        ),
      ),
    );
  }
}

/// A way into search without leaving the rail.
class _SearchField extends StatelessWidget {
  const _SearchField({required this.vaultId});

  final String vaultId;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    // Not a live text field: tapping opens the search screen, which owns the
    // query and its results. A second input here would be a second place the
    // search state lives.
    return InkWell(
      borderRadius: BorderRadius.circular(t.rControl),
      onTap: vaultId.isEmpty ? null : () => context.go(Routes.search(vaultId)),
      child: Container(
        padding: EdgeInsets.symmetric(
          horizontal: t.sp * 1.25,
          vertical: t.sp * 1.1,
        ),
        decoration: BoxDecoration(
          color: t.surface2,
          borderRadius: BorderRadius.circular(t.rControl),
          border: Border.all(color: t.border, width: t.bw),
        ),
        child: Row(
          children: [
            Icon(Icons.search, size: 15, color: t.text3),
            SizedBox(width: t.sp),
            Text(
              'Search…',
              style: TextStyle(
                fontFamily: StormTokens.sansFamily,
                fontSize: t.codeSize,
                color: t.text3,
              ),
            ),
          ],
        ),
      ),
    );
  }
}

/// The vault's folders, expandable in place.
///
/// Deliberately *not* the phone's drill-down list wearing a different hat.
/// They are different navigation models — one replaces the screen, one opens
/// a branch while everything else stays visible — and a single widget doing
/// both would carry two sets of rules. What they share is the level below:
/// [childrenOfFolder] derives every level here exactly as it does there, and
/// [EntryTile] draws every row, so a folder looks and behaves the same in both.
class FolderTree extends StatefulWidget {
  const FolderTree({
    super.key,
    required this.notes,
    required this.knownFolders,
  });

  final List<NoteMeta> notes;
  final List<String> knownFolders;

  @override
  State<FolderTree> createState() => _FolderTreeState();
}

class _FolderTreeState extends State<FolderTree> {
  /// Expanded folder paths.
  ///
  /// Ordinary widget state, not a provider — the rule recorded in
  /// `nav_bubble.dart`: the providers here drive sync and cache, and pushing
  /// ephemeral UI state into them is how the editor once got rebuilt mid-open.
  /// It survives opening a note because the sidebar lives in a `ShellRoute`,
  /// which rebuilds only the pane beside it.
  final _expanded = <String>{};

  /// The note this tree last auto-expanded for, so opening a second note in
  /// the same folder does not fight a fold the user just closed.
  String? _revealedFor;

  @override
  Widget build(BuildContext context) {
    _revealOpenNote(context);

    final rows = <Widget>[];
    _appendLevel(rows, folder: '', depth: 0);

    if (rows.isEmpty) {
      return Center(
        child: Padding(
          padding: const EdgeInsets.all(24),
          child: Text(
            'This vault is empty',
            style: Theme.of(context).textTheme.bodySmall,
          ),
        ),
      );
    }
    return ListView(padding: const EdgeInsets.only(bottom: 24), children: rows);
  }

  /// Opens the folders leading to the note in the URL.
  ///
  /// A deep link should land with its surroundings visible rather than at a
  /// collapsed root with the note highlighted somewhere the user cannot see.
  void _revealOpenNote(BuildContext context) {
    final uri = GoRouterState.of(context).uri;
    final segments = uri.pathSegments;
    if (segments.length < 4 || segments[2] != 'note') return;

    final noteId = segments[3];
    if (noteId == _revealedFor) return;
    _revealedFor = noteId;

    final open = widget.notes.where((n) => n.id == noteId).firstOrNull;
    if (open == null) return;

    // Every ancestor of the note's folder, so `a/b/c` opens a, a/b and a/b/c.
    final parts = open.folder.split('/').where((p) => p.isNotEmpty).toList();
    for (var i = 1; i <= parts.length; i++) {
      _expanded.add(parts.take(i).join('/'));
    }
  }

  /// Rows for one folder's children, recursing into the expanded ones.
  void _appendLevel(
    List<Widget> rows, {
    required String folder,
    required int depth,
  }) {
    final entries = childrenOfFolder(widget.notes, folder, widget.knownFolders);
    final openNoteId = _openNoteId(context);

    for (final entry in entries) {
      if (entry.isFolder) {
        final isOpen = _expanded.contains(entry.path);
        rows.add(
          EntryTile(
            key: ValueKey('folder:${entry.path}'),
            entry: entry,
            contentPadding: _indent(depth),
            leading: Icon(
              isOpen ? Icons.expand_more : Icons.chevron_right,
              size: 18,
              color: Theme.of(context).colorScheme.onSurfaceVariant,
            ),
            onFolderTap: () => setState(() {
              if (!_expanded.remove(entry.path)) _expanded.add(entry.path);
            }),
          ),
        );
        if (isOpen) _appendLevel(rows, folder: entry.path, depth: depth + 1);
      } else {
        rows.add(
          EntryTile(
            key: ValueKey('note:${entry.note!.id}'),
            entry: entry,
            contentPadding: _indent(depth),
            selected: entry.note!.id == openNoteId,
            replaceRoute: true,
          ),
        );
      }
    }
  }

  String? _openNoteId(BuildContext context) {
    final segments = GoRouterState.of(context).uri.pathSegments;
    return segments.length > 3 && segments[2] == 'note' ? segments[3] : null;
  }

  EdgeInsets _indent(int depth) =>
      EdgeInsets.only(left: 8 + depth * 14.0, right: 8);
}

/// What the pane beside the sidebar shows when no note is open.
class NoNoteSelected extends StatelessWidget {
  const NoNoteSelected({super.key});

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    return Scaffold(
      body: Center(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Icon(
              Icons.article_outlined,
              size: 32,
              color: scheme.onSurfaceVariant,
            ),
            const SizedBox(height: 10),
            Text(
              'Select a note',
              style: TextStyle(color: scheme.onSurfaceVariant),
            ),
          ],
        ),
      ),
    );
  }
}
