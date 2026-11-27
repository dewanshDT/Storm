import 'package:flutter/material.dart';
import 'package:storm/ui/widgets.dart';
import 'package:widgetbook/widgetbook.dart';
import 'package:widgetbook_annotation/widgetbook_annotation.dart' as widgetbook;

const _path = '[Widgets]';

@widgetbook.UseCase(name: 'default', type: BrandMark, path: _path)
Widget brandMarkDefault(BuildContext context) => Padding(
  padding: const EdgeInsets.all(16),
  child: BrandMark(
    size: context.knobs.double.slider(
      label: 'size',
      initialValue: 44,
      min: 16,
      max: 160,
    ),
    withWordmark: context.knobs.boolean(
      label: 'withWordmark',
      initialValue: true,
    ),
  ),
);

@widgetbook.UseCase(name: 'mark only', type: BrandMark, path: _path)
Widget brandMarkOnly(BuildContext context) =>
    const Padding(padding: EdgeInsets.all(16), child: BrandMark());
