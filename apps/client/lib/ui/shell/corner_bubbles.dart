import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';

import 'package:flutter_lucide/flutter_lucide.dart';

import '../../agent/agent_state.dart';
import '../../router.dart';
import '../../state/app_state.dart';
import '../../state/health.dart';
import '../../state/nav_memory.dart';
import '../../sync/sync_engine.dart';
import '../accents.dart';
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
