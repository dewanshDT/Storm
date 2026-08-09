import 'package:flutter/material.dart';
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
            child: context.isExpanded
                // Wide: the grid flows and recents take a rail, rather than
                // the list becoming 1900px-wide rows.
                ? const _WideBody()
                : ListView(
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
/// The wordmark is set beside the real mark rather than as a text title: the
/// design is explicit that the mark is never substituted for type.
class _Masthead extends ConsumerWidget {
  const _Masthead();

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final t = context.tokens;
    final vaults = ref.watch(vaultsProvider).value ?? const [];
    final notes = vaults.fold<int>(0, (sum, v) => sum + v.noteCount);
    final synced = ref.watch(syncEngineProvider).lastSyncedAt;

    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        const Center(child: BrandMark(size: 30, withWordmark: true)),
        SizedBox(height: t.sp * 3),
        Row(
          children: [
            StatBlock(value: '$notes', label: notes == 1 ? 'note' : 'notes'),
            SizedBox(width: t.sp * 4),
            // Not persisted, so it reads "—" until this run syncs once. That
            // is honest; a stored timestamp would claim a sync that may not
            // have survived the restart.
            StatBlock(value: compactAge(synced), label: 'last synced'),
            SizedBox(width: t.sp * 4),
            StatBlock(
              value: '${vaults.length}',
              label: vaults.length == 1 ? 'vault' : 'vaults',
            ),
          ],
        ),
      ],
    );
  }
}

/// The dashboard at desk width.
class _WideBody extends StatelessWidget {
  const _WideBody();

  @override
  Widget build(BuildContext context) {
    return Row(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Expanded(
          child: ListView(
            padding: const EdgeInsets.fromLTRB(24, 12, 12, 40),
            children: const [_Vaults()],
          ),
        ),
        SizedBox(
          width: 340,
          child: ListView(
            padding: const EdgeInsets.fromLTRB(12, 12, 24, 40),
            children: const [_Recents(limit: 12)],
          ),
        ),
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
      loading: () => const Padding(
        padding: EdgeInsets.symmetric(vertical: 40),
        child: Center(child: CircularProgressIndicator()),
      ),
      error: (e, _) => _Message(
        icon: Icons.cloud_off,
        text: '$e',
        action: 'Retry',
        onAction: () => ref.invalidate(vaultsProvider),
      ),
      data: (list) {
        if (list.isEmpty) {
          return _Message(
            icon: Icons.folder_special_outlined,
            text: 'No vaults yet.',
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
  const _Recents({this.limit = 5});

  /// How many to show before the vaults below them.
  ///
  /// Twenty rows pushed the vault grid off the bottom of the phone entirely.
  final int limit;

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
          loading: () => const Padding(
            padding: EdgeInsets.all(12),
            child: LinearProgressIndicator(),
          ),
          error: (e, _) => _Message(icon: Icons.cloud_off, text: '$e'),
          data: (list) => list.isEmpty
              ? const _Message(
                  icon: Icons.history,
                  text: 'Notes you open will show up here.',
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

class _Message extends StatelessWidget {
  const _Message({
    required this.icon,
    required this.text,
    this.action,
    this.onAction,
  });

  final IconData icon;
  final String text;
  final String? action;
  final VoidCallback? onAction;

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 32),
      child: Column(
        children: [
          Icon(icon, size: 30, color: scheme.onSurfaceVariant),
          const SizedBox(height: 10),
          Text(
            text,
            textAlign: TextAlign.center,
            style: TextStyle(color: scheme.onSurfaceVariant),
          ),
          if (action != null) ...[
            const SizedBox(height: 12),
            OutlinedButton(onPressed: onAction, child: Text(action!)),
          ],
        ],
      ),
    );
  }
}
