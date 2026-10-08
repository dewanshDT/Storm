import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../api/storm_connection.dart' show CandidateTier;
import '../../state/app_state.dart';
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

/// The first bytes of the pinned key, as `4F:2A:91…`; null if none is pinned.
String? keyFingerprint(String publicKey) {
  if (publicKey.isEmpty) return null;
  try {
    final padded = publicKey.padRight((publicKey.length + 3) ~/ 4 * 4, '=');
    final bytes = base64Url.decode(padded);
    return '${bytes.take(3).map((b) => b.toRadixString(16).padLeft(2, '0').toUpperCase()).join(':')}…';
  } catch (_) {
    return null;
  }
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
        GroupLabel('Relays', top: t.sp * 3),
        const SettingsNote(
          "Used when this device can't reach the server directly. A relay "
          "can't read your notes.",
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

  Future<void> _add(
    BuildContext context,
    WidgetRef ref,
    List<String> current,
  ) async {
    final raw = await promptForText(
      context,
      title: 'Add relay',
      hint: 'wss://relay.example.net',
      help: 'The relay’s address. It carries traffic and cannot read it.',
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
