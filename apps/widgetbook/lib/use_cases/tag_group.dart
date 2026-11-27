import 'package:flutter/material.dart';
import 'package:storm/ui/widgets.dart';
import 'package:widgetbook/widgetbook.dart';
import 'package:widgetbook_annotation/widgetbook_annotation.dart' as widgetbook;

const _path = '[Widgets]';

@widgetbook.UseCase(name: 'default', type: TagGroup, path: _path)
Widget tagGroupDefault(BuildContext context) {
  final count = context.knobs.int.slider(
    label: 'tags',
    initialValue: 2,
    min: 0,
    max: 20,
  );
  return Padding(
    padding: const EdgeInsets.all(16),
    child: TagGroup(
      name: context.knobs.string(label: 'name', initialValue: 'proj'),
      tags: [for (var i = 1; i <= count; i++) 'proj/area-$i'],
      onTagTap: context.knobs.boolean(label: 'tappable') ? (_) {} : null,
    ),
  );
}

@widgetbook.UseCase(name: 'empty', type: TagGroup, path: _path)
Widget tagGroupEmpty(BuildContext context) => const Padding(
  padding: EdgeInsets.all(16),
  child: TagGroup(name: 'proj', tags: []),
);

@widgetbook.UseCase(name: 'many tags (wraps)', type: TagGroup, path: _path)
Widget tagGroupMany(BuildContext context) => Padding(
  padding: const EdgeInsets.all(16),
  child: SizedBox(
    width: 280,
    child: TagGroup(
      name: 'projects',
      tags: [
        for (var i = 1; i <= 14; i++) 'projects/storm/decision-${100 + i}',
      ],
      onTagTap: (_) {},
    ),
  ),
);
