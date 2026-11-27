import 'package:flutter/material.dart';
import 'package:storm/ui/controls.dart';
import 'package:widgetbook/widgetbook.dart';
import 'package:widgetbook_annotation/widgetbook_annotation.dart' as widgetbook;

const _path = '[Controls]';

Widget _steps(
  List<NumberedStep> steps, {
  int current = 0,
  bool inline = false,
  double? width,
}) => Padding(
  padding: const EdgeInsets.all(16),
  child: SizedBox(
    width: width,
    child: NumberedSteps(steps: steps, current: current, inline: inline),
  ),
);

List<NumberedStep> _three() => [
  NumberedStep(
    'Enroll a host',
    action: StormButton.primary(label: 'Enroll a host', onPressed: () {}),
  ),
  const NumberedStep('Sign Claude Code or OpenCode in on that host'),
  const NumberedStep('Start a session'),
];

@widgetbook.UseCase(name: 'default', type: NumberedSteps, path: _path)
Widget numberedStepsDefault(BuildContext context) => _steps(
  _three(),
  current: context.knobs.int.slider(
    label: 'current',
    initialValue: 0,
    min: 0,
    max: 2,
  ),
  inline: context.knobs.boolean(label: 'inline'),
);

@widgetbook.UseCase(name: 'inline action', type: NumberedSteps, path: _path)
Widget numberedStepsInline(BuildContext context) =>
    _steps(_three(), inline: true);

@widgetbook.UseCase(name: 'empty', type: NumberedSteps, path: _path)
Widget numberedStepsEmpty(BuildContext context) => _steps(const []);

@widgetbook.UseCase(
  name: 'long text (overflow)',
  type: NumberedSteps,
  path: _path,
)
Widget numberedStepsLong(BuildContext context) => _steps([
  NumberedStep(
    'Enroll the host by running the one-line installer on it and then '
    'copying the pairing code it prints back into this window',
    action: StormButton.primary(label: 'Enroll a host', onPressed: () {}),
  ),
  const NumberedStep(
    'Sign Claude Code or OpenCode in on that host, using whichever account '
    'you want its sessions to run under',
  ),
], width: 280);
