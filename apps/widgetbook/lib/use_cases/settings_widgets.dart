import 'package:flutter/material.dart';
import 'package:storm/ui/controls.dart';
import 'package:storm/ui/settings/settings_widgets.dart';
import 'package:storm/ui/tokens.dart';
import 'package:widgetbook/widgetbook.dart';
import 'package:widgetbook_annotation/widgetbook_annotation.dart' as widgetbook;

const _path = '[Settings]';
const _long =
    'A sentence of settings copy that is long enough to wrap onto several '
    'lines on a phone and still has to read well at every text scale. ';

Widget _pad(Widget child, {double? width}) => Padding(
  padding: const EdgeInsets.all(16),
  child: SizedBox(width: width, child: child),
);

@widgetbook.UseCase(name: 'default', type: GroupLabel, path: _path)
Widget groupLabelDefault(BuildContext context) => _pad(
  GroupLabel(
    context.knobs.string(label: 'text', initialValue: 'Connected apps'),
    top: context.knobs.double.slider(
      label: 'top',
      initialValue: 0,
      min: 0,
      max: 48,
    ),
  ),
);

@widgetbook.UseCase(name: 'long text (overflow)', type: GroupLabel, path: _path)
Widget groupLabelLong(BuildContext context) => _pad(
  const GroupLabel('Apps and agents that may read or write your notes'),
  width: 200,
);

@widgetbook.UseCase(name: 'default', type: SettingsDivider, path: _path)
Widget settingsDividerDefault(BuildContext context) =>
    _pad(const SettingsDivider(), width: 280);

@widgetbook.UseCase(name: 'default', type: TextAction, path: _path)
Widget textActionDefault(BuildContext context) => _pad(
  TextAction(
    label: context.knobs.string(label: 'label', initialValue: 'Revoke'),
    accent: context.knobs.boolean(label: 'accent'),
    danger: context.knobs.boolean(label: 'danger'),
    large: context.knobs.boolean(label: 'large'),
    onTap: context.knobs.boolean(label: 'enabled', initialValue: true)
        ? () {}
        : null,
  ),
);

@widgetbook.UseCase(name: 'every variant', type: TextAction, path: _path)
Widget textActionVariants(BuildContext context) => _pad(
  Wrap(
    spacing: 16,
    runSpacing: 8,
    children: [
      TextAction(label: 'Default', onTap: () {}),
      TextAction(label: 'Accent', accent: true, onTap: () {}),
      TextAction(label: 'Danger', danger: true, onTap: () {}),
      TextAction(label: 'Large', large: true, onTap: () {}),
    ],
  ),
);

@widgetbook.UseCase(name: 'disabled', type: TextAction, path: _path)
Widget textActionDisabled(BuildContext context) =>
    _pad(const TextAction(label: 'Revoke', onTap: null));

@widgetbook.UseCase(name: 'long text (overflow)', type: TextAction, path: _path)
Widget textActionLong(BuildContext context) => _pad(
  TextAction(
    label: 'Revoke every access key issued to this device',
    onTap: () {},
  ),
  width: 160,
);

@widgetbook.UseCase(name: 'default', type: RowActions, path: _path)
Widget rowActionsDefault(BuildContext context) => _pad(
  RowActions(
    children: [
      TextAction(label: 'Rename', onTap: () {}),
      TextAction(label: 'Revoke', danger: true, onTap: () {}),
    ],
  ),
);

@widgetbook.UseCase(name: 'default', type: MonoValue, path: _path)
Widget monoValueDefault(BuildContext context) => _pad(
  MonoValue(
    context.knobs.string(label: 'text', initialValue: 'stk_••••3f9a'),
    color: context.knobs.boolean(label: 'danger colour')
        ? context.tokens.danger
        : null,
  ),
  width: 240,
);

@widgetbook.UseCase(name: 'long text (overflow)', type: MonoValue, path: _path)
Widget monoValueLong(BuildContext context) => _pad(
  const MonoValue(
    '/home/dewansh/storm/vaults/personal/a/very/deep/folder/path',
  ),
  width: 180,
);

@widgetbook.UseCase(name: 'default', type: SettingsNote, path: _path)
Widget settingsNoteDefault(BuildContext context) => _pad(
  SettingsNote(
    context.knobs.string(
      label: 'text',
      initialValue: 'Edits merge like any other device.',
    ),
    small: context.knobs.boolean(label: 'small'),
  ),
  width: 320,
);

@widgetbook.UseCase(
  name: 'long text (overflow)',
  type: SettingsNote,
  path: _path,
)
Widget settingsNoteLong(BuildContext context) =>
    _pad(SettingsNote(_long * 3), width: 280);

@widgetbook.UseCase(name: 'default', type: InfoBox, path: _path)
Widget infoBoxDefault(BuildContext context) => _pad(
  InfoBox(
    context.knobs.string(
      label: 'text',
      initialValue: 'Serves your vaults to apps that hold an access key.',
    ),
  ),
  width: 320,
);

@widgetbook.UseCase(name: 'long text (overflow)', type: InfoBox, path: _path)
Widget infoBoxLong(BuildContext context) =>
    _pad(InfoBox(_long * 3), width: 280);

@widgetbook.UseCase(name: 'default', type: SettingsMuted, path: _path)
Widget settingsMutedDefault(BuildContext context) => _pad(
  SettingsMuted(
    context.knobs.string(label: 'text', initialValue: 'No access keys yet.'),
    danger: context.knobs.boolean(label: 'danger'),
  ),
);

@widgetbook.UseCase(name: 'error', type: SettingsMuted, path: _path)
Widget settingsMutedError(BuildContext context) =>
    _pad(const SettingsMuted('Could not load your keys.', danger: true));

@widgetbook.UseCase(
  name: 'long text (overflow)',
  type: SettingsMuted,
  path: _path,
)
Widget settingsMutedLong(BuildContext context) =>
    _pad(SettingsMuted(_long * 2), width: 280);

@widgetbook.UseCase(name: 'default', type: SettingsButtonRow, path: _path)
Widget settingsButtonRowDefault(BuildContext context) => _pad(
  SettingsButtonRow(
    top: context.knobs.doubleOrNull.slider(
      label: 'top',
      initialValue: null,
      min: 0,
      max: 48,
    ),
    child: StormButton.outline(label: 'Add a device', onPressed: () {}),
  ),
  width: 320,
);

@widgetbook.UseCase(name: 'default', type: BetaTag, path: _path)
Widget betaTagDefault(BuildContext context) => _pad(const BetaTag());
