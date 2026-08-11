import 'package:flutter/material.dart';
import 'package:flutter_lucide/flutter_lucide.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';

import '../../api/models.dart';
import '../../state/app_state.dart';
import '../../router.dart';
import '../accents.dart';
import '../breakpoints.dart';
import '../icons.dart';
import '../states.dart';
import '../surfaces.dart';
import '../tokens.dart';
import '../widgets.dart';
import '../browse_screen.dart' show EntryTile, childrenOfFolder;
import 'vault_actions.dart';
import 'vault_gate.dart';

/// The sidebar's floor. Its actual width is `context.sidebarWidth`, which
/// scales with the window — see `breakpoints.dart` for why a fixed 260 made a
/// wide display look unbalanced.
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
        width: context.sidebarWidth,
        child: SafeArea(
          right: false,
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              _VaultSwitcher(vault: vault, accent: accent),
              Padding(
                padding: EdgeInsets.fromLTRB(t.sp * 2, 0, t.sp * 2, t.sp * 2),
                child: _SearchField(vaultId: vaultId),
              ),
              Expanded(
                child: notes.when(
                  loading: () => Padding(
                    padding: EdgeInsets.symmetric(horizontal: t.sp * 1.5),
                    child: const SkeletonRows(),
                  ),
                  error: (e, _) => EmptyState(
                    icon: LucideIcons.cloud_off,
                    title: 'Could not list this vault',
                    detail: describeFailure(e),
                  ),
                  data: (list) => Padding(
                    padding: EdgeInsets.symmetric(horizontal: t.sp),
                    child: FolderTree(notes: list, knownFolders: known),
                  ),
                ),
              ),
              Divider(height: t.bw, color: t.border),
              // The same actions the floating pill carries on a phone, drawn
              // from the same list so the two can never offer different
              // things — minus the two the rail already *is*. The tree is the
              // directory and the field above is search, so repeating them
              // here would be two buttons that do what is already on screen.
              Padding(
                padding: EdgeInsets.symmetric(
                  horizontal: t.sp * 1.5,
                  vertical: t.sp * 1.25,
                ),
                child: Row(
                  spacing: t.sp * 0.25,
                  children: [
                    for (final action in vaultActions(
                      context,
                      ref,
                      uri,
                    ).where((a) => a.inSidebar))
                      IconButton(
                        icon: StormIcon(
                          action.glyph,
                          size: t.bodySize,
                          color: action.selected ? t.accent : t.text3,
                        ),
                        iconSize: t.bodySize,
                        tooltip: action.tooltip,
                        visualDensity: VisualDensity.compact,
                        // A 38px square, as the design draws it. At icon-plus-
                        // six the row read as four glyphs dropped on a border
                        // rather than as a toolbar.
                        padding: EdgeInsets.zero,
                        constraints: BoxConstraints.tight(
                          Size.square(t.sp * 4.75),
                        ),
                        onPressed: action.onTap,
                      ),
                    // Last in the row, not pushed to the far edge: the design
                    // groups all four at the left. This is the *appearance*
                    // menu — theme, text size, note font — which lives on the
                    // top-right corner bubble on a phone, and the corners are
                    // empty at this width. Server settings are in the vault
                    // switcher, next to the vault they configure.
                    const _SettingsButton(),
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
/// Which vault you are in, and the way to another one.
///
/// It used to `go(dashboard)` on tap, so the one control labelled with the
/// current vault navigated away from every vault. It opens the switcher now;
/// "All vaults" inside it is what goes home.
class _VaultSwitcher extends ConsumerStatefulWidget {
  const _VaultSwitcher({required this.vault, required this.accent});

  final VaultInfo? vault;
  final Accent accent;

  @override
  ConsumerState<_VaultSwitcher> createState() => _VaultSwitcherState();
}

class _VaultSwitcherState extends ConsumerState<_VaultSwitcher> {
  final _anchor = GlobalKey();
  bool _open = false;

  Future<void> _toggle() async {
    final t = context.tokens;
    setState(() => _open = true);
    await showStormPopover<void>(
      context: context,
      anchorKey: _anchor,
      width: context.sidebarWidth - t.sp * 4,
      builder: (popContext) => Consumer(
        builder: (popContext, ref, _) {
          final engine = ref.watch(syncEngineProvider);
          final activeId = ref.watch(activeVaultProvider);
          final vaults = ref.watch(vaultsProvider).value ?? const [];
          final status = dotStatusFor(
            online: engine.isOnline,
            syncing: engine.isSyncing,
            pending: engine.pendingCount,
          );

          return StormPopover(
            children: [
              Padding(
                padding: EdgeInsets.only(
                  left: t.sp * 0.75,
                  bottom: t.sp * 0.75,
                ),
                child: const SectionLabel('Vaults'),
              ),
              for (final v in vaults)
                PopoverItem(
                  label: v.name,
                  subtitle: v.missing
                      ? 'Directory not found'
                      : '${v.noteCount} note${v.noteCount == 1 ? '' : 's'}',
                  selected: v.id == activeId,
                  leading: StatusDot(
                    status: v.missing
                        ? DotStatus.offline
                        : v.id == activeId
                        ? status
                        : DotStatus.offline,
                  ),
                  onTap: v.missing
                      ? null
                      : () {
                          Navigator.pop(popContext);
                          // Selecting the vault you are already in should do
                          // nothing, not navigate.
                          if (v.id == activeId) return;
                          context.go(Routes.browse(v.id));
                        },
                ),
              const PopoverDivider(),
              // Not "All vaults": there is no dashboard at this width, and an
              // entry that navigates to a screen the layout does not have is
              // worse than no entry. Sync now sits above settings, same order
              // as the phone vault bubble.
              PopoverItem(
                label: 'Sync now',
                tone: PopoverTone.accent,
                onTap: () async {
                  Navigator.pop(popContext);
                  await ref.read(syncEngineProvider).sync();
                  ref.invalidate(treeProvider);
                },
              ),
              PopoverItem(
                label: 'Server settings ›',
                tone: PopoverTone.accent,
                onTap: () {
                  Navigator.pop(popContext);
                  context.push(Routes.serverSettingsIn(activeId));
                },
              ),
            ],
          );
        },
      ),
    );
    if (mounted) setState(() => _open = false);
  }

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final vault = widget.vault;

    return Padding(
      padding: EdgeInsets.fromLTRB(
        t.sp * 1.5,
        t.sp * 2,
        t.sp * 1.5,
        t.sp * 1.5,
      ),
      child: InkWell(
        key: _anchor,
        onTap: _toggle,
        borderRadius: BorderRadius.circular(t.rControl),
        child: Padding(
          padding: EdgeInsets.symmetric(
            horizontal: t.sp * 0.5,
            vertical: t.sp * 0.5,
          ),
          child: Row(
            children: [
              Container(
                width: t.sp * 4.25,
                height: t.sp * 4.25,
                alignment: Alignment.center,
                decoration: BoxDecoration(
                  color: widget.accent.tile(t),
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
                    color: widget.accent.isNone ? t.text2 : kTileInk,
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
                    fontSize: t.bodySize * 0.95,
                    fontWeight: FontWeight.w600,
                    color: t.text,
                  ),
                ),
              ),
              // Rotates, so the control says whether the menu is down.
              AnimatedRotation(
                turns: _open ? 0.5 : 0,
                duration: t.duration,
                child: Icon(
                  LucideIcons.chevron_down,
                  size: t.bodySize,
                  color: t.text3,
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

/// The footer's gear: theme, text size, note font.
///
/// Not server settings — those are in the vault switcher, next to the vault
/// they configure. This is the menu the phone's top-right corner bubble
/// drops, and the corners are empty at this width.
class _SettingsButton extends StatefulWidget {
  const _SettingsButton();

  @override
  State<_SettingsButton> createState() => _SettingsButtonState();
}

class _SettingsButtonState extends State<_SettingsButton> {
  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return IconButton(
      icon: Icon(LucideIcons.settings, size: t.bodySize, color: t.text3),
      iconSize: t.bodySize,
      tooltip: 'Client settings',
      visualDensity: VisualDensity.compact,
      padding: EdgeInsets.zero,
      constraints: BoxConstraints.tight(Size.square(t.sp * 4.75)),
      // A page, not a popover. Anchored to a button at the foot of the
      // sidebar, a menu opens below the bottom of the window — which is what
      // made this button look like it did nothing at all.
      onPressed: () =>
          context.push(Routes.clientSettingsIn(VaultGate.of(context))),
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
          horizontal: t.sp * 1.5,
          vertical: t.sp * 1.5,
        ),
        decoration: BoxDecoration(
          color: t.surface2,
          borderRadius: BorderRadius.circular(t.rControl),
          border: Border.all(color: t.border, width: t.bw),
        ),
        child: Row(
          children: [
            Icon(LucideIcons.search, size: t.codeSize, color: t.text3),
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

    final t = context.tokens;
    if (rows.isEmpty) {
      return const EmptyState(
        icon: LucideIcons.folder_open,
        title: 'This vault is empty',
      );
    }
    return ListView(
      padding: EdgeInsets.only(bottom: t.cardPad),
      children: rows,
    );
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
    final t = context.tokens;
    final entries = childrenOfFolder(widget.notes, folder, widget.knownFolders);
    final openNoteId = _openNoteId(context);

    void addRow(Widget row, {required bool leading}) {
      rows.add(
        Padding(
          padding: EdgeInsets.only(
            left: _indent(context, depth, leading: leading),
          ),
          child: row,
        ),
      );
    }

    for (final entry in entries) {
      if (entry.isFolder) {
        final isOpen = _expanded.contains(entry.path);
        addRow(
          leading: false,
          EntryTile(
            key: ValueKey('folder:${entry.path}'),
            entry: entry,
            inTree: true,
            contentPadding: _rowPadding(context),
            leading: Icon(
              isOpen ? LucideIcons.chevron_down : LucideIcons.chevron_right,
              size: t.codeSize,
              color: t.text3,
            ),
            onFolderTap: () => setState(() {
              if (!_expanded.remove(entry.path)) _expanded.add(entry.path);
            }),
          ),
        );
        if (isOpen) {
          _appendLevel(rows, folder: entry.path, depth: depth + 1);
          // A group and the group after it need daylight between them, or the
          // last child of one reads as a sibling of the next folder.
          rows.add(SizedBox(height: t.sp * 0.75));
        }
      } else {
        addRow(
          // A note has no chevron, so without this its title would start where
          // the parent folder's chevron does rather than under the folder's
          // own text — and one level of nesting would be invisible.
          leading: true,
          EntryTile(
            key: ValueKey('note:${entry.note!.id}'),
            entry: entry,
            inTree: true,
            contentPadding: _rowPadding(context),
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

  /// Every tree row's own padding. The indent is applied *outside* it, so a
  /// selected note's background sits inside the indented column rather than
  /// spanning the sidebar as if it were top-level.
  EdgeInsets _rowPadding(BuildContext context) {
    final sp = context.tokens.sp;
    return EdgeInsets.symmetric(horizontal: sp * 1.25, vertical: sp * 0.75);
  }

  /// One step of hierarchy per level, plus the width of the chevron a folder
  /// carries and a note does not — that second term is what puts a note's
  /// title under its folder's title rather than under its folder's chevron.
  double _indent(BuildContext context, int depth, {required bool leading}) {
    final t = context.tokens;
    return depth * t.sp * 2 + (leading ? t.codeSize + t.sp : 0);
  }
}

/// What the pane beside the sidebar shows when no note is open.
class NoNoteSelected extends StatelessWidget {
  const NoNoteSelected({super.key});

  @override
  Widget build(BuildContext context) => const Scaffold(
    body: EmptyState(
      icon: LucideIcons.file_text,
      title: 'Select a note',
      fill: true,
    ),
  );
}
