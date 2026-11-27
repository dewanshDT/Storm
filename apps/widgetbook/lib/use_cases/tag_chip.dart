import 'package:flutter/material.dart';
import 'package:storm/ui/widgets.dart';
import 'package:widgetbook/widgetbook.dart';
import 'package:widgetbook_annotation/widgetbook_annotation.dart' as widgetbook;

const _path = '[Widgets]';
const _long = '#projects/storm/decisions/164-widgetbook-component-catalogue';

@widgetbook.UseCase(name: 'default', type: TagChip, path: _path)
Widget tagChipDefault(BuildContext context) => Padding(
  padding: const EdgeInsets.all(16),
  child: TagChip(
    label: context.knobs.string(label: 'label', initialValue: '#proj/storm'),
    onTap: context.knobs.boolean(label: 'tappable') ? () {} : null,
  ),
);

@widgetbook.UseCase(name: 'tappable', type: TagChip, path: _path)
Widget tagChipTappable(BuildContext context) => Padding(
  padding: const EdgeInsets.all(16),
  child: TagChip(label: '#cooking', onTap: () {}),
);

@widgetbook.UseCase(name: 'long text (overflow)', type: TagChip, path: _path)
Widget tagChipLong(BuildContext context) => Padding(
  padding: const EdgeInsets.all(16),
  child: SizedBox(
    width: 200,
    child: TagChip(
      label: context.knobs.string(label: 'label', initialValue: _long),
      onTap: () {},
    ),
  ),
);

@widgetbook.UseCase(name: 'default', type: KeyChip, path: _path)
Widget keyChipDefault(BuildContext context) => Padding(
  padding: const EdgeInsets.all(16),
  child: KeyChip(
    label: context.knobs.string(label: 'label', initialValue: 'status'),
  ),
);

@widgetbook.UseCase(name: 'long text (overflow)', type: KeyChip, path: _path)
Widget keyChipLong(BuildContext context) => const Padding(
  padding: EdgeInsets.all(16),
  child: SizedBox(
    width: 160,
    child: KeyChip(label: 'a_frontmatter_key_that_runs_on_and_on'),
  ),
);
