import 'package:flutter/material.dart';
import 'package:flutter_lucide/flutter_lucide.dart';
import 'package:storm/ui/surfaces.dart';
import 'package:storm/ui/tokens.dart';
import 'package:storm/ui/widgets.dart';
import 'package:widgetbook/widgetbook.dart';
import 'package:widgetbook_annotation/widgetbook_annotation.dart' as widgetbook;

const _path = '[Surfaces]';

Widget _pad(Widget child) =>
    Padding(padding: const EdgeInsets.all(16), child: child);

@widgetbook.UseCase(name: 'default', type: StormPopover, path: _path)
Widget stormPopoverDefault(BuildContext context) => _pad(
  StormPopover(
    width: context.knobs.doubleOrNull.slider(
      label: 'width',
      initialValue: 240,
      min: 160,
      max: 400,
    ),
    padding: context.knobs.doubleOrNull.slider(
      label: 'padding',
      initialValue: null,
      min: 0,
      max: 32,
    ),
    children: [
      PopoverItem(
        label: 'Work',
        subtitle: '14 notes',
        selected: true,
        onTap: () {},
      ),
      PopoverItem(label: 'Personal', subtitle: '203 notes', onTap: () {}),
      const PopoverDivider(),
      PopoverItem(label: 'New vault', tone: PopoverTone.accent, onTap: () {}),
    ],
  ),
);

@widgetbook.UseCase(
  name: 'long text (overflow)',
  type: StormPopover,
  path: _path,
)
Widget stormPopoverLong(BuildContext context) => _pad(
  StormPopover(
    width: 200,
    children: [
      PopoverItem(
        label: 'A vault with a name far too long for this menu',
        subtitle: 'Shared with eleven people and two agents',
        onTap: () {},
      ),
    ],
  ),
);

@widgetbook.UseCase(name: 'default', type: PopoverDivider, path: _path)
Widget popoverDividerDefault(BuildContext context) =>
    _pad(const SizedBox(width: 240, child: PopoverDivider()));

PopoverItem _item(
  BuildContext context, {
  String label = 'Work',
  String? subtitle = '14 notes',
  PopoverTone tone = PopoverTone.normal,
  bool selected = false,
  bool tappable = true,
  bool leading = false,
  bool trailing = false,
  String? tooltip,
}) {
  final t = context.tokens;
  return PopoverItem(
    label: label,
    subtitle: subtitle,
    tone: tone,
    selected: selected,
    tooltip: tooltip,
    leading: leading
        ? Icon(LucideIcons.folder, size: 16, color: t.text2)
        : null,
    trailing: trailing
        ? Icon(LucideIcons.check, size: 16, color: t.accent)
        : null,
    onTap: tappable ? () {} : null,
  );
}

@widgetbook.UseCase(name: 'default', type: PopoverItem, path: _path)
Widget popoverItemDefault(BuildContext context) => _pad(
  SizedBox(
    width: 260,
    child: _item(
      context,
      label: context.knobs.string(label: 'label', initialValue: 'Work'),
      subtitle: context.knobs.stringOrNull(
        label: 'subtitle',
        initialValue: '14 notes',
      ),
      tone: context.knobs.object.dropdown<PopoverTone>(
        label: 'tone',
        options: PopoverTone.values,
        initialOption: PopoverTone.normal,
        labelBuilder: (t) => t.name,
      ),
      selected: context.knobs.boolean(label: 'selected'),
      leading: context.knobs.boolean(label: 'leading'),
      trailing: context.knobs.boolean(label: 'trailing'),
      tappable: context.knobs.boolean(label: 'tappable', initialValue: true),
      tooltip: context.knobs.stringOrNull(label: 'tooltip'),
    ),
  ),
);

@widgetbook.UseCase(name: 'every tone', type: PopoverItem, path: _path)
Widget popoverItemTones(BuildContext context) => _pad(
  SizedBox(
    width: 260,
    child: Column(
      children: [
        for (final tone in PopoverTone.values)
          _item(context, label: tone.name, subtitle: null, tone: tone),
      ],
    ),
  ),
);

@widgetbook.UseCase(name: 'selected', type: PopoverItem, path: _path)
Widget popoverItemSelected(BuildContext context) => _pad(
  SizedBox(width: 260, child: _item(context, selected: true, trailing: true)),
);

@widgetbook.UseCase(name: 'disabled (no onTap)', type: PopoverItem, path: _path)
Widget popoverItemDisabled(BuildContext context) =>
    _pad(SizedBox(width: 260, child: _item(context, tappable: false)));

@widgetbook.UseCase(
  name: 'long text (overflow)',
  type: PopoverItem,
  path: _path,
)
Widget popoverItemLong(BuildContext context) => _pad(
  SizedBox(
    width: 200,
    child: _item(
      context,
      label: 'A vault with a name far too long for this menu',
      subtitle: 'Shared with eleven people and two agents',
      leading: true,
      trailing: true,
    ),
  ),
);
