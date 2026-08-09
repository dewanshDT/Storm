import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import 'package:go_router/go_router.dart';

import '../../router.dart';
import '../../state/app_state.dart';
import '../widgets.dart';
import '../surfaces.dart';
import '../theme.dart';
import '../tokens.dart';

/// Top-left: the vault, whether it is reaching the server, and the switcher.
///
/// Shows the *vault's* initial rather than the server host's, because with
/// several vaults in play the question the bubble answers is "which one am I
/// in".
class VaultBubble extends ConsumerStatefulWidget {
  const VaultBubble({super.key});

  @override
  ConsumerState<VaultBubble> createState() => _VaultBubbleState();
}

class _VaultBubbleState extends ConsumerState<VaultBubble> {
  final _anchor = GlobalKey();

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    // Watching the engine is correct *here*: this widget renders its status,
    // which is exactly what its notifications mean. Anything needing the
    // engine's identity still uses ref.read — see app_state.dart.
    final engine = ref.watch(syncEngineProvider);
    final settings = ref.watch(settingsProvider).value;
    final host = hostOf(settings?.baseUrl ?? '');
    final activeId = ref.watch(activeVaultProvider);
    final vaults = ref.watch(vaultsProvider).value ?? const [];
    final active = vaults.where((v) => v.id == activeId).firstOrNull;
    final label = active?.name ?? '';

    final status = dotStatusFor(
      online: engine.isOnline,
      syncing: engine.isSyncing,
      pending: engine.pendingCount,
    );

    return Stack(
      key: _anchor,
      clipBehavior: Clip.none,
      children: [
        StormBubble(
          tooltip: label.isEmpty ? 'Vault' : label,
          onTap: () => _open(host),
          child: Text(
            (label.isEmpty ? host : label).characters.firstOrNull
                    ?.toUpperCase() ??
                'S',
            style: TextStyle(
              fontFamily: StormTokens.monoFamily,
              fontWeight: FontWeight.w500,
              fontSize: t.codeSize,
              color: t.text,
            ),
          ),
        ),
        Positioned(
          right: t.sp * 0.5,
          bottom: t.sp * 0.5,
          child: StatusDot(status: status, size: t.sp * 1.25, ring: t.surface),
        ),
      ],
    );
  }

  Future<void> _open(String host) async {
    final t = context.tokens;
    await showStormPopover<void>(
      context: context,
      anchorKey: _anchor,
      width: t.sp * 30,
      builder: (popContext) => Consumer(
        builder: (popContext, ref, _) {
          final engine = ref.watch(syncEngineProvider);
          final activeId = ref.watch(activeVaultProvider);
          final vaults = ref.watch(vaultsProvider).value ?? const [];
          final status = dotStatusFor(
            online: engine.isOnline,
            syncing: engine.isSyncing,
            pending: engine.pendingCount,
          );

          return StormPopover(
            children: [
              Padding(
                padding: EdgeInsets.only(
                  left: t.sp * 0.75,
                  bottom: t.sp * 0.75,
                ),
                child: const SectionLabel('Vaults'),
              ),
              // The switcher sits above the status, because switching is what
              // this bubble is now mostly for.
              for (final v in vaults)
                PopoverItem(
                  label: v.name,
                  subtitle: v.missing
                      ? 'Directory not found'
                      : '${v.noteCount} note${v.noteCount == 1 ? '' : 's'}',
                  selected: v.id == activeId,
                  leading: StatusDot(
                    status: v.missing
                        ? DotStatus.offline
                        : v.id == activeId
                        ? status
                        : DotStatus.offline,
                  ),
                  onTap: v.missing
                      ? null
                      : () {
                          Navigator.pop(popContext);
                          if (v.id == activeId) return;
                          // `go`, not `push`: switching vaults replaces where
                          // you are rather than stacking one on the other.
                          context.go(Routes.browse(v.id));
                        },
                ),
              if (vaults.isNotEmpty) const PopoverDivider(),
              Padding(
                padding: EdgeInsets.symmetric(horizontal: t.sp * 0.75),
                child: Text(
                  _syncLine(status, engine.pendingCount, engine.lastSyncedAt),
                  style: TextStyle(
                    fontFamily: StormTokens.sansFamily,
                    fontSize: t.labelSize,
                    color: t.text3,
                  ),
                ),
              ),
              Padding(
                padding: EdgeInsets.symmetric(horizontal: t.sp * 0.75),
                child: Text(
                  host.isEmpty ? 'no server set' : host,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: TextStyle(
                    fontFamily: StormTokens.monoFamily,
                    fontSize: t.labelSize,
                    color: t.text3,
                  ),
                ),
              ),
              SizedBox(height: t.sp * 0.5),
              PopoverItem(
                label: 'Sync now',
                tone: PopoverTone.accent,
                onTap: () async {
                  Navigator.pop(popContext);
                  await ref.read(syncEngineProvider).sync();
                  ref.invalidate(treeProvider);
                },
              ),
              PopoverItem(
                label: 'Server settings ›',
                tone: PopoverTone.accent,
                onTap: () {
                  Navigator.pop(popContext);
                  context.push(Routes.serverSettings);
                },
              ),
            ],
          );
        },
      ),
    );
  }
}

String _syncLine(DotStatus status, int pending, DateTime? lastSynced) =>
    switch (status) {
      DotStatus.synced => 'Synced ${relativeTime(lastSynced)}',
      DotStatus.syncing =>
        pending > 0
            ? 'Sending $pending edit${pending == 1 ? '' : 's'}…'
            : 'Syncing…',
      DotStatus.offline =>
        pending > 0
            ? 'Offline · $pending edit${pending == 1 ? '' : 's'} queued'
            : 'Offline · showing your cached copy',
    };

String hostOf(String url) =>
    Uri.tryParse(url)?.host ?? url.replaceAll(RegExp(r'^https?://'), '');

/// Top-right: settings today, account when multi-user ships.
class SettingsBubble extends ConsumerStatefulWidget {
  const SettingsBubble({super.key});

  @override
  ConsumerState<SettingsBubble> createState() => _SettingsBubbleState();
}

class _SettingsBubbleState extends ConsumerState<SettingsBubble> {
  final _anchor = GlobalKey();

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return StormBubble(
      key: _anchor,
      tooltip: 'Settings',
      onTap: _open,
      child: Text(
        'A',
        style: TextStyle(
          fontFamily: StormTokens.sansFamily,
          fontWeight: FontWeight.w600,
          fontSize: t.codeSize,
          color: t.text,
        ),
      ),
    );
  }

  Future<void> _open() async {
    final t = context.tokens;
    await showStormPopover<void>(
      context: context,
      anchorKey: _anchor,
      width: t.sp * 33,
      alignRight: true,
      builder: (popContext) =>
          AppearanceMenu(onClose: () => Navigator.pop(popContext)),
    );
  }
}

/// Theme, text size, note font, and the way out.
///
/// Public because it has two homes: the phone's top-right corner bubble, and
/// the wide sidebar's footer gear — the corners are empty above 900px, and
/// without a second entry these settings would be unreachable there.
class AppearanceMenu extends StatelessWidget {
  const AppearanceMenu({super.key, required this.onClose});

  final VoidCallback onClose;

  @override
  Widget build(BuildContext context) =>
      StormPopover(children: [ClientSettingsBody(onDone: onClose)]);
}

/// The controls themselves, without a container.
///
/// Two presentations: the phone's corner popover and, at desk width, a page in
/// the pane beside the sidebar. A popover anchored to the sidebar's footer
/// gear would open below the bottom of the window, which is how that button
/// came to look like it did nothing.
class ClientSettingsBody extends ConsumerWidget {
  const ClientSettingsBody({super.key, this.onDone});

  /// Dismisses the surface this is sitting in, where there is one.
  final VoidCallback? onDone;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final t = context.tokens;
    final settings = ref.watch(settingsProvider).value ?? const Settings();
    final notifier = ref.read(settingsProvider.notifier);

    return Column(
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Padding(
          padding: EdgeInsets.only(left: t.sp * 0.75, bottom: t.sp * 0.75),
          child: const SectionLabel('Appearance'),
        ),
        // A switch could only ever say two things, and there are three
        // identities to choose between.
        Wrap(
          spacing: t.sp * 0.5,
          runSpacing: t.sp * 0.5,
          children: [
            for (final preset in StormPreset.values)
              _Choice(
                label: preset.label,
                selected: settings.theme == preset,
                onTap: () => notifier.save(settings.copyWith(theme: preset)),
              ),
          ],
        ),
        SizedBox(height: t.sp),
        _SliderRow(
          label: 'Text size',
          value: settings.fontSize,
          display: '${settings.fontSize.round()}px',
          onChanged: (v) => notifier.save(settings.copyWith(fontSize: v)),
        ),
        SizedBox(height: t.sp * 0.5),
        Padding(
          padding: EdgeInsets.only(left: t.sp * 0.75, bottom: t.sp * 0.5),
          child: const SectionLabel('Note font'),
        ),
        Wrap(
          spacing: t.sp * 0.5,
          runSpacing: t.sp * 0.5,
          children: [
            for (final font in BodyFont.values)
              _Choice(
                label: font.label,
                // Each option is set in the face it names, so the choice
                // shows what it will do.
                family: font.family,
                selected: settings.bodyFont == font,
                onTap: () => notifier.save(settings.copyWith(bodyFont: font)),
              ),
          ],
        ),
        const PopoverDivider(),
        PopoverItem(
          label: 'Show note id',
          trailing: StormSwitch(
            value: settings.showNoteId,
            onChanged: (v) => notifier.save(settings.copyWith(showNoteId: v)),
          ),
        ),
        PopoverItem(
          label: 'Disconnect',
          subtitle: 'Forget this server and its token',
          tone: PopoverTone.muted,
          onTap: () {
            onDone?.call();
            notifier.save(const Settings());
          },
        ),
      ],
    );
  }
}

class _Choice extends StatelessWidget {
  const _Choice({
    required this.label,
    required this.selected,
    required this.onTap,
    this.family,
  });

  final String label;
  final bool selected;
  final VoidCallback onTap;
  final String? family;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return InkWell(
      onTap: onTap,
      borderRadius: BorderRadius.circular(t.rControl * 0.6),
      child: Container(
        padding: EdgeInsets.symmetric(
          horizontal: t.sp * 0.875,
          vertical: t.sp * 0.5,
        ),
        decoration: BoxDecoration(
          color: selected ? t.accentSoft : t.surface,
          borderRadius: BorderRadius.circular(t.rControl * 0.6),
          border: Border.all(
            color: selected ? t.accent : t.border,
            width: t.bw,
          ),
        ),
        child: Text(
          label,
          style: TextStyle(
            fontFamily: family ?? StormTokens.sansFamily,
            fontSize: t.labelSize,
            color: selected ? t.accent : t.text2,
          ),
        ),
      ),
    );
  }
}

class _SliderRow extends StatelessWidget {
  const _SliderRow({
    required this.label,
    required this.value,
    required this.display,
    required this.onChanged,
  });

  final String label;
  final double value;
  final String display;
  final ValueChanged<double> onChanged;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Padding(
          padding: EdgeInsets.symmetric(horizontal: t.sp * 0.75),
          child: Row(
            children: [
              Expanded(
                child: Text(
                  label,
                  style: TextStyle(
                    fontFamily: StormTokens.sansFamily,
                    fontSize: t.codeSize,
                    color: t.text,
                  ),
                ),
              ),
              Text(
                display,
                style: TextStyle(
                  fontFamily: StormTokens.monoFamily,
                  fontSize: t.labelSize,
                  color: t.text3,
                ),
              ),
            ],
          ),
        ),
        SliderTheme(
          data: SliderThemeData(
            trackHeight: t.sp * 0.25,
            overlayShape: SliderComponentShape.noOverlay,
            thumbShape: RoundSliderThumbShape(enabledThumbRadius: t.sp * 0.75),
          ),
          child: Slider(
            value: value,
            min: 12,
            max: 24,
            divisions: 12,
            onChanged: onChanged,
          ),
        ),
      ],
    );
  }
}

/// "3 minutes ago", for the places that show a timestamp.
String relativeTime(DateTime? then) {
  if (then == null) return 'not yet';
  final d = DateTime.now().difference(then);
  if (d.inSeconds < 45) return 'just now';
  if (d.inMinutes < 60) return '${d.inMinutes}m ago';
  if (d.inHours < 24) return '${d.inHours}h ago';
  return '${d.inDays}d ago';
}

/// "2m" — the same age as [relativeTime] with the words taken out, for the
/// dashboard's stat blocks.
String compactAge(DateTime? then) {
  if (then == null) return '—';
  final d = DateTime.now().difference(then);
  if (d.inSeconds < 45) return 'now';
  if (d.inMinutes < 60) return '${d.inMinutes}m';
  if (d.inHours < 24) return '${d.inHours}h';
  return '${d.inDays}d';
}
