import 'package:flutter/material.dart';
import 'package:storm/ui/widgets.dart';
import 'package:widgetbook/widgetbook.dart';
import 'package:widgetbook_annotation/widgetbook_annotation.dart' as widgetbook;

const _path = '[Widgets]';

Widget _bar({
  int version = 12,
  String label = 'Saved',
  SaveTone tone = SaveTone.good,
  String? error,
  double? width,
}) => Padding(
  padding: const EdgeInsets.all(16),
  child: SizedBox(
    width: width,
    child: StatusBar(version: version, label: label, tone: tone, error: error),
  ),
);

@widgetbook.UseCase(name: 'default', type: StatusBar, path: _path)
Widget statusBarDefault(BuildContext context) => _bar(
  version: context.knobs.int.slider(
    label: 'version',
    initialValue: 12,
    min: 1,
    max: 9999,
  ),
  label: context.knobs.string(label: 'label', initialValue: 'Saved'),
  tone: context.knobs.object.dropdown<SaveTone>(
    label: 'tone',
    options: SaveTone.values,
    initialOption: SaveTone.good,
    labelBuilder: (t) => t.name,
  ),
  error: context.knobs.stringOrNull(label: 'error'),
);

@widgetbook.UseCase(name: 'saving', type: StatusBar, path: _path)
Widget statusBarSaving(BuildContext context) =>
    _bar(label: 'Saving…', tone: SaveTone.working);

@widgetbook.UseCase(name: 'queued offline', type: StatusBar, path: _path)
Widget statusBarQueued(BuildContext context) =>
    _bar(label: 'Queued — offline', tone: SaveTone.waiting);

@widgetbook.UseCase(name: 'error', type: StatusBar, path: _path)
Widget statusBarError(BuildContext context) => _bar(
  label: 'Failed',
  tone: SaveTone.bad,
  error: 'The server rejected this edit',
);

@widgetbook.UseCase(name: 'long text (overflow)', type: StatusBar, path: _path)
Widget statusBarLong(BuildContext context) => _bar(
  version: 123456,
  label: 'Failed',
  tone: SaveTone.bad,
  error:
      'The server rejected this edit because another device saved a newer version first',
  width: 300,
);
