import 'package:flutter/material.dart';
import 'package:storm/ui/widgets.dart';
import 'package:widgetbook/widgetbook.dart';
import 'package:widgetbook_annotation/widgetbook_annotation.dart' as widgetbook;

const _path = '[Widgets]';

Widget _chip(String text) =>
    Padding(padding: const EdgeInsets.all(8), child: Text(text));

@widgetbook.UseCase(name: 'default', type: Tappable, path: _path)
Widget tappableDefault(BuildContext context) => Padding(
  padding: const EdgeInsets.all(24),
  child: Tappable(
    onTap: context.knobs.boolean(label: 'enabled', initialValue: true)
        ? () {}
        : null,
    tooltip: 'Opens the thing',
    semanticLabel: 'Open the thing',
    borderRadius: BorderRadius.circular(8),
    child: _chip('Tap, hover or press Enter'),
  ),
);

@widgetbook.UseCase(name: 'disabled', type: Tappable, path: _path)
Widget tappableDisabled(BuildContext context) => Padding(
  padding: const EdgeInsets.all(24),
  child: Tappable(onTap: null, child: _chip('Disabled')),
);

@widgetbook.UseCase(name: 'focused', type: Tappable, path: _path)
Widget tappableFocused(BuildContext context) => Padding(
  padding: const EdgeInsets.all(24),
  child: Focus(
    autofocus: true,
    child: Tappable(
      onTap: () {},
      borderRadius: BorderRadius.circular(8),
      child: _chip('Focused'),
    ),
  ),
);

@widgetbook.UseCase(name: 'small target', type: Tappable, path: _path)
Widget tappableSmall(BuildContext context) => Padding(
  padding: const EdgeInsets.all(24),
  child: Tappable(
    onTap: () {},
    semanticLabel: 'Close',
    child: const Icon(Icons.close, size: 12),
  ),
);

@widgetbook.UseCase(name: 'long label (overflow)', type: Tappable, path: _path)
Widget tappableLong(BuildContext context) => Padding(
  padding: const EdgeInsets.all(24),
  child: SizedBox(
    width: 200,
    child: Tappable(
      onTap: () {},
      child: const Padding(
        padding: EdgeInsets.all(8),
        child: Text(
          'A very long tappable label that has to ellipsise',
          maxLines: 1,
          overflow: TextOverflow.ellipsis,
        ),
      ),
    ),
  ),
);
