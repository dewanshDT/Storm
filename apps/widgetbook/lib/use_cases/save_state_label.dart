import 'package:flutter/material.dart';
import 'package:storm/ui/widgets.dart';
import 'package:widgetbook/widgetbook.dart';
import 'package:widgetbook_annotation/widgetbook_annotation.dart' as widgetbook;

const _path = '[Widgets]';

const _labels = {
  SaveTone.good: 'Saved',
  SaveTone.working: 'Saving…',
  SaveTone.waiting: 'Queued — offline',
  SaveTone.bad: 'Failed',
};

@widgetbook.UseCase(name: 'default', type: SaveStateLabel, path: _path)
Widget saveStateLabelDefault(BuildContext context) {
  final tone = context.knobs.object.dropdown<SaveTone>(
    label: 'tone',
    options: SaveTone.values,
    initialOption: SaveTone.good,
    labelBuilder: (t) => t.name,
  );
  return Padding(
    padding: const EdgeInsets.all(16),
    child: SaveStateLabel(
      label: context.knobs.string(label: 'label', initialValue: _labels[tone]!),
      tone: tone,
    ),
  );
}

@widgetbook.UseCase(name: 'every tone', type: SaveStateLabel, path: _path)
Widget saveStateLabelTones(BuildContext context) => Padding(
  padding: const EdgeInsets.all(16),
  child: Column(
    crossAxisAlignment: CrossAxisAlignment.start,
    children: [
      for (final e in _labels.entries)
        SaveStateLabel(label: e.value, tone: e.key),
    ],
  ),
);

@widgetbook.UseCase(
  name: 'long text (overflow)',
  type: SaveStateLabel,
  path: _path,
)
Widget saveStateLabelLong(BuildContext context) => const Padding(
  padding: EdgeInsets.all(16),
  child: SizedBox(
    width: 160,
    child: SaveStateLabel(
      label: 'Queued — 14 edits waiting for the server to come back',
      tone: SaveTone.waiting,
    ),
  ),
);
