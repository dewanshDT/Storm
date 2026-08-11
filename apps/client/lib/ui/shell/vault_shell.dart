import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';

import '../../keyboard/storm_global_shortcuts.dart';
import '../../router.dart';
import '../../state/app_state.dart';
import '../../state/vault_config.dart' show kColorKey;
import '../breakpoints.dart';
import '../browse_screen.dart' show createFolder;
import '../new_note_dialog.dart';
import 'nav_bubble.dart';
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
  Future<void> _createNote() async {
    // The folder follows where they are rather than being typed — a note made
    // from inside `Projects/Storm` almost always belongs there.
    final here = Routes.folderOf(GoRouterState.of(context).uri);
    final wanted = await promptForNewNote(context, folder: here);
    if (wanted == null || !mounted) return;

    final created = await ref
        .read(syncEngineProvider)
        .create(
          path: noteFileName(wanted.name, folder: here),
          // A colour chosen up front is just the note's first property.
          content: wanted.accent.isNone
              ? ''
              : '---\n$kColorKey: ${wanted.accent.name}\n---\n\n',
        );
    if (!mounted) return;

    if (created.meta == null) {
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(content: Text(created.error ?? 'Could not create the note')),
      );
      return;
    }
    ref.invalidate(treeProvider);
    final vaultId = Routes.vaultOf(GoRouterState.of(context).uri);
    context.push(Routes.note(vaultId, created.meta!.id));
  }

  /// Creating a folder, offered alongside it.
  Future<void> _createFolder() async {
    final here = Routes.folderOf(GoRouterState.of(context).uri);
    await createFolder(
      context,
      ref,
      Routes.vaultOf(GoRouterState.of(context).uri),
      here,
    );
  }

  @override
  Widget build(BuildContext context) {
    final vaultId = Routes.vaultOf(GoRouterState.of(context).uri);
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
                        if (!collapsed) ...[
                          const VaultSidebar(),
                          const VerticalDivider(width: 1),
                        ],
                        Expanded(child: widget.child),
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
