import 'package:flutter/material.dart';
import 'package:flutter_lucide/flutter_lucide.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';

import '../../router.dart';
import '../../state/app_state.dart';
import '../icons.dart';
import '../states.dart';
import '../surfaces.dart';
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
    required this.glyph,
    required this.tooltip,
    required this.onTap,
    this.selected = false,
    this.badge,
    this.primary = false,
    this.inSidebar = true,
  });

  /// One of the design's own shapes, not a Material name — the two sets are
  /// close enough to look like a mistake side by side.
  final StormGlyph glyph;
  final String tooltip;
  final VoidCallback onTap;
  final bool selected;

  /// A count shown on the icon, for linked mentions.
  final int? badge;

  /// The one action the bar exists for.
  ///
  /// Drawn as a filled accent circle standing proud of the pill rather than as
  /// an equal slot, because on a notes app "write something" is not one option
  /// among six. Only the toolbar treats it differently; the action itself is
  /// the same data.
  final bool primary;

  /// Whether the wide screen's sidebar footer repeats this action.
  ///
  /// False for Directory and Search: the tree *is* the directory and the field
  /// above it *is* search, so a button for either would be a control that does
  /// what is already on screen. The design's sidebar footer omits both.
  final bool inSidebar;
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
        glyph: StormGlyph.plus,
        tooltip: 'New vault',
        // Primary here too: the pill always has one filled slot, and on the
        // dashboard making a vault is the thing it is for.
        primary: true,
        onTap: () => NewNoteRequest.of(context)?.call(),
      ),
      VaultAction(
        glyph: StormGlyph.folder,
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
      glyph: StormGlyph.folder,
      tooltip: 'Directory',
      inSidebar: false,
      selected: uri.path.startsWith(Routes.browse(vaultId)),
      onTap: () => context.go(Routes.browse(vaultId)),
    ),
    VaultAction(
      glyph: StormGlyph.search,
      tooltip: 'Search',
      inSidebar: false,
      selected: uri.path == Routes.search(vaultId),
      onTap: () => context.go(Routes.search(vaultId)),
    ),
    VaultAction(
      glyph: StormGlyph.plus,
      tooltip: 'New note',
      primary: true,
      onTap: () => NewNoteRequest.of(context)?.call(),
    ),
    // Only where a folder can actually be made — inside a note there is no
    // "here" for it to land in.
    if (NewFolderRequest.of(context) != null)
      VaultAction(
        glyph: StormGlyph.newFolder,
        tooltip: 'New folder',
        onTap: () => NewFolderRequest.of(context)?.call(),
      ),
    // Six slots, always — the design's pill does not change shape as you
    // move around it. Mentions is about *a* note, so outside one it says so
    // rather than disappearing and shifting everything else along.
    VaultAction(
      glyph: StormGlyph.mentions,
      tooltip: inNote
          ? (mentions == 1 ? '1 linked mention' : '$mentions linked mentions')
          : 'Mentions',
      badge: inNote ? mentions : null,
      onTap: () {
        final toggle = NoteContextRequest.of(context);
        if (toggle != null) {
          toggle();
        } else {
          showStormSheet<void>(
            context: context,
            title: 'Mentions',
            heightFactor: 0.4,
            builder: (_) => const EmptyState(
              icon: LucideIcons.link_2,
              title: 'Open a note to see what links to it',
            ),
          );
        }
      },
    ),
    VaultAction(
      glyph: StormGlyph.hash,
      tooltip: 'Tags',
      selected: uri.path == Routes.tags(vaultId),
      onTap: () => context.go(Routes.tags(vaultId)),
    ),
  ];
}
