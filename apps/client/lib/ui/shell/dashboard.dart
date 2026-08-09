import 'package:flutter/material.dart';
import 'package:flutter_lucide/flutter_lucide.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';

import '../../api/models.dart';
import '../../router.dart';
import '../../state/app_state.dart';
import '../../state/vault_config.dart';
import '../server_settings_screen.dart' show createVault;
import '../accents.dart';
import '../widgets.dart';
import '../breakpoints.dart';
import '../states.dart';

import 'corner_bubbles.dart' show compactAge, relativeTime;
import 'nav_bubble.dart';
import '../tokens.dart';
import 'storm_scaffold.dart';

/// Home: a grid of vaults over the notes you opened most recently.
///
/// The vault grid is the top-level object now — one card per vault rather than
/// one screen per install. Recents sit underneath and cross vaults, which is
/// what makes the dashboard useful when several are in play: the note you want
/// is usually one of the last few you touched, whichever vault it lives in.
class DashboardScreen extends ConsumerWidget {
  const DashboardScreen({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final t = context.tokens;
    final vaults = ref.watch(vaultsProvider).value ?? const [];

    // There is no dashboard at desk width. Everything it offers is already in
    // the sidebar — the switcher lists the vaults, the tree is the browser —
    // so a whole screen for it is a page you pass through on the way to the
    // only thing you came for. It stays reachable with no vaults, because
    // then it is the only screen that can make one.
    if (context.isExpanded && vaults.isNotEmpty) {
      final active = ref.watch(activeVaultProvider);
      final target = vaults.any((v) => v.id == active && !v.missing)
          ? active
          : (vaults.firstWhere(
              (v) => !v.missing,
              orElse: () => vaults.first,
            )).id;
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (context.mounted) context.go(Routes.browse(target));
      });
      // Blank rather than the dashboard: rendering it for one frame is a
      // flash of a screen the user is never meant to see at this width.
      return Scaffold(backgroundColor: t.bg, body: const SizedBox.shrink());
    }

    return NewNoteRequest(
      // On the dashboard the `+` makes a vault: there is no note context here
      // to create a note into.
      onRequest: () => createVault(context, ref),
      child: Scaffold(
        body: StormChrome(
          showNav: !keyboardIsOpen(context),
          child: RefreshIndicator(
            onRefresh: () async {
              ref.invalidate(vaultsProvider);
              ref.invalidate(recentsProvider);
            },
            child: ListView(
              padding: EdgeInsets.fromLTRB(
                t.sp * 2.5,
                0,
                t.sp * 2.5,
                StormChrome.navClearance(context),
              ),
              children: [
                // Mark, then the shape of the vault, then what you were
                // last doing, then where things live. Recents sit above
                // vaults because the note you want is usually one of the
                // last few you touched.
                const _Masthead(),
                SizedBox(height: t.sectionRhythm * 0.5),
                const _Recents(),
                SizedBox(height: t.sectionRhythm * 0.5),
                const _Vaults(),
              ],
            ),
          ),
        ),
      ),
    );
  }
}

/// The mark, and the shape of the whole vault set in two numbers.
///
/// No mark here. The app's own name is the least useful thing on the screen
/// you already opened the app to see, and it cost a whole row above the two
/// numbers that are the point.
class _Masthead extends ConsumerWidget {
  const _Masthead();

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final t = context.tokens;
    final vaults = ref.watch(vaultsProvider).value ?? const [];
    final notes = vaults.fold<int>(0, (sum, v) => sum + v.noteCount);
    // Not persisted, so it reads "—" until this run syncs once. That is
    // honest; a stored timestamp would claim a sync that may not have
    // survived the restart.
    final synced = ref.watch(syncEngineProvider).lastSyncedAt;

    return Row(
      children: [
        StatBlock(value: '$notes', label: notes == 1 ? 'note' : 'notes'),
        SizedBox(width: t.sp * 4),
        StatBlock(value: compactAge(synced), label: 'last synced'),
      ],
    );
  }
}

class _Vaults extends ConsumerWidget {
  const _Vaults();

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final vaults = ref.watch(vaultsProvider);

    return vaults.when(
      loading: () => const SkeletonRows(rows: 2),
      error: (e, _) => EmptyState(
        icon: LucideIcons.cloud_off,
        title: 'Could not reach the server',
        detail: describeFailure(e),
        action: 'Try again',
        onAction: () => ref.invalidate(vaultsProvider),
      ),
      data: (list) {
        if (list.isEmpty) {
          return EmptyState(
            icon: LucideIcons.folder_cog,
            title: 'No vaults yet',
            detail: 'A vault is a directory under the storage root.',
            action: 'New vault',
            onAction: () => createVault(context, ref),
          );
        }
        // A maximum card size rather than a fixed column count. At 411px it
        // still works out to two columns, so the phone is unchanged; at 1900
        // it flows to eight card-sized cards instead of two enormous ones.
        return Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            const Align(
              alignment: Alignment.centerLeft,
              child: SectionLabel('Vaults'),
            ),
            const SizedBox(height: 10),
            GridView.extent(
              maxCrossAxisExtent: 220,
              shrinkWrap: true,
              physics: const NeverScrollableScrollPhysics(),
              crossAxisSpacing: 12,
              mainAxisSpacing: 12,
              childAspectRatio: 1.15,
              children: [for (final v in list) _VaultCard(vault: v)],
            ),
          ],
        );
      },
    );
  }
}

class _VaultCard extends ConsumerWidget {
  const _VaultCard({required this.vault});

  final VaultInfo vault;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final t = context.tokens;
    final engine = ref.watch(syncEngineProvider);
    final active = ref.watch(activeVaultProvider) == vault.id;
    final accent =
        ref.watch(vaultAccentsProvider).value?[vault.id] ?? Accent.none;

    // A missing vault is shown, greyed, rather than hidden. One that vanished
    // from the list would look exactly like one that never existed.
    final muted = vault.missing;

    return VaultCard(
      name: vault.name,
      tile: accent.tile(t),
      tinted: !accent.isNone,
      muted: muted,
      subtitle: muted
          ? 'Directory not found'
          : '${vault.noteCount} note${vault.noteCount == 1 ? '' : 's'}',
      // Only the vault currently open has an engine behind it, so only it can
      // honestly report sync state.
      status: muted
          ? null
          : active
          ? dotStatusFor(
              online: engine.isOnline,
              syncing: engine.isSyncing,
              pending: engine.pendingCount,
            )
          : DotStatus.offline,
      onTap: muted
          ? () => ScaffoldMessenger.of(context).showSnackBar(
              SnackBar(
                content: Text(
                  'The directory for \u201C${vault.name}\u201D is missing from '
                  'the storage root. Nothing was deleted.',
                ),
              ),
            )
          : () => context.push(Routes.browse(vault.id)),
      onLongPress: muted
          ? null
          : () => _pickVaultColour(context, ref, vault.id, accent),
    );
  }
}

/// Long-press a vault card to colour it, Keep-style.
///
/// Writes `storm.color` into that vault's own `_storm/vault.md`, so the choice
/// travels with the vault rather than living on this device.
Future<void> _pickVaultColour(
  BuildContext context,
  WidgetRef ref,
  String vaultId,
  Accent current,
) async {
  final chosen = await showModalBottomSheet<Accent>(
    context: context,
    showDragHandle: true,
    builder: (c) => SafeArea(
      child: Padding(
        padding: const EdgeInsets.fromLTRB(20, 4, 20, 24),
        child: AccentPicker(
          selected: current,
          onSelected: (accent) => Navigator.pop(c, accent),
        ),
      ),
    ),
  );
  if (chosen == null || chosen == current) return;

  final ok = await setVaultAccent(ref, vaultId, chosen);
  if (!ok && context.mounted) {
    ScaffoldMessenger.of(context).showSnackBar(
      const SnackBar(content: Text('Could not save the vault colour')),
    );
  }
}

class _Recents extends ConsumerWidget {
  const _Recents();

  /// How many to show before the vaults below them. Twenty rows pushed the
  /// vault grid off the bottom of the phone entirely.
  static const limit = 5;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final recents = ref.watch(recentsProvider);

    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        const Align(
          alignment: Alignment.centerLeft,
          child: SectionLabel('Recently opened'),
        ),
        const SizedBox(height: 10),
        recents.when(
          loading: () => const SkeletonRows(rows: 3),
          error: (e, _) => EmptyState(
            icon: LucideIcons.cloud_off,
            title: 'Could not load recents',
            detail: describeFailure(e),
          ),
          data: (list) => list.isEmpty
              ? const EmptyState(
                  icon: LucideIcons.history,
                  title: 'Nothing opened yet',
                  detail: 'Notes you open will show up here.',
                )
              : Column(
                  children: [
                    for (final r in list.take(limit)) _RecentCard(note: r),
                  ],
                ),
        ),
      ],
    );
  }
}

/// A text row: the note's name, and where it came from.
///
/// Deliberately not a card. Eight cards stacked is eight competing rectangles,
/// and what is actually being scanned here is the titles.
class _RecentCard extends ConsumerWidget {
  const _RecentCard({required this.note});

  final RecentNote note;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final t = context.tokens;

    return Padding(
      padding: EdgeInsets.only(bottom: t.sp * 0.5),
      child: InkWell(
        borderRadius: BorderRadius.circular(t.rControl),
        onTap: () => context.push(Routes.note(note.vaultId, note.noteId)),
        child: Padding(
          padding: EdgeInsets.symmetric(
            horizontal: t.sp * 0.5,
            vertical: t.sp * 1.25,
          ),
          child: Row(
            children: [
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(
                      note.displayTitle,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: TextStyle(
                        fontFamily: StormTokens.sansFamily,
                        fontSize: t.bodySize,
                        fontWeight: FontWeight.w600,
                        color: t.text,
                      ),
                    ),
                    SizedBox(height: t.sp * 0.25),
                    Text(
                      // The vault first: with several in play, "which vault"
                      // is what tells two same-named daily notes apart.
                      note.folder.isEmpty
                          ? note.vaultName
                          : '${note.vaultName} · ${note.folder}',
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: TextStyle(
                        fontFamily: StormTokens.sansFamily,
                        fontSize: t.codeSize,
                        color: t.text3,
                      ),
                    ),
                  ],
                ),
              ),
              SizedBox(width: t.sp),
              Text(
                relativeTime(DateTime.tryParse(note.openedAt)?.toLocal()),
                style: TextStyle(
                  fontFamily: StormTokens.sansFamily,
                  fontSize: t.codeSize,
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
