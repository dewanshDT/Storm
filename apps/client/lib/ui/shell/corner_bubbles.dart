import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import 'package:go_router/go_router.dart';

import '../../router.dart';
import '../../state/app_state.dart';
import '../theme.dart';

/// Top-left: the vault, whether it is reaching the server, and the switcher.
///
/// The slot the UI refactor reserved for this. It shows the *vault's* initial
/// rather than the server host's, because with several vaults in play the
/// question the bubble answers is "which one am I in".
class VaultBubble extends ConsumerWidget {
  const VaultBubble({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    // Watching the engine is correct *here*: this widget renders its status,
    // which is exactly what its notifications mean. Anything needing the
    // engine's identity still uses ref.read — see app_state.dart.
    final engine = ref.watch(syncEngineProvider);
    final settings = ref.watch(settingsProvider).value;
    final host = _host(settings?.baseUrl ?? '');
    final activeId = ref.watch(activeVaultProvider);
    final vaults = ref.watch(vaultsProvider).value ?? const [];
    final active = vaults.where((v) => v.id == activeId).firstOrNull;
    final label = active?.name ?? '';

    final status = !engine.isOnline
        ? StormStatus.offline
        : (engine.isSyncing || engine.pendingCount > 0)
        ? StormStatus.syncing
        : StormStatus.synced;

    return Stack(
      clipBehavior: Clip.none,
      children: [
        StormBubble(
          tooltip: label.isEmpty ? 'Vault' : label,
          onTap: () => _showSheet(context, ref, host, status),
          child: Text(
            (label.isEmpty ? host : label).characters.firstOrNull
                    ?.toUpperCase() ??
                'S',
            style: const TextStyle(fontWeight: FontWeight.w700, fontSize: 16),
          ),
        ),
        Positioned(
          right: -1,
          bottom: -1,
          child: StormStatusDot(status: status),
        ),
      ],
    );
  }

  static String _host(String url) =>
      Uri.tryParse(url)?.host ?? url.replaceAll(RegExp(r'^https?://'), '');

  void _showSheet(
    BuildContext context,
    WidgetRef ref,
    String host,
    StormStatus status,
  ) {
    final engine = ref.read(syncEngineProvider);
    final activeId = ref.read(activeVaultProvider);
    final vaults = ref.read(vaultsProvider).value ?? const [];

    showModalBottomSheet<void>(
      context: context,
      showDragHandle: true,
      builder: (c) => SafeArea(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            // The switcher sits above the status, because switching is what
            // this bubble is now mostly for.
            for (final v in vaults)
              ListTile(
                leading: Icon(
                  v.id == activeId
                      ? Icons.folder_special
                      : Icons.folder_special_outlined,
                  color: v.missing
                      ? Theme.of(c).colorScheme.error
                      : Theme.of(c).colorScheme.primary,
                ),
                title: Text(v.name),
                subtitle: v.missing
                    ? const Text('Directory not found')
                    : Text('${v.noteCount} note${v.noteCount == 1 ? '' : 's'}'),
                selected: v.id == activeId,
                onTap: v.missing
                    ? null
                    : () {
                        Navigator.pop(c);
                        if (v.id == activeId) return;
                        // `go`, not `push`: switching vaults replaces where you
                        // are rather than stacking one vault on top of another.
                        context.go(Routes.browse(v.id));
                      },
              ),
            if (vaults.isNotEmpty) const Divider(height: 1),
            ListTile(
              leading: StormStatusDot(status: status, size: 12),
              title: Text(switch (status) {
                StormStatus.synced => 'Synced',
                StormStatus.syncing => 'Syncing…',
                StormStatus.offline => 'Offline',
              }),
              subtitle: Text(
                engine.pendingCount > 0
                    ? '${engine.pendingCount} edit(s) waiting to send'
                    : 'Everything is on the server',
              ),
            ),
            ListTile(
              leading: const Icon(Icons.dns_outlined),
              title: const Text('Server'),
              subtitle: Text(host.isEmpty ? 'not set' : host),
            ),
            ListTile(
              leading: const Icon(Icons.schedule),
              title: const Text('Last synced'),
              subtitle: Text(_ago(engine.lastSyncedAt)),
            ),
            ListTile(
              leading: const Icon(Icons.sync),
              title: const Text('Sync now'),
              onTap: () async {
                Navigator.pop(c);
                await ref.read(syncEngineProvider).sync();
                ref.invalidate(treeProvider);
              },
            ),
            ListTile(
              leading: const Icon(Icons.settings_outlined),
              title: const Text('Server settings'),
              subtitle: const Text('Storage root and vaults'),
              onTap: () {
                Navigator.pop(c);
                context.push(Routes.serverSettings);
              },
            ),
          ],
        ),
      ),
    );
  }
}

/// Top-right: settings today, account when multi-user ships.
class ProfileBubble extends ConsumerWidget {
  const ProfileBubble({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    return StormBubble(
      tooltip: 'Settings',
      onTap: () => _showSheet(context, ref),
      child: const Icon(Icons.person_outline, size: 20),
    );
  }

  void _showSheet(BuildContext context, WidgetRef ref) {
    showModalBottomSheet<void>(
      context: context,
      showDragHandle: true,
      builder: (c) => Consumer(
        builder: (c, ref, _) {
          final settings =
              ref.watch(settingsProvider).value ?? const Settings();
          final notifier = ref.read(settingsProvider.notifier);

          return SafeArea(
            child: Column(
              mainAxisSize: MainAxisSize.min,
              children: [
                SwitchListTile(
                  secondary: const Icon(Icons.dark_mode_outlined),
                  title: const Text('Dark theme'),
                  value: settings.darkMode,
                  onChanged: (v) =>
                      notifier.save(settings.copyWith(darkMode: v)),
                ),
                ListTile(
                  leading: const Icon(Icons.format_size),
                  title: const Text('Text size'),
                  subtitle: Slider(
                    value: settings.fontSize,
                    min: 12,
                    max: 24,
                    divisions: 12,
                    label: '${settings.fontSize.round()}px',
                    onChanged: (v) =>
                        notifier.save(settings.copyWith(fontSize: v)),
                  ),
                ),
                ListTile(
                  leading: const Icon(Icons.text_fields),
                  title: const Text('Note font'),
                  subtitle: Padding(
                    padding: const EdgeInsets.only(top: 6),
                    child: SegmentedButton<BodyFont>(
                      showSelectedIcon: false,
                      style: const ButtonStyle(
                        visualDensity: VisualDensity.compact,
                      ),
                      segments: [
                        for (final font in BodyFont.values)
                          ButtonSegment(
                            value: font,
                            label: Text(
                              font.label,
                              // Each option is set in the face it names, so
                              // the choice shows what it will do.
                              style: TextStyle(fontFamily: font.family),
                            ),
                          ),
                      ],
                      selected: {settings.bodyFont},
                      onSelectionChanged: (picked) => notifier.save(
                        settings.copyWith(bodyFont: picked.first),
                      ),
                    ),
                  ),
                ),
                SwitchListTile(
                  secondary: const Icon(Icons.fingerprint),
                  title: const Text('Show note id'),
                  subtitle: const Text(
                    'The identifier Storm assigns each note',
                  ),
                  value: settings.showNoteId,
                  onChanged: (v) =>
                      notifier.save(settings.copyWith(showNoteId: v)),
                ),
                const Divider(),
                ListTile(
                  leading: const Icon(Icons.logout),
                  title: const Text('Disconnect'),
                  subtitle: const Text('Forget this server and its token'),
                  onTap: () {
                    Navigator.pop(c);
                    notifier.save(const Settings());
                  },
                ),
              ],
            ),
          );
        },
      ),
    );
  }
}

/// "3 minutes ago", for the places that show a timestamp.
String _ago(DateTime? then) {
  if (then == null) return 'not yet';
  final d = DateTime.now().difference(then);
  if (d.inSeconds < 45) return 'just now';
  if (d.inMinutes < 60) return '${d.inMinutes}m ago';
  if (d.inHours < 24) return '${d.inHours}h ago';
  return '${d.inDays}d ago';
}

/// Shared with the dashboard's recent-notes list.
String relativeTime(DateTime? then) => _ago(then);
