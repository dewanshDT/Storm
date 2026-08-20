import 'package:flutter/material.dart';
import 'package:flutter_lucide/flutter_lucide.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';

import '../api/models.dart';
import '../router.dart';
import 'tokens.dart';
import 'widgets.dart';
import '../state/app_state.dart';
import 'states.dart';

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
    final settings = ref.watch(settingsProvider).value;

    return Scaffold(
      appBar: AppBar(
        leading: IconButton(
          icon: const Icon(LucideIcons.arrow_left),
          onPressed: () => context.canPop()
              ? context.pop()
              // Mounted at two paths: the vault-scoped one goes back into the
              // vault, the dashboard's one goes home.
              : context.go(
                  Routes.vaultOf(GoRouterState.of(context).uri).isEmpty
                      ? Routes.dashboard
                      : Routes.browse(
                          Routes.vaultOf(GoRouterState.of(context).uri),
                        ),
                ),
        ),
        title: const Text('Server settings'),
      ),
      body: ListView(
        padding: const EdgeInsets.fromLTRB(16, 8, 16, 32),
        children: [
          _Section(label: 'Vault storage root'),
          config.when(
            loading: () => const SkeletonRows(rows: 2),
            error: (e, _) => _Muted(describeFailure(e)),
            data: (c) => c == null
                ? const _Muted('Not connected')
                : _RootCard(config: c),
          ),
          const SizedBox(height: 24),
          _Section(label: 'AI access (MCP)'),
          config.when(
            loading: () => const SkeletonRows(rows: 2),
            error: (e, _) => _Muted(describeFailure(e)),
            data: (c) =>
                c == null ? const _Muted('Not connected') : _McpCard(config: c),
          ),
          const SizedBox(height: 24),
          _Section(label: 'Accounts'),
          config.when(
            loading: () => const SkeletonRows(rows: 2),
            error: (e, _) => _Muted(describeFailure(e)),
            data: (c) => c == null
                ? const _Muted('Not connected')
                : _RegistrationCard(config: c),
          ),
          const SizedBox(height: 24),
          _Section(label: 'MCP keys'),
          // Only for a signed-in caller: minting is session tier, and a key
          // belongs to an account.
          if (settings?.hasSession ?? false)
            Align(
              alignment: Alignment.centerLeft,
              child: OutlinedButton.icon(
                key: const Key('manage-mcp-keys'),
                onPressed: () => context.push(Routes.mcpKeys),
                icon: const Icon(Icons.key),
                label: const Text('Manage MCP keys'),
              ),
            )
          else
            const _Muted('Sign in to create keys for MCP clients.'),
          const SizedBox(height: 24),
          _Section(label: 'Vaults'),
          vaults.when(
            loading: () => const SkeletonRows(rows: 2),
            error: (e, _) => _Muted(describeFailure(e)),
            data: (list) => Column(
              children: [
                for (final v in list) _VaultRow(vault: v),
                const SizedBox(height: 8),
                Align(
                  alignment: Alignment.centerLeft,
                  child: OutlinedButton.icon(
                    onPressed: () => createVault(context, ref),
                    icon: const Icon(LucideIcons.plus, size: 18),
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
  /// Set while a request is in flight so the switches show where they are
  /// going, not where they were. Without it they snap back until the provider
  /// refreshes, which reads as the toggle having failed.
  ({bool enabled, bool writable})? _pending;

  Future<void> _set({required bool enabled, required bool writable}) async {
    final api = ref.read(apiProvider);
    if (api == null) return;
    // Writes cannot outlive the endpoint, matching the server: leaving them
    // armed while MCP is off would silently restore write access the next time
    // it is switched on.
    final next = (enabled: enabled, writable: enabled && writable);
    setState(() => _pending = next);
    try {
      await api.setMcpEnabled(next.enabled, writable: next.writable);
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
    final on = _pending?.enabled ?? widget.config.mcpEnabled;
    final writable = _pending?.writable ?? widget.config.mcpWritable;

    // No card. A settings screen is a list of facts, and wrapping each group in
    // a bordered box made every one of them look like a separate object; the
    // label above and the hairlines below do the grouping instead.
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        SwitchListTile(
          contentPadding: EdgeInsets.zero,
          value: on,
          onChanged: _pending == null
              ? (v) => _set(enabled: v, writable: writable)
              : null,
          title: const Text('Let AI assistants read your notes'),
          subtitle: Text(
            on ? 'Serving at /mcp' : 'Off — /mcp refuses every request',
            style: theme.textTheme.bodySmall?.copyWith(
              color: scheme.onSurfaceVariant,
            ),
          ),
        ),
        // Nested under the first, and dead while it is off, because writes
        // without the endpoint mean nothing.
        SwitchListTile(
          contentPadding: const EdgeInsets.only(left: 16),
          value: writable,
          onChanged: on && _pending == null
              ? (v) => _set(enabled: on, writable: v)
              : null,
          title: Text(
            'Allow it to create, edit and delete',
            style: TextStyle(
              color: on ? null : scheme.onSurfaceVariant.withValues(alpha: 0.5),
            ),
          ),
          subtitle: Text(
            writable
                ? 'Edits merge like any other device’s'
                : 'Read-only — it can look, not change',
            style: theme.textTheme.bodySmall?.copyWith(
              color: scheme.onSurfaceVariant,
            ),
          ),
        ),
        Padding(
          padding: const EdgeInsets.fromLTRB(0, 2, 8, 0),
          child: Text(
            !on
                ? 'A server setting, not just this device — it applies to '
                      'every client and survives a restart.'
                : writable
                ? 'An assistant can change your notes. Edits carry the '
                      'version they were made from, so nothing overwrites a '
                      'change from another device — but Storm has no trash, '
                      'and a deleted note is gone from the vault at once.'
                : 'Search, read and history only. Anyone holding this '
                      'server’s token can read every vault this way.',
            style: theme.textTheme.bodySmall?.copyWith(
              color: scheme.onSurfaceVariant,
            ),
          ),
        ),
      ],
    );
  }
}

/// The A10 migration switch: whether the server still accepts `STORM_TOKEN`.
///
/// Whether this server takes new accounts (A13).
///
/// Off by default, and the copy says what "on" actually means rather than
/// leaving someone to infer it: with the web client bootstrapping its own
/// device, *anyone who can reach this server* can then create an account.
/// That is a reasonable thing to want on a home network and a bad thing to
/// discover afterwards.
class _RegistrationCard extends ConsumerStatefulWidget {
  const _RegistrationCard({required this.config});

  final ServerConfig config;

  @override
  ConsumerState<_RegistrationCard> createState() => _RegistrationCardState();
}

class _RegistrationCardState extends ConsumerState<_RegistrationCard> {
  bool? _pending;

  Future<void> _set(bool enabled) async {
    final api = ref.read(apiProvider);
    if (api == null) return;
    setState(() => _pending = enabled);
    try {
      await api.setRegistrationOpen(enabled);
      ref.invalidate(serverConfigProvider);
    } catch (e) {
      // Owner-only, server-side. A member sees the refusal rather than a
      // switch that appears to work and does not.
      if (mounted) _toast(context, describeFailure(e));
    } finally {
      if (mounted) setState(() => _pending = null);
    }
  }

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    final theme = Theme.of(context);
    final on = _pending ?? widget.config.allowRegistration;

    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        SwitchListTile(
          key: const Key('registration-switch'),
          contentPadding: EdgeInsets.zero,
          value: on,
          onChanged: _pending == null ? _set : null,
          title: const Text('Allow new accounts'),
          subtitle: Text(
            on
                ? 'Anyone who can reach this server can sign up'
                : 'Off — accounts are created on the server',
            style: theme.textTheme.bodySmall?.copyWith(
              color: scheme.onSurfaceVariant,
            ),
          ),
        ),
        Padding(
          padding: const EdgeInsets.fromLTRB(0, 2, 8, 0),
          child: Text(
            on
                ? 'The sign-in screen now offers "Create an account", and new '
                      'accounts are members — they can read and write notes, '
                      'and cannot change this setting. Turn it off when '
                      'everyone who needs an account has one.'
                : 'New accounts are added with `storm-server user add` on the '
                      'machine itself. Turn this on to let people sign up from '
                      'the app instead.',
            style: theme.textTheme.bodySmall?.copyWith(
              color: scheme.onSurfaceVariant,
            ),
          ),
        ),
      ],
    );
  }
}

class _RootCard extends ConsumerWidget {
  const _RootCard({required this.config});

  final ServerConfig config;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final t = context.tokens;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        SettingRow(label: 'Vault root', value: config.vaultRoot),
        SettingRow(label: 'Vaults', value: '${config.vaultCount}'),
        SizedBox(height: t.sp * 1.5),
        // Said before the action, not after: Storm points at directories
        // someone has already moved, and a mistyped path that silently
        // orphaned every vault would read as "my notes are gone".
        Text(
          'Changing this does not move your notes. Move the vault '
          'directories first, then change the root.',
          style: TextStyle(
            fontFamily: StormTokens.sansFamily,
            fontSize: t.codeSize,
            color: t.text3,
          ),
        ),
        SizedBox(height: t.sp * 1.25),
        OutlinedButton(
          onPressed: () => _change(context, ref),
          child: const Text('Change'),
        ),
      ],
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
        vault.missing ? LucideIcons.circle_alert : LucideIcons.folder_cog,
        color: vault.missing ? scheme.error : scheme.primary,
      ),
      title: Text(vault.name),
      subtitle: Text(
        vault.missing
            ? 'Directory “${vault.dir}” not found'
            : '${vault.dir} · ${vault.noteCount} '
                  'note${vault.noteCount == 1 ? '' : 's'}',
        style: TextStyle(
          fontSize: context.tokens.labelSize,
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
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Padding(
      padding: EdgeInsets.only(bottom: t.sp, top: t.sp * 0.5),
      child: SectionLabel(label),
    );
  }
}

/// One setting: what it is, and what it currently says.
///
/// A row with a hairline under it rather than a card. The design is flat here
/// because a settings screen is a list of facts — wrapping each in its own
/// bordered box made every one of them look like a separate object.
class SettingRow extends StatelessWidget {
  const SettingRow({
    super.key,
    required this.label,
    this.value,
    this.trailing,
    this.onTap,
    this.mono = true,
    this.muted = false,
  });

  final String label;
  final String? value;
  final Widget? trailing;
  final VoidCallback? onTap;

  /// Values that are paths, addresses or identifiers are mono; prose is not.
  final bool mono;
  final bool muted;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final row = Container(
      padding: EdgeInsets.symmetric(vertical: t.sp * 1.5),
      decoration: BoxDecoration(
        border: Border(
          bottom: BorderSide(color: t.border, width: t.bw),
        ),
      ),
      child: Row(
        children: [
          Expanded(
            child: Text(
              label,
              style: TextStyle(
                fontFamily: StormTokens.sansFamily,
                fontSize: t.bodySize,
                color: muted ? t.text3 : t.text,
              ),
            ),
          ),
          if (value != null)
            Flexible(
              child: Text(
                value!,
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                textAlign: TextAlign.right,
                style: TextStyle(
                  fontFamily: mono
                      ? StormTokens.monoFamily
                      : StormTokens.sansFamily,
                  fontSize: t.codeSize,
                  color: t.text2,
                ),
              ),
            ),
          if (trailing != null) ...[SizedBox(width: t.sp), trailing!],
        ],
      ),
    );
    if (onTap == null) return row;
    return InkWell(onTap: onTap, child: row);
  }
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
