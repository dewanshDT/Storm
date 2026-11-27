import 'package:flutter/material.dart';
import 'package:storm/ui/widgets.dart';
import 'package:storm_widgetbook/stateful_value.dart';
import 'package:widgetbook/widgetbook.dart';
import 'package:widgetbook_annotation/widgetbook_annotation.dart' as widgetbook;

const _path = '[Controls]';

Widget _box(bool initial, bool enabled, {double? size}) => StatefulValue<bool>(
  initial: initial,
  builder: (value, set) => Padding(
    padding: const EdgeInsets.all(16),
    child: StormCheckbox(
      value: value,
      size: size,
      onChanged: enabled ? set : null,
    ),
  ),
);

@widgetbook.UseCase(name: 'default', type: StormCheckbox, path: _path)
Widget stormCheckboxDefault(BuildContext context) => _box(
  context.knobs.boolean(label: 'checked'),
  context.knobs.boolean(label: 'enabled', initialValue: true),
  size: context.knobs.double.slider(
    label: 'size',
    initialValue: 18,
    min: 12,
    max: 40,
  ),
);

@widgetbook.UseCase(name: 'checked', type: StormCheckbox, path: _path)
Widget stormCheckboxChecked(BuildContext context) => _box(true, true);

@widgetbook.UseCase(name: 'disabled', type: StormCheckbox, path: _path)
Widget stormCheckboxDisabled(BuildContext context) => const Padding(
  padding: EdgeInsets.all(16),
  child: Row(
    mainAxisSize: MainAxisSize.min,
    spacing: 16,
    children: [
      StormCheckbox(value: false, onChanged: null),
      StormCheckbox(value: true, onChanged: null),
    ],
  ),
);

@widgetbook.UseCase(
  name: 'row (long text)',
  type: StormCheckboxRow,
  path: _path,
)
Widget stormCheckboxRowLong(BuildContext context) {
  final text = context.knobs.string(
    label: 'text',
    initialValue:
        'Include diagnostics, the last hundred log lines and the redacted '
        'connection details so the problem can be reproduced',
  );
  final enabled = context.knobs.boolean(label: 'enabled', initialValue: true);
  return StatefulValue<bool>(
    initial: true,
    builder: (value, set) => Padding(
      padding: const EdgeInsets.all(16),
      child: StormCheckboxRow(
        value: value,
        onChanged: enabled ? set : null,
        child: Text(text),
      ),
    ),
  );
}
