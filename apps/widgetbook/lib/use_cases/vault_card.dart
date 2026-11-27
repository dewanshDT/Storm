import 'package:flutter/material.dart';
import 'package:storm/ui/accents.dart';
import 'package:storm/ui/tokens.dart';
import 'package:storm/ui/widgets.dart';
import 'package:widgetbook/widgetbook.dart';
import 'package:widgetbook_annotation/widgetbook_annotation.dart' as widgetbook;

const _path = '[Widgets]';

Widget _card(
  BuildContext context, {
  String name = 'Work',
  String subtitle = '14 notes',
  Accent accent = Accent.lavender,
  DotStatus? status = DotStatus.synced,
  bool muted = false,
  bool tinted = true,
  bool tappable = true,
}) => Padding(
  padding: const EdgeInsets.all(16),
  child: SizedBox(
    width: 180,
    child: VaultCard(
      name: name,
      tile: accent.tile(context.tokens),
      subtitle: subtitle,
      status: status,
      muted: muted,
      tinted: tinted,
      onTap: tappable ? () {} : null,
    ),
  ),
);

@widgetbook.UseCase(name: 'default', type: VaultCard, path: _path)
Widget vaultCardDefault(BuildContext context) => _card(
  context,
  name: context.knobs.string(label: 'name', initialValue: 'Work'),
  subtitle: context.knobs.string(label: 'subtitle', initialValue: '14 notes'),
  accent: context.knobs.object.dropdown<Accent>(
    label: 'accent',
    options: Accent.values,
    initialOption: Accent.lavender,
    labelBuilder: (a) => a.label,
  ),
  status: context.knobs.object.dropdown<DotStatus?>(
    label: 'status',
    options: [null, ...DotStatus.values],
    initialOption: DotStatus.synced,
    labelBuilder: (s) => s?.name ?? 'none',
  ),
  muted: context.knobs.boolean(label: 'muted'),
  tinted: context.knobs.boolean(label: 'tinted', initialValue: true),
  tappable: context.knobs.boolean(label: 'tappable', initialValue: true),
);

@widgetbook.UseCase(name: 'syncing', type: VaultCard, path: _path)
Widget vaultCardSyncing(BuildContext context) =>
    _card(context, status: DotStatus.syncing, subtitle: 'Syncing 3 notes');

@widgetbook.UseCase(name: 'offline', type: VaultCard, path: _path)
Widget vaultCardOffline(BuildContext context) =>
    _card(context, status: DotStatus.offline, subtitle: 'Offline');

@widgetbook.UseCase(name: 'untrusted (error)', type: VaultCard, path: _path)
Widget vaultCardUntrusted(BuildContext context) => _card(
  context,
  status: DotStatus.untrusted,
  muted: true,
  subtitle: 'Identity check failed',
);

@widgetbook.UseCase(name: 'untinted', type: VaultCard, path: _path)
Widget vaultCardUntinted(BuildContext context) =>
    _card(context, accent: Accent.none, tinted: false);

@widgetbook.UseCase(name: 'disabled (no onTap)', type: VaultCard, path: _path)
Widget vaultCardDisabled(BuildContext context) =>
    _card(context, tappable: false);

@widgetbook.UseCase(name: 'long text (overflow)', type: VaultCard, path: _path)
Widget vaultCardLong(BuildContext context) => _card(
  context,
  name: 'A vault with a name that is far too long for its card',
  subtitle: '1,204 notes across 86 folders, last synced a while ago',
);
