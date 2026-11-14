import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';

import '../../router.dart';
import '../../state/app_state.dart';
import '../breakpoints.dart';
import 'create_here.dart';
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
/// It provides `NewNoteRequest` / `NewFolderRequest` for the nav pill and
/// the sidebar toolbar, which run the same create commands as ⌘N and ⇧⌘N.
class VaultShell extends ConsumerStatefulWidget {
  const VaultShell({super.key, required this.child});

  final Widget child;

  @override
  ConsumerState<VaultShell> createState() => _VaultShellState();
}

class _VaultShellState extends ConsumerState<VaultShell> {
  @override
  Widget build(BuildContext context) {
    final vaultId = Routes.vaultOf(GoRouterState.of(context).uri);
    final collapsed = ref.watch(sidebarCollapsedProvider);

    return VaultGate(
      vaultId: vaultId,
      child: NewNoteRequest(
        onRequest: () => createNote(context, ref),
        child: NewFolderRequest(
          onRequest: () => createFolderInVault(context, ref),
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
    );
  }
}
