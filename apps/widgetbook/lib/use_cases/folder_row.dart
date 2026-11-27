import 'package:flutter/material.dart';
import 'package:storm/ui/widgets.dart';
import 'package:widgetbook/widgetbook.dart';
import 'package:widgetbook_annotation/widgetbook_annotation.dart' as widgetbook;

const _path = '[Widgets]';

@widgetbook.UseCase(name: 'default', type: FolderRow, path: _path)
Widget folderRowDefault(BuildContext context) => FolderRow(
  name: context.knobs.string(label: 'name', initialValue: 'Projects'),
  count: context.knobs.intOrNull.slider(
    label: 'count',
    initialValue: 2,
    min: 0,
    max: 500,
  ),
  divider: context.knobs.boolean(label: 'divider', initialValue: true),
  chevron: context.knobs.boolean(label: 'chevron', initialValue: true),
  dense: context.knobs.boolean(label: 'dense'),
  onTap: context.knobs.boolean(label: 'tappable', initialValue: true)
      ? () {}
      : null,
);

@widgetbook.UseCase(name: 'dense (sidebar)', type: FolderRow, path: _path)
Widget folderRowDense(BuildContext context) =>
    FolderRow(name: 'Projects', count: 12, dense: true, onTap: () {});

@widgetbook.UseCase(name: 'no count, no chevron', type: FolderRow, path: _path)
Widget folderRowBare(BuildContext context) =>
    FolderRow(name: 'Projects', chevron: false, onTap: () {});

@widgetbook.UseCase(name: 'disabled (no onTap)', type: FolderRow, path: _path)
Widget folderRowDisabled(BuildContext context) =>
    const FolderRow(name: 'Projects', count: 2, divider: true);

@widgetbook.UseCase(name: 'long text (overflow)', type: FolderRow, path: _path)
Widget folderRowLong(BuildContext context) => FolderRow(
  name: 'A folder with a name far too long to fit on one line of a phone',
  count: 1204,
  divider: true,
  onTap: () {},
);
