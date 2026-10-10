import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter_lucide/flutter_lucide.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../state/health.dart' show relativeTime;
import '../ui/clipboard_copy.dart';
import '../ui/controls.dart';
import '../ui/settings/settings_widgets.dart';
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
    return SettingsPage(
      title: 'Hosts & default agent',
      intro: 'The machines agents run on.',
      children: [
        if (_error != null) SettingsMuted(_error!, danger: true),
        if (_loading && hosts == null)
          const SkeletonRows(rows: 2)
        else if (hosts != null && hosts.isEmpty)
          const SettingsMuted('No hosts enrolled yet.')
        else
          for (final host in hosts ?? <AgentHost>[])
            _HostTile(
              host: host,
              onRename: () => _rename(host),
              onRevoke: () => _revoke(host),
            ),
        SettingsButtonRow(
          child: StormButton.primary(
            key: const Key('enroll-host'),
            label: '＋ Enroll a host',
            onPressed: _loading ? null : _enroll,
          ),
        ),
        SizedBox(height: t.sp * 0.75),
        Text.rich(
          TextSpan(
            children: [
              const TextSpan(text: 'Enrolling shows a one-time string. Run '),
              TextSpan(
                text: 'storm-runtime enroll',
                style: TextStyle(
                  fontFamily: StormTokens.monoFamily,
                  color: t.text2,
                ),
              ),
              const TextSpan(text: ' on the machine and paste it.'),
            ],
          ),
          style: TextStyle(
            fontFamily: StormTokens.sansFamily,
            fontSize: t.labelSize * 1.09,
            color: t.text3,
            height: 1.5,
          ),
        ),
        if (_defaultProvider != null) ...[
          GroupLabel('Default agent', top: t.sp * 3, bottom: t.sp),
          _DefaultProvider(
            current: _defaultProvider!,
            hosts: hosts ?? const [],
            onChanged: _setDefault,
          ),
        ],
      ],
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
    final ids = <String>{
      'claude-code',
      'opencode',
      'shell',
      for (final h in hosts) ...h.providers.map((p) => p.id),
      current,
    }.toList();
    return ChoiceChips<String>(
      key: const Key('default-provider'),
      options: [for (final id in ids) ChoiceOption(id, providerLabel(id))],
      selected: current,
      onSelected: onChanged,
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
    final seen = DateTime.tryParse(host.lastSeen ?? '');
    final state = host.online
        ? 'online now'
        : revoked
        ? 'revoked'
        : seen == null
        ? 'offline'
        : 'offline · last seen ${relativeTime(seen)}';
    final agents = host.providers.isEmpty
        ? 'agents appear once it connects'
        : host.providers
              .map((p) => p.available ? p.label : '${p.label} (not installed)')
              .join(', ');
    // Freeze §12.2: V1 has no egress policy of its own, and says so.
    return Opacity(
      opacity: revoked ? 0.55 : 1,
      child: SettingsRow(
        key: Key('host-${host.id}'),
        leading: Container(
          width: t.sp,
          height: t.sp,
          decoration: BoxDecoration(
            shape: BoxShape.circle,
            color: host.online ? t.green : t.text3,
          ),
        ),
        label: host.name,
        sub: '$state · $agents · network: host policy',
        monoSub: true,
        trailing: revoked
            ? null
            : RowActions(
                children: [
                  TextAction(
                    key: Key('rename-host-${host.id}'),
                    label: 'Rename',
                    onTap: onRename,
                  ),
                  TextAction(
                    key: Key('revoke-host-${host.id}'),
                    label: 'Revoke',
                    onTap: onRevoke,
                  ),
                ],
              ),
      ),
    );
  }
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

/// The machine being enrolled. It is not necessarily the one this client
/// runs on, so the enroll sheet offers both (AM40).
enum HostPlatform { linux, macos }

/// The platform the enroll sheet opens on: macOS on a Mac (native or web),
/// Linux everywhere else.
HostPlatform defaultHostPlatform() =>
    defaultTargetPlatform == TargetPlatform.macOS
    ? HostPlatform.macos
    : HostPlatform.linux;

/// One command the operator runs on the host, in order.
class EnrollStep {
  const EnrollStep(this.command, {this.pastesString = false});

  final String command;

  /// The step that asks for the enrollment string.
  final bool pastesString;
}

/// What to run on a host of [platform], and nothing of the other's: Linux is
/// the `.deb` and systemd (decision 77e), macOS is Homebrew and a launchd
/// LaunchDaemon (D14, AM35).
List<EnrollStep> enrollSteps(HostPlatform platform) => switch (platform) {
  HostPlatform.linux => const [
    EnrollStep('sudo apt install storm-runtime'),
    EnrollStep(
      'sudo -u storm-runtime storm-runtime enroll',
      pastesString: true,
    ),
    EnrollStep('sudo systemctl enable --now storm-runtime'),
  ],
  HostPlatform.macos => const [
    EnrollStep('brew tap dewanshdt/storm https://github.com/dewanshDT/Storm'),
    EnrollStep('brew install storm-runtime'),
    // The full path: `sudo` need not search Homebrew's prefix.
    EnrollStep('sudo "\$(brew --prefix)/bin/storm-runtime" install'),
    // `cd /` first: the hidden account cannot read the operator's current
    // directory, and the command fails before asking for the string (B-4).
    EnrollStep(
      'cd / && sudo -u _stormruntime /Library/StormRuntime/bin/storm-runtime enroll',
      pastesString: true,
    ),
  ],
};

/// Where the host runs and keeps its workspaces, in a line.
String enrollFootnote(HostPlatform platform) => switch (platform) {
  HostPlatform.linux =>
    'It runs under systemd as the storm-runtime account. Workspaces are the '
        'folders in /var/lib/storm-runtime/workspaces.',
  HostPlatform.macos =>
    'launchd starts it by itself once it is enrolled (the LaunchDaemon '
        'dev.storm.runtime), as the dedicated _stormruntime account. '
        'Workspaces are the folders in /Library/StormRuntime/workspaces.',
};

/// The enrollment string, shown once: it carries a single-use secret that
/// nothing stores.
class _EnrollDialog extends StatefulWidget {
  const _EnrollDialog({required this.enrollment, required this.expires});

  final String enrollment;
  final String expires;

  @override
  State<_EnrollDialog> createState() => _EnrollDialogState();
}

/// The enrollment string with its secret hidden: the scheme, then the last
/// four characters, enough to tell two strings apart (B-6). The sheet ends up
/// in screenshots; the string works once, but there is no need to show it.
String maskEnrollment(String s) {
  const shown = 'storm-enroll:v1:';
  if (s.length <= shown.length + 4) return '•' * s.length;
  final head = s.startsWith(shown) ? shown : '';
  return '$head${'•' * 12}${s.substring(s.length - 4)}';
}

class _EnrollDialogState extends State<_EnrollDialog> {
  bool _copied = false;
  bool _revealed = false;
  HostPlatform _platform = defaultHostPlatform();

  /// The command whose copy button last succeeded, by index.
  int? _copiedCommand;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final until = DateTime.tryParse(widget.expires)?.toLocal();
    final when = until == null
        ? 'soon'
        : '${until.hour.toString().padLeft(2, '0')}:'
              '${until.minute.toString().padLeft(2, '0')}';
    final steps = enrollSteps(_platform);
    final mono = TextStyle(
      fontFamily: StormTokens.monoFamily,
      fontSize: t.labelSize,
    );
    final box = BoxDecoration(
      color: t.surface2,
      borderRadius: BorderRadius.circular(t.rControl),
    );
    return AlertDialog(
      title: const Text('Enroll a host'),
      content: SizedBox(
        width: 520,
        child: SingleChildScrollView(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              ChoiceChips<HostPlatform>(
                key: const Key('enroll-platform'),
                options: const [
                  ChoiceOption(HostPlatform.linux, 'Linux'),
                  ChoiceOption(HostPlatform.macos, 'macOS'),
                ],
                selected: _platform,
                onSelected: (p) => setState(() {
                  _platform = p;
                  _copiedCommand = null;
                }),
              ),
              SizedBox(height: t.sp * 1.5),
              Text(
                'On the host, run these in order, and paste this string when '
                'enroll asks for it. It works once, until $when.',
              ),
              SizedBox(height: t.sp),
              for (final (i, step) in steps.indexed) ...[
                Container(
                  key: Key('enroll-command-$i'),
                  padding: EdgeInsets.only(left: t.sp),
                  decoration: box,
                  child: Row(
                    children: [
                      Expanded(
                        child: SelectableText(step.command, style: mono),
                      ),
                      IconButton(
                        key: Key('copy-command-$i'),
                        tooltip: 'Copy',
                        iconSize: 16,
                        icon: Icon(
                          _copiedCommand == i
                              ? LucideIcons.check
                              : LucideIcons.copy,
                        ),
                        onPressed: () async {
                          final ok = await copyToClipboard(step.command);
                          if (mounted) {
                            setState(() => _copiedCommand = ok ? i : null);
                          }
                        },
                      ),
                    ],
                  ),
                ),
                SizedBox(height: t.sp * 0.5),
              ],
              SizedBox(height: t.sp * 0.5),
              Container(
                padding: EdgeInsets.only(left: t.sp),
                decoration: box,
                child: Row(
                  children: [
                    Expanded(
                      child: SelectableText(
                        _revealed
                            ? widget.enrollment
                            : maskEnrollment(widget.enrollment),
                        key: const Key('enrollment-string'),
                        style: mono,
                      ),
                    ),
                    IconButton(
                      key: const Key('reveal-enrollment'),
                      tooltip: _revealed ? 'Hide' : 'Show',
                      iconSize: 16,
                      icon: Icon(
                        _revealed ? LucideIcons.eye_off : LucideIcons.eye,
                      ),
                      onPressed: () => setState(() => _revealed = !_revealed),
                    ),
                  ],
                ),
              ),
              SizedBox(height: t.sp),
              Text(
                enrollFootnote(_platform),
                key: const Key('enroll-footnote'),
                style: TextStyle(fontSize: t.labelSize, color: t.text3),
              ),
              SizedBox(height: t.sp * 0.5),
              Text(
                'Close this and the string is gone. Issue a new one if you need '
                'it.',
                style: TextStyle(fontSize: t.labelSize, color: t.text3),
              ),
            ],
          ),
        ),
      ),
      actions: [
        TextButton.icon(
          onPressed: () async {
            final ok = await copyToClipboard(widget.enrollment);
            if (mounted) setState(() => _copied = ok);
          },
          icon: Icon(_copied ? LucideIcons.check : LucideIcons.copy, size: 16),
          label: Text(_copied ? 'Copied' : 'Copy string'),
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
