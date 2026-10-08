import 'package:flutter/material.dart';
import 'package:flutter_lucide/flutter_lucide.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';

import '../../api/models.dart';
import '../../router.dart';
import '../../state/app_state.dart';
import '../../state/health.dart' show syncLine;
import '../../state/vault_config.dart' show setVaultAccent;
import '../../sync/sync_engine.dart';
import '../accents.dart';
import '../surfaces.dart';
import '../tokens.dart';
import '../widgets.dart';
import 'vault_tile.dart';

DotStatus engineStatus(SyncEngine engine) => dotStatusFor(
  online: engine.isOnline,
  identityFailed: engine.serverIdentityFailed,
  syncing: engine.isSyncing,
  pending: engine.pendingCount,
  tier: engine.connectionTier,
);

/// "Synced 2m ago", or what is wrong instead.
String placeSyncLine(SyncEngine engine) => syncLine(
  engineStatus(engine),
  engine.pendingCount,
  engine.lastSyncedAt,
).replaceFirst('Notes synced', 'Synced');

/// The desktop Notes sidebar's top row: which vault, how fresh, and the way
/// to another one (handoff §2.2).
class VaultHeader extends ConsumerStatefulWidget {
  const VaultHeader({super.key});

  @override
  ConsumerState<VaultHeader> createState() => _VaultHeaderState();
}

class _VaultHeaderState extends ConsumerState<VaultHeader> {
  final _anchor = GlobalKey();
  bool _open = false;

  Future<void> _toggle() async {
    final t = context.tokens;
    setState(() => _open = true);
    await showStormPopover<void>(
      context: context,
      anchorKey: _anchor,
      width: t.sp * 30,
      builder: (popContext) => VaultSwitcherPopover(
        onVault: (id) {
          Navigator.pop(popContext);
          if (id != ref.read(activeVaultProvider)) {
            context.go(Routes.browse(id));
          }
        },
        onManage: () {
          Navigator.pop(popContext);
          context.go(Routes.settingsPage('vaults'));
        },
      ),
    );
    if (mounted) setState(() => _open = false);
  }

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final engine = ref.watch(syncEngineProvider);
    final vaultId = ref.watch(activeVaultProvider);
    final vault = (ref.watch(vaultsProvider).value ?? const <VaultInfo>[])
        .where((v) => v.id == vaultId)
        .firstOrNull;
    final accent =
        ref.watch(vaultAccentsProvider).value?[vaultId] ?? Accent.none;
    final sync = placeSyncLine(engine);

    return InkWell(
      key: _anchor,
      onTap: _toggle,
      borderRadius: BorderRadius.circular(t.rControl),
      hoverColor: t.surface2,
      child: Padding(
        padding: EdgeInsets.symmetric(horizontal: t.sp, vertical: t.sp * 0.75),
        child: Row(
          children: [
            ExcludeSemantics(
              child: VaultTile(
                name: vault?.name ?? '',
                accent: accent,
                size: t.sp * 3.75,
              ),
            ),
            SizedBox(width: t.sp * 1.25),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                mainAxisSize: MainAxisSize.min,
                children: [
                  Text(
                    vault?.name ?? 'Storm',
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: TextStyle(
                      fontFamily: StormTokens.sansFamily,
                      fontSize: t.uiSize,
                      fontWeight: FontWeight.w600,
                      color: t.text,
                      height: 1.25,
                    ),
                  ),
                  Text(
                    sync.isEmpty
                        ? sync
                        : sync[0].toLowerCase() + sync.substring(1),
                    key: const Key('vault-sync-line'),
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: TextStyle(
                      fontFamily: StormTokens.monoFamily,
                      fontSize: t.labelSize,
                      color: t.text3,
                    ),
                  ),
                ],
              ),
            ),
            AnimatedRotation(
              turns: _open ? 0.5 : 0,
              duration: t.duration,
              child: Icon(
                LucideIcons.chevron_down,
                size: t.codeSize,
                color: t.text3,
              ),
            ),
          ],
        ),
      ),
    );
  }
}

/// Vaults with their note counts, the sync line and Manage vaults ›. A
/// long-press on a vault colours it (Q7).
class VaultSwitcherPopover extends ConsumerWidget {
  const VaultSwitcherPopover({
    super.key,
    required this.onVault,
    required this.onManage,
  });

  final ValueChanged<String> onVault;
  final VoidCallback onManage;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final t = context.tokens;
    final activeId = ref.watch(activeVaultProvider);
    final vaults = ref.watch(vaultsProvider).value ?? const <VaultInfo>[];
    final accents = ref.watch(vaultAccentsProvider).value ?? const {};

    return StormPopover(
      padding: t.sp,
      children: [
        for (final v in vaults)
          InkWell(
            key: Key('switcher-vault-${v.id}'),
            onTap: v.missing ? null : () => onVault(v.id),
            onLongPress: v.missing
                ? null
                : () => pickVaultAccent(
                    context,
                    ref,
                    v.id,
                    accents[v.id] ?? Accent.none,
                  ),
            hoverColor: t.surface,
            borderRadius: BorderRadius.circular(t.rControl),
            child: Padding(
              padding: EdgeInsets.all(t.sp),
              child: Row(
                children: [
                  ExcludeSemantics(
                    child: VaultTile(
                      name: v.name,
                      accent: accents[v.id] ?? Accent.none,
                      size: t.sp * 3.5,
                    ),
                  ),
                  SizedBox(width: t.sp * 1.25),
                  Expanded(
                    child: Text(
                      v.name,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: TextStyle(
                        fontFamily: StormTokens.sansFamily,
                        fontSize: t.uiSize,
                        color: v.missing ? t.text3 : t.text,
                      ),
                    ),
                  ),
                  Text(
                    v.missing ? 'missing' : '${v.noteCount}',
                    style: TextStyle(
                      fontFamily: StormTokens.monoFamily,
                      fontSize: t.labelSize,
                      color: t.text3,
                    ),
                  ),
                  SizedBox(
                    width: t.sp * 3,
                    child: v.id == activeId
                        ? Icon(
                            LucideIcons.check,
                            key: const Key('switcher-current'),
                            size: t.codeSize,
                            color: t.accent,
                          )
                        : null,
                  ),
                ],
              ),
            ),
          ),
        Padding(
          padding: EdgeInsets.symmetric(
            horizontal: t.sp * 0.5,
            vertical: t.sp * 0.75,
          ),
          child: Container(height: t.bw, color: t.border),
        ),
        const SyncNowLine(),
        InkWell(
          onTap: onManage,
          borderRadius: BorderRadius.circular(t.rControl),
          child: Padding(
            padding: EdgeInsets.fromLTRB(t.sp, t.sp * 0.75, t.sp, t.sp * 0.5),
            child: Text(
              'Manage vaults ›',
              style: TextStyle(
                fontFamily: StormTokens.sansFamily,
                fontSize: t.codeSize,
                color: t.accent,
              ),
            ),
          ),
        ),
      ],
    );
  }
}

/// "Synced 2m ago · Sync now", shared by the switcher and the place picker.
class SyncNowLine extends ConsumerWidget {
  const SyncNowLine({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final t = context.tokens;
    final style = TextStyle(
      fontFamily: StormTokens.monoFamily,
      fontSize: t.labelSize,
      color: t.text3,
    );
    return Padding(
      padding: EdgeInsets.symmetric(horizontal: t.sp, vertical: t.sp * 0.75),
      child: Row(
        children: [
          Flexible(
            child: Text(
              placeSyncLine(ref.watch(syncEngineProvider)),
              overflow: TextOverflow.ellipsis,
              style: style,
            ),
          ),
          Text(' · ', style: style),
          GestureDetector(
            key: const Key('place-sync-now'),
            onTap: () async {
              await ref.read(syncEngineProvider).sync();
              ref.invalidate(treeProvider);
            },
            child: Text('Sync now', style: style.copyWith(color: t.text2)),
          ),
        ],
      ),
    );
  }
}

/// The vault colour sheet, reached by long-pressing a vault row. Writes
/// `storm.color` into that vault's own config note.
Future<void> pickVaultAccent(
  BuildContext context,
  WidgetRef ref,
  String vaultId,
  Accent current,
) async {
  final chosen = await showStormSheet<Accent>(
    context: context,
    title: 'Vault colour',
    heightFactor: 0.4,
    builder: (sheetContext) => AccentPicker(
      selected: current,
      onSelected: (accent) => Navigator.pop(sheetContext, accent),
    ),
  );
  if (chosen == null || chosen == current) return;
  final ok = await setVaultAccent(ref, vaultId, chosen);
  if (!ok && context.mounted) {
    ScaffoldMessenger.of(context).showSnackBar(
      const SnackBar(content: Text('Could not save the vault colour')),
    );
  }
}
