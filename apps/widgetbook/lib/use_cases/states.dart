import 'package:flutter/material.dart';
import 'package:flutter_lucide/flutter_lucide.dart';
import 'package:storm/ui/controls.dart';
import 'package:storm/ui/states.dart';
import 'package:widgetbook/widgetbook.dart';
import 'package:widgetbook_annotation/widgetbook_annotation.dart' as widgetbook;

const _path = '[States]';

Widget _empty(
  BuildContext context, {
  String title = 'Nothing in this folder',
  String? detail = 'New notes made here will land in Projects.',
  String? action = 'New note',
  List<NumberedStep>? steps,
  int step = 0,
  bool fill = false,
}) => EmptyState(
  icon: LucideIcons.folder_open,
  title: title,
  detail: detail,
  action: action,
  onAction: action == null ? null : () {},
  steps: steps,
  currentStep: step,
  fill: fill,
);

@widgetbook.UseCase(name: 'default', type: EmptyState, path: _path)
Widget emptyStateDefault(BuildContext context) => _empty(
  context,
  title: context.knobs.string(
    label: 'title',
    initialValue: 'Nothing in this folder',
  ),
  detail: context.knobs.stringOrNull(
    label: 'detail',
    initialValue: 'New notes made here will land in Projects.',
  ),
  action: context.knobs.stringOrNull(label: 'action', initialValue: 'New note'),
  fill: context.knobs.boolean(label: 'fill'),
);

@widgetbook.UseCase(name: 'title only', type: EmptyState, path: _path)
Widget emptyStateTitleOnly(BuildContext context) =>
    _empty(context, detail: null, action: null);

@widgetbook.UseCase(name: 'with steps', type: EmptyState, path: _path)
Widget emptyStateSteps(BuildContext context) => _empty(
  context,
  title: 'No sessions yet',
  detail: 'Three steps to your first agent session.',
  action: null,
  step: context.knobs.int.slider(
    label: 'currentStep',
    initialValue: 0,
    min: 0,
    max: 2,
  ),
  steps: const [
    NumberedStep('Enroll a host'),
    NumberedStep('Sign Claude Code or OpenCode in on that host'),
    NumberedStep('Start a session'),
  ],
);

@widgetbook.UseCase(name: 'long text (overflow)', type: EmptyState, path: _path)
Widget emptyStateLong(BuildContext context) => _empty(
  context,
  title:
      'There is nothing in this folder yet, and there has not been for a while',
  detail:
      'New notes made here will land in a folder with a name long enough that '
      'this sentence has to wrap across several lines on a phone.',
  action: 'Create the first note in this folder',
);

@widgetbook.UseCase(name: 'default (loading)', type: SkeletonRows, path: _path)
Widget skeletonRowsDefault(BuildContext context) => Padding(
  padding: const EdgeInsets.all(16),
  child: SkeletonRows(
    rows: context.knobs.int.slider(
      label: 'rows',
      initialValue: 4,
      min: 0,
      max: 12,
    ),
  ),
);

@widgetbook.UseCase(name: 'default', type: OfflineNotice, path: _path)
Widget offlineNoticeDefault(BuildContext context) => OfflineNotice(
  queued: context.knobs.int.slider(
    label: 'queued',
    initialValue: 0,
    min: 0,
    max: 20,
  ),
  onRetry: context.knobs.boolean(label: 'retry') ? () {} : null,
);

@widgetbook.UseCase(name: 'one edit queued', type: OfflineNotice, path: _path)
Widget offlineNoticeOne(BuildContext context) => const OfflineNotice(queued: 1);

@widgetbook.UseCase(
  name: 'edits queued, retry',
  type: OfflineNotice,
  path: _path,
)
Widget offlineNoticeQueued(BuildContext context) =>
    OfflineNotice(queued: 14, onRetry: () {});

@widgetbook.UseCase(name: 'default', type: ConflictCard, path: _path)
Widget conflictCardDefault(BuildContext context) => ConflictCard(
  onDismiss: context.knobs.boolean(label: 'dismissible', initialValue: true)
      ? () {}
      : null,
);

@widgetbook.UseCase(name: 'not dismissible', type: ConflictCard, path: _path)
Widget conflictCardFixed(BuildContext context) => const ConflictCard();
