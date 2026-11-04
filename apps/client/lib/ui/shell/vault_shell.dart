import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';

import '../../api/models.dart' show NoteMeta;
import '../../keyboard/storm_global_shortcuts.dart';
import '../../router.dart';
import '../../state/app_state.dart';
import '../../state/vault_config.dart' show kColorKey;
import '../breakpoints.dart';
import '../browse_screen.dart' show createFolder;
import '../new_note_dialog.dart';
import 'nav_bubble.dart';
import 'sidebar_frame.dart';
import 'vault_gate.dart';
import 'vault_sidebar.dart';

/// The frame every vault-scoped screen sits in.
///
/// On a phone it is nothing at all — the child *is* the screen, exactly as
/// before. On a wide screen it puts the folder tree beside it. The phone
/// layout is the default and this branch is additive, so anything that changes
/// what compact renders is a defect rather than a design choice.
///
/// It is built by a `ShellRoute`, which is what lets the sidebar keep its
/// state. Wrapping each route's child individually — as this used to — rebuilt
/// the whole subtree on every navigation, so a tree would collapse the moment
/// you opened a note.
///
/// [VaultGate] lives here too, so it wraps once rather than once per route.
///
/// The global shortcut layer (⌘K / ⌘N / ⌘⇧N / ⌘\) also lives here rather than
/// on each screen. The desk-width browse route is the empty pane beside the
/// sidebar — it has no `StormScaffold` for shortcuts to hang from — so the
/// chords and the create callbacks they invoke must sit at the shell, which
/// wraps *every* vault route. Screens inside it still resolve
/// `NewNoteRequest` / `NewFolderRequest` for the nav pill and the sidebar
/// toolbar; this one provider is the callback source for all of them.
class VaultShell extends ConsumerStatefulWidget {
  const VaultShell({super.key, required this.child});

  final Widget child;

  @override
  ConsumerState<VaultShell> createState() => _VaultShellState();
}

class _VaultShellState extends ConsumerState<VaultShell> {
  Uri? _page;
  DateTime _arrived = DateTime.now();

  /// Where a new note or folder goes: whichever was picked last, a folder
  /// row clicked in the sidebar or the page itself (an opened folder, or an
  /// open note's folder); otherwise the top of the vault.
  String _here() {
    final uri = GoRouterState.of(context).uri;
    final picked = ref.read(pickedFolderProvider);
    if (picked != null &&
        picked.vault == Routes.vaultOf(uri) &&
        picked.at.isAfter(_arrived)) {
      return picked.folder;
    }
    final noteId = openNoteIdOf(uri);
    if (noteId != null) {
      final notes = ref.read(treeProvider).value ?? const <NoteMeta>[];
      final note = notes.where((n) => n.id == noteId).firstOrNull;
      if (note != null) return note.folder;
    }
    return Routes.folderOf(uri);
  }

  Future<void> _createNote() async {
    final here = _here();
    NoteMeta? made;
    final wanted = await promptForNewNote(
      context,
      folder: here,
      create: (note) async {
        final path = noteFileName(note.name, folder: here);
        final notes = ref.read(treeProvider).value ?? const <NoteMeta>[];
        if (notes.any((n) => n.path.toLowerCase() == path.toLowerCase())) {
          return 'A note with that name is already '
              '${newNoteDestination(path)}.';
        }
        final created = await ref
            .read(syncEngineProvider)
            .create(
              path: path,
              // A colour chosen up front is just the note's first property.
              content: note.accent.isNone
                  ? ''
                  : '---\n$kColorKey: ${note.accent.name}\n---\n\n',
            );
        made = created.meta;
        if (made != null) return null;
        final why = (created.error ?? 'the server did not make the note')
            .replaceAll(RegExp(r'\.+$'), '');
        return 'Nothing was created: $why.';
      },
    );
    final note = made;
    if (wanted == null || note == null || !mounted) return;
    ref.invalidate(treeProvider);
    final vaultId = Routes.vaultOf(GoRouterState.of(context).uri);
    context.push(Routes.note(vaultId, note.id));
  }

  /// Creating a folder, offered alongside it.
  Future<void> _createFolder() async {
    final here = _here();
    await createFolder(
      context,
      ref,
      Routes.vaultOf(GoRouterState.of(context).uri),
      here,
    );
  }

  @override
  Widget build(BuildContext context) {
    final uri = GoRouterState.of(context).uri;
    if (uri != _page) {
      _page = uri;
      _arrived = DateTime.now();
    }
    final vaultId = Routes.vaultOf(uri);
    final collapsed = ref.watch(sidebarCollapsedProvider);

    return VaultGate(
      vaultId: vaultId,
      // Global shortcuts sit inside the InheritedWidget callbacks so ⌘N / ⌘⇧N
      // hit the same create paths as the nav pill and the sidebar toolbar, and
      // below [VaultGate] so their contexts can resolve the vault.
      child: NewNoteRequest(
        onRequest: _createNote,
        child: NewFolderRequest(
          onRequest: _createFolder,
          child: StormGlobalShortcuts(
            // Phone layout unchanged. At desk width ⌘\ hides the rail so the
            // note pane can use the full width — collapsed is additive chrome,
            // not a second compact layout.
            child: context.isExpanded
                ? Scaffold(
                    body: Row(
                      crossAxisAlignment: CrossAxisAlignment.stretch,
                      children: [
                        if (!collapsed) ...[const VaultSidebar()],
                        Expanded(child: PaneSemantics(child: widget.child)),
                      ],
                    ),
                  )
                : widget.child,
          ),
        ),
      ),
    );
  }
}
