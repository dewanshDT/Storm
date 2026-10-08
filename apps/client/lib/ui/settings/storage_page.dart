import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../api/models.dart';
import '../../state/app_state.dart';
import '../controls.dart';
import '../states.dart' show describeFailure;
import '../tokens.dart';
import 'settings_widgets.dart';

/// Settings › Storage: the vault storage root. Changing it never moves files;
/// it points the server at directories someone already moved.
class StoragePage extends ConsumerWidget {
  const StoragePage({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final t = context.tokens;
    final config = ref.watch(serverConfigProvider);

    return SettingsPage(
      title: 'Storage',
      intro: 'Where your vaults live on the server.',
      children: config.when(
        loading: () => [const SettingsMuted('Loading…')],
        error: (e, _) => [SettingsMuted(describeFailure(e), danger: true)],
        data: (c) => c == null
            ? [const SettingsMuted('Not connected.')]
            : [
                const GroupLabel('Vault storage root', bottom: 10),
                Container(
                  key: const Key('storage-root'),
                  padding: EdgeInsets.symmetric(
                    horizontal: t.sp * 1.5,
                    vertical: t.sp * 1.25,
                  ),
                  decoration: BoxDecoration(
                    color: t.surface2,
                    borderRadius: BorderRadius.circular(t.rControl),
                    border: Border.all(color: t.border, width: t.bw),
                  ),
                  child: SelectableText(
                    c.vaultRoot,
                    style: TextStyle(
                      fontFamily: StormTokens.monoFamily,
                      fontSize: t.uiSize,
                      color: t.text,
                    ),
                  ),
                ),
                SizedBox(height: t.sp * 1.25),
                SettingsNote(
                  '${c.vaultCount} vault${c.vaultCount == 1 ? '' : 's'} '
                  '${c.vaultCount == 1 ? 'lives' : 'live'} here. Changing the '
                  "root doesn't move any files. Storm refuses a change that "
                  'would leave vaults behind.',
                ),
                SettingsButtonRow(
                  top: t.sp * 1.25,
                  child: StormButton.outline(
                    key: const Key('change-root'),
                    label: 'Change…',
                    onPressed: () => _change(context, ref, c),
                  ),
                ),
              ],
      ),
    );
  }

  Future<void> _change(
    BuildContext context,
    WidgetRef ref,
    ServerConfig config,
  ) async {
    final path = await promptForText(
      context,
      title: 'Vault storage root',
      hint: '/srv/storm/vaults',
      initial: config.vaultRoot,
      help:
          'Changing this does not move your notes. Move the vault '
          'directories first, then change the root.',
    );
    if (path == null || path.trim().isEmpty || !context.mounted) return;
    final api = ref.read(apiProvider);
    if (api == null) return;

    try {
      await api.setVaultRoot(path.trim());
    } catch (e) {
      if (!context.mounted) return;
      // A 409 means none of the registered vaults were found there; only
      // proceed once the user has read what would be left behind.
      if (e is! StormApiException || e.statusCode != 409) {
        settingsToast(context, describeFailure(e));
        return;
      }
      final proceed = await confirmSetting(
        context,
        title: 'Leave those vaults behind?',
        body:
            '${e.message}\n\nNo files are deleted either way. Vaults left '
            'behind stay in the list, marked as missing.',
        action: 'Change anyway',
        confirmKey: const Key('confirm-orphan'),
      );
      if (!proceed) return;
      try {
        await api.setVaultRoot(path.trim(), orphanOk: true);
      } catch (e2) {
        if (context.mounted) settingsToast(context, describeFailure(e2));
        return;
      }
    }

    ref.invalidate(serverConfigProvider);
    ref.invalidate(vaultsProvider);
    ref.read(vaultRevisionProvider.notifier).state++;
  }
}
