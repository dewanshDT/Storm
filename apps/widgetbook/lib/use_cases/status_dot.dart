import 'package:flutter/material.dart';
import 'package:storm/ui/tokens.dart';
import 'package:storm/ui/widgets.dart';
import 'package:widgetbook/widgetbook.dart';
import 'package:widgetbook_annotation/widgetbook_annotation.dart' as widgetbook;

const _path = '[Widgets]';

@widgetbook.UseCase(name: 'default', type: StatusDot, path: _path)
Widget statusDotDefault(BuildContext context) => Padding(
  padding: const EdgeInsets.all(16),
  child: StatusDot(
    status: context.knobs.object.dropdown<DotStatus>(
      label: 'status',
      options: DotStatus.values,
      initialOption: DotStatus.synced,
      labelBuilder: (s) => s.name,
    ),
    size: context.knobs.double.slider(
      label: 'size',
      initialValue: 8,
      min: 4,
      max: 32,
    ),
    ring: context.knobs.boolean(label: 'ring') ? context.tokens.bg : null,
  ),
);

@widgetbook.UseCase(name: 'every status', type: StatusDot, path: _path)
Widget statusDotAll(BuildContext context) => Padding(
  padding: const EdgeInsets.all(16),
  child: Wrap(
    spacing: 16,
    runSpacing: 12,
    crossAxisAlignment: WrapCrossAlignment.center,
    children: [
      for (final s in DotStatus.values)
        Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            StatusDot(status: s, size: 12),
            const SizedBox(width: 8),
            Text(s.name),
          ],
        ),
    ],
  ),
);

@widgetbook.UseCase(
  name: 'custom colour and hollow',
  type: StatusDot,
  path: _path,
)
Widget statusDotCustom(BuildContext context) {
  final t = context.tokens;
  return Padding(
    padding: const EdgeInsets.all(16),
    child: Row(
      spacing: 16,
      children: [
        StatusDot(color: t.text2, size: 12),
        StatusDot(color: t.text3, hollow: true, size: 12),
        StatusDot(color: t.accent, size: 12),
      ],
    ),
  );
}
