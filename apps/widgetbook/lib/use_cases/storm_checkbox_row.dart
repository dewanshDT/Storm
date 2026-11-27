import 'package:flutter/material.dart';
import 'package:storm/ui/widgets.dart';
import 'package:storm_widgetbook/stateful_value.dart';
import 'package:widgetbook/widgetbook.dart';
import 'package:widgetbook_annotation/widgetbook_annotation.dart' as widgetbook;

const _path = '[Controls]';

Widget _row(bool initial, bool enabled, String label, {double? width}) =>
    StatefulValue<bool>(
      initial: initial,
      builder: (value, set) => Padding(
        padding: const EdgeInsets.all(16),
        child: SizedBox(
          width: width,
          child: StormCheckboxRow(
            value: value,
            onChanged: enabled ? set : null,
            child: Text(label),
          ),
        ),
      ),
    );

@widgetbook.UseCase(name: 'default', type: StormCheckboxRow, path: _path)
Widget stormCheckboxRowDefault(BuildContext context) => _row(
  context.knobs.boolean(label: 'checked', initialValue: true),
  context.knobs.boolean(label: 'enabled', initialValue: true),
  context.knobs.string(label: 'label', initialValue: 'Include diagnostics'),
);

@widgetbook.UseCase(name: 'unchecked', type: StormCheckboxRow, path: _path)
Widget stormCheckboxRowOff(BuildContext context) =>
    _row(false, true, 'Include diagnostics');

@widgetbook.UseCase(name: 'disabled', type: StormCheckboxRow, path: _path)
Widget stormCheckboxRowDisabled(BuildContext context) =>
    _row(true, false, 'Include diagnostics');

@widgetbook.UseCase(
  name: 'long text (overflow)',
  type: StormCheckboxRow,
  path: _path,
)
Widget stormCheckboxRowLong(BuildContext context) => _row(
  true,
  true,
  'Include diagnostics, the last hundred log lines, my device name and the '
  'list of vaults this device has open, so the report can be reproduced',
  width: 280,
);
