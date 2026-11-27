import 'package:flutter/material.dart';
import 'package:storm/ui/widgets.dart';
import 'package:widgetbook/widgetbook.dart';
import 'package:widgetbook_annotation/widgetbook_annotation.dart' as widgetbook;

const _path = '[Widgets]';

@widgetbook.UseCase(name: 'default', type: SectionLabel, path: _path)
Widget sectionLabelDefault(BuildContext context) => Padding(
  padding: const EdgeInsets.all(16),
  child: SectionLabel(
    context.knobs.string(label: 'text', initialValue: 'Recently opened'),
  ),
);

@widgetbook.UseCase(
  name: 'long text (overflow)',
  type: SectionLabel,
  path: _path,
)
Widget sectionLabelLong(BuildContext context) => const Padding(
  padding: EdgeInsets.all(16),
  child: SizedBox(
    width: 180,
    child: SectionLabel('Notes shared with you by people outside this vault'),
  ),
);
