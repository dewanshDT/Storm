import 'package:flutter/material.dart';
import 'package:flutter_lucide/flutter_lucide.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';

import '../../router.dart';
import 'nav_bubble.dart' show NewFolderRequest, NewNoteRequest;

/// One slot of the phone pill.
class VaultAction {
  const VaultAction({
    required this.icon,
    required this.tooltip,
    required this.onTap,
    this.onLongPress,
    this.primary = false,
  });

  /// Lucide's outlined set, as the v2 prototype draws the pill.
  final IconData icon;
  final String tooltip;
  final VoidCallback onTap;
  final VoidCallback? onLongPress;

  /// Drawn as the filled accent circle.
  final bool primary;
}

/// Directory, Search, New note and Tags (handoff §2.4); none outside a vault.
List<VaultAction> vaultActions(BuildContext context, WidgetRef ref, Uri uri) {
  final vaultId = Routes.vaultOf(uri);
  if (vaultId.isEmpty) return const [];

  return [
    VaultAction(
      icon: LucideIcons.folder,
      tooltip: 'Directory',
      onTap: () => context.go(Routes.browse(vaultId)),
    ),
    VaultAction(
      icon: LucideIcons.search,
      tooltip: 'Search',
      onTap: () => context.go(Routes.search(vaultId)),
    ),
    VaultAction(
      icon: LucideIcons.plus,
      tooltip: 'New note',
      primary: true,
      onTap: () => NewNoteRequest.of(context)?.call(),
      // The pill has no folder slot; this keeps folders makeable on a phone.
      onLongPress: NewFolderRequest.of(context),
    ),
    VaultAction(
      icon: LucideIcons.hash,
      tooltip: 'Tags',
      onTap: () => context.go(Routes.tags(vaultId)),
    ),
  ];
}
