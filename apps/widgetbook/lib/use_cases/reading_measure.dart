import 'package:flutter/material.dart';
import 'package:storm/ui/widgets.dart';
import 'package:widgetbook/widgetbook.dart';
import 'package:widgetbook_annotation/widgetbook_annotation.dart' as widgetbook;

const _path = '[Widgets]';

@widgetbook.UseCase(name: 'default', type: ReadingMeasure, path: _path)
Widget readingMeasureDefault(BuildContext context) => Padding(
  padding: const EdgeInsets.all(16),
  child: ReadingMeasure(
    child: Text(
      context.knobs.string(
        label: 'text',
        initialValue:
            'A paragraph set at the reading measure: on a wide window the '
                'lines stop at a comfortable width instead of running edge to '
                'edge, and on a phone it simply fills the screen. ' *
            4,
        maxLines: 6,
      ),
    ),
  ),
);
