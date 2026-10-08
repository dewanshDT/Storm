import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';

import '../../api/models.dart';
import '../../router.dart';
import '../../state/app_state.dart';
import '../../state/health.dart' show relativeTime;
import '../clipboard_copy.dart';
import '../controls.dart';
import '../states.dart' show describeFailure;
import '../tokens.dart';
import 'settings_widgets.dart';

/// Settings › Devices & access: where you are signed in, and the access keys
/// AI apps outside Storm hold (handoff §5.3).
class AccessPage extends ConsumerStatefulWidget {
  const AccessPage({super.key});

  @override
  ConsumerState<AccessPage> createState() => _AccessPageState();
}

class _AccessPageState extends ConsumerState<AccessPage> {
  List<PairedDevice>? _devices;
  List<McpKey>? _keys;
  String? _error;

  @override
  void initState() {
    super.initState();
    _load();
  }

  Future<void> _load() async {
    final api = ref.read(apiProvider);
    if (api == null) {
      setState(() => _error = 'Not connected.');
      return;
    }
    try {
      final devices = await api.devices();
      final keys = await api.mcpKeys();
      if (!mounted) return;
      setState(() {
        _devices = devices.where((d) => !d.isRevoked).toList();
        _keys = keys.where((k) => !k.isRevoked).toList();
        _error = null;
      });
    } catch (e) {
      if (mounted) setState(() => _error = describeFailure(e));
    }
  }

  Future<void> _revokeDevice(PairedDevice d) async {
    final ok = await confirmSetting(
      context,
      title: 'Revoke ${d.name}?',
      body:
          'It is signed out at once and can no longer connect. To use it '
          'again, add it as a new device.',
      action: 'Revoke',
      confirmKey: const Key('confirm-revoke'),
    );
    if (!ok || !mounted) return;
    try {
      await ref.read(apiProvider)?.revokeDevice(d.id);
    } catch (e) {
      if (mounted) settingsToast(context, describeFailure(e));
    }
    await _load();
  }

  Future<void> _newKey() async {
    final name = await promptForText(
      context,
      title: 'New access key',
      label: 'Name',
      hint: 'Claude Code, work laptop',
      help:
          'Name it after the machine that will hold it. That name is how you '
          'will know what you are revoking later.',
    );
    if (name == null || name.trim().isEmpty || !mounted) return;
    final api = ref.read(apiProvider);
    if (api == null) return;
    try {
      final created = await api.createMcpKey(name: name.trim());
      if (!mounted) return;
      // Straight into the reveal, with no await in between that could drop
      // it: this value cannot be fetched again.
      await showDialog<void>(
        context: context,
        barrierDismissible: false,
        builder: (_) => RevealMcpKeyDialog(created: created),
      );
    } catch (e) {
      if (mounted) settingsToast(context, describeFailure(e));
    }
    await _load();
  }

  Future<void> _revokeKey(McpKey key) async {
    final ok = await confirmSetting(
      context,
      title: 'Revoke “${key.name}”?',
      body:
          'Whatever is using this key stops working on its next request. '
          'This cannot be undone: make a new key instead.',
      action: 'Revoke',
      confirmKey: const Key('confirm-revoke'),
    );
    if (!ok || !mounted) return;
    try {
      await ref.read(apiProvider)?.revokeMcpKey(key.id);
    } catch (e) {
      if (mounted) settingsToast(context, describeFailure(e));
    }
    await _load();
  }

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final settings = ref.watch(settingsProvider).value ?? const Settings();
    final devices = _devices;
    final keys = _keys;

    return SettingsPage(
      title: 'Devices & access',
      intro: 'Where you’re signed in, and keys for AI apps outside Storm.',
      children: [
        const GroupLabel('Signed-in devices'),
        if (_error != null)
          SettingsMuted(_error!, danger: true)
        else if (devices == null)
          const SettingsMuted('Loading…')
        else
          for (final d in _thisDeviceFirst(devices, settings.deviceId))
            d.id == settings.deviceId
                ? SettingsRow(
                    key: Key('device-${d.id}'),
                    label: '${d.name} (this device)',
                    sub: 'active now',
                    monoSub: true,
                    trailing: TextAction(
                      key: const Key('sign-out'),
                      label: 'Sign out',
                      onTap: () => ref.read(settingsProvider.notifier).logout(),
                    ),
                  )
                : SettingsRow(
                    key: Key('device-${d.id}'),
                    label: d.name,
                    sub: _deviceMeta(d),
                    monoSub: true,
                    trailing: TextAction(
                      key: Key('revoke-device-${d.id}'),
                      label: 'Revoke',
                      onTap: () => _revokeDevice(d),
                    ),
                  ),
        if (settings.hasSession)
          SettingsButtonRow(
            child: StormButton.outline(
              key: const Key('add-device'),
              label: '＋ Add a device',
              onPressed: () => context.push(Routes.addDevice),
            ),
          ),
        GroupLabel('Access keys', top: t.sp * 3.25),
        const SettingsNote(
          'For AI apps outside Storm, such as Claude on a laptop. A key acts '
          'as you, and is shown once.',
        ),
        if (keys != null)
          for (final k in keys)
            SettingsRow(
              key: Key('key-${k.id}'),
              label: k.name,
              sub: _keyMeta(k),
              monoSub: true,
              trailing: TextAction(
                key: Key('revoke-key-${k.id}'),
                label: 'Revoke',
                onTap: () => _revokeKey(k),
              ),
            ),
        if (keys != null && keys.isEmpty) const SettingsMuted('No keys yet.'),
        SettingsButtonRow(
          child: StormButton.outline(
            key: const Key('new-key'),
            label: '＋ New key',
            onPressed: settings.hasSession ? _newKey : null,
          ),
        ),
      ],
    );
  }
}

List<PairedDevice> _thisDeviceFirst(List<PairedDevice> all, String mine) => [
  ...all.where((d) => d.id == mine),
  ...all.where((d) => d.id != mine),
];

String _deviceMeta(PairedDevice d) {
  final seen = DateTime.tryParse(d.lastSeen ?? '');
  if (seen == null) return 'paired ${_day(d.paired)}';
  final ago = relativeTime(seen);
  return ago == 'just now' ? 'active now' : 'active $ago';
}

String _keyMeta(McpKey k) {
  final used = DateTime.tryParse(k.lastUsed ?? '');
  final parts = [
    used == null ? 'never used' : 'used ${relativeTime(used)}',
    if (k.expires != null) 'expires ${_day(k.expires!)}',
  ];
  return parts.join(' · ');
}

String _day(String iso) =>
    DateTime.tryParse(iso)?.toLocal().toString().split(' ').first ?? iso;

/// The one-time reveal.
///
/// Deliberately hard to dismiss by accident: no barrier dismiss, and the only
/// way out says what it means. The server keeps a hash, so "show it again" is
/// not a feature anyone can build.
class RevealMcpKeyDialog extends ConsumerWidget {
  const RevealMcpKeyDialog({super.key, required this.created});

  final CreatedMcpKey created;

  /// A ready-to-paste MCP client entry; assembling it by hand is where keys
  /// get mangled.
  static String configSnippet(String baseUrl, String secret) =>
      '{\n'
      '  "mcpServers": {\n'
      '    "storm": {\n'
      '      "type": "http",\n'
      '      "url": "$baseUrl/mcp",\n'
      '      "headers": {\n'
      '        "Authorization": "Bearer $secret"\n'
      '      }\n'
      '    }\n'
      '  }\n'
      '}';

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final t = context.tokens;
    final baseUrl = ref.read(settingsProvider).value?.baseUrl ?? '';
    final snippet = configSnippet(baseUrl, created.secret);

    return AlertDialog(
      title: const Text('Copy it now'),
      content: SingleChildScrollView(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text(
              'This is the only time this key is shown. Storm keeps only a '
              'hash of it, so it cannot be shown again — if you lose it, '
              'revoke this key and make another.',
              style: TextStyle(color: t.amber),
            ),
            SizedBox(height: t.sp * 1.5),
            _Copyable(label: 'Key', value: created.secret),
            SizedBox(height: t.sp * 1.5),
            Text(
              'Or paste this straight into your AI app’s MCP config:',
              style: TextStyle(fontSize: t.labelSize, color: t.text3),
            ),
            SizedBox(height: t.sp * 0.5),
            _Copyable(label: 'Config', value: snippet),
          ],
        ),
      ),
      actions: [
        FilledButton(
          key: const Key('reveal-done'),
          onPressed: () => Navigator.of(context).pop(),
          child: const Text("I've copied it"),
        ),
      ],
    );
  }
}

class _Copyable extends StatelessWidget {
  const _Copyable({required this.label, required this.value});

  final String label;
  final String value;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Container(
          width: double.infinity,
          padding: EdgeInsets.all(t.sp),
          decoration: BoxDecoration(
            color: t.codePlate,
            borderRadius: BorderRadius.circular(t.rControl),
          ),
          child: SelectableText(
            value,
            style: TextStyle(
              fontFamily: StormTokens.monoFamily,
              fontSize: t.codeSize,
              color: t.onCodePlate,
            ),
          ),
        ),
        Align(
          alignment: Alignment.centerRight,
          child: TextButton.icon(
            key: Key('copy-${label.toLowerCase()}'),
            // No `await` before the copy: the web's `execCommand` fallback
            // only works inside the click that triggered it.
            onPressed: () => copyToClipboard(value).then((copied) {
              if (!context.mounted) return;
              ScaffoldMessenger.of(context).showSnackBar(
                SnackBar(
                  content: Text(
                    copied
                        ? '$label copied'
                        : "Couldn't copy — select the $label above and copy "
                              'it by hand',
                  ),
                ),
              );
            }),
            icon: const Icon(Icons.copy, size: 16),
            label: Text('Copy $label'),
          ),
        ),
      ],
    );
  }
}
