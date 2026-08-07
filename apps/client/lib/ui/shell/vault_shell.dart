import 'package:flutter/material.dart';
import 'package:go_router/go_router.dart';

import '../../router.dart';
import '../breakpoints.dart';
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
class VaultShell extends StatelessWidget {
  const VaultShell({super.key, required this.child});

  final Widget child;

  @override
  Widget build(BuildContext context) {
    final vaultId = Routes.vaultOf(GoRouterState.of(context).uri);

    return VaultGate(
      vaultId: vaultId,
      child: context.isExpanded
          ? Scaffold(
              body: Row(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  const VaultSidebar(),
                  const VerticalDivider(width: 1),
                  Expanded(child: child),
                ],
              ),
            )
          : child,
    );
  }
}
