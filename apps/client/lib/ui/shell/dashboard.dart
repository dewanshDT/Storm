import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';

import '../../api/models.dart';
import '../../router.dart';
import '../../state/app_state.dart';
import '../server_settings_screen.dart' show createVault;
import 'corner_bubbles.dart' show relativeTime;
import 'nav_bubble.dart';
import '../theme.dart';
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
                child: ListView(
                  padding: const EdgeInsets.fromLTRB(16, 8, 16, 110),
                  children: const [_Vaults(), SizedBox(height: 24), _Recents()],
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
        return GridView.count(
          crossAxisCount: 2,
          shrinkWrap: true,
          physics: const NeverScrollableScrollPhysics(),
          crossAxisSpacing: 12,
          mainAxisSpacing: 12,
          childAspectRatio: 1.35,
          children: [for (final v in list) _VaultCard(vault: v)],
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
    final scheme = Theme.of(context).colorScheme;
    final engine = ref.watch(syncEngineProvider);
    final active = ref.watch(activeVaultProvider) == vault.id;

    // A missing vault is shown, greyed, rather than hidden. One that vanished
    // from the list would look exactly like one that never existed.
    final muted = vault.missing;

    return InkWell(
      borderRadius: BorderRadius.circular(16),
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
      child: Container(
        padding: const EdgeInsets.all(14),
        decoration: BoxDecoration(
          color: scheme.surfaceContainerHigh,
          borderRadius: BorderRadius.circular(16),
          border: Border.all(
            color: muted
                ? scheme.error.withValues(alpha: 0.5)
                : scheme.outlineVariant.withValues(alpha: 0.4),
          ),
        ),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Row(
              children: [
                Container(
                  width: 32,
                  height: 32,
                  alignment: Alignment.center,
                  decoration: BoxDecoration(
                    color: muted
                        ? scheme.error.withValues(alpha: 0.15)
                        : scheme.primary.withValues(alpha: 0.18),
                    borderRadius: BorderRadius.circular(10),
                  ),
                  child: Text(
                    _initial(vault.name),
                    style: TextStyle(
                      fontWeight: FontWeight.w700,
                      color: muted ? scheme.error : scheme.primary,
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
            const SizedBox(height: 2),
            Text(
              muted
                  ? 'Directory not found'
                  : '${vault.noteCount} note${vault.noteCount == 1 ? '' : 's'}',
              style: TextStyle(
                fontSize: 12,
                color: muted ? scheme.error : scheme.onSurfaceVariant,
              ),
            ),
          ],
        ),
      ),
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
    final scheme = Theme.of(context).colorScheme;

    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Padding(
          padding: const EdgeInsets.only(left: 4, bottom: 6),
          child: Text(
            'Recently opened',
            style: Theme.of(
              context,
            ).textTheme.labelLarge?.copyWith(color: scheme.onSurfaceVariant),
          ),
        ),
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

/// A full-width card: the note's name, and the vault it came from.
class _RecentCard extends StatelessWidget {
  const _RecentCard({required this.note});

  final RecentNote note;

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;

    return Padding(
      padding: const EdgeInsets.only(bottom: 8),
      child: InkWell(
        borderRadius: BorderRadius.circular(12),
        onTap: () => context.push(Routes.note(note.vaultId, note.noteId)),
        child: Container(
          padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 12),
          decoration: BoxDecoration(
            color: scheme.surfaceContainerHigh,
            borderRadius: BorderRadius.circular(12),
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
                      style: const TextStyle(fontWeight: FontWeight.w500),
                    ),
                    const SizedBox(height: 3),
                    Text(
                      // The vault first: with several in play, "which vault"
                      // is what tells two same-named daily notes apart.
                      note.folder.isEmpty
                          ? note.vaultName
                          : '${note.vaultName} · ${note.folder}',
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: TextStyle(
                        fontSize: 12,
                        color: scheme.onSurfaceVariant,
                      ),
                    ),
                  ],
                ),
              ),
              const SizedBox(width: 10),
              Text(
                relativeTime(DateTime.tryParse(note.openedAt)?.toLocal()),
                style: TextStyle(fontSize: 11, color: scheme.onSurfaceVariant),
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
