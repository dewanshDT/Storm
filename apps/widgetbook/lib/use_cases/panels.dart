import 'package:flutter/material.dart';
import 'package:storm/ui/panels.dart';
import 'package:widgetbook/widgetbook.dart';
import 'package:widgetbook_annotation/widgetbook_annotation.dart' as widgetbook;

const _path = '[Panels]';

@widgetbook.UseCase(name: 'default', type: MonoTag, path: _path)
Widget monoTagDefault(BuildContext context) => Padding(
  padding: const EdgeInsets.all(16),
  child: MonoTag(context.knobs.string(label: 'text', initialValue: 'Work')),
);

@widgetbook.UseCase(name: 'long text (overflow)', type: MonoTag, path: _path)
Widget monoTagLong(BuildContext context) => const Padding(
  padding: EdgeInsets.all(16),
  child: SizedBox(width: 120, child: MonoTag('A vault with a very long name')),
);
