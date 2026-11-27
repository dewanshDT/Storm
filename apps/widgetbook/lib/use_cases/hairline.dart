import 'package:flutter/material.dart';
import 'package:storm/ui/widgets.dart';
import 'package:widgetbook/widgetbook.dart';
import 'package:widgetbook_annotation/widgetbook_annotation.dart' as widgetbook;

const _path = '[Widgets]';

@widgetbook.UseCase(name: 'default', type: Hairline, path: _path)
Widget hairlineDefault(BuildContext context) => Padding(
  padding: const EdgeInsets.all(16),
  child: Hairline(
    top: context.knobs.boolean(label: 'top'),
    bottom: context.knobs.boolean(label: 'bottom', initialValue: true),
    left: context.knobs.boolean(label: 'left'),
    right: context.knobs.boolean(label: 'right'),
    padding: const EdgeInsets.all(12),
    child: const Text('Content between rules'),
  ),
);

@widgetbook.UseCase(name: 'edges', type: Hairline, path: _path)
Widget hairlineEdges(BuildContext context) => Padding(
  padding: const EdgeInsets.all(16),
  child: Wrap(
    spacing: 12,
    runSpacing: 12,
    children: [
      for (final (name, h) in [
        ('top', const Hairline(top: true)),
        ('bottom', const Hairline(bottom: true)),
        ('left', const Hairline(left: true)),
        ('right', const Hairline(right: true)),
        (
          'all',
          const Hairline(top: true, bottom: true, left: true, right: true),
        ),
      ])
        SizedBox(
          width: 100,
          height: 56,
          child: Hairline(
            top: h.top,
            bottom: h.bottom,
            left: h.left,
            right: h.right,
            padding: const EdgeInsets.all(8),
            child: Text(name),
          ),
        ),
    ],
  ),
);

@widgetbook.UseCase(name: 'long text (overflow)', type: Hairline, path: _path)
Widget hairlineLong(BuildContext context) => const Padding(
  padding: EdgeInsets.all(16),
  child: SizedBox(
    width: 200,
    child: Hairline(
      bottom: true,
      padding: EdgeInsets.symmetric(vertical: 8),
      child: Text('A long line of text that wraps inside a narrow ruled box.'),
    ),
  ),
);
