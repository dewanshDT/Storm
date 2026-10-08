import 'package:flutter/material.dart';
import 'package:flutter_lucide/flutter_lucide.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';

import '../../api/models.dart';
import '../../keyboard/storm_activators.dart' show stormChordLabel;
import '../../router.dart';
import '../../state/agent_writes.dart' show unseenNotesProvider;
import '../../state/app_state.dart';
import '../browse_screen.dart' show childrenOfFolder, showFolderActions;
import '../controls.dart';
import '../icons.dart';
import '../states.dart';
import '../tokens.dart';
import '../widgets.dart';
import 'nav_bubble.dart' show NewFolderRequest, NewNoteRequest;
import 'sidebar_frame.dart';
import 'sidebar_rows.dart';
import 'vault_gate.dart';
import 'vault_header.dart';

/// The sidebar's floor; its width is `context.sidebarWidth`.
const kSidebarWidth = 260.0;

/// The desktop Notes sidebar (handoff §2.2): vault header, search, the
/// cross-vault RECENT rows, the FOLDERS tree, and the create footer.
class VaultSidebar extends ConsumerWidget {
  const VaultSidebar({super.key, this.unseen});

  /// Whether a note changed under an agent since it was last opened here;
  /// by default, [unseenNotesProvider] for the vault.
  final bool Function(String noteId)? unseen;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final t = context.tokens;
    final notes = ref.watch(treeProvider);
    final known = ref.watch(vaultFoldersProvider);
    final vaultId = ref.watch(activeVaultProvider);
    final unseenIds = ref.watch(unseenNotesProvider(vaultId));
    final vaultName =
        (ref.watch(vaultsProvider).value ?? const <VaultInfo>[])
            .where((v) => v.id == vaultId)
            .firstOrNull
            ?.name ??
        '';

    return SidebarFrame(
      body: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Padding(
            padding: EdgeInsets.fromLTRB(
              t.sp * 1.5,
              t.sp * 2,
              t.sp * 1.5,
              t.sp,
            ),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              spacing: t.sp,
              children: [
                const VaultHeader(),
                _SearchField(vaultId: vaultId, vaultName: vaultName),
              ],
            ),
          ),
          Expanded(
            child: ListView(
              padding: EdgeInsets.fromLTRB(t.sp, t.sp * 0.5, t.sp, t.sp * 1.5),
              children: [
                const _Recents(),
                _Label('Folders', top: t.sp * 2),
                notes.when(
                  loading: () => const SkeletonRows(),
                  error: (e, _) => EmptyState(
                    icon: LucideIcons.cloud_off,
                    title: 'Could not list this vault',
                    detail: describeFailure(e),
                  ),
                  data: (list) => FolderTree(
                    notes: list,
                    knownFolders: known,
                    unseen: unseen ?? unseenIds.contains,
                  ),
                ),
              ],
            ),
          ),
        ],
      ),
      footer: const _Footer(),
    );
  }
}

class _Label extends StatelessWidget {
  const _Label(this.text, {required this.top});

  final String text;
  final double top;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Padding(
      padding: EdgeInsets.fromLTRB(t.sp, top, t.sp, t.sp * 0.75),
      child: SectionLabel(text),
    );
  }
}

/// The four most recently opened notes, from every vault.
class _Recents extends ConsumerWidget {
  const _Recents();

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final t = context.tokens;
    final recents = (ref.watch(recentsProvider).value ?? const <RecentNote>[])
        .take(4)
        .toList();
    if (recents.isEmpty) return const SizedBox.shrink();
    final open = openNoteIdOf(GoRouterState.of(context).uri);

    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        _Label('Recent', top: t.sp * 1.25),
        for (final r in recents)
          SidebarRow(
            key: Key('recent:${r.noteId}'),
            label: r.displayTitle,
            vertical: t.sp * 0.75,
            selected: r.noteId == open,
            trailing: VaultTag(r.vaultName),
            onTap: () => context.go(Routes.note(r.vaultId, r.noteId)),
          ),
      ],
    );
  }
}

/// The note id in a `/v/<vault>/note/<id>` location, else null.
String? openNoteIdOf(Uri uri) {
  final s = uri.pathSegments;
  return s.length > 3 && s[2] == 'note' ? s[3] : null;
}

/// Opens the search screen; not a live field, so the query lives in one place.
class _SearchField extends StatelessWidget {
  const _SearchField({required this.vaultId, required this.vaultName});

  final String vaultId;
  final String vaultName;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final style = TextStyle(
      fontFamily: StormTokens.sansFamily,
      fontSize: t.codeSize,
      color: t.text3,
    );
    return InkWell(
      key: const Key('sidebar-search'),
      borderRadius: BorderRadius.circular(t.rControl),
      onTap: vaultId.isEmpty ? null : () => context.go(Routes.search(vaultId)),
      child: Container(
        padding: EdgeInsets.symmetric(horizontal: t.sp * 1.25, vertical: t.sp),
        decoration: BoxDecoration(
          color: t.surface2,
          borderRadius: BorderRadius.circular(t.rControl),
          border: Border.all(color: t.border, width: t.bw),
        ),
        child: Row(
          children: [
            Icon(LucideIcons.search, size: t.uiSize, color: t.text3),
            SizedBox(width: t.sp),
            Expanded(
              child: Text(
                vaultName.isEmpty ? 'Search' : 'Search $vaultName',
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: style,
              ),
            ),
            Text(
              stormChordLabel('K'),
              style: style.copyWith(
                fontFamily: StormTokens.monoFamily,
                fontSize: t.labelSize,
              ),
            ),
          ],
        ),
      ),
    );
  }
}

/// New note, plus the folder and tags squares beside it.
class _Footer extends StatelessWidget {
  const _Footer();

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final uri = GoRouterState.of(context).uri;
    final vaultId = Routes.vaultOf(uri);
    final newFolder = NewFolderRequest.of(context);
    final onTags = uri.path == Routes.tags(vaultId);

    return Padding(
      padding: EdgeInsets.symmetric(
        horizontal: t.sp * 1.5,
        vertical: t.sp * 1.25,
      ),
      child: SizedBox(
        height: t.sp * 4.25,
        child: Row(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          spacing: t.sp * 0.75,
          children: [
            Expanded(
              child: Tooltip(
                message: 'New note',
                child: StormButton.primary(
                  label: 'New note',
                  icon: LucideIcons.plus,
                  expand: true,
                  onPressed: () => NewNoteRequest.of(context)?.call(),
                ),
              ),
            ),
            if (newFolder != null)
              _SquareAction(
                tooltip: 'New folder',
                onTap: newFolder,
                child: Icon(LucideIcons.folder, size: t.uiSize, color: t.text2),
              ),
            _SquareAction(
              tooltip: 'Tags',
              selected: onTags,
              onTap: vaultId.isEmpty
                  ? null
                  : () => context.go(Routes.tags(vaultId)),
              child: StormIcon(
                StormGlyph.hash,
                size: t.uiSize,
                color: onTags ? t.accent : t.text2,
              ),
            ),
          ],
        ),
      ),
    );
  }
}

class _SquareAction extends StatelessWidget {
  const _SquareAction({
    required this.tooltip,
    required this.onTap,
    required this.child,
    this.selected = false,
  });

  final String tooltip;
  final VoidCallback? onTap;
  final Widget child;
  final bool selected;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final shape = RoundedRectangleBorder(
      borderRadius: BorderRadius.circular(t.rControl),
      side: BorderSide(color: selected ? t.accent : t.border, width: t.bw),
    );
    return Tooltip(
      message: tooltip,
      child: Material(
        color: Colors.transparent,
        shape: shape,
        child: InkWell(
          onTap: onTap,
          customBorder: shape,
          child: SizedBox(
            width: t.sp * 4.5,
            child: Center(child: child),
          ),
        ),
      ),
    );
  }
}

/// The vault's folders, expandable in place. The phone's drill-down list
/// shares [childrenOfFolder], so both derive every level the same way.
class FolderTree extends ConsumerStatefulWidget {
  const FolderTree({
    super.key,
    required this.notes,
    required this.knownFolders,
    this.unseen,
  });

  final List<NoteMeta> notes;
  final List<String> knownFolders;
  final bool Function(String noteId)? unseen;

  @override
  ConsumerState<FolderTree> createState() => _FolderTreeState();
}

class _FolderTreeState extends ConsumerState<FolderTree> {
  // Widget state, not a provider: the sidebar lives in a ShellRoute, so it
  // survives opening a note without touching the providers that drive sync.
  final _expanded = <String>{};
  String? _revealedFor;

  @override
  Widget build(BuildContext context) {
    final uri = GoRouterState.of(context).uri;
    final open = openNoteIdOf(uri);
    _revealOpenNote(open);

    final rows = <Widget>[];
    _appendLevel(rows, folder: '', depth: 0, open: open);
    if (rows.isEmpty) {
      return const EmptyState(
        icon: LucideIcons.folder_open,
        title: 'This vault is empty',
      );
    }
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: rows,
    );
  }

  /// A deep link lands with the note's folders open, once per note, so a fold
  /// the user closes afterwards stays closed.
  void _revealOpenNote(String? noteId) {
    if (noteId == null || noteId == _revealedFor) return;
    _revealedFor = noteId;
    final note = widget.notes.where((n) => n.id == noteId).firstOrNull;
    if (note == null) return;
    final parts = note.folder.split('/').where((p) => p.isNotEmpty).toList();
    for (var i = 1; i <= parts.length; i++) {
      _expanded.add(parts.take(i).join('/'));
    }
  }

  bool _unseenUnder(String folder) {
    final unseen = widget.unseen;
    if (unseen == null) return false;
    final prefix = '$folder/';
    return widget.notes.any((n) => n.path.startsWith(prefix) && unseen(n.id));
  }

  void _appendLevel(
    List<Widget> rows, {
    required String folder,
    required int depth,
    required String? open,
  }) {
    final t = context.tokens;
    final vaultId = VaultGate.of(context);
    final indent = depth * t.sp * 1.75;
    final entries = childrenOfFolder(widget.notes, folder, widget.knownFolders);

    for (final entry in entries) {
      if (entry.isFolder) {
        final isOpen = _expanded.contains(entry.path);
        rows.add(
          SidebarRow(
            key: ValueKey('folder:${entry.path}'),
            label: entry.name,
            indent: indent,
            leading: Twisty(open: isOpen),
            unseen: !isOpen && _unseenUnder(entry.path),
            onTap: () => setState(() {
              if (!_expanded.remove(entry.path)) _expanded.add(entry.path);
            }),
            onLongPress: () => showFolderActions(context, ref, vaultId, entry),
          ),
        );
        if (isOpen) {
          _appendLevel(rows, folder: entry.path, depth: depth + 1, open: open);
        }
      } else {
        final note = entry.note!;
        rows.add(
          _NoteTreeRow(
            key: ValueKey('note:${note.id}'),
            note: note,
            name: entry.name,
            indent: indent,
            selected: note.id == open,
            unseen: widget.unseen?.call(note.id) ?? false,
            onTap: () => context.go(Routes.note(vaultId, note.id)),
          ),
        );
      }
    }
  }
}

class _NoteTreeRow extends ConsumerWidget {
  const _NoteTreeRow({
    super.key,
    required this.note,
    required this.name,
    required this.indent,
    required this.selected,
    required this.unseen,
    required this.onTap,
  });

  final NoteMeta note;
  final String name;
  final double indent;
  final bool selected;
  final bool unseen;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final t = context.tokens;
    final pinned = ref.watch(pinnedNotesProvider).value ?? const <String>{};
    return SidebarRow(
      label: name,
      indent: indent,
      // The empty twisty column keeps a note's title under its siblings'.
      leading: SizedBox(width: t.sp * 1.5),
      selected: selected,
      unseen: unseen,
      trailing: pinned.contains(note.id)
          ? Icon(LucideIcons.pin, size: t.labelSize, color: t.accent)
          : null,
      onTap: onTap,
    );
  }
}

/// The pane beside the sidebar when no note is open.
class NoNoteSelected extends StatelessWidget {
  const NoNoteSelected({super.key});

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Scaffold(
      body: Center(
        child: Text(
          'Select a note, or press ${stormChordLabel('K')} to search',
          style: TextStyle(
            fontFamily: StormTokens.sansFamily,
            fontSize: t.uiSize,
            color: t.text3,
          ),
        ),
      ),
    );
  }
}
