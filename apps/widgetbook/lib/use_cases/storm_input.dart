import 'package:flutter/material.dart';
import 'package:flutter_lucide/flutter_lucide.dart';
import 'package:storm/ui/widgets.dart';
import 'package:widgetbook/widgetbook.dart';
import 'package:widgetbook_annotation/widgetbook_annotation.dart' as widgetbook;

const _path = '[Controls]';

Widget _pad(Widget child) =>
    Padding(padding: const EdgeInsets.all(16), child: child);

@widgetbook.UseCase(name: 'default', type: StormInput, path: _path)
Widget stormInputDefault(BuildContext context) => _pad(
  StormInput(
    hintText: context.knobs.stringOrNull(
      label: 'hintText',
      initialValue: 'Search notes',
    ),
    labelText: context.knobs.stringOrNull(label: 'labelText'),
    prefixIcon: context.knobs.boolean(label: 'prefixIcon')
        ? LucideIcons.search
        : null,
    obscureText: context.knobs.boolean(label: 'obscureText'),
    enabled: context.knobs.boolean(label: 'enabled', initialValue: true),
    minLines: context.knobs.int.slider(
      label: 'minLines',
      initialValue: 1,
      min: 1,
      max: 6,
    ),
    maxLines: context.knobs.int.slider(
      label: 'maxLines',
      initialValue: 1,
      min: 1,
      max: 8,
    ),
  ),
);

@widgetbook.UseCase(name: 'with text', type: StormInput, path: _path)
Widget stormInputText(BuildContext context) => _pad(
  StormInput(
    controller: TextEditingController(text: 'Sprint retro notes'),
    labelText: 'Title',
  ),
);

@widgetbook.UseCase(name: 'disabled', type: StormInput, path: _path)
Widget stormInputDisabled(BuildContext context) => _pad(
  const StormInput(
    hintText: 'Search notes',
    prefixIcon: LucideIcons.search,
    enabled: false,
  ),
);

@widgetbook.UseCase(name: 'multiline', type: StormInput, path: _path)
Widget stormInputMultiline(BuildContext context) => _pad(
  const StormInput(hintText: 'What happened?', minLines: 3, maxLines: 6),
);

@widgetbook.UseCase(name: 'password (obscured)', type: StormInput, path: _path)
Widget stormInputObscured(BuildContext context) => _pad(
  StormInput(
    controller: TextEditingController(text: 'hunter2'),
    labelText: 'Password',
    obscureText: true,
  ),
);

@widgetbook.UseCase(name: 'long text (overflow)', type: StormInput, path: _path)
Widget stormInputLong(BuildContext context) => _pad(
  SizedBox(
    width: 220,
    child: StormInput(
      controller: TextEditingController(
        text:
            'A single line of text far longer than the field is wide, which '
            'has to scroll sideways rather than grow the box',
      ),
      labelText: 'A label that is also rather long for this field',
      prefixIcon: LucideIcons.search,
    ),
  ),
);
