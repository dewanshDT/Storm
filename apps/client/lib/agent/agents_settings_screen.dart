import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';

import '../router.dart';
import '../ui/controls.dart';
import '../ui/settings/settings_widgets.dart';
import '../ui/states.dart';
import '../ui/tokens.dart';
import 'agent_api.dart';
import 'agent_models.dart';
import 'agent_state.dart';
import 'agent_widgets.dart';

/// Why a host cannot install an agent, in words (decision 133).
String installBlockedText(String? code, String agent) => switch (code) {
  'no_node' => 'Needs Node.js on this host',
  'node_too_old' =>
    agent == 'command-code'
        ? 'Needs a newer Node.js on this host (22 or later)'
        : 'Needs a newer Node.js on this host',
  'unsupported_platform' => "Not available on this host's system yet",
  'disabled' => 'Installs are off on this host',
  _ => "Can't be installed on this host",
};

/// One line for a failed install, update or removal, from the job's or the
/// refusal's stable code. An unknown code is shown as it is.
String describeAgentJobError(String? code, {String agent = ''}) =>
    switch (code) {
      'download_failed' =>
        "The download failed, so nothing was installed. Check the host's "
            'network and try again.',
      'checksum_mismatch' =>
        "The download didn't match its checksum, so nothing was installed.",
      'unpack_failed' =>
        "The download couldn't be unpacked, so nothing was installed.",
      'npm_failed' => "npm couldn't install it on this host.",
      'version_probe_failed' =>
        "It installed, but didn't answer when asked its version.",
      'disk_full' => 'The host is out of disk space.',
      'not_managed' =>
        "Storm didn't install this copy, so it can't change it. Update or "
            'remove it the way you installed it.',
      'in_use' ||
      'agent_in_use' => 'Sessions are using it. End them and try again.',
      'busy' => 'The host is already changing an agent. Try again after.',
      'timeout' => 'It took too long, so the host stopped.',
      'no_node' ||
      'node_too_old' ||
      'unsupported_platform' ||
      'disabled' => '${installBlockedText(code, agent)}.',
      'unknown_agent' =>
        "This host doesn't know that agent. Update storm-runtime on it.",
      'host_too_old' => 'Update storm-runtime to manage agents here.',
      'host_offline' => 'The host is offline.',
      null || 'unknown' => 'Something went wrong on the host.',
      _ => 'Something went wrong on the host ($code).',
    };

String _signInCommand(String agent) => switch (agent) {
  'opencode' => 'opencode auth login',
  'codex' => 'codex login --device-auth',
  'cursor' => 'cursor-agent login',
  _ => '',
};

/// An action in flight on one agent on one host, or the last one's failure.
class _Run {
  const _Run({required this.op, this.step, this.error});

  final String op;
  final String? step;
  final String? error;

  bool get active => error == null;

  String get progress => switch ((op, step)) {
    ('remove', _) => 'Removing…',
    (_, 'Downloading') => 'Downloading…',
    (_, 'Verifying') => 'Verifying…',
    (_, 'Installing') => 'Installing…',
    _ => 'Starting…',
  };
}

/// Settings › Agents: the agent CLIs on each Runtime Host, installed,
/// updated and removed from here (decision 133).
class AgentsSettingsScreen extends ConsumerStatefulWidget {
  const AgentsSettingsScreen({
    super.key,
    this.pollEvery = const Duration(seconds: 1),
  });

  /// How often a running job is asked about.
  final Duration pollEvery;

  @override
  ConsumerState<AgentsSettingsScreen> createState() =>
      _AgentsSettingsScreenState();
}

class _AgentsSettingsScreenState extends ConsumerState<AgentsSettingsScreen> {
  List<AgentHost>? _hosts;
  String? _error;
  bool _loading = false;

  /// By `<host id>/<agent>`.
  final _runs = <String, _Run>{};
  final _polls = <String, Timer>{};

  @override
  void initState() {
    super.initState();
    _load();
  }

  @override
  void dispose() {
    for (final t in _polls.values) {
      t.cancel();
    }
    super.dispose();
  }

  Future<void> _load() async {
    final api = agentApi(ref);
    if (api == null) {
      setState(() => _error = 'Sign in first.');
      return;
    }
    setState(() {
      _loading = true;
      _error = null;
    });
    try {
      final hosts = [
        for (final h in await api.hosts())
          if (h.status != 'revoked') h,
      ];
      if (mounted) setState(() => _hosts = hosts);
    } catch (e) {
      if (mounted) setState(() => _error = describeFailure(e));
    } finally {
      api.dispose();
      if (mounted) setState(() => _loading = false);
    }
  }

  String _failure(Object e, String agent) => switch (e) {
    AgentRefusal(statusCode: 503) => 'The host is offline.',
    AgentRefusal(:final code?) => describeAgentJobError(code, agent: agent),
    _ => describeFailure(e),
  };

  Future<void> _start(AgentHost host, String agent, String op) async {
    if (op == 'remove') {
      final ok = await confirmSetting(
        context,
        title: 'Remove ${providerLabel(agent)} from ${host.name}?',
        body:
            'Storm deletes the copy it installed. Its sign-in and settings on '
            'the host stay.',
        action: 'Remove',
        confirmKey: const Key('agent-remove-confirm'),
      );
      if (!ok || !mounted) return;
    }
    final key = '${host.id}/$agent';
    final api = agentApi(ref);
    if (api == null) return;
    setState(() => _runs[key] = _Run(op: op));
    try {
      final job = await api.startAgentJob(host.id, agent, op);
      if (mounted) _adopt(key, host.id, job);
    } catch (e) {
      if (mounted) {
        setState(() => _runs[key] = _Run(op: op, error: _failure(e, agent)));
      }
    } finally {
      api.dispose();
    }
  }

  void _adopt(String key, String hostId, AgentJob job) {
    if (job.done) {
      setState(() => _runs.remove(key));
      _load();
      ref.invalidate(agentOverviewProvider);
      return;
    }
    setState(
      () => _runs[key] = job.failed
          ? _Run(
              op: job.op,
              error: describeAgentJobError(job.error, agent: job.agent),
            )
          : _Run(op: job.op, step: job.step),
    );
    // A failed install may have left nothing changed, or a removal half
    // done: the hosts are re-read either way.
    if (job.failed) {
      _load();
      return;
    }
    _polls[key] = Timer(widget.pollEvery, () => _poll(key, hostId, job));
  }

  Future<void> _poll(String key, String hostId, AgentJob job) async {
    _polls.remove(key);
    final api = agentApi(ref);
    if (api == null) return;
    try {
      final next = await api.agentJob(hostId, job.id);
      if (mounted) _adopt(key, hostId, next);
    } on AgentRefusal catch (e) {
      if (mounted) {
        setState(
          () => _runs[key] = _Run(op: job.op, error: _failure(e, job.agent)),
        );
      }
    } catch (_) {
      // The server could not be asked; the job carries on without us.
      if (mounted) {
        _polls[key] = Timer(widget.pollEvery, () => _poll(key, hostId, job));
      }
    } finally {
      api.dispose();
    }
  }

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final hosts = _hosts;
    return SettingsPage(
      title: 'Agents',
      beta: true,
      intro:
          'The agent CLIs on your hosts. Install and update them here, then '
          'sign in to each from a Shell session on its host.',
      children: [
        if (_error != null) SettingsMuted(_error!, danger: true),
        if (hosts == null && _loading)
          const SkeletonRows(rows: 3)
        else if (hosts != null)
          for (final (i, agent) in kInstallableAgents.indexed)
            Padding(
              key: Key('agent-section-$agent'),
              padding: EdgeInsets.only(top: i == 0 ? 0 : t.sp * 3),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  _AgentHeading(agent),
                  SizedBox(height: t.sp * 0.5),
                  if (hosts.isEmpty)
                    const SettingsMuted(
                      'No hosts are enrolled. Enroll one in Settings › Hosts.',
                    )
                  else
                    for (final host in hosts)
                      _HostAgentRow(
                        host: host,
                        agent: agent,
                        run: _runs['${host.id}/$agent'],
                        hostBusy: _runs.entries.any(
                          (e) =>
                              e.value.active && e.key.startsWith('${host.id}/'),
                        ),
                        onAction: (op) => _start(host, agent, op),
                      ),
                ],
              ),
            ),
      ],
    );
  }
}

/// The agent's mark and name, and how it signs in.
class _AgentHeading extends StatelessWidget {
  const _AgentHeading(this.agent);

  final String agent;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final side = t.sp * 3.5;
    final small = TextStyle(
      fontFamily: StormTokens.sansFamily,
      fontSize: t.codeSize,
      color: t.text2,
      height: 1.5,
    );
    final command = _signInCommand(agent);
    final signIn = switch (agent) {
      'claude-code' => Align(
        alignment: Alignment.centerLeft,
        child: TextAction(
          key: const Key('agents-claude-login'),
          label: 'Signs in with Settings › Claude login ›',
          accent: true,
          onTap: () => context.go(Routes.settingsPage('claude-login')),
        ),
      ),
      _ when command.isNotEmpty => Text.rich(
        TextSpan(
          children: [
            const TextSpan(text: 'Sign in with '),
            TextSpan(
              text: command,
              style: TextStyle(
                fontFamily: StormTokens.monoFamily,
                fontSize: t.labelSize * 1.09,
              ),
            ),
            const TextSpan(text: ' in a Shell session on the host.'),
          ],
        ),
        style: small,
      ),
      _ => Text('Sign in from a Shell session on the host.', style: small),
    };
    return Row(
      crossAxisAlignment: CrossAxisAlignment.start,
      spacing: t.sp * 1.5,
      children: [
        Container(
          width: side,
          height: side,
          alignment: Alignment.center,
          decoration: BoxDecoration(
            color: t.surface2,
            borderRadius: BorderRadius.circular(t.rControl),
          ),
          child: ProviderGlyph(agent, size: side * 0.6),
        ),
        Expanded(
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Text(
                providerLabel(agent),
                style: TextStyle(
                  fontFamily: StormTokens.sansFamily,
                  fontSize: t.uiSize,
                  fontWeight: FontWeight.w600,
                  color: t.text,
                ),
              ),
              signIn,
            ],
          ),
        ),
      ],
    );
  }
}

/// One host's copy of one agent: its version and where it came from, and
/// what can be done to it from here.
class _HostAgentRow extends StatelessWidget {
  const _HostAgentRow({
    required this.host,
    required this.agent,
    required this.run,
    required this.hostBusy,
    required this.onAction,
  });

  final AgentHost host;
  final String agent;
  final _Run? run;

  /// Something is being installed, updated or removed on this host: it does
  /// one at a time.
  final bool hostBusy;
  final ValueChanged<String> onAction;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final p = host.providers.where((p) => p.id == agent).firstOrNull;
    final installed = p != null && (p.available || p.source != null);
    final upToDate =
        p != null &&
        p.latest != null &&
        p.version != null &&
        p.latest == p.version;
    final String detail;
    if (!host.online) {
      detail = 'offline';
    } else if (!installed) {
      detail = 'Not installed';
    } else {
      detail = [
        ?p.version,
        switch (p.source) {
          'storm' => 'installed by Storm',
          'system' => 'your own install',
          _ => 'installed',
        },
        if (p.managed && upToDate) 'up to date',
      ].join(' · ');
    }

    final manages = host.online && host.agentInstall;
    final running = run?.active ?? false;
    final enabled = !hostBusy;
    final actions = <Widget>[
      if (manages && !running && p != null) ...[
        if (!installed && p.installable)
          TextAction(
            key: Key('agent-install-${host.id}-$agent'),
            label: 'Install',
            accent: true,
            onTap: enabled ? () => onAction('install') : null,
          ),
        if (p.managed && !upToDate)
          TextAction(
            key: Key('agent-update-${host.id}-$agent'),
            label: p.latest != null && p.latest != p.version
                ? 'Update to ${p.latest}'
                : 'Update',
            accent: true,
            onTap: enabled ? () => onAction('update') : null,
          ),
        if (p.managed)
          TextAction(
            key: Key('agent-remove-${host.id}-$agent'),
            label: 'Remove',
            onTap: enabled ? () => onAction('remove') : null,
          ),
      ],
    ];

    final notes = <Widget>[
      if (!host.online)
        SettingsNote(
          "It's offline. Its agents can be changed once it reconnects.",
          small: true,
        )
      else if (!host.agentInstall)
        const SettingsNote(
          'Update storm-runtime to manage agents here.',
          small: true,
        )
      else if (p != null && !installed && !p.installable)
        SettingsNote(
          installBlockedText(p.installBlocked, agent),
          key: Key('agent-blocked-${host.id}-$agent'),
          small: true,
        ),
      if (host.online && p != null && !p.checksumVerified)
        const SettingsNote('Not checksum-verified by Cursor', small: true),
      if (running)
        Text(
          run!.progress,
          key: Key('agent-progress-${host.id}-$agent'),
          style: TextStyle(
            fontFamily: StormTokens.sansFamily,
            fontSize: t.codeSize,
            color: t.text2,
          ),
        ),
      if (run?.error case final error?)
        Text(
          error,
          key: Key('agent-error-${host.id}-$agent'),
          style: TextStyle(
            fontFamily: StormTokens.sansFamily,
            fontSize: t.codeSize,
            color: t.danger,
          ),
        ),
    ];

    return Container(
      key: Key('agent-row-${host.id}-$agent'),
      decoration: BoxDecoration(
        border: Border(
          bottom: BorderSide(color: t.border, width: t.bw),
        ),
      ),
      padding: EdgeInsets.only(bottom: notes.isEmpty ? 0 : t.sp * 1.25),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          SettingsRow(
            label: host.name,
            sub: detail,
            monoSub: true,
            muted: !host.online,
            divider: false,
            trailing: actions.isEmpty ? null : RowActions(children: actions),
          ),
          for (final n in notes)
            Padding(
              padding: EdgeInsets.only(top: t.sp * 0.25),
              child: n,
            ),
        ],
      ),
    );
  }
}
