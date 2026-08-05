import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../state/app_state.dart';
import '../theme.dart';

/// Top-left: the vault, and whether it is reaching the server.
///
/// Not a placeholder — the status dot is today's connection affordance. It
/// becomes a vault switcher when multi-vault ships, in the same slot at the
/// same visual weight.
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

    final status = !engine.isOnline
        ? StormStatus.offline
        : (engine.isSyncing || engine.pendingCount > 0)
            ? StormStatus.syncing
            : StormStatus.synced;

    return Stack(
      clipBehavior: Clip.none,
      children: [
        StormBubble(
          tooltip: 'Vault',
          onTap: () => _showSheet(context, ref, host, status),
          child: Text(
            host.isEmpty ? 'S' : host.characters.first.toUpperCase(),
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
    showModalBottomSheet<void>(
      context: context,
      showDragHandle: true,
      builder: (c) => SafeArea(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
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
          final settings = ref.watch(settingsProvider).value ?? const Settings();
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
