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
import 'relayed_on_lan.dart';
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
        RelayedOnLanHint(
          relayed: route == 'relayed',
          serverId: settings.serverId,
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
                // Configured is not reachable (SG-75): the server advertises
                // only relays it holds a trunk with, and that set is what the
                // app last adopted.
                sub: settings.relays.any((r) => r.url == relay)
                    ? null
                    : 'Not connected',
                trailing: TextAction(
                  key: Key('remove-relay-$relay'),
                  label: 'Remove',
                  onTap: () => _remove(context, ref, c!.relays, relay),
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

  Future<bool> _setRelays(
    BuildContext context,
    WidgetRef ref,
    List<String> relays,
  ) async {
    final api = ref.read(apiProvider);
    if (api == null) return false;
    var ok = true;
    try {
      await api.setRelays(relays);
    } catch (e) {
      ok = false;
      if (context.mounted) settingsToast(context, describeFailure(e));
    }
    ref.invalidate(serverConfigProvider);
    return ok;
  }

  /// Removes a relay, then offers to undo what it could have seen (SRP §7,
  /// decision 116j): a relay reads every token and password that crossed it,
  /// so leaving one behind is a reason to change the password and sign every
  /// other device out.
  Future<void> _remove(
    BuildContext context,
    WidgetRef ref,
    List<String> current,
    String relay,
  ) async {
    if (!await _setRelays(context, ref, [...current]..remove(relay))) return;
    if (!context.mounted) return;
    final go = await confirmSetting(
      context,
      title: 'Sign out everywhere and change your password?',
      body:
          '${_relayHost(relay)} could read the sign-ins and notes it carried. '
          'If you no longer trust it, change your password and sign out every '
          'other device; this one stays signed in.',
      action: 'Change password',
      confirmKey: const Key('confirm-after-relay'),
    );
    if (!go || !context.mounted) return;
    final now = await promptForText(
      context,
      title: 'Current password',
      hint: 'Current password',
      obscure: true,
    );
    if (now == null || now.isEmpty || !context.mounted) return;
    final next = await promptForText(
      context,
      title: 'New password',
      hint: 'New password',
      obscure: true,
    );
    if (next == null || next.isEmpty || !context.mounted) return;
    final api = ref.read(apiProvider);
    if (api == null) return;
    try {
      await api.changePassword(now, next);
      final me = ref.read(settingsProvider).value?.deviceId ?? '';
      var out = 0;
      for (final d in await api.devices()) {
        if (d.isRevoked || d.id == me) continue;
        await api.revokeDevice(d.id);
        out++;
      }
      if (context.mounted) {
        settingsToast(
          context,
          'Password changed; $out other device${out == 1 ? '' : 's'} signed out',
        );
      }
    } catch (e) {
      if (context.mounted) settingsToast(context, describeFailure(e));
    }
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
    // Revoke this device on the server first, so a disconnect does not leave
    // a row behind on Devices & access for every pair-again. Offline, it
    // still disconnects: forgetting the server must never depend on reaching
    // it.
    final deviceId = ref.read(settingsProvider).value?.deviceId ?? '';
    final api = ref.read(apiProvider);
    var revoked = deviceId.isEmpty;
    if (!revoked && api != null) {
      try {
        await api.revokeDevice(deviceId).timeout(const Duration(seconds: 10));
        revoked = true;
      } catch (_) {}
    }
    await ref.read(settingsProvider.notifier).save(const Settings());
    if (!revoked && context.mounted) {
      settingsToast(
        context,
        "Disconnected. The server couldn't be reached, so revoke this "
        'device from Devices & access on another one.',
      );
    }
  }
}
