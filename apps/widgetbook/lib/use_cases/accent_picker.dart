import 'package:flutter/material.dart';
import 'package:storm/ui/accents.dart';
import 'package:storm_widgetbook/stateful_value.dart';
import 'package:widgetbook/widgetbook.dart';
import 'package:widgetbook_annotation/widgetbook_annotation.dart' as widgetbook;

const _path = '[Controls]';

@widgetbook.UseCase(name: 'default', type: AccentPicker, path: _path)
Widget accentPickerDefault(BuildContext context) => StatefulValue<Accent>(
  initial: context.knobs.object.dropdown<Accent>(
    label: 'selected',
    options: Accent.values,
    initialOption: Accent.sage,
    labelBuilder: (a) => a.label,
  ),
  builder: (value, set) => Padding(
    padding: const EdgeInsets.all(16),
    child: AccentPicker(
      selected: value,
      onSelected: set,
      size: context.knobs.double.slider(
        label: 'size',
        initialValue: 30,
        min: 16,
        max: 64,
      ),
    ),
  ),
);

@widgetbook.UseCase(name: 'none selected', type: AccentPicker, path: _path)
Widget accentPickerNone(BuildContext context) => StatefulValue<Accent>(
  initial: Accent.none,
  builder: (value, set) => Padding(
    padding: const EdgeInsets.all(16),
    child: AccentPicker(selected: value, onSelected: set),
  ),
);

@widgetbook.UseCase(name: 'narrow (wraps)', type: AccentPicker, path: _path)
Widget accentPickerNarrow(BuildContext context) => Padding(
  padding: const EdgeInsets.all(16),
  child: SizedBox(
    width: 140,
    child: AccentPicker(selected: Accent.coral, onSelected: (_) {}),
  ),
);

@widgetbook.UseCase(name: 'default', type: AccentSwatch, path: _path)
Widget accentSwatchDefault(BuildContext context) => Padding(
  padding: const EdgeInsets.all(16),
  child: AccentSwatch(
    accent: context.knobs.object.dropdown<Accent>(
      label: 'accent',
      options: Accent.values,
      initialOption: Accent.mint,
      labelBuilder: (a) => a.label,
    ),
    selected: context.knobs.boolean(label: 'selected'),
    size: context.knobs.double.slider(
      label: 'size',
      initialValue: 30,
      min: 16,
      max: 64,
    ),
    onTap: () {},
  ),
);

@widgetbook.UseCase(name: 'selected', type: AccentSwatch, path: _path)
Widget accentSwatchSelected(BuildContext context) => Padding(
  padding: const EdgeInsets.all(16),
  child: AccentSwatch(
    accent: Accent.sky,
    selected: true,
    size: 30,
    onTap: () {},
  ),
);

@widgetbook.UseCase(name: 'none (no colour)', type: AccentSwatch, path: _path)
Widget accentSwatchNone(BuildContext context) => Padding(
  padding: const EdgeInsets.all(16),
  child: AccentSwatch(
    accent: Accent.none,
    selected: false,
    size: 30,
    onTap: () {},
  ),
);
