import 'package:flutter/material.dart';
import 'package:flutter_lucide/flutter_lucide.dart';
import 'package:storm/ui/tokens.dart';
import 'package:storm/ui/widgets.dart';
import 'package:widgetbook/widgetbook.dart';
import 'package:widgetbook_annotation/widgetbook_annotation.dart' as widgetbook;

const _path = '[Widgets]';

Widget _header(
  BuildContext context, {
  required String title,
  required int unread,
  int actions = 3,
  double? width,
}) {
  final t = context.tokens;
  final all = [
    IconBadgeButton(
      icon: LucideIcons.check_check,
      tooltip: 'Mark all read',
      dimension: t.sp * 3.25,
      onTap: () {},
    ),
    IconBadgeButton(
      icon: LucideIcons.settings,
      tooltip: 'Notification settings',
      dimension: t.sp * 3.25,
      onTap: () {},
    ),
    IconBadgeButton(
      icon: LucideIcons.ellipsis_vertical,
      tooltip: 'More',
      dimension: t.sp * 3.25,
      onTap: () {},
    ),
  ];
  return Padding(
    padding: const EdgeInsets.all(16),
    child: SizedBox(
      width: width,
      child: InboxHeader(
        title: title,
        unreadCount: unread,
        actions: all.take(actions).toList(),
      ),
    ),
  );
}

@widgetbook.UseCase(name: 'default', type: InboxHeader, path: _path)
Widget inboxHeaderDefault(BuildContext context) => _header(
  context,
  title: context.knobs.string(label: 'title', initialValue: 'Notifications'),
  unread: context.knobs.int.slider(
    label: 'unreadCount',
    initialValue: 3,
    min: 0,
    max: 30,
  ),
  actions: context.knobs.int.slider(
    label: 'actions',
    initialValue: 3,
    min: 0,
    max: 3,
  ),
);

@widgetbook.UseCase(name: 'all read', type: InboxHeader, path: _path)
Widget inboxHeaderRead(BuildContext context) =>
    _header(context, title: 'Notifications', unread: 0);

@widgetbook.UseCase(
  name: 'long text (overflow)',
  type: InboxHeader,
  path: _path,
)
Widget inboxHeaderLong(BuildContext context) => _header(
  context,
  title: 'Everything that needs your attention across every vault and host',
  unread: 12,
  width: 280,
);
