import 'package:flutter/material.dart';
import 'package:storm/ui/panels.dart';
import 'package:widgetbook/widgetbook.dart';
import 'package:widgetbook_annotation/widgetbook_annotation.dart' as widgetbook;

const _path = '[Panels]';

Widget _confirm(
  BuildContext context, {
  required String title,
  required String message,
  required String confirmLabel,
}) => Padding(
  padding: const EdgeInsets.all(16),
  child: InlineConfirm(
    title: context.knobs.stringOrNull(label: 'title', initialValue: title),
    message: context.knobs.string(label: 'message', initialValue: message),
    confirmLabel: context.knobs.string(
      label: 'confirmLabel',
      initialValue: confirmLabel,
    ),
    onConfirm:
        context.knobs.boolean(label: 'confirm enabled', initialValue: true)
        ? () {}
        : null,
    onCancel: () {},
  ),
);

@widgetbook.UseCase(name: 'default', type: InlineConfirm, path: _path)
Widget inlineConfirmDefault(BuildContext context) => _confirm(
  context,
  title: 'Delete note?',
  message: 'It moves to the Trash and can be restored from there.',
  confirmLabel: 'Delete',
);

@widgetbook.UseCase(
  name: 'long text (overflow)',
  type: InlineConfirm,
  path: _path,
)
Widget inlineConfirmLong(BuildContext context) => _confirm(
  context,
  title: 'Remove this device and every session it has opened on the server?',
  message:
      'Every vault synced to it stays on the server, but anything not yet '
      'uploaded from this device is lost, and it has to be paired again '
      'before it can sync.',
  confirmLabel: 'Remove device and sessions',
);
