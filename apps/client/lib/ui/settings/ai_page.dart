import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';

import '../../api/models.dart';
import '../../router.dart';
import '../../state/app_state.dart';
import '../controls.dart';
import '../states.dart' show describeFailure;
import '../tokens.dart';
import 'settings_widgets.dart';

/// Settings › AI access: what apps outside Storm (the MCP endpoint) and agents
/// inside it may do. Both are the server's settings, not this device's.
class AiPage extends ConsumerStatefulWidget {
  const AiPage({super.key});

  @override
  ConsumerState<AiPage> createState() => _AiPageState();
}

typedef _Access = ({bool enabled, bool writable, bool? agentWrites});

class _AiPageState extends ConsumerState<AiPage> {
  /// Where the toggles are going while a request is in flight, so they do
  /// not snap back until the config refreshes.
  _Access? _pending;

  Future<void> _run(_Access next, Future<void> Function() send) async {
    setState(() => _pending = next);
    try {
      await send();
      ref.invalidate(serverConfigProvider);
      await ref.read(serverConfigProvider.future);
    } catch (e) {
      if (mounted) settingsToast(context, describeFailure(e));
    } finally {
      if (mounted) setState(() => _pending = null);
    }
  }

  Future<void> _setMcp(ServerConfig c, {required bool on, required bool w}) {
    final api = ref.read(apiProvider);
    if (api == null) return Future.value();
    // Writes cannot outlive the endpoint, matching the server.
    final writable = on && w;
    return _run((
      enabled: on,
      writable: writable,
      agentWrites: c.agentWrites,
    ), () => api.setMcpEnabled(on, writable: writable));
  }

  Future<void> _setAgentWrites(ServerConfig c, bool on) {
    final api = ref.read(apiProvider);
    if (api == null) return Future.value();
    return _run((
      enabled: c.mcpEnabled,
      writable: c.mcpWritable,
      agentWrites: on,
    ), () => api.setAgentWrites(on));
  }

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final config = ref.watch(serverConfigProvider);

    return SettingsPage(
      title: 'AI access',
      intro:
          'What AI apps outside Storm, and agents inside it, can do with your '
          'notes.',
      children: config.when(
        loading: () => [const SettingsMuted('Loading…')],
        error: (e, _) => [SettingsMuted(describeFailure(e), danger: true)],
        data: (c) {
          if (c == null) return [const SettingsMuted('Not connected.')];
          final p = _pending;
          final on = p?.enabled ?? c.mcpEnabled;
          final writable = p?.writable ?? c.mcpWritable;
          final agentWrites = p == null ? c.agentWrites : p.agentWrites;
          final idle = p == null;
          return [
            const GroupLabel('AI apps outside Storm'),
            SettingsRow(
              label: 'Let AI apps read your notes',
              sub: 'Serves your vaults to apps that hold an access key.',
              trailing: StormToggle(
                key: const Key('mcp-read'),
                value: on,
                onChanged: idle ? (v) => _setMcp(c, on: v, w: writable) : null,
              ),
            ),
            SettingsRow(
              label: 'Let them create, edit and delete',
              sub: 'Storm has no trash. A deleted note is gone.',
              muted: !on,
              trailing: StormToggle(
                key: const Key('mcp-write'),
                value: writable,
                onChanged: idle && on ? (v) => _setMcp(c, on: on, w: v) : null,
              ),
            ),
            Padding(
              padding: EdgeInsets.symmetric(vertical: t.sp * 0.75),
              child: Align(
                alignment: Alignment.centerLeft,
                child: TextAction(
                  key: const Key('go-access'),
                  label: 'Access keys are in Devices & access ›',
                  accent: true,
                  onTap: () => context.go(Routes.settingsPage('access')),
                ),
              ),
            ),
            GroupLabel('Storm agents', top: t.sp * 2.5),
            SettingsRow(
              label: 'Sessions can read your vaults',
              sub: 'Every session except a shell.',
              trailing: const MonoValue('always'),
            ),
            // Wired only against a server that reports the setting; an older
            // one has no way to store it.
            SettingsRow(
              label: 'Allow writes when chosen at launch',
              sub: agentWrites == null
                  ? 'Needs a newer server'
                  : 'You pick one vault per session. Agents never delete '
                        'notes.',
              muted: agentWrites == null,
              trailing: StormToggle(
                key: const Key('agent-writes'),
                value: agentWrites ?? false,
                onChanged: idle && agentWrites != null
                    ? (v) => _setAgentWrites(c, v)
                    : null,
              ),
            ),
          ];
        },
      ),
    );
  }
}
