import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../api/models.dart';
import '../../state/app_state.dart';
import '../../state/vault_config.dart' show setVaultAccent;
import '../accents.dart';
import '../controls.dart';
import '../shell/vault_tile.dart';
import '../states.dart' show describeFailure;
import '../surfaces.dart';
import '../tokens.dart';
import 'settings_widgets.dart';

/// Settings › Vaults. Removing a vault only unregisters it, and the page says
/// so before anyone asks.
class VaultsPage extends ConsumerWidget {
  const VaultsPage({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final vaults = ref.watch(vaultsProvider);
    final root = ref.watch(serverConfigProvider).value?.vaultRoot ?? '';
    final accents = ref.watch(vaultAccentsProvider).value ?? const {};

    return SettingsPage(
      title: 'Vaults',
      intro: 'Each vault is a folder of Markdown under the storage root.',
      children: [
        ...vaults.when(
          loading: () => [const SettingsMuted('Loading…')],
          error: (e, _) => [SettingsMuted(describeFailure(e), danger: true)],
          data: (list) => [
            for (final v in list)
              v.missing
                  ? _MissingRow(vault: v)
                  : _VaultRow(
                      vault: v,
                      root: root,
                      accent: accents[v.id] ?? Accent.none,
                    ),
          ],
        ),
        Padding(
          padding: EdgeInsets.only(top: context.tokens.sp * 1.5),
          child: const SettingsNote(
            'Removing a vault takes it out of Storm. Its directory and every '
            'note stay where they are.',
            small: true,
          ),
        ),
        SettingsButtonRow(
          child: StormButton.primary(
            key: const Key('new-vault'),
            label: '＋ New vault',
            onPressed: () => _create(context, ref),
          ),
        ),
      ],
    );
  }

  Future<void> _create(BuildContext context, WidgetRef ref) async {
    final name = await promptForText(
      context,
      title: 'New vault',
      hint: 'Personal',
    );
    if (name == null || name.trim().isEmpty || !context.mounted) return;
    final api = ref.read(apiProvider);
    if (api == null) return;
    try {
      await api.createVault(name.trim());
      ref.invalidate(vaultsProvider);
      ref.invalidate(serverConfigProvider);
    } catch (e) {
      if (context.mounted) settingsToast(context, describeFailure(e));
    }
  }
}

String _notes(int n) => '$n note${n == 1 ? '' : 's'}';

String _pathOf(String root, String dir) {
  if (root.isEmpty) return dir;
  return root.endsWith('/') ? '$root$dir' : '$root/$dir';
}

class _VaultRow extends ConsumerStatefulWidget {
  const _VaultRow({
    required this.vault,
    required this.root,
    required this.accent,
  });

  final VaultInfo vault;
  final String root;
  final Accent accent;

  @override
  ConsumerState<_VaultRow> createState() => _VaultRowState();
}

class _VaultRowState extends ConsumerState<_VaultRow> {
  final _tile = GlobalKey();

  /// Q7: the tile is where a vault's colour is chosen.
  Future<void> _pickColour() async {
    final t = context.tokens;
    final chosen = await showStormPopover<Accent>(
      context: context,
      anchorKey: _tile,
      width: t.sp * 31,
      builder: (pop) => StormPopover(
        children: [
          Padding(
            padding: EdgeInsets.only(bottom: t.sp),
            child: const GroupLabel('Vault colour'),
          ),
          AccentPicker(
            key: const Key('vault-accent-picker'),
            size: t.sp * 3.5,
            selected: widget.accent,
            onSelected: (a) => Navigator.pop(pop, a),
          ),
        ],
      ),
    );
    if (chosen == null || chosen == widget.accent || !mounted) return;
    final ok = await setVaultAccent(ref, widget.vault.id, chosen);
    if (!ok && mounted) settingsToast(context, "Couldn't save the colour.");
  }

  Future<void> _rename() async {
    final v = widget.vault;
    final name = await promptForText(
      context,
      title: 'Rename vault',
      hint: 'Personal',
      initial: v.name,
    );
    if (name == null || name.trim().isEmpty || !mounted) return;
    final api = ref.read(apiProvider);
    if (api == null) return;
    try {
      await api.renameVault(v.id, name.trim());
      ref.invalidate(vaultsProvider);
    } catch (e) {
      if (mounted) settingsToast(context, describeFailure(e));
    }
  }

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final v = widget.vault;
    return SettingsRow(
      key: Key('vault-row-${v.id}'),
      leading: Semantics(
        button: true,
        label: 'Colour of ${v.name}',
        onTap: _pickColour,
        excludeSemantics: true,
        child: InkWell(
          key: _tile,
          onTap: _pickColour,
          borderRadius: BorderRadius.circular(t.rControl * 0.8),
          child: VaultTile(
            key: Key('vault-tile-${v.id}'),
            name: v.name,
            accent: widget.accent,
            size: t.sp * 3.5,
          ),
        ),
      ),
      label: v.name,
      sub: '${_notes(v.noteCount)} · ${_pathOf(widget.root, v.dir)}',
      monoSub: true,
      trailing: RowActions(
        children: [
          TextAction(
            key: Key('rename-vault-${v.id}'),
            label: 'Rename',
            onTap: _rename,
          ),
          TextAction(
            key: Key('remove-vault-${v.id}'),
            label: 'Remove',
            onTap: () => _remove(context, ref, v),
          ),
        ],
      ),
    );
  }
}

Future<void> _remove(BuildContext context, WidgetRef ref, VaultInfo v) async {
  final ok = await confirmSetting(
    context,
    title: 'Remove “${v.name}”?',
    body:
        'Storm stops tracking this vault. Its directory and every note inside '
        'it are left exactly where they are — nothing is deleted.',
    action: 'Remove',
    confirmKey: const Key('confirm-remove'),
  );
  if (!ok || !context.mounted) return;
  final api = ref.read(apiProvider);
  if (api == null) return;
  try {
    await api.removeVault(v.id);
    ref.invalidate(vaultsProvider);
    ref.invalidate(serverConfigProvider);
  } catch (e) {
    if (context.mounted) settingsToast(context, describeFailure(e));
  }
}

/// A vault whose directory is gone: greyed, dashed, and still removable,
/// because removing it is the only way to clear it.
class _MissingRow extends ConsumerWidget {
  const _MissingRow({required this.vault});

  final VaultInfo vault;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final t = context.tokens;
    return Opacity(
      opacity: 0.7,
      child: SettingsRow(
        key: Key('vault-row-${vault.id}'),
        leading: CustomPaint(
          painter: _DashedTile(
            color: t.border,
            radius: t.rControl,
            width: t.bw,
          ),
          child: SizedBox.square(dimension: t.sp * 3.5),
        ),
        label: vault.name,
        muted: true,
        sub: 'Directory not found. Nothing was deleted.',
        monoSub: true,
        trailing: TextAction(
          key: Key('remove-vault-${vault.id}'),
          label: 'Remove',
          onTap: () => _remove(context, ref, vault),
        ),
      ),
    );
  }
}

class _DashedTile extends CustomPainter {
  const _DashedTile({
    required this.color,
    required this.radius,
    required this.width,
  });

  final Color color;
  final double radius;
  final double width;

  @override
  void paint(Canvas canvas, Size size) {
    final paint = Paint()
      ..color = color
      ..style = PaintingStyle.stroke
      ..strokeWidth = width;
    final path = Path()
      ..addRRect(
        RRect.fromRectAndRadius(
          (Offset.zero & size).deflate(width / 2),
          Radius.circular(radius),
        ),
      );
    final dash = width * 3;
    for (final metric in path.computeMetrics()) {
      for (var d = 0.0; d < metric.length; d += dash * 2) {
        canvas.drawPath(metric.extractPath(d, d + dash), paint);
      }
    }
  }

  @override
  bool shouldRepaint(_DashedTile old) =>
      old.color != color || old.radius != radius || old.width != width;
}
