import 'package:flutter/material.dart';
import 'package:storm/ui/icons.dart';
import 'package:storm/ui/tokens.dart';
import 'package:widgetbook/widgetbook.dart';
import 'package:widgetbook_annotation/widgetbook_annotation.dart' as widgetbook;

const _path = '[Widgets]';

@widgetbook.UseCase(name: 'default', type: StormIcon, path: _path)
Widget stormIconDefault(BuildContext context) => Padding(
  padding: const EdgeInsets.all(16),
  child: StormIcon(
    context.knobs.object.dropdown<StormGlyph>(
      label: 'glyph',
      options: StormGlyph.values,
      initialOption: StormGlyph.folder,
      labelBuilder: (g) => g.name,
    ),
    size: context.knobs.double.slider(
      label: 'size',
      initialValue: 20,
      min: 8,
      max: 96,
    ),
    color: context.tokens.text,
  ),
);

@widgetbook.UseCase(name: 'every glyph', type: StormIcon, path: _path)
Widget stormIconAll(BuildContext context) => Padding(
  padding: const EdgeInsets.all(16),
  child: Wrap(
    spacing: 20,
    runSpacing: 20,
    children: [
      for (final g in StormGlyph.values)
        StormIcon(g, size: 32, color: context.tokens.text2),
    ],
  ),
);
