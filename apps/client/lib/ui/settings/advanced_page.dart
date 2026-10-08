import 'package:flutter/material.dart';
import 'package:flutter_lucide/flutter_lucide.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../state/app_state.dart';
import '../../state/client_version.dart';
import '../controls.dart';
import '../pairing_screen.dart';
import '../tokens.dart';
import 'connection_page.dart' show serverAddress;
import 'settings_widgets.dart';

/// Settings › Advanced: details for troubleshooting and for other tools. The
/// one page that says "MCP".
class AdvancedPage extends ConsumerWidget {
  const AdvancedPage({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final t = context.tokens;
    final settings = ref.watch(settingsProvider).value ?? const Settings();
    final client = ref.watch(clientVersionProvider).value;
    final server = ref.watch(serverConfigProvider).value?.version;
    final versions = [?client, ?server].join(' · ');

    return SettingsPage(
      title: 'Advanced',
      intro: 'Details for troubleshooting and for connecting other tools.',
      children: [
        SettingsRow(
          label: 'MCP endpoint',
          sub: 'For AI apps that connect with an access key.',
          trailing: SelectableText(
            '${serverAddress(settings.baseUrl)}/mcp',
            key: const Key('mcp-endpoint'),
            style: TextStyle(
              fontFamily: StormTokens.monoFamily,
              fontSize: t.labelSize * 1.09,
              color: t.text2,
            ),
          ),
        ),
        SettingsRow(
          label: 'Versions',
          // The server reports its version from slice 5 on; before that only
          // the client's is known.
          sub: server == null
              ? 'Client. This server does not say.'
              : 'Client and server.',
          trailing: MonoValue(versions, key: const Key('versions')),
        ),
        SettingsRow(
          key: const Key('re-pair'),
          label: 'Re-pair this device',
          sub: 'Scan a new pairing code without signing out.',
          trailing: Icon(
            LucideIcons.chevron_right,
            size: t.codeSize,
            color: t.text2,
          ),
          onTap: () => Navigator.of(context, rootNavigator: true).push(
            MaterialPageRoute<bool>(
              builder: (_) => const PairingScreen(rePair: true),
            ),
          ),
        ),
      ],
    );
  }
}
