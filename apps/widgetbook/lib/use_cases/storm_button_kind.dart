import 'package:flutter/material.dart';
import 'package:storm/ui/controls.dart';
import 'package:storm/ui/tokens.dart';
import 'package:widgetbook_annotation/widgetbook_annotation.dart' as widgetbook;

const _path = '[Controls]';

Widget _kinds(BuildContext context, {Color? color}) => Padding(
  padding: const EdgeInsets.all(16),
  child: Wrap(
    spacing: 12,
    runSpacing: 12,
    children: [
      for (final kind in StormButtonKind.values)
        StormButton(
          label: kind.name,
          kind: kind,
          color: color,
          onPressed: () {},
        ),
    ],
  ),
);

@widgetbook.UseCase(name: 'danger text', type: StormButtonKind, path: _path)
Widget stormButtonKindDangerText(BuildContext context) => Padding(
  padding: const EdgeInsets.all(16),
  child: StormButton(
    label: 'Delete',
    kind: StormButtonKind.dangerText,
    onPressed: () {},
  ),
);

@widgetbook.UseCase(name: 'colour override', type: StormButtonKind, path: _path)
Widget stormButtonKindColour(BuildContext context) =>
    _kinds(context, color: context.tokens.amber);
