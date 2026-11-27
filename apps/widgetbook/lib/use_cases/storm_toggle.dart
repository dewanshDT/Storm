import 'package:flutter/material.dart';
import 'package:storm/ui/controls.dart';
import 'package:storm_widgetbook/stateful_value.dart';
import 'package:widgetbook/widgetbook.dart';
import 'package:widgetbook_annotation/widgetbook_annotation.dart' as widgetbook;

const _path = '[Controls]';

Widget _toggle(bool initial, {bool enabled = true, bool listening = true}) =>
    StatefulValue<bool>(
      initial: initial,
      builder: (value, set) => Padding(
        padding: const EdgeInsets.all(16),
        child: StormToggle(
          value: value,
          enabled: enabled,
          onChanged: listening ? set : null,
        ),
      ),
    );

@widgetbook.UseCase(name: 'default', type: StormToggle, path: _path)
Widget stormToggleDefault(BuildContext context) => _toggle(
  context.knobs.boolean(label: 'on', initialValue: true),
  enabled: context.knobs.boolean(label: 'enabled', initialValue: true),
);

@widgetbook.UseCase(name: 'off', type: StormToggle, path: _path)
Widget stormToggleOff(BuildContext context) => _toggle(false);

@widgetbook.UseCase(name: 'disabled', type: StormToggle, path: _path)
Widget stormToggleDisabled(BuildContext context) => const Padding(
  padding: EdgeInsets.all(16),
  child: Row(
    mainAxisSize: MainAxisSize.min,
    children: [
      StormToggle(value: true, onChanged: null),
      SizedBox(width: 12),
      StormToggle(value: false, onChanged: null),
      SizedBox(width: 12),
      StormToggle(value: true, enabled: false, onChanged: null),
    ],
  ),
);
