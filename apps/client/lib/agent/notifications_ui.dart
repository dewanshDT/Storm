/// Decision 161, slice 161-3: the bell, its unread badge, and the inbox
/// panel shared by the desktop popover and the phone sheet.
library;

import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_lucide/flutter_lucide.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';

import '../router.dart';
import '../ui/settings/settings_widgets.dart'
    show TextAction, confirmSetting, settingsToast;
import '../ui/states.dart' show describeFailure;
import '../ui/surfaces.dart';
import '../ui/tokens.dart';
import '../ui/widgets.dart' show IconBadgeButton, SectionLabel, UnreadRow;
import 'agent_models.dart' show providerLabel;
import 'notifications_models.dart';
import 'notifications_state.dart';
import 'sidebar_rules.dart' show agoWords;

const _kHeldReasons = {
  'quiet': 'Held: quiet hours',
  'paused': 'Held: paused',
  'off': 'Held: off',
  'muted': 'Held: muted',
};

/// "From {session} ({Agent})", plus " via its terminal" for a terminal
/// notice; null for Storm's own.
String? notificationSourceLine(AgentNotification n) {
  if (n.source == 'storm') return null;
  final name = n.sessionTitle ?? 'a session';
  final agent = n.provider == null ? 'Agent' : providerLabel(n.provider!);
  final suffix = n.source == 'terminal' ? ' via its terminal' : '';
  return 'From $name ($agent)$suffix';
}

String notificationAgo(AgentNotification n) {
  final created = DateTime.tryParse(n.created);
  if (created == null) return '';
  return agoWords(created.toLocal(), DateTime.now());
}

/// Runs [action], showing any failure rather than swallowing it.
Future<void> _guard(
  BuildContext context,
  Future<void> Function() action,
) async {
  try {
    await action();
  } catch (e) {
    if (context.mounted) settingsToast(context, describeFailure(e));
  }
}

/// The inbox's content, shared by the desktop's [NotificationBellButton]
/// popover and the phone's sheet (opened via [openNotificationsSheet] from
/// the place picker).
class NotificationsPanel extends ConsumerWidget {
  const NotificationsPanel({super.key, this.onOpened});

  /// Called after a row or an action is tapped, to let the caller close the
  /// popover or sheet it is shown in.
  final VoidCallback? onOpened;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final t = context.tokens;
    final state = ref.watch(notificationsStateProvider);
    if (state == null) return const SizedBox.shrink();
    return ListenableBuilder(
      listenable: state,
      builder: (context, _) {
        if (state.unsupported) return const SizedBox.shrink();
        final items = state.items;
        return ConstrainedBox(
          constraints: BoxConstraints(maxHeight: t.sp * 40),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              Padding(
                padding: EdgeInsets.fromLTRB(
                  t.sp * 0.75,
                  t.sp * 0.5,
                  t.sp * 0.75,
                  t.sp * 0.5,
                ),
                child: Row(
                  children: [
                    const Expanded(child: SectionLabel('Notifications')),
                    if (state.unreadCount > 0)
                      TextAction(
                        key: const Key('notify-mark-all-read'),
                        label: 'Mark all read',
                        onTap: () => _guard(context, state.markAllRead),
                      ),
                  ],
                ),
              ),
              if (state.loading)
                Padding(
                  padding: EdgeInsets.symmetric(vertical: t.sp * 2.5),
                  child: const Center(
                    child: SizedBox.square(
                      dimension: 20,
                      child: CircularProgressIndicator(strokeWidth: 2),
                    ),
                  ),
                )
              else if (state.backfillFailed)
                Padding(
                  padding: EdgeInsets.symmetric(
                    horizontal: t.sp * 0.75,
                    vertical: t.sp * 1.5,
                  ),
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Text(
                        "Couldn't load notifications.",
                        style: TextStyle(
                          fontFamily: StormTokens.sansFamily,
                          fontSize: t.codeSize,
                          color: t.text3,
                        ),
                      ),
                      SizedBox(height: t.sp * 0.75),
                      TextAction(
                        key: const Key('notify-retry'),
                        label: 'Retry',
                        onTap: () => _guard(context, state.refresh),
                      ),
                    ],
                  ),
                )
              else if (items.isEmpty)
                Padding(
                  padding: EdgeInsets.symmetric(
                    horizontal: t.sp * 0.75,
                    vertical: t.sp * 1.5,
                  ),
                  child: Text(
                    'Nothing yet.',
                    style: TextStyle(
                      fontFamily: StormTokens.sansFamily,
                      fontSize: t.codeSize,
                      color: t.text3,
                    ),
                  ),
                )
              else
                Flexible(
                  child: ListView(
                    shrinkWrap: true,
                    children: [
                      for (final n in items)
                        _InboxRow(
                          key: Key('notify-row-${n.id}'),
                          notification: n,
                          onOpened: onOpened,
                        ),
                    ],
                  ),
                ),
              const PopoverDivider(),
              Padding(
                padding: EdgeInsets.symmetric(horizontal: t.sp * 0.75),
                child: Align(
                  alignment: Alignment.centerLeft,
                  child: TextAction(
                    key: const Key('notify-clear-inbox-popover'),
                    danger: true,
                    label: 'Clear inbox',
                    onTap: () async {
                      final ok = await confirmSetting(
                        context,
                        title: 'Clear the inbox?',
                        body:
                            'Every notice is removed for every device. This '
                            'cannot be undone.',
                        action: 'Clear',
                      );
                      if (ok && context.mounted) {
                        await _guard(context, state.clear);
                      }
                    },
                  ),
                ),
              ),
            ],
          ),
        );
      },
    );
  }
}

class _InboxRow extends ConsumerWidget {
  const _InboxRow({super.key, required this.notification, this.onOpened});

  final AgentNotification notification;
  final VoidCallback? onOpened;

  void _open(BuildContext context, WidgetRef ref) {
    onOpened?.call();
    final n = notification;
    final router = GoRouter.maybeOf(context);
    unawaited(
      _guard(
        context,
        () async => ref.read(notificationsStateProvider)?.markRead([n.id]),
      ),
    );
    if (n.sessionId != null) {
      router?.go(Routes.agentSession(n.sessionId!));
    } else if (n.noteVaultId != null && n.noteId != null) {
      router?.go(Routes.note(n.noteVaultId!, n.noteId!));
    }
  }

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final t = context.tokens;
    final n = notification;
    final source = notificationSourceLine(n);
    final held = n.silentReason == null ? null : _kHeldReasons[n.silentReason];
    final canOpen =
        n.sessionId != null || (n.noteVaultId != null && n.noteId != null);
    return UnreadRow(
      unread: n.unread,
      onTap: canOpen ? () => _open(context, ref) : null,
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          if (source != null) ...[
            Text(
              source,
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: TextStyle(
                fontFamily: StormTokens.sansFamily,
                fontSize: t.labelSize * 1.09,
                fontWeight: FontWeight.w600,
                color: t.text2,
              ),
            ),
            SizedBox(height: t.sp * 0.25),
          ],
          Text(
            n.title,
            style: TextStyle(
              fontFamily: StormTokens.sansFamily,
              fontSize: t.codeSize,
              color: t.text,
            ),
          ),
          if (n.body != null) ...[
            SizedBox(height: t.sp * 0.25),
            Text(
              n.body!,
              maxLines: 3,
              overflow: TextOverflow.ellipsis,
              style: TextStyle(
                fontFamily: StormTokens.sansFamily,
                fontSize: t.labelSize * 1.09,
                color: t.text3,
              ),
            ),
          ],
          SizedBox(height: t.sp * 0.25),
          Row(
            children: [
              Text(
                notificationAgo(n),
                style: TextStyle(
                  fontFamily: StormTokens.monoFamily,
                  fontSize: t.labelSize,
                  color: t.text3,
                ),
              ),
              if (held != null) ...[
                SizedBox(width: t.sp),
                Text(
                  held,
                  style: TextStyle(
                    fontFamily: StormTokens.sansFamily,
                    fontSize: t.labelSize,
                    color: t.text3,
                  ),
                ),
              ],
              const Spacer(),
              if (n.sessionId != null)
                TextAction(
                  key: Key('notify-mute-row-${n.id}'),
                  label: 'Mute this session',
                  onTap: () => _mute(context, ref, n.sessionId!),
                ),
            ],
          ),
        ],
      ),
    );
  }

  Future<void> _mute(
    BuildContext context,
    WidgetRef ref,
    String sessionId,
  ) async {
    try {
      await ref
          .read(notificationsStateProvider)
          ?.createMute('session', sessionId);
    } catch (e) {
      if (context.mounted) settingsToast(context, describeFailure(e));
    }
  }
}

/// The desktop app bar's bell, beside [BarHealthDot] (161-3).
class NotificationBellButton extends ConsumerStatefulWidget {
  const NotificationBellButton({super.key});

  @override
  ConsumerState<NotificationBellButton> createState() =>
      _NotificationBellButtonState();
}

class _NotificationBellButtonState
    extends ConsumerState<NotificationBellButton> {
  final _anchor = GlobalKey();

  @override
  Widget build(BuildContext context) {
    final state = ref.watch(notificationsStateProvider);
    if (state == null) return const SizedBox.shrink();
    return ListenableBuilder(
      listenable: state,
      builder: (context, _) {
        if (state.unsupported) return const SizedBox.shrink();
        return IconBadgeButton(
          key: const Key('bar-notifications'),
          anchorKey: _anchor,
          icon: LucideIcons.bell,
          tooltip: 'Notifications',
          count: state.unreadCount,
          onTap: _open,
        );
      },
    );
  }

  Future<void> _open() async {
    final t = context.tokens;
    await showStormPopover<void>(
      context: context,
      anchorKey: _anchor,
      width: t.sp * 40,
      alignRight: true,
      builder: (popContext) => StormPopover(
        padding: 0,
        children: [
          NotificationsPanel(
            onOpened: () => Navigator.maybeOf(popContext)?.pop(),
          ),
        ],
      ),
    );
  }
}

/// Opens the inbox as a sheet — the phone's way in from the place picker
/// (161-3's review: reachable from every screen, not only a corner bubble
/// gated on having no nav pill).
Future<void> openNotificationsSheet(BuildContext context) =>
    showStormSheet<void>(
      context: context,
      title: 'Notifications',
      builder: (sheetContext) => NotificationsPanel(
        onOpened: () => Navigator.maybeOf(sheetContext)?.pop(),
      ),
    );
