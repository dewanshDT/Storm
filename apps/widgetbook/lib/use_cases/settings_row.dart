import 'package:flutter/material.dart';
import 'package:flutter_lucide/flutter_lucide.dart';
import 'package:storm/ui/controls.dart';
import 'package:storm/ui/tokens.dart';
import 'package:storm_widgetbook/stateful_value.dart';
import 'package:widgetbook/widgetbook.dart';
import 'package:widgetbook_annotation/widgetbook_annotation.dart' as widgetbook;

const _path = '[Controls]';
const _sub = 'Serves your vaults to apps that hold an access key.';

Widget _pad(Widget child, {double? width}) => Padding(
  padding: const EdgeInsets.all(16),
  child: SizedBox(width: width, child: child),
);

@widgetbook.UseCase(name: 'default', type: SettingsRow, path: _path)
Widget settingsRowDefault(BuildContext context) => _pad(
  StatefulValue<bool>(
    initial: context.knobs.boolean(label: 'toggle on', initialValue: true),
    builder: (value, set) => SettingsRow(
      label: context.knobs.string(
        label: 'label',
        initialValue: 'Let AI apps read your notes',
      ),
      sub: context.knobs.stringOrNull(label: 'sub', initialValue: _sub),
      monoSub: context.knobs.boolean(label: 'monoSub'),
      muted: context.knobs.boolean(label: 'muted'),
      large: context.knobs.boolean(label: 'large'),
      divider: context.knobs.boolean(label: 'divider', initialValue: true),
      trailing: StormToggle(value: value, onChanged: set),
    ),
  ),
);

@widgetbook.UseCase(name: 'with action', type: SettingsRow, path: _path)
Widget settingsRowAction(BuildContext context) => _pad(
  SettingsRow(
    label: 'laptop-claude',
    sub: 'stk_••••3f9a · used 1h ago',
    monoSub: true,
    trailing: StormButton.text(label: 'Revoke', onPressed: () {}),
  ),
);

@widgetbook.UseCase(
  name: 'tappable with leading icon',
  type: SettingsRow,
  path: _path,
)
Widget settingsRowTappable(BuildContext context) => _pad(
  SettingsRow(
    label: 'Appearance',
    sub: 'Theme, accent and text size',
    leading: Icon(LucideIcons.palette, size: 18, color: context.tokens.text2),
    trailing: Icon(
      LucideIcons.chevron_right,
      size: 18,
      color: context.tokens.text3,
    ),
    onTap: () {},
  ),
);

@widgetbook.UseCase(name: 'destructive', type: SettingsRow, path: _path)
Widget settingsRowDestructive(BuildContext context) => _pad(
  SettingsRow(
    label: 'Disconnect',
    labelColor: context.tokens.danger,
    onTap: () {},
  ),
);

@widgetbook.UseCase(name: 'muted', type: SettingsRow, path: _path)
Widget settingsRowMuted(BuildContext context) => _pad(
  const SettingsRow(
    label: 'Not available on this device',
    sub: _sub,
    muted: true,
  ),
);

@widgetbook.UseCase(name: 'disabled toggle', type: SettingsRow, path: _path)
Widget settingsRowDisabled(BuildContext context) => _pad(
  const SettingsRow(
    label: 'Let AI apps write',
    sub: 'Turn on reading first.',
    trailing: StormToggle(value: false, onChanged: null),
  ),
);

@widgetbook.UseCase(
  name: 'long text (overflow)',
  type: SettingsRow,
  path: _path,
)
Widget settingsRowLong(BuildContext context) => _pad(
  SettingsRow(
    label: 'A setting whose label is long enough to wrap on a narrow screen',
    sub: '$_sub $_sub',
    trailing: StormButton.text(label: 'Revoke', onPressed: () {}),
  ),
  width: 300,
);
