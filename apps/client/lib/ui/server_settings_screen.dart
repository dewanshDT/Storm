import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';

import '../api/models.dart';
import '../router.dart';
import '../state/app_state.dart';
import 'browse_screen.dart' show describeFailure;

/// Where vaults live on the server, and which ones exist.
///
/// The two destructive-sounding actions here are not destructive, and say so:
/// changing the storage root never moves files, and removing a vault only
/// unregisters it.
class ServerSettingsScreen extends ConsumerWidget {
  const ServerSettingsScreen({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final config = ref.watch(serverConfigProvider);
    final vaults = ref.watch(vaultsProvider);

    return Scaffold(
      appBar: AppBar(
        leading: IconButton(
          icon: const Icon(Icons.arrow_back),
          onPressed: () =>
              context.canPop() ? context.pop() : context.go(Routes.dashboard),
        ),
        title: const Text('Server'),
      ),
      body: ListView(
        padding: const EdgeInsets.fromLTRB(16, 8, 16, 32),
        children: [
          _Section(label: 'Vault storage root'),
          config.when(
            loading: () => const Padding(
              padding: EdgeInsets.all(12),
              child: LinearProgressIndicator(),
            ),
            error: (e, _) => _Muted('$e'),
            data: (c) => c == null
                ? const _Muted('Not connected')
                : _RootCard(config: c),
          ),
          const SizedBox(height: 24),
          _Section(label: 'AI access (MCP)'),
          config.when(
            loading: () => const Padding(
              padding: EdgeInsets.all(12),
              child: LinearProgressIndicator(),
            ),
            error: (e, _) => _Muted('$e'),
            data: (c) =>
                c == null ? const _Muted('Not connected') : _McpCard(config: c),
          ),
          const SizedBox(height: 24),
          _Section(label: 'Vaults'),
          vaults.when(
            loading: () => const Padding(
              padding: EdgeInsets.all(12),
              child: LinearProgressIndicator(),
            ),
            error: (e, _) => _Muted('$e'),
            data: (list) => Column(
              children: [
                for (final v in list) _VaultRow(vault: v),
                const SizedBox(height: 8),
                Align(
                  alignment: Alignment.centerLeft,
                  child: OutlinedButton.icon(
                    onPressed: () => createVault(context, ref),
                    icon: const Icon(Icons.add, size: 18),
                    label: const Text('New vault'),
                  ),
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }
}

/// Prompts for a name and creates a vault. Shared with the dashboard.
Future<void> createVault(BuildContext context, WidgetRef ref) async {
  final name = await promptForText(
    context,
    title: 'New vault',
    hint: 'Personal',
  );
  if (name == null || name.trim().isEmpty || !context.mounted) return;

  final api = ref.read(apiProvider);
  if (api == null) return;
  try {
    await api.createVault(name.trim());
    ref.invalidate(vaultsProvider);
    ref.invalidate(serverConfigProvider);
  } catch (e) {
    if (context.mounted) _toast(context, describeFailure(e));
  }
}

void _toast(BuildContext context, String message) => ScaffoldMessenger.of(
  context,
).showSnackBar(SnackBar(content: Text(message)));

/// Whether the server lets an AI assistant read these vaults, and a switch.
///
/// This is a *server* setting, not this device's: turning it off here turns it
/// off for every client and every agent, and it survives a restart. The
/// subtitle says so, because a switch on a settings screen otherwise reads as
/// local to the app.
class _McpCard extends ConsumerStatefulWidget {
  const _McpCard({required this.config});

  final ServerConfig config;

  @override
  ConsumerState<_McpCard> createState() => _McpCardState();
}

class _McpCardState extends ConsumerState<_McpCard> {
  /// Set while the request is in flight so the switch shows where it is going,
  /// not where it was. Without it the switch snaps back until the provider
  /// refreshes, which reads as the toggle having failed.
  bool? _pending;

  Future<void> _set(bool enabled) async {
    final api = ref.read(apiProvider);
    if (api == null) return;
    setState(() => _pending = enabled);
    try {
      await api.setMcpEnabled(enabled);
      ref.invalidate(serverConfigProvider);
    } catch (e) {
      if (mounted) _toast(context, describeFailure(e));
    } finally {
      if (mounted) setState(() => _pending = null);
    }
  }

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    final theme = Theme.of(context);
    final on = _pending ?? widget.config.mcpEnabled;

    // `Material`, not a coloured `Container`: SwitchListTile is a ListTile, and
    // a ListTile paints its background and ink splash onto the nearest Material
    // ancestor — a DecoratedBox in between swallows them, and Flutter asserts
    // about it. The same mistake was made in the M12 sidebar; see decision 34.
    return Material(
      color: scheme.surfaceContainerHigh,
      borderRadius: BorderRadius.circular(14),
      child: Padding(
        padding: const EdgeInsets.fromLTRB(14, 4, 6, 10),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            SwitchListTile(
              contentPadding: EdgeInsets.zero,
              value: on,
              onChanged: _pending == null ? _set : null,
              title: const Text('Let AI assistants read your notes'),
              subtitle: Text(
                on ? 'Serving at /mcp' : 'Off — /mcp refuses every request',
                style: theme.textTheme.bodySmall?.copyWith(
                  color: scheme.onSurfaceVariant,
                ),
              ),
            ),
            Padding(
              padding: const EdgeInsets.fromLTRB(0, 2, 8, 0),
              child: Text(
                on
                    ? 'Read-only: search, read and history. Nothing can create, '
                          'edit or delete a note. Anyone holding this server’s '
                          'token can read every vault this way.'
                    : 'A server setting, not just this device — it applies to '
                          'every client and survives a restart.',
                style: theme.textTheme.bodySmall?.copyWith(
                  color: scheme.onSurfaceVariant,
                ),
              ),
            ),
          ],
        ),
      ),
    );
  }
}

class _RootCard extends ConsumerWidget {
  const _RootCard({required this.config});

  final ServerConfig config;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final scheme = Theme.of(context).colorScheme;
    return Container(
      padding: const EdgeInsets.all(14),
      decoration: BoxDecoration(
        color: scheme.surfaceContainerHigh,
        borderRadius: BorderRadius.circular(14),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          SelectableText(
            config.vaultRoot,
            style: const TextStyle(fontFamily: 'monospace', fontSize: 13),
          ),
          const SizedBox(height: 6),
          Text(
            '${config.vaultCount} vault${config.vaultCount == 1 ? '' : 's'}',
            style: TextStyle(fontSize: 12, color: scheme.onSurfaceVariant),
          ),
          const SizedBox(height: 12),
          // Said before the action, not after: Storm points at directories
          // someone has already moved, and a mistyped path that silently
          // orphaned every vault would read as "my notes are gone".
          Text(
            'Changing this does not move your notes. Move the vault '
            'directories first, then change the root.',
            style: TextStyle(fontSize: 12, color: scheme.onSurfaceVariant),
          ),
          const SizedBox(height: 10),
          OutlinedButton(
            onPressed: () => _change(context, ref),
            child: const Text('Change'),
          ),
        ],
      ),
    );
  }

  Future<void> _change(BuildContext context, WidgetRef ref) async {
    final path = await promptForText(
      context,
      title: 'Vault storage root',
      hint: '/srv/storm/vaults',
      initial: config.vaultRoot,
    );
    if (path == null || path.trim().isEmpty || !context.mounted) return;

    final api = ref.read(apiProvider);
    if (api == null) return;

    try {
      await api.setVaultRoot(path.trim());
    } catch (e) {
      if (!context.mounted) return;
      // A 409 means none of the registered vaults were found there. The server
      // refuses rather than applying it, and only proceeds once the user has
      // read what would be left behind. Anything that is not a refusal — a
      // dead socket, say — is just reported.
      if (e is! StormApiException || e.statusCode != 409) {
        _toast(context, describeFailure(e));
        return;
      }
      final proceed = await showDialog<bool>(
        context: context,
        builder: (c) => AlertDialog(
          title: const Text('Leave those vaults behind?'),
          content: Text(
            '${e.message}\n\nNo files are deleted either way. Vaults left '
            'behind stay in the list, marked as missing.',
          ),
          actions: [
            TextButton(
              onPressed: () => Navigator.pop(c, false),
              child: const Text('Cancel'),
            ),
            FilledButton(
              onPressed: () => Navigator.pop(c, true),
              child: const Text('Change anyway'),
            ),
          ],
        ),
      );
      if (proceed != true) return;
      try {
        await api.setVaultRoot(path.trim(), orphanOk: true);
      } catch (e2) {
        if (context.mounted) _toast(context, describeFailure(e2));
        return;
      }
    }

    ref.invalidate(serverConfigProvider);
    ref.invalidate(vaultsProvider);
    ref.read(vaultRevisionProvider.notifier).state++;
  }
}

class _VaultRow extends ConsumerWidget {
  const _VaultRow({required this.vault});

  final VaultInfo vault;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final scheme = Theme.of(context).colorScheme;
    return ListTile(
      contentPadding: EdgeInsets.zero,
      leading: Icon(
        vault.missing ? Icons.error_outline : Icons.folder_special_outlined,
        color: vault.missing ? scheme.error : scheme.primary,
      ),
      title: Text(vault.name),
      subtitle: Text(
        vault.missing
            ? 'Directory “${vault.dir}” not found'
            : '${vault.dir} · ${vault.noteCount} '
                  'note${vault.noteCount == 1 ? '' : 's'}',
        style: TextStyle(
          fontSize: 12,
          color: vault.missing ? scheme.error : scheme.onSurfaceVariant,
        ),
      ),
      trailing: PopupMenuButton<String>(
        onSelected: (action) => switch (action) {
          'rename' => _rename(context, ref),
          'remove' => _remove(context, ref),
          _ => null,
        },
        itemBuilder: (_) => const [
          PopupMenuItem(value: 'rename', child: Text('Rename')),
          PopupMenuItem(value: 'remove', child: Text('Remove from Storm')),
        ],
      ),
    );
  }

  Future<void> _rename(BuildContext context, WidgetRef ref) async {
    final name = await promptForText(
      context,
      title: 'Rename vault',
      hint: 'Personal',
      initial: vault.name,
    );
    if (name == null || name.trim().isEmpty || !context.mounted) return;
    final api = ref.read(apiProvider);
    if (api == null) return;
    try {
      await api.renameVault(vault.id, name.trim());
      ref.invalidate(vaultsProvider);
    } catch (e) {
      if (context.mounted) _toast(context, describeFailure(e));
    }
  }

  Future<void> _remove(BuildContext context, WidgetRef ref) async {
    final confirmed = await showDialog<bool>(
      context: context,
      builder: (c) => AlertDialog(
        title: Text('Remove “${vault.name}”?'),
        content: const Text(
          'Storm stops tracking this vault. Its directory and every note '
          'inside it are left exactly where they are — nothing is deleted.',
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(c, false),
            child: const Text('Cancel'),
          ),
          FilledButton(
            onPressed: () => Navigator.pop(c, true),
            child: const Text('Remove'),
          ),
        ],
      ),
    );
    if (confirmed != true || !context.mounted) return;

    final api = ref.read(apiProvider);
    if (api == null) return;
    try {
      await api.removeVault(vault.id);
      ref.invalidate(vaultsProvider);
      ref.invalidate(serverConfigProvider);
    } catch (e) {
      if (context.mounted) _toast(context, describeFailure(e));
    }
  }
}

class _Section extends StatelessWidget {
  const _Section({required this.label});

  final String label;

  @override
  Widget build(BuildContext context) => Padding(
    padding: const EdgeInsets.only(left: 4, bottom: 8, top: 4),
    child: Text(
      label,
      style: Theme.of(context).textTheme.labelLarge?.copyWith(
        color: Theme.of(context).colorScheme.onSurfaceVariant,
      ),
    ),
  );
}

class _Muted extends StatelessWidget {
  const _Muted(this.text);

  final String text;

  @override
  Widget build(BuildContext context) => Padding(
    padding: const EdgeInsets.all(12),
    child: Text(
      text,
      style: TextStyle(color: Theme.of(context).colorScheme.onSurfaceVariant),
    ),
  );
}

/// A one-field prompt.
///
/// A `StatefulWidget` so the controller outlives the closing animation — the
/// same reason `_PathDialog` is one; disposing it while the route animates out
/// produced a red screen once already.
Future<String?> promptForText(
  BuildContext context, {
  required String title,
  required String hint,
  String initial = '',
}) => showDialog<String>(
  context: context,
  builder: (_) => _TextDialog(title: title, hint: hint, initial: initial),
);

class _TextDialog extends StatefulWidget {
  const _TextDialog({
    required this.title,
    required this.hint,
    required this.initial,
  });

  final String title;
  final String hint;
  final String initial;

  @override
  State<_TextDialog> createState() => _TextDialogState();
}

class _TextDialogState extends State<_TextDialog> {
  late final _controller = TextEditingController(text: widget.initial);

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  void _submit() => Navigator.pop(context, _controller.text);

  @override
  Widget build(BuildContext context) => AlertDialog(
    title: Text(widget.title),
    content: TextField(
      controller: _controller,
      autofocus: true,
      decoration: InputDecoration(hintText: widget.hint),
      onSubmitted: (_) => _submit(),
    ),
    actions: [
      TextButton(
        onPressed: () => Navigator.pop(context),
        child: const Text('Cancel'),
      ),
      FilledButton(onPressed: _submit, child: const Text('OK')),
    ],
  );
}
