import 'package:flutter/material.dart';
import 'package:storm/ui/session_status.dart';
import 'package:storm/ui/tokens.dart';
import 'package:widgetbook/widgetbook.dart';
import 'package:widgetbook_annotation/widgetbook_annotation.dart' as widgetbook;

const _path = '[Widgets]';

const _statuses = [
  'creating',
  'starting',
  'running',
  'parked',
  'unknown',
  'completed',
  'stopped',
  'failed',
];

String _status(BuildContext context) => context.knobs.object.dropdown<String>(
  label: 'status',
  options: _statuses,
  initialOption: 'running',
);

@widgetbook.UseCase(name: 'default', type: SessionStatusDot, path: _path)
Widget sessionStatusDotDefault(BuildContext context) => Padding(
  padding: const EdgeInsets.all(16),
  child: SessionStatusDot(
    status: _status(context),
    size: context.knobs.doubleOrNull.slider(
      label: 'size',
      initialValue: 8,
      min: 4,
      max: 40,
    ),
  ),
);

@widgetbook.UseCase(name: 'every status', type: SessionStatusDot, path: _path)
Widget sessionStatusDotAll(BuildContext context) => Padding(
  padding: const EdgeInsets.all(16),
  child: Wrap(
    spacing: 16,
    runSpacing: 12,
    crossAxisAlignment: WrapCrossAlignment.center,
    children: [
      for (final s in _statuses)
        Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            SessionStatusDot(status: s, size: 12),
            const SizedBox(width: 8),
            Text(s),
          ],
        ),
    ],
  ),
);

@widgetbook.UseCase(name: 'default', type: SessionStatusChip, path: _path)
Widget sessionStatusChipDefault(BuildContext context) => Padding(
  padding: const EdgeInsets.all(16),
  child: SessionStatusChip(status: _status(context)),
);

@widgetbook.UseCase(name: 'every status', type: SessionStatusChip, path: _path)
Widget sessionStatusChipAll(BuildContext context) => Padding(
  padding: const EdgeInsets.all(16),
  child: Wrap(
    spacing: 12,
    runSpacing: 12,
    children: [for (final s in _statuses) SessionStatusChip(status: s)],
  ),
);

@widgetbook.UseCase(
  name: 'unrecognised status',
  type: SessionStatusChip,
  path: _path,
)
Widget sessionStatusChipUnknown(BuildContext context) => const Padding(
  padding: EdgeInsets.all(16),
  child: SessionStatusChip(status: 'waiting-for-quota'),
);

@widgetbook.UseCase(name: 'default', type: StatusPill, path: _path)
Widget statusPillDefault(BuildContext context) {
  final t = context.tokens;
  return Padding(
    padding: const EdgeInsets.all(16),
    child: StatusPill(
      label: context.knobs.string(label: 'label', initialValue: 'Connected'),
      color: t.green,
      dot: context.knobs.boolean(label: 'dot', initialValue: true)
          ? Icon(Icons.circle, size: 8, color: t.green)
          : null,
    ),
  );
}

@widgetbook.UseCase(name: 'error', type: StatusPill, path: _path)
Widget statusPillError(BuildContext context) {
  final t = context.tokens;
  return Padding(
    padding: const EdgeInsets.all(16),
    child: StatusPill(
      label: 'Unreachable',
      color: t.danger,
      dot: Icon(Icons.circle, size: 8, color: t.danger),
    ),
  );
}
