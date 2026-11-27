import 'package:flutter/material.dart';
import 'package:storm/ui/controls.dart';
import 'package:storm_widgetbook/stateful_value.dart';
import 'package:widgetbook/widgetbook.dart';
import 'package:widgetbook_annotation/widgetbook_annotation.dart' as widgetbook;

const _path = '[Controls]';

const _none = 'none';

const _sync = [
  ChoiceOption('all', 'All'),
  ChoiceOption('unread', 'Unread'),
  ChoiceOption('mentions', 'Mentions'),
  ChoiceOption('pinned', 'Pinned'),
];

Widget _chips(
  BuildContext context,
  List<ChoiceOption<String>> options, {
  bool enabledDefault = true,
}) {
  final selected = context.knobs.object.dropdown<String>(
    label: 'selected',
    options: [_none, ...options.map((o) => o.value)],
    initialOption: options.isEmpty ? _none : options.first.value,
  );
  final enabled = context.knobs.boolean(
    label: 'enabled',
    initialValue: enabledDefault,
  );
  return StatefulValue<String?>(
    initial: selected == _none ? null : selected,
    builder: (value, set) => Padding(
      padding: const EdgeInsets.all(16),
      child: ChoiceChips<String>(
        options: options,
        selected: value,
        dense: context.knobs.boolean(label: 'dense'),
        onSelected: enabled ? set : null,
      ),
    ),
  );
}

@widgetbook.UseCase(name: 'default', type: ChoiceChips, path: _path)
Widget choiceChipsDefault(BuildContext context) => _chips(context, _sync);

@widgetbook.UseCase(name: 'disabled', type: ChoiceChips, path: _path)
Widget choiceChipsDisabled(BuildContext context) =>
    _chips(context, _sync, enabledDefault: false);

@widgetbook.UseCase(name: 'empty', type: ChoiceChips, path: _path)
Widget choiceChipsEmpty(BuildContext context) => _chips(context, const []);

@widgetbook.UseCase(name: 'long labels (wrap)', type: ChoiceChips, path: _path)
Widget choiceChipsLong(BuildContext context) => _chips(context, const [
  ChoiceOption('a', 'Notes shared with me by other people'),
  ChoiceOption('b', 'Everything modified in the last thirty days'),
  ChoiceOption('c', 'Archived'),
  ChoiceOption('d', 'A_single_unbroken_label_with_no_spaces_in_it_at_all'),
]);
