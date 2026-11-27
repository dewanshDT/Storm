import 'package:flutter/material.dart';
import 'package:flutter_lucide/flutter_lucide.dart';
import 'package:storm/ui/tokens.dart';
import 'package:storm/ui/widgets.dart';
import 'package:widgetbook/widgetbook.dart';
import 'package:widgetbook_annotation/widgetbook_annotation.dart' as widgetbook;

const _path = '[Widgets]';

@widgetbook.UseCase(name: 'default', type: CountBadge, path: _path)
Widget countBadgeDefault(BuildContext context) => Padding(
  padding: const EdgeInsets.all(16),
  child: CountBadge(
    count: context.knobs.int.slider(
      label: 'count',
      initialValue: 3,
      min: 0,
      max: 150,
    ),
    color: context.tokens.danger,
    small: context.knobs.boolean(label: 'small'),
  ),
);

@widgetbook.UseCase(name: 'capped (99+ / 9+)', type: CountBadge, path: _path)
Widget countBadgeCapped(BuildContext context) => Padding(
  padding: const EdgeInsets.all(16),
  child: Row(
    mainAxisSize: MainAxisSize.min,
    children: [
      CountBadge(count: 120, color: context.tokens.accent),
      const SizedBox(width: 16),
      CountBadge(count: 12, color: context.tokens.accent, small: true),
    ],
  ),
);

@widgetbook.UseCase(name: 'default', type: IconBadgeButton, path: _path)
Widget iconBadgeButtonDefault(BuildContext context) => Padding(
  padding: const EdgeInsets.all(16),
  child: IconBadgeButton(
    icon: LucideIcons.bell,
    tooltip: context.knobs.string(
      label: 'tooltip',
      initialValue: 'Notifications',
    ),
    count: context.knobs.int.slider(
      label: 'count',
      initialValue: 4,
      min: 0,
      max: 150,
    ),
    smallBadge: context.knobs.boolean(label: 'smallBadge'),
    onTap: () {},
  ),
);

@widgetbook.UseCase(name: 'no unread', type: IconBadgeButton, path: _path)
Widget iconBadgeButtonQuiet(BuildContext context) => Padding(
  padding: const EdgeInsets.all(16),
  child: IconBadgeButton(
    icon: LucideIcons.bell,
    tooltip: 'Notifications',
    onTap: () {},
  ),
);

@widgetbook.UseCase(
  name: 'many unread (capped)',
  type: IconBadgeButton,
  path: _path,
)
Widget iconBadgeButtonCapped(BuildContext context) => Padding(
  padding: const EdgeInsets.all(16),
  child: Row(
    mainAxisSize: MainAxisSize.min,
    children: [
      IconBadgeButton(
        icon: LucideIcons.bell,
        tooltip: 'Notifications',
        count: 240,
        onTap: () {},
      ),
      const SizedBox(width: 24),
      IconBadgeButton(
        icon: LucideIcons.bell,
        tooltip: 'Notifications',
        count: 24,
        smallBadge: true,
        onTap: () {},
      ),
    ],
  ),
);
