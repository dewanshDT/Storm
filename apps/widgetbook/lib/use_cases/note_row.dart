import 'package:flutter/material.dart';
import 'package:flutter_lucide/flutter_lucide.dart';
import 'package:storm/ui/tokens.dart';
import 'package:storm/ui/widgets.dart';
import 'package:widgetbook/widgetbook.dart';
import 'package:widgetbook_annotation/widgetbook_annotation.dart' as widgetbook;

const _path = '[Widgets]';
const _longTitle =
    'Notes from the quarterly planning offsite, including every action item '
    'that nobody has picked up yet';

@widgetbook.UseCase(name: 'default', type: NoteRow, path: _path)
Widget noteRowDefault(BuildContext context) => NoteRow(
  title: context.knobs.string(label: 'title', initialValue: 'Sprint Retro'),
  meta: context.knobs.stringOrNull(
    label: 'meta',
    initialValue: 'Projects · 2h ago',
  ),
  divider: context.knobs.boolean(label: 'divider', initialValue: true),
  selected: context.knobs.boolean(label: 'selected'),
  dense: context.knobs.boolean(label: 'dense'),
  leading: context.knobs.boolean(label: 'leading')
      ? Icon(LucideIcons.file_text, size: 18, color: context.tokens.text2)
      : null,
  trailing: context.knobs.boolean(label: 'trailing')
      ? Icon(LucideIcons.pin, size: 16, color: context.tokens.text3)
      : null,
  onTap: context.knobs.boolean(label: 'tappable', initialValue: true)
      ? () {}
      : null,
);

@widgetbook.UseCase(name: 'selected', type: NoteRow, path: _path)
Widget noteRowSelected(BuildContext context) => NoteRow(
  title: 'Sprint Retro',
  meta: 'Projects · 2h ago',
  selected: true,
  onTap: () {},
);

@widgetbook.UseCase(name: 'dense (sidebar)', type: NoteRow, path: _path)
Widget noteRowDense(BuildContext context) => Padding(
  padding: const EdgeInsets.all(8),
  child: Column(
    children: [
      NoteRow(title: 'Sprint Retro', dense: true, onTap: () {}),
      NoteRow(
        title: 'Weekly review',
        dense: true,
        selected: true,
        onTap: () {},
      ),
    ],
  ),
);

@widgetbook.UseCase(name: 'disabled (no onTap)', type: NoteRow, path: _path)
Widget noteRowDisabled(BuildContext context) => const NoteRow(
  title: 'Sprint Retro',
  meta: 'Projects · 2h ago',
  divider: true,
);

@widgetbook.UseCase(name: 'long text (overflow)', type: NoteRow, path: _path)
Widget noteRowLong(BuildContext context) => NoteRow(
  title: _longTitle,
  meta:
      'A vault with a long name · in a deeply nested folder · edited by an agent',
  divider: true,
  trailing: Icon(LucideIcons.pin, size: 16, color: context.tokens.text3),
  onTap: () {},
);
