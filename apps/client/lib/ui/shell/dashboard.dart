import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';

import '../../api/models.dart';
import '../../router.dart';
import '../../state/app_state.dart';
import '../../state/vault_config.dart';
import '../server_settings_screen.dart' show createVault;
import '../accents.dart';
import '../atoms.dart';
import '../breakpoints.dart';
import 'corner_bubbles.dart' show relativeTime;
import 'nav_bubble.dart';
import '../theme.dart';
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
    final keyboard = keyboardIsOpen(context);

    return NewNoteRequest(
      // On the dashboard the `+` makes a vault: there is no note context here
      // to create a note into.
      onRequest: () => createVault(context, ref),
      child: Scaffold(
        appBar: const DashboardHeader(),
        body: Stack(
          children: [
            Positioned.fill(
              child: RefreshIndicator(
                onRefresh: () async {
                  ref.invalidate(vaultsProvider);
                  ref.invalidate(recentsProvider);
                },
                child: context.isExpanded
                    // Wide: the grid flows and recents take a rail, rather
                    // than the list becoming 1900px-wide rows.
                    ? const _WideBody()
                    : ListView(
                        padding: const EdgeInsets.fromLTRB(20, 4, 20, 120),
                        children: const [
                          // Mark, then the shape of the vault, then what you
                          // were last doing, then where things live. The design
                          // puts recents above vaults because the note you want
                          // is usually one of the last few you touched.
                          _Masthead(),
                          SizedBox(height: 28),
                          _Recents(),
                          SizedBox(height: 28),
                          _Vaults(),
                        ],
                      ),
              ),
            ),
            if (!keyboard)
              const Positioned(
                left: 0,
                right: 0,
                bottom: 0,
                child: NavBubble(),
              ),
          ],
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

    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        const Center(child: BrandMark(size: 30, withWordmark: true)),
        SizedBox(height: t.sp * 3),
        Row(
          children: [
            StatBlock(value: '$notes', label: notes == 1 ? 'note' : 'notes'),
            SizedBox(width: t.sp * 4),
            // The design's second figure is "last synced". Storm does not
            // record a sync timestamp anywhere, and inventing one would be a
            // number that lies; the vault count is the honest thing it does
            // know.
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
            children: const [_Recents()],
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

    return InkWell(
      borderRadius: BorderRadius.circular(t.rCard),
      onTap: muted
          ? () => ScaffoldMessenger.of(context).showSnackBar(
              SnackBar(
                content: Text(
                  'The directory for “${vault.name}” is missing from the '
                  'storage root. Nothing was deleted.',
                ),
              ),
            )
          : () => context.push(Routes.browse(vault.id)),
      onLongPress: muted
          ? null
          : () => _pickVaultColour(context, ref, vault.id, accent),
      child: Container(
        padding: EdgeInsets.all(t.cardPad * 0.6),
        decoration: BoxDecoration(
          // The card is a plain surface; the accent is the *tile* inside it.
          // Tinting the whole card was the old behaviour and read as a slab of
          // unrelated colour sitting on the page.
          color: t.surface,
          borderRadius: BorderRadius.circular(t.rCard),
          border: Border.all(
            color: muted ? t.danger.withValues(alpha: 0.5) : t.border,
            width: t.bw,
          ),
        ),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Row(
              children: [
                Container(
                  width: 40,
                  height: 40,
                  alignment: Alignment.center,
                  decoration: BoxDecoration(
                    color: muted
                        ? t.danger.withValues(alpha: 0.15)
                        : accent.tile(t),
                    borderRadius: BorderRadius.circular(t.rControl),
                  ),
                  child: Text(
                    _initial(vault.name),
                    style: TextStyle(
                      fontFamily: StormTokens.sansFamily,
                      fontWeight: FontWeight.w600,
                      // Dark ink on the tile whatever the theme — the tile is
                      // always a light colour, and the accent test measures
                      // this exact pairing.
                      color: muted ? t.danger : const Color(0xFF1A1626),
                    ),
                  ),
                ),
                const Spacer(),
                // Only the vault currently open has an engine behind it, so
                // only it can honestly report sync state.
                if (active && !muted)
                  StormStatusDot(
                    status: !engine.isOnline
                        ? StormStatus.offline
                        : (engine.isSyncing || engine.pendingCount > 0)
                        ? StormStatus.syncing
                        : StormStatus.synced,
                  ),
              ],
            ),
            const Spacer(),
            Text(
              vault.name,
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: Theme.of(
                context,
              ).textTheme.titleMedium?.copyWith(fontWeight: FontWeight.w600),
            ),
            SizedBox(height: t.sp * 0.5),
            Row(
              children: [
                if (!muted) ...[
                  StatusDot(
                    status: active && !engine.isOnline
                        ? DotStatus.offline
                        : active
                        ? DotStatus.synced
                        : DotStatus.offline,
                    size: 6,
                  ),
                  SizedBox(width: t.sp * 0.75),
                ],
                // Flexible, because "Directory not found" is far longer
                // than "14 notes" and the card is a fixed 220px at most.
                Flexible(
                  child: Text(
                    muted
                        ? 'Directory not found'
                        : '${vault.noteCount} note'
                              '${vault.noteCount == 1 ? '' : 's'}',
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: TextStyle(
                      fontFamily: StormTokens.sansFamily,
                      fontSize: t.codeSize,
                      color: muted ? t.danger : t.text3,
                    ),
                  ),
                ),
              ],
            ),
          ],
        ),
      ),
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

String _initial(String name) =>
    name.trim().isEmpty ? 'S' : name.trim().characters.first.toUpperCase();

class _Recents extends ConsumerWidget {
  const _Recents();

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
              : Column(children: [for (final r in list) _RecentCard(note: r)]),
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
