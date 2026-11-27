import 'package:flutter/material.dart';
import 'package:storm/ui/controls.dart';
import 'package:widgetbook/widgetbook.dart';
import 'package:widgetbook_annotation/widgetbook_annotation.dart' as widgetbook;

const _path = '[Controls]';
const _longLabel =
    'Reconnect to the homelab server and resync every vault on this device';

Widget _button(BuildContext context, {StormButtonKind? kind}) {
  final k =
      kind ??
      context.knobs.object.dropdown<StormButtonKind>(
        label: 'kind',
        options: StormButtonKind.values,
        initialOption: StormButtonKind.primary,
        labelBuilder: (k) => k.name,
      );
  final enabled = context.knobs.boolean(label: 'enabled', initialValue: true);
  final expand = context.knobs.boolean(label: 'expand');
  final withIcon = context.knobs.boolean(label: 'icon');
  return Padding(
    padding: const EdgeInsets.all(16),
    child: StormButton(
      label: context.knobs.string(label: 'label', initialValue: 'Connect'),
      kind: k,
      icon: withIcon ? Icons.cloud_outlined : null,
      expand: expand,
      onPressed: enabled ? () {} : null,
    ),
  );
}

@widgetbook.UseCase(name: 'default', type: StormButton, path: _path)
Widget stormButtonDefault(BuildContext context) => _button(context);

@widgetbook.UseCase(name: 'primary', type: StormButton, path: _path)
Widget stormButtonPrimary(BuildContext context) =>
    _button(context, kind: StormButtonKind.primary);

@widgetbook.UseCase(name: 'outline', type: StormButton, path: _path)
Widget stormButtonOutline(BuildContext context) =>
    _button(context, kind: StormButtonKind.outline);

@widgetbook.UseCase(name: 'soft', type: StormButton, path: _path)
Widget stormButtonSoft(BuildContext context) =>
    _button(context, kind: StormButtonKind.soft);

@widgetbook.UseCase(name: 'danger (error)', type: StormButton, path: _path)
Widget stormButtonDanger(BuildContext context) =>
    _button(context, kind: StormButtonKind.danger);

@widgetbook.UseCase(name: 'text', type: StormButton, path: _path)
Widget stormButtonText(BuildContext context) =>
    _button(context, kind: StormButtonKind.text);

@widgetbook.UseCase(name: 'disabled', type: StormButton, path: _path)
Widget stormButtonDisabled(BuildContext context) => Padding(
  padding: const EdgeInsets.all(16),
  child: Wrap(
    spacing: 12,
    runSpacing: 12,
    children: [
      for (final kind in StormButtonKind.values)
        StormButton(label: kind.name, kind: kind, onPressed: null),
    ],
  ),
);

@widgetbook.UseCase(
  name: 'long label (overflow)',
  type: StormButton,
  path: _path,
)
Widget stormButtonLongLabel(BuildContext context) => Padding(
  padding: const EdgeInsets.all(16),
  child: SizedBox(
    width: 240,
    child: StormButton(
      label: context.knobs.string(label: 'label', initialValue: _longLabel),
      icon: Icons.cloud_outlined,
      expand: true,
      onPressed: () {},
    ),
  ),
);
