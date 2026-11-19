/// Decision 161, slice 161-3, redesigned: the bell, its unread badge, and
/// the inbox panel shared by the desktop popover and the phone sheet.
///
/// The flat, identical-row list that shipped with 161-3 is replaced by: a
/// proper popover/sheet surface, a header with real actions (no more a red
/// "Clear inbox" link), an All/Unread filter, day sections, same-session
/// grouping, a leading kind icon per notice, held reasons as readable tags,
/// and ↑/↓/Enter/Esc keyboard navigation. Server shapes are unchanged —
/// this file only decides how to show them.
library;

import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart'
    show HardwareKeyboard, KeyEvent, KeyDownEvent, LogicalKeyboardKey;
import 'package:flutter_lucide/flutter_lucide.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';
import 'package:shared_preferences/shared_preferences.dart';

import '../router.dart';
import '../ui/controls.dart' show ChoiceChips, ChoiceOption;
import '../ui/settings/settings_widgets.dart'
    show TextAction, confirmSetting, settingsToast;
import '../ui/states.dart' show describeFailure;
import '../ui/surfaces.dart';
import '../ui/tokens.dart';
import '../ui/widgets.dart'
    show
        IconBadgeButton,
        InboxHeader,
        KindBadge,
        NotificationGroupRow,
        NotificationRow,
        PopoverItem,
        PopoverTone,
        SectionLabel;
import 'agent_models.dart' show providerLabel;
import 'agent_widgets.dart' show ProviderGlyph;
import 'notifications_models.dart';
import 'notifications_state.dart';
import 'sidebar_rules.dart' show agoWords;

/// Unread notices that actually notified somewhere — a silent one (held by
/// quiet hours, a pause, a mute, or its kind turned off) never counts
/// toward the bell's badge (item 10 of the redesign). Shared with the
/// phone place picker's own count, so the two can never disagree.
int notifyingUnreadCount(NotificationsState state) =>
    state.items.where((n) => n.unread && !n.silent).length;

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

// ---- kind → icon/colour, held reasons, title & body text -----------------

class _KindLook {
  const _KindLook(this.icon, this.color, {this.glyph = false});
  final IconData icon;
  final Color color;

  /// Draw the session's own provider mark instead of [icon].
  final bool glyph;
}

_KindLook _lookFor(StormTokens t, String kind) => switch (kind) {
  'needs_you' => _KindLook(LucideIcons.circle_question_mark, t.accent),
  'delete_request' => _KindLook(LucideIcons.trash_2, t.amber),
  'error' => _KindLook(LucideIcons.circle_alert, t.danger),
  'usage_limit' => _KindLook(LucideIcons.gauge, t.amber),
  'finished' ||
  'child_finished' => _KindLook(LucideIcons.circle_check_big, t.green),
  'stopped_working' => _KindLook(LucideIcons.pause, t.amber),
  'host_offline' => _KindLook(LucideIcons.server_off, t.text3),
  'terminal_notice' => _KindLook(LucideIcons.square_terminal, t.text2),
  'summary' => _KindLook(LucideIcons.inbox, t.text2),
  'test' => _KindLook(LucideIcons.bell, t.accent),
  'agent_message' ||
  'agent_rollup' => _KindLook(LucideIcons.bot, t.text2, glyph: true),
  _ => _KindLook(LucideIcons.bell, t.text3),
};

/// ~30px at the default scale, within the coordinator's 28–32px range.
double _kindBadgeSize(StormTokens t) => t.sp * 3.75;

Widget _kindBadgeFor(StormTokens t, AgentNotification n, {double? size}) {
  final look = _lookFor(t, n.kind);
  final side = size ?? _kindBadgeSize(t);
  if (look.glyph && n.provider != null) {
    return KindBadge(
      glyph: ProviderGlyph(n.provider!, size: side * 0.5),
      color: look.color,
      size: side,
    );
  }
  return KindBadge(icon: look.icon, color: look.color, size: side);
}

/// Every known `silent_reason`, each its own icon and words — never a raw
/// wire code (item 7). A reason this doesn't know about draws no tag.
const _kHeldTags = {
  'quiet': (LucideIcons.moon, 'During quiet hours'),
  'paused': (LucideIcons.pause, 'While paused'),
  'muted': (LucideIcons.bell_off, 'Muted'),
  'off': (LucideIcons.bell_minus, 'Notifications off'),
};

Widget? heldTag(StormTokens t, String? reason) {
  final entry = _kHeldTags[reason];
  if (entry == null) return null;
  final (icon, label) = entry;
  return Row(
    mainAxisSize: MainAxisSize.min,
    children: [
      Icon(icon, size: t.labelSize * 1.1, color: t.text3),
      SizedBox(width: t.sp * 0.375),
      Flexible(
        child: Text(
          label,
          maxLines: 1,
          overflow: TextOverflow.ellipsis,
          style: TextStyle(
            fontFamily: StormTokens.sansFamily,
            fontSize: t.labelSize * 1.09,
            color: t.text3,
          ),
        ),
      ),
    ],
  );
}

/// [title] with its leading [sessionTitle] cut off, for a group's "with the
/// latest one's text" line — the server always writes the session's name
/// first (`display_title(r) + " " + verb`), so this is a plain prefix cut.
String _stripSessionPrefix(String title, String? sessionTitle) {
  if (sessionTitle == null || sessionTitle.isEmpty) return title;
  if (!title.startsWith(sessionTitle)) return title;
  return title.substring(sessionTitle.length).trim();
}

/// Line 1, bold where it names the session — true for every `storm`-sourced
/// title (the server always writes `"{session} {verb}"`), and synthesised
/// here for an agent's or a terminal's own text, which carries no such
/// prefix.
List<InlineSpan> titleSpansFor(AgentNotification n) {
  const bold = TextStyle(fontWeight: FontWeight.w600);
  final line1 = switch (n.source) {
    'agent' || 'terminal' => '${n.sessionTitle ?? 'A session'} sent a message',
    _ => n.title,
  };
  final name = n.sessionTitle;
  if (name != null && name.isNotEmpty && line1.startsWith(name)) {
    return [
      TextSpan(text: name, style: bold),
      TextSpan(text: line1.substring(name.length)),
    ];
  }
  return [TextSpan(text: line1)];
}

/// Line 2: the agent alone — the relative time already has its own place
/// on the row's trailing edge, and showing it twice was the coordinator's
/// first review finding.
String? subtitleFor(AgentNotification n) =>
    n.provider == null ? null : providerLabel(n.provider!);

/// An agent's or a terminal's own words, as a quoted block: a subtle left
/// rule, a small quiet "From the agent" label, then the text — never
/// styled like Storm's own voice, but not italic-heavy either (item 6,
/// coordinator review item 5).
Widget? bodyFor(StormTokens t, AgentNotification n) {
  if (n.source != 'agent' && n.source != 'terminal') return null;
  final text = [n.title, ?n.body].join('\n');
  final marker = n.source == 'terminal'
      ? 'From the agent, via its terminal'
      : 'From the agent';
  return Container(
    margin: EdgeInsets.only(top: t.sp * 0.375),
    padding: EdgeInsets.only(left: t.sp * 0.875),
    decoration: BoxDecoration(
      border: Border(
        left: BorderSide(color: t.border, width: t.bw * 2),
      ),
    ),
    child: Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      mainAxisSize: MainAxisSize.min,
      children: [
        Text(
          marker,
          style: TextStyle(
            fontFamily: StormTokens.sansFamily,
            fontSize: t.labelSize,
            fontWeight: FontWeight.w500,
            color: t.text3,
          ),
        ),
        SizedBox(height: t.sp * 0.25),
        Text(
          text,
          maxLines: 2,
          overflow: TextOverflow.ellipsis,
          style: TextStyle(
            fontFamily: StormTokens.sansFamily,
            fontSize: t.labelSize * 1.09,
            color: t.text2,
          ),
        ),
      ],
    ),
  );
}

String noticeSemanticLabel(AgentNotification n) => [
  if (n.unread) 'Unread',
  noticeKindLabel(n.kind),
  ?n.sessionTitle,
  notificationAgo(n),
].join(', ');

// ---- day sections and same-session grouping -------------------------------

enum _Day { today, yesterday, earlier }

_Day _dayOf(AgentNotification n, DateTime now) {
  final created = DateTime.tryParse(n.created)?.toLocal() ?? now;
  final c = DateTime(created.year, created.month, created.day);
  final t0 = DateTime(now.year, now.month, now.day);
  final diff = t0.difference(c).inDays;
  if (diff <= 0) return _Day.today;
  if (diff == 1) return _Day.yesterday;
  return _Day.earlier;
}

String _dayLabel(_Day d) => switch (d) {
  _Day.today => 'Today',
  _Day.yesterday => 'Yesterday',
  _Day.earlier => 'Earlier',
};

sealed class _Entry {
  const _Entry();
}

class _Single extends _Entry {
  const _Single(this.n);
  final AgentNotification n;
}

/// Consecutive notices from one session, within one day (item 5).
class _Group extends _Entry {
  const _Group(this.items);

  /// Newest first.
  final List<AgentNotification> items;
  AgentNotification get latest => items.first;
  String get key => 'grp:${items.first.id}';
  bool get unread => items.any((n) => n.unread);
}

/// Folds consecutive same-session notices in [items] (already one day,
/// newest first) into [_Group]s; anything without a session, or alone,
/// stays a [_Single].
List<_Entry> _group(List<AgentNotification> items) {
  final out = <_Entry>[];
  var i = 0;
  while (i < items.length) {
    final n = items[i];
    if (n.sessionId == null) {
      out.add(_Single(n));
      i++;
      continue;
    }
    var j = i + 1;
    while (j < items.length && items[j].sessionId == n.sessionId) {
      j++;
    }
    final run = items.sublist(i, j);
    out.add(run.length == 1 ? _Single(run.first) : _Group(run));
    i = j;
  }
  return out;
}

// ---- the panel --------------------------------------------------------

const _kUnreadOnlyPrefsKey = 'storm.notifications.unreadOnly';

/// The inbox's content, shared by the desktop's [NotificationBellButton]
/// popover and the phone's sheet (opened via [openNotificationsSheet] from
/// the place picker).
///
/// Paints its own popover surface (coordinator review item 1): a caller
/// anchors or positions it, never re-styles it, so it looks right wherever
/// it is hosted rather than depending on the host to match its tokens.
class NotificationsPanel extends ConsumerStatefulWidget {
  const NotificationsPanel({super.key, this.onOpened, this.phone = false});

  /// Called after a row or a header action closes the surface this panel
  /// is shown in.
  final VoidCallback? onOpened;

  /// The phone sheet's shape (top corners only, its own drag handle, no
  /// floating shadow) rather than the desktop popover's (every corner,
  /// elevated).
  final bool phone;

  @override
  ConsumerState<NotificationsPanel> createState() => _NotificationsPanelState();
}

class _NotificationsPanelState extends ConsumerState<NotificationsPanel> {
  bool _unreadOnly = false;
  final _expanded = <String>{};
  int? _focusIndex;
  List<_Entry> _flat = const [];
  final _focusNode = FocusNode(debugLabel: 'notifications-panel');
  final _scroll = ScrollController();

  @override
  void initState() {
    super.initState();
    unawaited(_loadFilter());
    HardwareKeyboard.instance.addHandler(_onKey);
  }

  @override
  void dispose() {
    HardwareKeyboard.instance.removeHandler(_onKey);
    _focusNode.dispose();
    _scroll.dispose();
    super.dispose();
  }

  Future<void> _loadFilter() async {
    try {
      final prefs = await SharedPreferences.getInstance();
      final v = prefs.getBool(_kUnreadOnlyPrefsKey);
      if (v != null && mounted) setState(() => _unreadOnly = v);
    } catch (_) {
      // No preferences store (a test, a preview): the default stands.
    }
  }

  Future<void> _setUnreadOnly(bool v) async {
    setState(() {
      _unreadOnly = v;
      _focusIndex = null;
    });
    try {
      final prefs = await SharedPreferences.getInstance();
      await prefs.setBool(_kUnreadOnlyPrefsKey, v);
    } catch (_) {}
  }

  bool _focusInside() {
    final route = ModalRoute.of(context);
    return route == null || route.isCurrent;
  }

  bool _onKey(KeyEvent e) {
    if (e is! KeyDownEvent || !mounted || !_focusInside()) return false;
    switch (e.logicalKey) {
      case LogicalKeyboardKey.arrowDown:
        if (_flat.isEmpty) return false;
        setState(
          () => _focusIndex = (_focusIndex == null)
              ? 0
              : (_focusIndex! + 1).clamp(0, _flat.length - 1),
        );
        return true;
      case LogicalKeyboardKey.arrowUp:
        if (_flat.isEmpty) return false;
        setState(
          () => _focusIndex = (_focusIndex == null)
              ? _flat.length - 1
              : (_focusIndex! - 1).clamp(0, _flat.length - 1),
        );
        return true;
      case LogicalKeyboardKey.enter:
      case LogicalKeyboardKey.numpadEnter:
        final i = _focusIndex;
        if (i == null || i >= _flat.length) return false;
        final entry = _flat[i];
        if (entry is _Single) {
          _open(entry.n);
        } else if (entry is _Group) {
          _toggleGroup(entry.key);
        }
        return true;
      case LogicalKeyboardKey.escape:
        scheduleMicrotask(() {
          if (mounted) Navigator.of(context).maybePop();
        });
        return true;
      default:
        return false;
    }
  }

  void _toggleGroup(String key) {
    setState(() {
      if (!_expanded.remove(key)) _expanded.add(key);
    });
  }

  void _open(AgentNotification n) {
    widget.onOpened?.call();
    unawaited(
      _guard(
        context,
        () async => ref.read(notificationsStateProvider)?.markRead([n.id]),
      ),
    );
    final router = GoRouter.maybeOf(context);
    if (n.sessionId != null) {
      router?.go(Routes.agentSession(n.sessionId!));
    } else if (n.noteVaultId != null && n.noteId != null) {
      router?.go(Routes.note(n.noteVaultId!, n.noteId!));
    }
  }

  void _openSettings() {
    widget.onOpened?.call();
    GoRouter.maybeOf(context)?.go(Routes.settingsPage('notifications'));
  }

  Future<void> _mute(String sessionId, String? until) async {
    try {
      await ref
          .read(notificationsStateProvider)
          ?.createMute('session', sessionId, until: until);
    } catch (e) {
      if (mounted) settingsToast(context, describeFailure(e));
    }
  }

  Future<void> _openRowMenu(AgentNotification n, GlobalKey anchor) async {
    final t = context.tokens;
    final canOpen =
        n.sessionId != null || (n.noteVaultId != null && n.noteId != null);
    final openLabel = n.sessionId != null ? 'Open session' : 'Open note';
    await showStormPopover<void>(
      context: context,
      anchorKey: anchor,
      alignRight: true,
      width: fitPopoverWidth(context, [
        if (n.unread) 'Mark as read' else 'Mark as unread',
        if (n.sessionId != null) ...['Mute for 1 hour', 'Mute until it ends'],
        if (canOpen) openLabel,
      ]),
      builder: (popContext) => StormPopover(
        padding: 0,
        children: [
          if (n.unread)
            PopoverItem(
              key: Key('notify-menu-read-${n.id}'),
              label: 'Mark as read',
              leading: Icon(
                LucideIcons.check_check,
                size: t.codeSize,
                color: t.text2,
              ),
              onTap: () {
                Navigator.pop(popContext);
                unawaited(
                  _guard(
                    context,
                    () async =>
                        ref.read(notificationsStateProvider)?.markRead([n.id]),
                  ),
                );
              },
            )
          else
            PopoverItem(
              key: Key('notify-menu-unread-${n.id}'),
              label: 'Mark as unread',
              leading: Icon(
                LucideIcons.circle_dot,
                size: t.codeSize,
                color: t.text2,
              ),
              onTap: () {
                Navigator.pop(popContext);
                unawaited(
                  _guard(
                    context,
                    () async => ref
                        .read(notificationsStateProvider)
                        ?.markUnread([n.id]),
                  ),
                );
              },
            ),
          if (n.sessionId != null) ...[
            PopoverItem(
              key: Key('notify-menu-mute1h-${n.id}'),
              label: 'Mute for 1 hour',
              leading: Icon(
                LucideIcons.bell_off,
                size: t.codeSize,
                color: t.text2,
              ),
              onTap: () {
                Navigator.pop(popContext);
                unawaited(
                  _mute(
                    n.sessionId!,
                    DateTime.now()
                        .toUtc()
                        .add(const Duration(hours: 1))
                        .toIso8601String(),
                  ),
                );
              },
            ),
            PopoverItem(
              key: Key('notify-menu-muteend-${n.id}'),
              label: 'Mute until it ends',
              leading: Icon(
                LucideIcons.bell_off,
                size: t.codeSize,
                color: t.text2,
              ),
              onTap: () {
                Navigator.pop(popContext);
                unawaited(_mute(n.sessionId!, null));
              },
            ),
          ],
          if (canOpen)
            PopoverItem(
              key: Key('notify-menu-open-${n.id}'),
              label: openLabel,
              leading: Icon(
                LucideIcons.arrow_up_right,
                size: t.codeSize,
                color: t.text2,
              ),
              onTap: () {
                Navigator.pop(popContext);
                _open(n);
              },
            ),
        ],
      ),
    );
  }

  Future<void> _openOverflowMenu(
    GlobalKey anchor,
    NotificationsState state,
  ) async {
    final t = context.tokens;
    await showStormPopover<void>(
      context: context,
      anchorKey: anchor,
      alignRight: true,
      width: fitPopoverWidth(context, ['Clear inbox']),
      builder: (popContext) => StormPopover(
        padding: 0,
        children: [
          PopoverItem(
            key: const Key('notify-clear-inbox'),
            label: 'Clear inbox',
            tone: PopoverTone.danger,
            leading: Icon(
              LucideIcons.trash_2,
              size: t.codeSize,
              color: t.danger,
            ),
            onTap: () async {
              Navigator.pop(popContext);
              final ok = await confirmSetting(
                context,
                title: 'Clear the inbox?',
                body:
                    'Every notice is removed for every device. This cannot '
                    'be undone.',
                action: 'Clear',
              );
              if (ok && mounted) await _guard(context, state.clear);
            },
          ),
        ],
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final state = ref.watch(notificationsStateProvider);
    if (state == null) return const SizedBox.shrink();
    return ListenableBuilder(
      listenable: state,
      builder: (context, _) {
        if (state.unsupported) return const SizedBox.shrink();
        final radius = widget.phone
            ? BorderRadius.vertical(top: Radius.circular(t.rCard))
            : BorderRadius.circular(t.rCard);
        // The exact two-layer shape the ⌘K palette uses (coordinator
        // review item 2): shadow on an unclipped outer `DecoratedBox` —
        // clipping it along with the content would cut it off — and
        // `t.surface` (not `t.surface2`, which read as a muddy grey darker
        // than the page in light themes) filled and clipped by the inner
        // `Material`, which also gives every `InkWell` below it the
        // ancestor it needs.
        return DecoratedBox(
          decoration: BoxDecoration(
            borderRadius: radius,
            boxShadow: widget.phone ? null : t.shadow,
          ),
          child: ConstrainedBox(
            constraints: BoxConstraints(
              maxHeight: MediaQuery.sizeOf(context).height * 0.7,
            ),
            child: Material(
              color: t.surface,
              clipBehavior: Clip.antiAlias,
              shape: RoundedRectangleBorder(
                borderRadius: radius,
                side: BorderSide(color: t.border, width: t.bw),
              ),
              child: Focus(
                focusNode: _focusNode,
                child: Column(
                  // `min`, not `max`: the popover sizes to its content up
                  // to the cap above, rather than always filling it
                  // (coordinator review item 1).
                  mainAxisSize: MainAxisSize.min,
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  children: [
                    if (widget.phone)
                      Padding(
                        padding: EdgeInsets.only(top: t.sp),
                        child: Center(
                          child: Container(
                            width: t.sp * 4,
                            height: t.sp * 0.5,
                            decoration: BoxDecoration(
                              color: t.border,
                              borderRadius: BorderRadius.circular(999),
                            ),
                          ),
                        ),
                      ),
                    Padding(
                      padding: EdgeInsets.fromLTRB(
                        t.sp * 1.25,
                        t.sp * 1.25,
                        t.sp * 0.75,
                        t.sp * 0.75,
                      ),
                      child: _header(t, state),
                    ),
                    Padding(
                      padding: EdgeInsets.fromLTRB(
                        t.sp * 1.25,
                        0,
                        t.sp * 1.25,
                        t.sp,
                      ),
                      child: ChoiceChips<bool>(
                        dense: true,
                        options: const [
                          ChoiceOption(false, 'All'),
                          ChoiceOption(true, 'Unread'),
                        ],
                        selected: _unreadOnly,
                        onSelected: (v) => unawaited(_setUnreadOnly(v)),
                      ),
                    ),
                    Flexible(child: _body(context, t, state)),
                  ],
                ),
              ),
            ),
          ),
        );
      },
    );
  }

  Widget _header(StormTokens t, NotificationsState state) {
    final markAllAnchor = GlobalKey();
    final settingsAnchor = GlobalKey();
    final overflowAnchor = GlobalKey();
    return InboxHeader(
      title: 'Notifications',
      unreadCount: notifyingUnreadCount(state),
      actions: [
        IconBadgeButton(
          key: const Key('notify-mark-all-read'),
          anchorKey: markAllAnchor,
          icon: LucideIcons.check_check,
          tooltip: 'Mark all read',
          dimension: t.sp * 3.25,
          onTap: () => unawaited(_guard(context, state.markAllRead)),
        ),
        IconBadgeButton(
          key: const Key('notify-settings'),
          anchorKey: settingsAnchor,
          icon: LucideIcons.settings,
          tooltip: 'Notification settings',
          dimension: t.sp * 3.25,
          onTap: _openSettings,
        ),
        IconBadgeButton(
          key: const Key('notify-overflow'),
          anchorKey: overflowAnchor,
          icon: LucideIcons.ellipsis_vertical,
          tooltip: 'More',
          dimension: t.sp * 3.25,
          onTap: () => unawaited(_openOverflowMenu(overflowAnchor, state)),
        ),
      ],
    );
  }

  Widget _body(BuildContext context, StormTokens t, NotificationsState state) {
    if (state.loading) {
      return Padding(
        padding: EdgeInsets.symmetric(vertical: t.sp * 2.5),
        child: const Center(
          child: SizedBox.square(
            dimension: 20,
            child: CircularProgressIndicator(strokeWidth: 2),
          ),
        ),
      );
    }
    if (state.backfillFailed) {
      return Padding(
        padding: EdgeInsets.symmetric(
          horizontal: t.sp * 1.25,
          vertical: t.sp * 1.5,
        ),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          mainAxisSize: MainAxisSize.min,
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
              onTap: () => unawaited(_guard(context, state.refresh)),
            ),
          ],
        ),
      );
    }

    final now = DateTime.now();
    final items = _unreadOnly
        ? state.items.where((n) => n.unread).toList()
        : state.items;
    if (items.isEmpty) {
      return _unreadOnly
          ? _emptyPanel(
              t,
              icon: LucideIcons.check_check,
              title: 'No unread notifications',
            )
          : _emptyPanel(
              t,
              icon: LucideIcons.inbox,
              title: "You're all caught up",
            );
    }

    final buckets = <_Day, List<AgentNotification>>{};
    for (final n in items) {
      (buckets[_dayOf(n, now)] ??= []).add(n);
    }

    final widgets = <Widget>[];
    final flat = <_Entry>[];
    for (final day in _Day.values) {
      final dayItems = buckets[day];
      if (dayItems == null || dayItems.isEmpty) continue;
      widgets.add(
        Padding(
          padding: EdgeInsets.fromLTRB(
            t.sp * 1.25,
            t.sp * 1.25,
            t.sp * 1.25,
            t.sp * 0.5,
          ),
          child: SectionLabel(_dayLabel(day)),
        ),
      );
      for (final entry in _group(dayItems)) {
        final idx = flat.length;
        flat.add(entry);
        widgets.add(_entryWidget(t, entry, idx));
        if (entry is _Group && _expanded.contains(entry.key)) {
          for (final child in entry.items) {
            final cIdx = flat.length;
            flat.add(_Single(child));
            widgets.add(
              Padding(
                padding: EdgeInsets.only(left: t.sp * 3),
                child: _singleRow(t, child, cIdx),
              ),
            );
          }
        }
      }
    }
    _flat = flat;
    if (_focusIndex != null && _focusIndex! >= flat.length) {
      _focusIndex = flat.isEmpty ? null : flat.length - 1;
    }

    return ListView(
      controller: _scroll,
      // Sizes to its rows rather than always filling the `Flexible` it
      // sits in — a 3-row inbox stays a 3-row popover, not a fixed slab
      // (coordinator review item 1). The `ConstrainedBox` above still caps
      // it and makes a long inbox scroll.
      shrinkWrap: true,
      padding: EdgeInsets.only(bottom: t.sp),
      children: widgets,
    );
  }

  Widget _emptyPanel(
    StormTokens t, {
    required IconData icon,
    required String title,
  }) {
    return Padding(
      padding: EdgeInsets.symmetric(horizontal: t.sp * 2, vertical: t.sp * 4),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          Icon(icon, size: t.headingSize * 1.4, color: t.text3),
          SizedBox(height: t.sp * 1.25),
          Text(
            title,
            textAlign: TextAlign.center,
            style: TextStyle(
              fontFamily: StormTokens.sansFamily,
              fontSize: t.codeSize,
              fontWeight: FontWeight.w500,
              color: t.text2,
            ),
          ),
          SizedBox(height: t.sp),
          TextAction(label: 'Notification settings', onTap: _openSettings),
        ],
      ),
    );
  }

  Widget _entryWidget(StormTokens t, _Entry entry, int idx) {
    final focused = _focusIndex == idx;
    if (entry is _Group) {
      final latest = entry.latest;
      final rest = _stripSessionPrefix(latest.title, latest.sessionTitle);
      final latestLine = rest.isEmpty ? latest.title : rest;
      return NotificationGroupRow(
        key: ValueKey(entry.key),
        leading: _kindBadgeFor(t, latest),
        title: [
          TextSpan(
            text: latest.sessionTitle ?? 'A session',
            style: const TextStyle(fontWeight: FontWeight.w600),
          ),
          TextSpan(text: ' · ${entry.items.length} updates'),
        ],
        // "latest: " makes clear this line is the newest member's own
        // words, not a summary of all of them (coordinator review item 4).
        subtitle: ['latest: $latestLine', ?subtitleFor(latest)].join(' · '),
        unread: entry.unread,
        expanded: _expanded.contains(entry.key),
        onToggle: () => _toggleGroup(entry.key),
        focused: focused,
        semanticLabel:
            '${entry.unread ? 'Unread, ' : ''}'
            '${entry.items.length} updates from '
            '${latest.sessionTitle ?? 'a session'}',
      );
    }
    return _singleRow(t, (entry as _Single).n, idx);
  }

  Widget _singleRow(StormTokens t, AgentNotification n, int idx) {
    final canOpen =
        n.sessionId != null || (n.noteVaultId != null && n.noteId != null);
    return NotificationRow(
      key: Key('notify-row-${n.id}'),
      leading: _kindBadgeFor(t, n),
      title: titleSpansFor(n),
      subtitle: subtitleFor(n),
      tag: heldTag(t, n.silentReason),
      body: bodyFor(t, n),
      unread: n.unread,
      time: notificationAgo(n),
      focused: _focusIndex == idx,
      semanticLabel: noticeSemanticLabel(n),
      onTap: canOpen ? () => _open(n) : null,
      onMenu: (anchor) => unawaited(_openRowMenu(n, anchor)),
    );
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
          count: notifyingUnreadCount(state),
          smallBadge: true,
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
      // ~380px at the default scale (item 1).
      width: t.sp * 47.5,
      alignRight: true,
      // No [StormPopover] wrapper: [NotificationsPanel] paints its own
      // surface, and wrapping it in another would double the border,
      // radius and shadow.
      builder: (popContext) => NotificationsPanel(
        onOpened: () => Navigator.maybeOf(popContext)?.pop(),
      ),
    );
  }
}

/// Opens the inbox as a sheet — the phone's way in from the place picker
/// (161-3's review: reachable from every screen, not only a corner bubble
/// gated on having no nav pill).
///
/// Built on the raw sheet rather than [showStormSheet]: that helper draws
/// its own mono [SectionLabel] title and close button, which would double
/// up with [NotificationsPanel]'s own [InboxHeader] (item 2 — the header
/// is never `SectionLabel`/mono here). The route itself stays transparent
/// and chromeless — [NotificationsPanel] paints its own surface and its
/// own drag handle (`phone: true`), so there is only ever one.
Future<void> openNotificationsSheet(BuildContext context) =>
    showModalBottomSheet<void>(
      context: context,
      isScrollControlled: true,
      backgroundColor: Colors.transparent,
      builder: (sheetContext) => SafeArea(
        child: NotificationsPanel(
          phone: true,
          onOpened: () => Navigator.maybeOf(sheetContext)?.pop(),
        ),
      ),
    );
