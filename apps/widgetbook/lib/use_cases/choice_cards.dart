import 'package:flutter/material.dart';
import 'package:storm/ui/controls.dart';
import 'package:storm/ui/tokens.dart';
import 'package:storm_widgetbook/stateful_value.dart';
import 'package:widgetbook/widgetbook.dart';
import 'package:widgetbook_annotation/widgetbook_annotation.dart' as widgetbook;

const _path = '[Controls]';

const _none = 'none';

const _modes = [
  ChoiceCardOption(
    'plan',
    'Plan',
    subtitle: 'Reads the vault and proposes changes without making them.',
  ),
  ChoiceCardOption(
    'edit',
    'Edit',
    subtitle: 'Applies changes after you approve each one.',
    note: 'Recommended',
  ),
  ChoiceCardOption(
    'auto',
    'Auto',
    subtitle: 'Applies changes without asking.',
    note: 'No approval prompts',
  ),
];

Widget _cards(
  BuildContext context,
  List<ChoiceCardOption<String>> options, {
  bool enabledDefault = true,
}) {
  final selected = context.knobs.object.dropdown<String>(
    label: 'selected',
    options: [_none, ...options.map((o) => o.value)],
    initialOption: options.length > 1 ? options[1].value : _none,
  );
  final enabled = context.knobs.boolean(
    label: 'enabled',
    initialValue: enabledDefault,
  );
  final minCardWidth = context.knobs.double.slider(
    label: 'minCardWidth',
    initialValue: 220,
    min: 120,
    max: 400,
  );
  return StatefulValue<String?>(
    initial: selected == _none ? null : selected,
    builder: (value, set) => Padding(
      padding: const EdgeInsets.all(16),
      child: ChoiceCards<String>(
        options: options,
        selected: value,
        minCardWidth: minCardWidth,
        onSelected: enabled ? set : null,
      ),
    ),
  );
}

@widgetbook.UseCase(name: 'default', type: ChoiceCards, path: _path)
Widget choiceCardsDefault(BuildContext context) => _cards(context, _modes);

@widgetbook.UseCase(name: 'disabled', type: ChoiceCards, path: _path)
Widget choiceCardsDisabled(BuildContext context) =>
    _cards(context, _modes, enabledDefault: false);

@widgetbook.UseCase(name: 'one option disabled', type: ChoiceCards, path: _path)
Widget choiceCardsOptionDisabled(BuildContext context) => _cards(context, [
  _modes[0],
  _modes[1],
  const ChoiceCardOption(
    'auto',
    'Auto',
    subtitle: 'Not available on this host.',
    enabled: false,
  ),
]);

@widgetbook.UseCase(name: 'error (alert)', type: ChoiceCards, path: _path)
Widget choiceCardsError(BuildContext context) => _cards(context, [
  _modes[0],
  ChoiceCardOption(
    'edit',
    'Edit',
    subtitle: 'Applies changes after you approve each one.',
    alert: Text(
      'This host could not be reached.',
      style: TextStyle(color: Theme.of(context).colorScheme.error),
    ),
  ),
]);

@widgetbook.UseCase(name: 'empty', type: ChoiceCards, path: _path)
Widget choiceCardsEmpty(BuildContext context) => _cards(context, const []);

@widgetbook.UseCase(
  name: 'long text (overflow)',
  type: ChoiceCards,
  path: _path,
)
Widget choiceCardsLong(BuildContext context) => _cards(context, const [
  ChoiceCardOption(
    'a',
    'A deliberately long title that has to wrap onto several lines',
    subtitle:
        'And a subtitle that goes on for long enough that the card must grow '
        'taller than its neighbour, which then has to match its height.',
    note: 'A_note_with_no_spaces_that_cannot_wrap_at_all_in_a_narrow_card',
  ),
  ChoiceCardOption('b', 'Short'),
]);

Widget _card(
  BuildContext context, {
  String title = 'Catppuccin',
  String? subtitle = 'Mocha',
  String? note = 'Inspired by Catppuccin',
  bool selected = false,
  bool enabled = true,
  Widget? alert,
  double width = 220,
}) => Padding(
  padding: const EdgeInsets.all(16),
  child: SizedBox(
    width: width,
    child: ChoiceCard(
      title: title,
      subtitle: subtitle,
      note: note,
      alert: alert,
      selected: selected,
      onTap: enabled ? () {} : null,
    ),
  ),
);

@widgetbook.UseCase(name: 'default', type: ChoiceCard, path: _path)
Widget choiceCardDefault(BuildContext context) => _card(
  context,
  title: context.knobs.string(label: 'title', initialValue: 'Catppuccin'),
  subtitle: context.knobs.stringOrNull(
    label: 'subtitle',
    initialValue: 'Mocha',
  ),
  note: context.knobs.stringOrNull(
    label: 'note',
    initialValue: 'Inspired by Catppuccin',
  ),
  selected: context.knobs.boolean(label: 'selected'),
  enabled: context.knobs.boolean(label: 'enabled', initialValue: true),
);

@widgetbook.UseCase(name: 'selected', type: ChoiceCard, path: _path)
Widget choiceCardSelected(BuildContext context) =>
    _card(context, selected: true);

@widgetbook.UseCase(name: 'disabled', type: ChoiceCard, path: _path)
Widget choiceCardDisabled(BuildContext context) =>
    _card(context, enabled: false);

@widgetbook.UseCase(name: 'alert', type: ChoiceCard, path: _path)
Widget choiceCardAlert(BuildContext context) => _card(
  context,
  alert: Text(
    'Needs a newer host',
    style: TextStyle(color: context.tokens.danger),
  ),
);

@widgetbook.UseCase(name: 'long text (overflow)', type: ChoiceCard, path: _path)
Widget choiceCardLong(BuildContext context) => _card(
  context,
  title: 'A card whose title is far too long for the width it is given',
  subtitle: 'And a subtitle that is every bit as long as the title above it',
  note: 'Plus a note that runs on for a while as well',
  width: 200,
);
