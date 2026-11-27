import 'package:flutter/material.dart';
import 'package:storm/ui/widgets.dart';
import 'package:storm/ui/tokens.dart';
import 'package:widgetbook/widgetbook.dart';
import 'package:widgetbook_annotation/widgetbook_annotation.dart' as widgetbook;

const _path = '[Widgets]';

Widget _body(String text) => Text(text);

@widgetbook.UseCase(name: 'default', type: StormCard, path: _path)
Widget stormCardDefault(BuildContext context) => Padding(
  padding: const EdgeInsets.all(16),
  child: StormCard(
    density: context.knobs.object.dropdown<StormDensity>(
      label: 'density',
      options: StormDensity.values,
      initialOption: StormDensity.regular,
      labelBuilder: (v) => v.name,
    ),
    border: context.knobs.boolean(label: 'border', initialValue: true),
    clip: context.knobs.boolean(label: 'clip'),
    onTap: context.knobs.boolean(label: 'tappable') ? () {} : null,
    child: _body('A card on the surface colour.'),
  ),
);

@widgetbook.UseCase(name: 'densities', type: StormCard, path: _path)
Widget stormCardDensities(BuildContext context) => Padding(
  padding: const EdgeInsets.all(16),
  child: Column(
    crossAxisAlignment: CrossAxisAlignment.stretch,
    children: [
      for (final d in StormDensity.values) ...[
        StormCard(density: d, child: _body(d.name)),
        const SizedBox(height: 8),
      ],
    ],
  ),
);

@widgetbook.UseCase(name: 'colours and no border', type: StormCard, path: _path)
Widget stormCardColours(BuildContext context) {
  final t = context.tokens;
  return Padding(
    padding: const EdgeInsets.all(16),
    child: Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        StormCard(color: t.surface2, border: false, child: _body('surface2')),
        const SizedBox(height: 8),
        StormCard(
          color: t.accentSoft,
          borderColor: t.accent,
          child: _body('accent border'),
        ),
        const SizedBox(height: 8),
        StormCard(radius: 4, child: _body('radius 4')),
      ],
    ),
  );
}

@widgetbook.UseCase(name: 'tappable', type: StormCard, path: _path)
Widget stormCardTappable(BuildContext context) => Padding(
  padding: const EdgeInsets.all(16),
  child: StormCard(
    onTap: () {},
    onSecondaryTap: () {},
    child: _body('Tap, hover or right-click'),
  ),
);

@widgetbook.UseCase(name: 'focused', type: StormCard, path: _path)
Widget stormCardFocused(BuildContext context) => Padding(
  padding: const EdgeInsets.all(16),
  child: Focus(
    autofocus: true,
    child: StormCard(onTap: () {}, child: _body('Focused')),
  ),
);

@widgetbook.UseCase(name: 'long text (overflow)', type: StormCard, path: _path)
Widget stormCardLong(BuildContext context) => Padding(
  padding: const EdgeInsets.all(16),
  child: SizedBox(
    width: 220,
    child: StormCard(
      clip: true,
      child: Text(
        context.knobs.string(
          label: 'text',
          initialValue:
              'A long line of card text that wraps across several lines '
              'inside a narrow card without overflowing it.',
        ),
      ),
    ),
  ),
);
