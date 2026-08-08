import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';

import '../../router.dart';
import '../../state/app_state.dart';
import 'nav_bubble.dart'
    show NewFolderRequest, NewNoteRequest, NoteContextRequest;

/// One thing you can do from wherever you are.
///
/// A description rather than a widget, because the same five actions are drawn
/// two ways: as a floating pill on a phone, and as a toolbar at the top of the
/// sidebar on a wide screen. Keeping them as data means the two placements
/// cannot drift into offering different things.
class VaultAction {
  const VaultAction({
    required this.icon,
    required this.tooltip,
    required this.onTap,
    this.selected = false,
    this.badge,
  });

  final IconData icon;
  final String tooltip;
  final VoidCallback onTap;
  final bool selected;

  /// A count shown on the icon, for linked mentions.
  final int? badge;
}

/// The actions for the current location.
///
/// Outside a vault — on the dashboard — there is nothing to browse or search,
/// so it offers what does apply there instead.
List<VaultAction> vaultActions(BuildContext context, WidgetRef ref, Uri uri) {
  final vaultId = Routes.vaultOf(uri);

  if (vaultId.isEmpty) {
    return [
      VaultAction(
        icon: Icons.add,
        tooltip: 'New vault',
        onTap: () => NewNoteRequest.of(context)?.call(),
      ),
      VaultAction(
        icon: Icons.dns_outlined,
        tooltip: 'Server',
        onTap: () => context.push(Routes.serverSettings),
      ),
    ];
  }

  final segments = uri.pathSegments;
  // `/v/<vault>/note/<id>` — the marker is the segment after the vault.
  final inNote = segments.length > 3 && segments[2] == 'note';
  final noteId = inNote ? segments[3] : null;
  final mentions = noteId == null
      ? 0
      : (ref.watch(backlinksProvider(noteId)).value?.length ?? 0);

  return [
    VaultAction(
      icon: Icons.folder_outlined,
      tooltip: 'Directory',
      selected: uri.path.startsWith(Routes.browse(vaultId)),
      onTap: () => context.go(Routes.browse(vaultId)),
    ),
    VaultAction(
      icon: Icons.search,
      tooltip: 'Search',
      selected: uri.path == Routes.search(vaultId),
      onTap: () => context.go(Routes.search(vaultId)),
    ),
    VaultAction(
      icon: Icons.add,
      tooltip: 'New note',
      onTap: () => NewNoteRequest.of(context)?.call(),
    ),
    // Only where a folder can actually be made — inside a note there is no
    // "here" for it to land in.
    if (NewFolderRequest.of(context) != null)
      VaultAction(
        icon: Icons.create_new_folder_outlined,
        tooltip: 'New folder',
        onTap: () => NewFolderRequest.of(context)?.call(),
      ),
    // The dynamic slot: tags outside a note, that note's linked mentions
    // inside one. Its meaning follows the router rather than a separate flag,
    // so there is exactly one answer to "where am I".
    if (inNote)
      VaultAction(
        icon: Icons.hub_outlined,
        tooltip: mentions == 1
            ? '1 linked mention'
            : '$mentions linked mentions',
        badge: mentions,
        onTap: () => NoteContextRequest.of(context)?.call(),
      )
    else
      VaultAction(
        icon: Icons.label_outline,
        tooltip: 'Tags',
        selected: uri.path == Routes.tags(vaultId),
        onTap: () => context.go(Routes.tags(vaultId)),
      ),
  ];
}
