import 'package:flutter/material.dart';
import 'package:storm/ui/widgets.dart';
import 'package:widgetbook/widgetbook.dart';
import 'package:widgetbook_annotation/widgetbook_annotation.dart' as widgetbook;

const _path = '[Widgets]';

@widgetbook.UseCase(name: 'default', type: StatBlock, path: _path)
Widget statBlockDefault(BuildContext context) => Padding(
  padding: const EdgeInsets.all(16),
  child: StatBlock(
    value: context.knobs.string(label: 'value', initialValue: '27'),
    label: context.knobs.string(label: 'label', initialValue: 'notes'),
  ),
);

@widgetbook.UseCase(name: 'long text (overflow)', type: StatBlock, path: _path)
Widget statBlockLong(BuildContext context) => const Padding(
  padding: EdgeInsets.all(16),
  child: SizedBox(
    width: 120,
    child: StatBlock(
      value: '1,204,339',
      label: 'notes synced across every vault',
    ),
  ),
);
