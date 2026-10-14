import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../api/connection_log.dart';
import '../../api/lan_discovery.dart' show keyFingerprint;
import '../../api/models.dart' show DiscoveryConfig;
import '../../api/storm_connection.dart' show CandidateTier;
import '../../state/app_state.dart';
import '../clipboard_copy.dart';
import '../controls.dart';
import '../states.dart' show describeFailure;
import '../tokens.dart';
import 'settings_widgets.dart';

/// `http://storm.home:7420` → `storm.home:7420`.
String serverAddress(String baseUrl) {
  final uri = Uri.tryParse(baseUrl);
  if (uri == null || uri.host.isEmpty) {
    return baseUrl.replaceAll(RegExp(r'^https?://'), '');
  }
  return uri.hasPort ? '${uri.host}:${uri.port}' : uri.host;
}

String _relayHost(String url) => url.replaceFirst(RegExp(r'^wss?://'), '');

/// Settings › Connection: how this device reaches the server (device-level)
/// and the relays the server registers with (server-level).
class ConnectionPage extends ConsumerWidget {
  const ConnectionPage({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final t = context.tokens;
    final settings = ref.watch(settingsProvider).value ?? const Settings();
    final engine = ref.watch(syncEngineProvider);
    final config = ref.watch(serverConfigProvider);

    final route = !engine.isOnline
        ? 'offline'
        : engine.connectionTier == CandidateTier.direct
        ? 'direct'
        : 'relayed';
    final fingerprint = keyFingerprint(settings.serverPublicKey);
    final identity = engine.serverIdentityFailed
        ? 'failed'
        : fingerprint == null
        ? 'not pinned'
        : 'verified · $fingerprint';

    return SettingsPage(
      title: 'Connection',
      intro: 'How this device reaches your Storm.',
      children: [
        SettingsRow(
          label: 'Server address',
          trailing: MonoValue(
            serverAddress(settings.baseUrl),
            key: const Key('server-address'),
          ),
        ),
        SettingsRow(
          label: 'Route',
          trailing: MonoValue(route, key: const Key('route')),
        ),
        SettingsRow(
          label: 'Server identity',
          trailing: MonoValue(
            identity,
            key: const Key('server-identity'),
            color: engine.serverIdentityFailed ? t.danger : null,
          ),
        ),
        ...config.when(
          loading: () => const <Widget>[],
          error: (_, _) => const <Widget>[],
          data: (c) => c?.discovery == null
              ? const <Widget>[]
              : [_discoverable(ref, c!.discovery!)],
        ),
        GroupLabel('Relays', top: t.sp * 3),
        const SettingsNote(
          'Another way to reach your server, tried alongside its address. '
          'Until Storm has end-to-end encryption, whoever runs a relay can '
          'read what passes through it, notes and sign-in included. Run your '
          'own relay or use one you trust.',
          key: Key('relays-note'),
        ),
        ...config.when(
          loading: () => [const SettingsMuted('Loading…')],
          error: (e, _) => [SettingsMuted(describeFailure(e), danger: true)],
          data: (c) => [
            for (final relay in c?.relays ?? const <String>[])
              SettingsRow(
                key: Key('relay-$relay'),
                label: _relayHost(relay),
                trailing: TextAction(
                  key: Key('remove-relay-$relay'),
                  label: 'Remove',
                  onTap: () =>
                      _setRelays(context, ref, [...c!.relays]..remove(relay)),
                ),
              ),
            if (c != null)
              SettingsButtonRow(
                child: StormButton.outline(
                  key: const Key('add-relay'),
                  label: '＋ Add relay',
                  onPressed: () => _add(context, ref, c.relays),
                ),
              ),
          ],
        ),
        GroupLabel('Diagnostics', top: t.sp * 3),
        SettingsRow(
          key: const Key('connection-log'),
          label: 'Connection log',
          sub:
              'Races, relays, resets and status changes since the app '
              'started. No notes, keys or tokens.',
          trailing: TextAction(
            key: const Key('copy-connection-log'),
            label: 'Copy',
            onTap: () => _copyLog(context),
          ),
        ),
        Padding(
          padding: EdgeInsets.only(top: t.sp * 1.5),
          child: Container(height: t.bw, color: t.border),
        ),
        SizedBox(height: t.sp),
        SettingsRow(
          key: const Key('disconnect'),
          label: 'Disconnect this device',
          sub:
              'Forgets this server and its sign-in. Your notes stay on the '
              'server.',
          divider: false,
          labelColor: t.danger,
          onTap: () => _disconnect(context, ref),
        ),
      ],
    );
  }

  /// The server's "Discoverable on this network" switch (decision 118).
  Widget _discoverable(WidgetRef ref, DiscoveryConfig d) => SettingsRow(
    key: const Key('discoverable'),
    label: 'Discoverable on this network',
    sub: d.forcedOff
        ? 'Turned off on the server (--no-discovery or STORM_DISCOVERY=0). '
              'Pair by QR.'
        : 'Lets devices on your Wi-Fi find this server and pair with your '
              'password. Turn off to pair by QR only.',
    muted: d.forcedOff,
    trailing: StormToggle(
      key: const Key('discoverable-toggle'),
      value: d.discoverable && !d.forcedOff,
      onChanged: d.forcedOff ? null : (on) => _setDiscoverable(ref, on),
    ),
  );

  Future<void> _setDiscoverable(WidgetRef ref, bool on) async {
    final api = ref.read(apiProvider);
    if (api == null) return;
    try {
      await api.setDiscoverable(on);
    } finally {
      ref.invalidate(serverConfigProvider);
    }
  }

  Future<void> _copyLog(BuildContext context) async {
    final log = ConnectionLog.instance;
    final copied = await copyToClipboard(log.toString());
    if (!context.mounted) return;
    settingsToast(
      context,
      copied
          ? 'Copied ${log.entries.length} lines'
          : "Couldn't copy the connection log",
    );
  }

  Future<void> _add(
    BuildContext context,
    WidgetRef ref,
    List<String> current,
  ) async {
    final raw = await promptForText(
      context,
      title: 'Add relay',
      hint: 'wss://relay.example.net',
      help:
          'The relay’s address (wss://). Whoever runs it can read the '
          'traffic it carries.',
    );
    if (raw == null || raw.trim().isEmpty || !context.mounted) return;
    final url = raw.trim().contains('://') ? raw.trim() : 'wss://${raw.trim()}';
    await _setRelays(context, ref, [...current, url]);
  }

  Future<void> _setRelays(
    BuildContext context,
    WidgetRef ref,
    List<String> relays,
  ) async {
    final api = ref.read(apiProvider);
    if (api == null) return;
    try {
      await api.setRelays(relays);
    } catch (e) {
      if (context.mounted) settingsToast(context, describeFailure(e));
    }
    ref.invalidate(serverConfigProvider);
  }

  Future<void> _disconnect(BuildContext context, WidgetRef ref) async {
    final ok = await confirmSetting(
      context,
      title: 'Disconnect this device?',
      body:
          'This device forgets the server and its sign-in. Your notes stay '
          'on the server; pair again to come back.',
      action: 'Disconnect',
      confirmKey: const Key('confirm-disconnect'),
    );
    if (!ok) return;
    await ref.read(settingsProvider.notifier).save(const Settings());
  }
}
