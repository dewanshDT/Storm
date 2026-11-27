import 'package:flutter/material.dart';
import 'package:flutter_lucide/flutter_lucide.dart';
import 'package:storm/ui/tokens.dart';
import 'package:storm/ui/widgets.dart';
import 'package:widgetbook/widgetbook.dart';
import 'package:widgetbook_annotation/widgetbook_annotation.dart' as widgetbook;

const _path = '[Widgets]';

List<InlineSpan> _title(String name, String rest) => [
  TextSpan(
    text: name,
    style: const TextStyle(fontWeight: FontWeight.w600),
  ),
  TextSpan(text: rest),
];

Widget _quote(StormTokens t) => Container(
  margin: EdgeInsets.only(top: t.sp * 0.375),
  padding: EdgeInsets.only(left: t.sp * 0.875),
  decoration: BoxDecoration(
    border: Border(
      left: BorderSide(color: t.border, width: t.bw * 2),
    ),
  ),
  child: Text(
    'tests failed on the second run',
    style: TextStyle(
      fontFamily: StormTokens.sansFamily,
      fontSize: t.labelSize,
      color: t.text2,
    ),
  ),
);

Widget _heldTag(StormTokens t) => Row(
  mainAxisSize: MainAxisSize.min,
  children: [
    Icon(LucideIcons.moon, size: t.labelSize * 1.1, color: t.text3),
    SizedBox(width: t.sp * 0.375),
    Flexible(
      child: Text(
        'During quiet hours',
        maxLines: 1,
        overflow: TextOverflow.ellipsis,
        style: TextStyle(
          fontFamily: StormTokens.sansFamily,
          fontSize: t.labelSize,
          color: t.text3,
        ),
      ),
    ),
  ],
);

Widget _row(
  BuildContext context, {
  String name = 'tidier',
  String rest = ' needs you',
  String? subtitle = 'Claude Code',
  String? time = '3m',
  bool unread = true,
  bool focused = false,
  bool tag = false,
  bool body = false,
  bool tappable = true,
}) {
  final t = context.tokens;
  return Padding(
    padding: const EdgeInsets.all(8),
    child: NotificationRow(
      leading: KindBadge(
        icon: LucideIcons.circle_question_mark,
        color: t.accent,
      ),
      title: _title(name, rest),
      subtitle: subtitle,
      tag: tag ? _heldTag(t) : null,
      body: body ? _quote(t) : null,
      unread: unread,
      time: time,
      focused: focused,
      onTap: tappable ? () {} : null,
      onMenu: (_) {},
    ),
  );
}

@widgetbook.UseCase(name: 'default', type: NotificationRow, path: _path)
Widget notificationRowDefault(BuildContext context) => _row(
  context,
  name: context.knobs.string(label: 'session', initialValue: 'tidier'),
  rest: context.knobs.string(label: 'title', initialValue: ' needs you'),
  subtitle: context.knobs.stringOrNull(
    label: 'subtitle',
    initialValue: 'Claude Code',
  ),
  time: context.knobs.stringOrNull(label: 'time', initialValue: '3m'),
  unread: context.knobs.boolean(label: 'unread', initialValue: true),
  focused: context.knobs.boolean(label: 'focused'),
  tag: context.knobs.boolean(label: 'held tag'),
  body: context.knobs.boolean(label: 'quoted body'),
  tappable: context.knobs.boolean(label: 'tappable', initialValue: true),
);

@widgetbook.UseCase(name: 'read', type: NotificationRow, path: _path)
Widget notificationRowRead(BuildContext context) =>
    _row(context, unread: false, rest: ' finished its turn');

@widgetbook.UseCase(
  name: 'focused (keyboard ring)',
  type: NotificationRow,
  path: _path,
)
Widget notificationRowFocused(BuildContext context) =>
    _row(context, focused: true);

@widgetbook.UseCase(
  name: 'held tag and quoted body',
  type: NotificationRow,
  path: _path,
)
Widget notificationRowRich(BuildContext context) =>
    _row(context, rest: ' sent a message', tag: true, body: true, time: '1h');

@widgetbook.UseCase(
  name: 'disabled (no onTap)',
  type: NotificationRow,
  path: _path,
)
Widget notificationRowDisabled(BuildContext context) =>
    _row(context, tappable: false);

@widgetbook.UseCase(
  name: 'long text (overflow)',
  type: NotificationRow,
  path: _path,
)
Widget notificationRowLong(BuildContext context) => _row(
  context,
  name: 'a-session-with-an-unreasonably-long-name',
  rest:
      ' is waiting on you to approve a change to a file whose path is long '
      'enough that the title has to wrap and then clip at two lines',
  subtitle: 'Claude Code running on the build host in the homelab rack',
  tag: true,
  body: true,
);

Widget _group(
  BuildContext context, {
  bool expanded = false,
  bool unread = false,
  bool focused = false,
  String name = 'build-vm',
  String subtitle = 'latest: finished its turn · Claude Code',
}) {
  final t = context.tokens;
  return Padding(
    padding: const EdgeInsets.all(8),
    child: NotificationGroupRow(
      leading: KindBadge(icon: LucideIcons.circle_check_big, color: t.green),
      title: _title(name, ' · 3 updates'),
      subtitle: subtitle,
      unread: unread,
      expanded: expanded,
      focused: focused,
      onToggle: () {},
    ),
  );
}

@widgetbook.UseCase(name: 'default', type: NotificationGroupRow, path: _path)
Widget notificationGroupRowDefault(BuildContext context) => _group(
  context,
  name: context.knobs.string(label: 'session', initialValue: 'build-vm'),
  subtitle: context.knobs.string(
    label: 'subtitle',
    initialValue: 'latest: finished its turn · Claude Code',
  ),
  expanded: context.knobs.boolean(label: 'expanded'),
  unread: context.knobs.boolean(label: 'unread'),
  focused: context.knobs.boolean(label: 'focused'),
);

@widgetbook.UseCase(name: 'expanded', type: NotificationGroupRow, path: _path)
Widget notificationGroupRowExpanded(BuildContext context) =>
    _group(context, expanded: true);

@widgetbook.UseCase(name: 'unread', type: NotificationGroupRow, path: _path)
Widget notificationGroupRowUnread(BuildContext context) =>
    _group(context, unread: true);

@widgetbook.UseCase(
  name: 'long text (overflow)',
  type: NotificationGroupRow,
  path: _path,
)
Widget notificationGroupRowLong(BuildContext context) => _group(
  context,
  name: 'a-session-with-an-unreasonably-long-name-for-a-build-host',
  subtitle:
      'latest: needs you to approve a change to a file whose path is long · Claude Code',
);
