import 'package:flutter/material.dart';
import 'package:storm/ui/tokens.dart';
import 'package:storm/ui/widgets.dart';
import 'package:widgetbook/widgetbook.dart';
import 'package:widgetbook_annotation/widgetbook_annotation.dart' as widgetbook;

const _path = '[Widgets]';

Widget _text(BuildContext context, String s) =>
    Text(s, style: TextStyle(color: context.tokens.text));

@widgetbook.UseCase(name: 'default', type: UnreadRow, path: _path)
Widget unreadRowDefault(BuildContext context) => UnreadRow(
  unread: context.knobs.boolean(label: 'unread', initialValue: true),
  onTap: context.knobs.boolean(label: 'tappable', initialValue: true)
      ? () {}
      : null,
  child: _text(
    context,
    context.knobs.string(
      label: 'text',
      initialValue: 'tidier finished its turn',
    ),
  ),
);

@widgetbook.UseCase(name: 'read', type: UnreadRow, path: _path)
Widget unreadRowRead(BuildContext context) =>
    UnreadRow(unread: false, onTap: () {}, child: _text(context, 'A read row'));

@widgetbook.UseCase(name: 'disabled (no onTap)', type: UnreadRow, path: _path)
Widget unreadRowDisabled(BuildContext context) =>
    UnreadRow(unread: true, child: _text(context, 'A row with no onTap'));

@widgetbook.UseCase(name: 'long text (overflow)', type: UnreadRow, path: _path)
Widget unreadRowLong(BuildContext context) => UnreadRow(
  unread: true,
  onTap: () {},
  child: _text(
    context,
    'Every vault synced to this device stays on the server, but anything not '
    'yet uploaded is lost, and the device has to be paired again before it '
    'can sync. A_single_unbroken_token_with_no_spaces_to_wrap_at_all_in_it',
  ),
);
