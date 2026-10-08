import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';

import 'package:flutter_lucide/flutter_lucide.dart';

import '../../agent/agent_state.dart';
import '../../router.dart';
import '../../state/app_state.dart';
import '../../state/client_version.dart';
import '../../state/health.dart';
import '../../state/nav_memory.dart';
import '../../sync/sync_engine.dart';
import '../accents.dart';
import '../controls.dart';
import '../widgets.dart';
import '../surfaces.dart';
import '../theme.dart';
import '../tokens.dart';
import 'vault_tile.dart';

/// Top-left on a phone: where you are (the vault, or Agents) and the place
/// picker (handoff §2.5).
class PlacesBubble extends ConsumerStatefulWidget {
  const PlacesBubble({super.key});

  @override
  ConsumerState<PlacesBubble> createState() => _PlacesBubbleState();
}

class _PlacesBubbleState extends ConsumerState<PlacesBubble> {
  final _anchor = GlobalKey();

  DotStatus _sync(SyncEngine engine) => dotStatusFor(
    online: engine.isOnline,
    identityFailed: engine.serverIdentityFailed,
    syncing: engine.isSyncing,
    pending: engine.pendingCount,
    tier: engine.connectionTier,
  );

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final inAgents =
        activityOf(GoRouterState.of(context).uri.path) == Activity.agents;
    final engine = ref.watch(syncEngineProvider);
    final live = ref.watch(agentOverviewProvider).value?.live.length ?? 0;
    final activeId = ref.watch(activeVaultProvider);
    final vault = (ref.watch(vaultsProvider).value ?? const [])
        .where((v) => v.id == activeId)
        .firstOrNull;
    final accent =
        ref.watch(vaultAccentsProvider).value?[activeId] ?? Accent.none;

    return Stack(
      key: _anchor,
      clipBehavior: Clip.none,
      children: [
        StormBubble(
          key: const Key('places-bubble'),
          tooltip: 'Places',
          onTap: _open,
          child: inAgents
              ? Icon(
                  LucideIcons.square_terminal,
                  size: t.sp * 2.5,
                  color: t.text2,
                )
              : VaultTile(
                  name: vault?.name ?? '',
                  accent: accent,
                  size: t.sp * 3.75,
                ),
        ),
        Positioned(
          top: -t.sp * 0.375,
          right: -t.sp * 0.375,
          child: inAgents
              ? Container(
                  width: t.sp * 1.25,
                  height: t.sp * 1.25,
                  decoration: BoxDecoration(
                    shape: BoxShape.circle,
                    color: live > 0 ? t.accent : t.text3,
                    border: Border.all(color: t.bg, width: t.bw * 2),
                  ),
                )
              : StatusDot(status: _sync(engine), size: t.sp * 1.25, ring: t.bg),
        ),
      ],
    );
  }

  Future<void> _open() async {
    final t = context.tokens;
    final inAgents =
        activityOf(GoRouterState.of(context).uri.path) == Activity.agents;
    await showStormPopover<void>(
      context: context,
      anchorKey: _anchor,
      width: t.sp * 31.25,
      builder: (popContext) => PlacePicker(
        inAgents: inAgents,
        onVault: (id) {
          Navigator.pop(popContext);
          context.go(Routes.browse(id));
        },
        onAgents: () {
          Navigator.pop(popContext);
          context.go(ref.read(navMemoryProvider).entryOf(Activity.agents));
        },
      ),
    );
  }
}

class PlacePicker extends ConsumerWidget {
  const PlacePicker({
    super.key,
    required this.inAgents,
    required this.onVault,
    required this.onAgents,
  });

  final bool inAgents;
  final ValueChanged<String> onVault;
  final VoidCallback onAgents;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final t = context.tokens;
    final engine = ref.watch(syncEngineProvider);
    final activeId = ref.watch(activeVaultProvider);
    final vaults = ref.watch(vaultsProvider).value ?? const [];
    final accents = ref.watch(vaultAccentsProvider).value ?? const {};
    final live = ref.watch(agentOverviewProvider).value?.live.length ?? 0;
    final status = dotStatusFor(
      online: engine.isOnline,
      identityFailed: engine.serverIdentityFailed,
      syncing: engine.isSyncing,
      pending: engine.pendingCount,
      tier: engine.connectionTier,
    );
    final check = Icon(LucideIcons.check, size: t.uiSize, color: t.accent);

    Widget row({
      required Key key,
      required Widget lead,
      required String label,
      Widget? meta,
      bool selected = false,
      VoidCallback? onTap,
    }) => InkWell(
      key: key,
      onTap: onTap,
      borderRadius: BorderRadius.circular(t.rControl),
      child: Padding(
        padding: EdgeInsets.symmetric(
          horizontal: t.sp * 0.75,
          vertical: t.sp * 1.125,
        ),
        child: Row(
          children: [
            lead,
            SizedBox(width: t.sp * 1.5),
            Expanded(
              child: Text(
                label,
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: TextStyle(
                  fontFamily: StormTokens.sansFamily,
                  fontSize: t.uiSize,
                  color: onTap == null ? t.text3 : t.text,
                ),
              ),
            ),
            ?meta,
            if (selected) ...[SizedBox(width: t.sp), check],
          ],
        ),
      ),
    );

    return StormPopover(
      children: [
        Padding(
          padding: EdgeInsets.fromLTRB(t.sp * 0.75, t.sp * 0.5, 0, t.sp * 0.5),
          child: const SectionLabel('Notes'),
        ),
        for (final v in vaults)
          row(
            key: Key('place-vault-${v.id}'),
            lead: VaultTile(
              name: v.name,
              accent: accents[v.id] ?? Accent.none,
              size: t.sp * 3.5,
            ),
            label: v.name,
            selected: !inAgents && v.id == activeId,
            onTap: v.missing ? null : () => onVault(v.id),
          ),
        const PopoverDivider(),
        row(
          key: const Key('place-agents'),
          lead: Container(
            width: t.sp * 3.5,
            height: t.sp * 3.5,
            decoration: BoxDecoration(
              color: t.surface,
              borderRadius: BorderRadius.circular(t.rControl * 0.8),
            ),
            child: Icon(
              LucideIcons.square_terminal,
              size: t.uiSize,
              color: t.text2,
            ),
          ),
          label: 'Agents',
          meta: live > 0
              ? Text(
                  '$live running',
                  style: TextStyle(
                    fontFamily: StormTokens.monoFamily,
                    fontSize: t.labelSize,
                    color: t.accent,
                  ),
                )
              : null,
          selected: inAgents,
          onTap: onAgents,
        ),
        const PopoverDivider(),
        Padding(
          padding: EdgeInsets.symmetric(
            horizontal: t.sp * 0.75,
            vertical: t.sp * 0.5,
          ),
          child: Row(
            children: [
              Flexible(
                child: Text(
                  syncLine(
                    status,
                    engine.pendingCount,
                    engine.lastSyncedAt,
                  ).replaceFirst('Notes synced', 'Synced'),
                  overflow: TextOverflow.ellipsis,
                  style: TextStyle(
                    fontFamily: StormTokens.monoFamily,
                    fontSize: t.labelSize,
                    color: t.text3,
                  ),
                ),
              ),
              Text(
                ' · ',
                style: TextStyle(
                  fontFamily: StormTokens.monoFamily,
                  fontSize: t.labelSize,
                  color: t.text3,
                ),
              ),
              GestureDetector(
                key: const Key('place-sync-now'),
                onTap: () async {
                  await ref.read(syncEngineProvider).sync();
                  ref.invalidate(treeProvider);
                },
                child: Text(
                  'Sync now',
                  style: TextStyle(
                    fontFamily: StormTokens.monoFamily,
                    fontSize: t.labelSize,
                    color: t.text2,
                  ),
                ),
              ),
            ],
          ),
        ),
      ],
    );
  }
}

String hostOf(String url) =>
    Uri.tryParse(url)?.host ?? url.replaceAll(RegExp(r'^https?://'), '');

/// Top-right on a phone: the way into Settings, lit while you are there.
class SettingsBubble extends StatelessWidget {
  const SettingsBubble({super.key});

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final here =
        activityOf(GoRouterState.of(context).uri.path) == Activity.settings;
    return StormBubble(
      key: const Key('settings-bubble'),
      tooltip: 'Settings',
      borderColor: here ? t.accent : null,
      onTap: () => context.go(Routes.settings),
      child: Icon(
        LucideIcons.settings_2,
        size: t.sp * 2.5,
        color: here ? t.accent : t.text2,
      ),
    );
  }
}

/// The controls themselves, without a container.
///
/// Two presentations: the phone's corner popover and, at desk width, a page in
/// the pane beside the sidebar. A popover anchored to the sidebar's footer
/// gear would open below the bottom of the window, which is how that button
/// came to look like it did nothing.
///
/// Grouped like Server settings — section labels and spacing, not cards — so
/// Appearance / Notes / Connection / About read as one list of facts rather
/// than a flat pile of switches.
class ClientSettingsBody extends ConsumerWidget {
  const ClientSettingsBody({super.key, this.onDone});

  /// Dismisses the surface this is sitting in, where there is one.
  final VoidCallback? onDone;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final t = context.tokens;
    final settings = ref.watch(settingsProvider).value ?? const Settings();
    final notifier = ref.read(settingsProvider.notifier);
    final version = ref.watch(clientVersionProvider).value;

    return Column(
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        _ClientSection(
          label: 'Appearance',
          first: true,
          children: [
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
                    onTap: () =>
                        notifier.save(settings.copyWith(theme: preset)),
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
            SizedBox(height: t.sp * 0.75),
            // Field label, not a peer SectionLabel — "Note font" is one control
            // under Appearance, not a second top-level group.
            const _FieldLabel('Note font'),
            SizedBox(height: t.sp * 0.5),
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
                    onTap: () =>
                        notifier.save(settings.copyWith(bodyFont: font)),
                  ),
              ],
            ),
          ],
        ),
        _ClientSection(
          label: 'Notes',
          children: [
            PopoverItem(
              label: 'Read mode',
              subtitle: 'Document view with a Read / Edit switch',
              trailing: StormToggle(
                key: const Key('setting-read-mode'),
                value: settings.readMode,
                onChanged: (v) => notifier.save(settings.copyWith(readMode: v)),
              ),
            ),
            PopoverItem(
              label: 'Show note id',
              subtitle: 'UUID in the properties strip',
              trailing: StormToggle(
                value: settings.showNoteId,
                onChanged: (v) =>
                    notifier.save(settings.copyWith(showNoteId: v)),
              ),
            ),
          ],
        ),
        _ClientSection(
          label: 'Connection',
          children: [
            // Only for a session. A legacy token install has nothing to sign
            // out *of* — there is no account behind it — and offering the
            // action would leave it at a login screen it cannot satisfy.
            // Session-only for the same reason as sign-out: minting a pairing
            // invite is `POST /v1/pairings`, session tier. A legacy-token
            // install has no account to vouch with.
            if (settings.hasSession)
              PopoverItem(
                key: const Key('add-device'),
                label: 'Add a device',
                subtitle: 'Show a QR for a phone or laptop to scan',
                onTap: () {
                  onDone?.call();
                  context.push(Routes.addDevice);
                },
              ),
            if (settings.hasSession)
              PopoverItem(
                key: const Key('sign-out'),
                label: 'Sign out',
                subtitle: 'Stay paired, ask for the password again',
                tone: PopoverTone.muted,
                onTap: () {
                  onDone?.call();
                  notifier.logout();
                },
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
        ),
        // Release builds stamp --build-name from the tag; without this line a
        // web hard-refresh is the only way to tell whether the service worker
        // still has yesterday's bundle.
        if (version != null)
          _ClientSection(
            label: 'About',
            children: [
              Padding(
                padding: EdgeInsets.symmetric(horizontal: t.sp * 0.75),
                child: Text(
                  'Version $version',
                  key: const Key('client-version'),
                  style: TextStyle(
                    fontFamily: StormTokens.monoFamily,
                    fontSize: t.labelSize,
                    color: t.text3,
                  ),
                ),
              ),
            ],
          ),
      ],
    );
  }
}

/// A top-level group on Client settings — same job as Server settings'
/// `_Section`, kept local so the popover and the desk page stay in lockstep.
class _ClientSection extends StatelessWidget {
  const _ClientSection({
    required this.label,
    required this.children,
    this.first = false,
  });

  final String label;
  final List<Widget> children;
  final bool first;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Column(
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Padding(
          padding: EdgeInsets.only(
            left: t.sp * 0.75,
            top: first ? 0 : t.sp * 1.75,
            bottom: t.sp * 0.75,
          ),
          child: SectionLabel(label),
        ),
        ...children,
      ],
    );
  }
}

/// Quiet label for a control inside a section (matches the Text size row).
class _FieldLabel extends StatelessWidget {
  const _FieldLabel(this.text);

  final String text;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Padding(
      padding: EdgeInsets.symmetric(horizontal: t.sp * 0.75),
      child: Text(
        text,
        style: TextStyle(
          fontFamily: StormTokens.sansFamily,
          fontSize: t.codeSize,
          color: t.text,
        ),
      ),
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
