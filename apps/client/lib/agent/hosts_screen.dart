import 'package:flutter/material.dart';
import 'package:flutter_lucide/flutter_lucide.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../ui/clipboard_copy.dart';
import '../ui/states.dart';
import '../ui/tokens.dart';
import 'agent_api.dart';
import 'agent_models.dart';
import 'agent_state.dart';

/// Runtime Hosts: the machines that run agents for this server (freeze §10,
/// item 1). Owner only; the route is reachable only from an entry point the
/// owner check shows.
class HostsScreen extends ConsumerStatefulWidget {
  const HostsScreen({super.key});

  @override
  ConsumerState<HostsScreen> createState() => _HostsScreenState();
}

class _HostsScreenState extends ConsumerState<HostsScreen> {
  List<AgentHost>? _hosts;
  String? _defaultProvider;
  String? _error;
  bool _loading = false;

  @override
  void initState() {
    super.initState();
    _load();
  }

  Future<T?> _with<T>(Future<T> Function(AgentApi api) run) async {
    final api = agentApi(ref);
    if (api == null) {
      setState(() => _error = 'Sign in first.');
      return null;
    }
    try {
      return await run(api);
    } catch (e) {
      if (mounted) setState(() => _error = describeFailure(e));
      return null;
    } finally {
      api.dispose();
    }
  }

  Future<void> _load() async {
    setState(() {
      _loading = true;
      _error = null;
    });
    final result = await _with(
      (api) async => (await api.hosts(), await api.defaultProvider()),
    );
    if (!mounted) return;
    setState(() {
      if (result != null) {
        _hosts = result.$1;
        _defaultProvider = result.$2;
      }
      _loading = false;
    });
    // Every change here ends in this reload, and at desk width the sessions
    // sidebar sits beside this screen showing the same hosts: refresh it too
    // rather than leave it up to a poll behind.
    ref.invalidate(agentOverviewProvider);
  }

  Future<void> _enroll() async {
    final issued = await _with((api) => api.enroll());
    if (issued == null || !mounted) return;
    await showDialog<void>(
      context: context,
      barrierDismissible: false,
      builder: (_) =>
          _EnrollDialog(enrollment: issued.enrollment, expires: issued.expires),
    );
    if (mounted) await _load();
  }

  Future<void> _rename(AgentHost host) async {
    final name = await showDialog<String>(
      context: context,
      builder: (_) => _RenameDialog(initial: host.name),
    );
    if (name == null || name.trim().isEmpty || !mounted) return;
    await _with((api) => api.renameHost(host.id, name.trim()));
    if (mounted) await _load();
  }

  Future<void> _revoke(AgentHost host) async {
    final confirmed = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: Text('Revoke ${host.name}?'),
        content: const Text(
          'Its sessions end and it can no longer connect. To use the machine '
          'again, enroll it again.',
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.of(context).pop(false),
            child: const Text('Cancel'),
          ),
          FilledButton(
            onPressed: () => Navigator.of(context).pop(true),
            child: const Text('Revoke'),
          ),
        ],
      ),
    );
    if (confirmed != true || !mounted) return;
    await _with((api) => api.revokeHost(host.id));
    if (mounted) await _load();
  }

  Future<void> _setDefault(String id) async {
    await _with((api) => api.setDefaultProvider(id));
    if (mounted) setState(() => _defaultProvider = id);
  }

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final hosts = _hosts;
    return Scaffold(
      appBar: AppBar(
        title: const Text('Hosts'),
        actions: [
          IconButton(
            tooltip: 'Refresh',
            onPressed: _loading ? null : _load,
            icon: const Icon(LucideIcons.refresh_cw, size: 18),
          ),
        ],
      ),
      floatingActionButton: FloatingActionButton.extended(
        key: const Key('enroll-host'),
        onPressed: _loading ? null : _enroll,
        icon: const Icon(LucideIcons.plus),
        label: const Text('Enroll a host'),
      ),
      body: RefreshIndicator(
        onRefresh: _load,
        child: ListView(
          padding: EdgeInsets.all(t.sp * 2),
          children: [
            Text(
              'A host is a machine that runs agents for this server, under its '
              'own account. Agents work on its files, not on your vaults.',
              style: TextStyle(fontSize: t.labelSize, color: t.text3),
            ),
            SizedBox(height: t.sp * 2),
            if (_error != null) ...[
              Text(_error!, style: TextStyle(color: t.danger)),
              SizedBox(height: t.sp * 2),
            ],
            if (_defaultProvider != null && (hosts?.isNotEmpty ?? false))
              _DefaultProvider(
                current: _defaultProvider!,
                hosts: hosts!,
                onChanged: _setDefault,
              ),
            SizedBox(height: t.sp * 2),
            if (_loading && hosts == null)
              const SkeletonRows(rows: 3)
            else if (hosts != null && hosts.isEmpty)
              EmptyState(
                icon: LucideIcons.server,
                title: 'No hosts yet',
                detail:
                    'Install storm-runtime on a machine, then enroll it here.',
                action: 'Enroll a host',
                onAction: _enroll,
              )
            else
              for (final host in hosts ?? <AgentHost>[])
                _HostTile(
                  host: host,
                  onRename: () => _rename(host),
                  onRevoke: () => _revoke(host),
                ),
          ],
        ),
      ),
    );
  }
}

class _DefaultProvider extends StatelessWidget {
  const _DefaultProvider({
    required this.current,
    required this.hosts,
    required this.onChanged,
  });

  final String current;
  final List<AgentHost> hosts;
  final ValueChanged<String> onChanged;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final ids = <String>{
      'claude-code',
      'opencode',
      'shell',
      for (final h in hosts) ...h.providers.map((p) => p.id),
      current,
    }.toList();
    return Row(
      children: [
        Expanded(
          child: Text(
            'Default provider',
            style: TextStyle(fontWeight: FontWeight.w600, color: t.text),
          ),
        ),
        DropdownButton<String>(
          key: const Key('default-provider'),
          value: current,
          onChanged: (v) => v == null ? null : onChanged(v),
          items: [
            for (final id in ids)
              DropdownMenuItem(value: id, child: Text(providerLabel(id))),
          ],
        ),
      ],
    );
  }
}

class _HostTile extends StatelessWidget {
  const _HostTile({
    required this.host,
    required this.onRename,
    required this.onRevoke,
  });

  final AgentHost host;
  final VoidCallback onRename;
  final VoidCallback onRevoke;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final revoked = host.status == 'revoked';
    final providers = host.providers.isEmpty
        ? 'Providers appear once it connects'
        : host.providers
              .map((p) => p.available ? p.label : '${p.label} (not installed)')
              .join(', ');
    return Opacity(
      opacity: revoked ? 0.55 : 1,
      child: Container(
        margin: EdgeInsets.only(bottom: t.sp),
        padding: EdgeInsets.all(t.sp * 1.5),
        decoration: BoxDecoration(
          color: t.surface,
          border: Border.all(color: t.border, width: t.bw),
          borderRadius: BorderRadius.circular(t.rCard),
        ),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Row(
              children: [
                Expanded(
                  child: Text(
                    host.name,
                    style: const TextStyle(fontWeight: FontWeight.w600),
                  ),
                ),
                StatusChip(
                  label: switch (host.status) {
                    'online' => 'Online',
                    'revoked' => 'Revoked',
                    _ => 'Offline',
                  },
                  tone: host.online ? ChipTone.good : ChipTone.muted,
                ),
                // Rename and revoke are rare, and revoke is destructive: a
                // menu, not two equal buttons on every card.
                if (!revoked)
                  PopupMenuButton<String>(
                    key: Key('host-menu-${host.id}'),
                    tooltip: 'Host actions',
                    icon: const Icon(LucideIcons.ellipsis_vertical, size: 18),
                    onSelected: (v) => v == 'rename' ? onRename() : onRevoke(),
                    itemBuilder: (_) => [
                      const PopupMenuItem(
                        value: 'rename',
                        child: Text('Rename'),
                      ),
                      PopupMenuItem(
                        value: 'revoke',
                        child: Text(
                          'Revoke',
                          style: TextStyle(color: t.danger),
                        ),
                      ),
                    ],
                  ),
              ],
            ),
            SizedBox(height: t.sp * 0.5),
            Text(
              host.online || host.lastSeen == null
                  ? providers
                  : '$providers. Last seen ${_when(host.lastSeen!)}.',
              style: TextStyle(fontSize: t.labelSize, color: t.text2),
            ),
            SizedBox(height: t.sp * 0.25),
            // Freeze §12.2: V1 has no egress policy of its own, and says so.
            Text(
              "Network: inherits this host's policy",
              style: TextStyle(fontSize: t.labelSize, color: t.text3),
            ),
          ],
        ),
      ),
    );
  }
}

/// A timestamp as a person reads it: the time today, the date otherwise.
String _when(String iso) {
  final at = DateTime.tryParse(iso)?.toLocal();
  if (at == null) return iso;
  final now = DateTime.now();
  final hm =
      '${at.hour.toString().padLeft(2, '0')}:${at.minute.toString().padLeft(2, '0')}';
  return at.year == now.year && at.month == now.month && at.day == now.day
      ? 'at $hm'
      : 'on ${at.year}-${at.month.toString().padLeft(2, '0')}-${at.day.toString().padLeft(2, '0')}';
}

enum ChipTone { good, warn, bad, muted }

/// A status label with a tinted background. It carries a real state, never
/// decoration (decision 77d).
class StatusChip extends StatelessWidget {
  const StatusChip({super.key, required this.label, required this.tone});

  final String label;
  final ChipTone tone;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final color = switch (tone) {
      ChipTone.good => t.green,
      ChipTone.warn => t.amber,
      ChipTone.bad => t.danger,
      ChipTone.muted => t.text3,
    };
    return Container(
      padding: EdgeInsets.symmetric(horizontal: t.sp, vertical: t.sp * 0.25),
      decoration: BoxDecoration(
        color: color.withValues(alpha: 0.14),
        borderRadius: BorderRadius.circular(t.rControl),
      ),
      // The family is set, not inherited: a chip inside a ListTile's trailing
      // slot would otherwise pick up the mono label style, and the same
      // component would look different on two screens.
      child: Text(
        label,
        style: TextStyle(
          fontFamily: StormTokens.sansFamily,
          fontSize: t.labelSize,
          color: color,
          fontWeight: FontWeight.w600,
          height: 1.2,
        ),
      ),
    );
  }
}

/// The enrollment string, shown once: it carries a single-use secret that
/// nothing stores.
class _EnrollDialog extends StatefulWidget {
  const _EnrollDialog({required this.enrollment, required this.expires});

  final String enrollment;
  final String expires;

  @override
  State<_EnrollDialog> createState() => _EnrollDialogState();
}

class _EnrollDialogState extends State<_EnrollDialog> {
  bool _copied = false;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final until = DateTime.tryParse(widget.expires)?.toLocal();
    final when = until == null
        ? 'soon'
        : '${until.hour.toString().padLeft(2, '0')}:'
              '${until.minute.toString().padLeft(2, '0')}';
    return AlertDialog(
      title: const Text('Enroll a host'),
      content: SizedBox(
        width: 520,
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text(
              'On the host, run the command below and paste this string when '
              'it asks. It works once, until $when.',
            ),
            SizedBox(height: t.sp * 1.5),
            Container(
              padding: EdgeInsets.all(t.sp),
              decoration: BoxDecoration(
                color: t.surface2,
                borderRadius: BorderRadius.circular(t.rControl),
              ),
              child: SelectableText(
                'sudo -u storm-runtime storm-runtime enroll',
                style: TextStyle(
                  fontFamily: StormTokens.monoFamily,
                  fontSize: t.labelSize,
                ),
              ),
            ),
            SizedBox(height: t.sp),
            Container(
              padding: EdgeInsets.all(t.sp),
              decoration: BoxDecoration(
                color: t.surface2,
                borderRadius: BorderRadius.circular(t.rControl),
              ),
              child: SelectableText(
                widget.enrollment,
                key: const Key('enrollment-string'),
                style: TextStyle(
                  fontFamily: StormTokens.monoFamily,
                  fontSize: t.labelSize,
                ),
              ),
            ),
            SizedBox(height: t.sp),
            Text(
              'Close this and the string is gone. Issue a new one if you need it.',
              style: TextStyle(fontSize: t.labelSize, color: t.text3),
            ),
          ],
        ),
      ),
      actions: [
        TextButton.icon(
          onPressed: () async {
            final ok = await copyToClipboard(widget.enrollment);
            if (mounted) setState(() => _copied = ok);
          },
          icon: Icon(_copied ? LucideIcons.check : LucideIcons.copy, size: 16),
          label: Text(_copied ? 'Copied' : 'Copy'),
        ),
        FilledButton(
          onPressed: () => Navigator.of(context).pop(),
          child: const Text('Done'),
        ),
      ],
    );
  }
}

class _RenameDialog extends StatefulWidget {
  const _RenameDialog({required this.initial});

  final String initial;

  @override
  State<_RenameDialog> createState() => _RenameDialogState();
}

class _RenameDialogState extends State<_RenameDialog> {
  late final _controller = TextEditingController(text: widget.initial);

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return AlertDialog(
      title: const Text('Rename host'),
      content: TextField(
        controller: _controller,
        autofocus: true,
        maxLength: 64,
        decoration: const InputDecoration(labelText: 'Name'),
        onSubmitted: (v) => Navigator.of(context).pop(v),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(),
          child: const Text('Cancel'),
        ),
        FilledButton(
          onPressed: () => Navigator.of(context).pop(_controller.text),
          child: const Text('Save'),
        ),
      ],
    );
  }
}
