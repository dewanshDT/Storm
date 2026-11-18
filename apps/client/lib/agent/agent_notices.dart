import 'dart:async';
import 'dart:math' as math;

import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';
import 'package:shared_preferences/shared_preferences.dart';

import '../router.dart';
import '../ui/tokens.dart';
import 'agent_events.dart';
import 'agent_models.dart';
import 'delete_request.dart' show shownSessionOf, showAgentToast;
import 'notifications_models.dart';
import 'notifications_state.dart';
import 'output_seen.dart';
import 'sidebar_rules.dart';

/// What a live change of a session's says to someone not looking at it
/// (decision 154, Notices; decision 155, AM115). In-app only: Storm has no
/// OS notifications.
enum AgentNotice {
  needsYou,

  /// The program said `done`. "Its turn": a `run_in_background` shell can
  /// still be running (decision 155 spike, case 9).
  finishedTurn,

  /// The program said `error`.
  hitError,

  /// A session known only by its title went idle, or a program went from
  /// working to idle without `done` (Esc, Ctrl+C). Never "done": a title
  /// can't tell a prompt from a finish (134 A3).
  stoppedWorking,
}

/// A need the 153h delete toast does not already say.
bool _needsMoreThanADelete(AgentSession s) {
  final kind = attentionKindOf(s);
  return (kind != 'delete_approval' && kNeedsYouKinds.contains(kind)) ||
      s.pendingDecisions.any((d) => d.kind != 'delete_request');
}

/// The notice [before] → [after] raises, before asking who is looking or
/// what they have read.
AgentNotice? noticeOf(AgentSession before, AgentSession after) {
  if (!needsYou(before) && needsYou(after) && _needsMoreThanADelete(after)) {
    return AgentNotice.needsYou;
  }
  final idle = after.status == 'running' && after.activity == 'idle';
  final program = after.status == 'running' ? after.programStatus : null;
  final was = before.programStatus;
  bool entered(String state) =>
      program?.state == state &&
      (was?.state != state || was?.since != program?.since);
  if (program != null && idle) {
    if (entered('done')) return AgentNotice.finishedTurn;
    if (entered('error')) return AgentNotice.hitError;
  }
  if (program != null && program.state != 'idle') return null;
  final wasWorking = before.status == 'running' && before.working;
  return wasWorking && (after.ended || idle)
      ? AgentNotice.stoppedWorking
      : null;
}

/// One transition, as this device remembers it toasted: the same prompt or
/// the same stop seen again (a repeated view, a flap) is not said twice.
String noticeKey(AgentNotice n, AgentSession s) => switch (n) {
  AgentNotice.needsYou =>
    '${s.id} needs ${attentionKindOf(s) ?? 'decision'}@'
        '${s.pendingDecisions.firstOrNull?.id ?? s.spawn.attention?.since?.toIso8601String() ?? s.outputEnd ?? ''}',
  AgentNotice.finishedTurn =>
    '${s.id} done@${s.programStatus?.since?.toIso8601String() ?? ''}',
  AgentNotice.hitError =>
    '${s.id} error@${s.programStatus?.since?.toIso8601String() ?? ''}',
  AgentNotice.stoppedWorking => '${s.id} stopped@${s.outputEnd ?? ''}',
};

/// The transitions this device has toasted, kept so nothing is said twice
/// across a restart. Device-local, like what it has seen (Q12).
const kNoticeToastedKey = 'storm.agents.notice_toasted';

/// Decision 161: server-sourced toasts (`needs_you`/`finished`/`error`, and
/// every agent- or terminal-sourced notice) are deduped by the
/// [AgentNotification.id] instead, and kept separately — an old server's
/// fallback still uses [kNoticeToastedKey] alone.
const kServerNoticeToastedKey = 'storm.agents.server_notice_toasted';
const _toastedLimit = 200;

/// Kinds a non-silent `notification` push never toasts from this path:
/// `delete_request` has its own toast (153h), and `stopped_working` is kept
/// client-only on purpose (the draft's `noticeOf` is always its source).
/// Every other kind toasts once the server's inbox is [supported] —
/// `needs_you`/`finished`/`error` replacing `noticeOf` for those three, and
/// `usage_limit`/`host_offline`/`child_finished`/`summary`/`test` and any
/// agent- or terminal-sourced notice having no client equivalent at all.
const _kNeverToastKinds = {'delete_request', 'stopped_working'};

/// The three kinds `noticeOf` also computes client-side; see
/// [_AgentNoticeWatcherState._serverCovers].
const _kOverlapsClientKinds = {'needs_you', 'finished', 'error'};

/// Raises "{name} needs you · Open", "{name} finished its turn · Open",
/// "{name} hit an error · Open" and "{name} stopped working · Open" from
/// live session changes on every screen while signed in (decision 154,
/// slice 154-4; decision 155, slice 155-1). Never from a snapshot or a
/// resync, so a reconnect or a return to the app says nothing; never for
/// the session on screen; anything but a need only while its output is
/// unread here, the same rule as the row's dot.
class AgentNoticeWatcher extends ConsumerStatefulWidget {
  const AgentNoticeWatcher({
    super.key,
    required this.location,
    required this.child,
  });

  final String location;
  final Widget child;

  @override
  ConsumerState<AgentNoticeWatcher> createState() => _AgentNoticeWatcherState();
}

class _AgentNoticeWatcherState extends ConsumerState<AgentNoticeWatcher> {
  StreamSubscription<AgentEventUpdate>? _updates;
  Set<String>? _toasted;
  Set<String>? _serverToasted;

  ScaffoldFeatureController<SnackBar, SnackBarClosedReason>? _toast;
  String? _toastFor;
  AgentNotice? _toastKind;

  /// The id of a currently shown server-sourced toast, so a
  /// `notification_read` for it (read from elsewhere) dismisses it here.
  String? _toastNotificationId;

  /// Whether the account's inbox has proven itself present (161-3): once
  /// true, `needs_you`/`finished`/`error` toasts come from its own
  /// `notification` pushes, never from [noticeOf]. `NotificationsState.
  /// supported` (not `!loading && !unsupported`) is the right test — the
  /// naive one is false while still loading, so both paths would toast at
  /// once, and stays false forever after a transient (non-404) failure,
  /// which would silence `noticeOf` for good on a server that in fact has
  /// no support for this at all.
  bool get _serverCovers =>
      ref.read(notificationsStateProvider)?.supported ?? false;

  @override
  void initState() {
    super.initState();
    ref.listenManual(seenOutputProvider, (_, _) {});
    // Keeps the inbox warm everywhere this watcher is mounted (every screen
    // while signed in), not only where the bell happens to be on screen.
    ref.listenManual(notificationsStateProvider, (_, _) {});
    ref.listenManual<AgentEvents?>(
      agentEventsProvider,
      (_, events) => _follow(events),
      fireImmediately: true,
    );
    unawaited(_loadToasted());
  }

  @override
  void didUpdateWidget(AgentNoticeWatcher old) {
    super.didUpdateWidget(old);
    if (_toastFor != null && shownSessionOf(widget.location) == _toastFor) {
      _hideToast();
    }
  }

  @override
  void dispose() {
    _updates?.cancel();
    super.dispose();
  }

  void _follow(AgentEvents? events) {
    _updates?.cancel();
    _updates = events?.updates.listen(_heard);
  }

  Future<Set<String>> _loadToasted() async {
    if (_toasted case final t?) return t;
    var loaded = <String>{};
    try {
      final prefs = await SharedPreferences.getInstance();
      loaded = {...?prefs.getStringList(kNoticeToastedKey)};
    } catch (_) {
      // No preferences store (tests, previews): memory still works.
    }
    return _toasted ??= loaded;
  }

  Future<void> _remember(Set<String> toasted) async {
    final list = toasted.toList();
    try {
      final prefs = await SharedPreferences.getInstance();
      await prefs.setStringList(
        kNoticeToastedKey,
        list.sublist(math.max(0, list.length - _toastedLimit)),
      );
    } catch (_) {}
  }

  Future<Set<String>> _loadServerToasted() async {
    if (_serverToasted case final t?) return t;
    var loaded = <String>{};
    try {
      final prefs = await SharedPreferences.getInstance();
      loaded = {...?prefs.getStringList(kServerNoticeToastedKey)};
    } catch (_) {
      // No preferences store (tests, previews): memory still works.
    }
    return _serverToasted ??= loaded;
  }

  Future<void> _rememberServer(Set<String> toasted) async {
    final list = toasted.toList();
    try {
      final prefs = await SharedPreferences.getInstance();
      await prefs.setStringList(
        kServerNoticeToastedKey,
        list.sublist(math.max(0, list.length - _toastedLimit)),
      );
    } catch (_) {}
  }

  void _heard(AgentEventUpdate u) {
    switch (u.event) {
      case AgentSessionChanged(:final session):
        if (_toastFor == session.id &&
            _toastKind == AgentNotice.needsYou &&
            !needsYou(session)) {
          _hideToast();
        }
        final before = u.before;
        if (before == null) return;
        if (noticeOf(before, session) case final n?) {
          // Once the server's own inbox raises these three, `noticeOf`
          // would only say the same thing twice; `stoppedWorking` has no
          // server equivalent on purpose and is always this path's.
          if (n == AgentNotice.stoppedWorking || !_serverCovers) {
            unawaited(_consider(n, session));
          }
        }
      case AgentSessionRemoved(:final id) when id == _toastFor:
        _hideToast();
      case AgentNotificationPushed(:final notification):
        unawaited(_considerServer(notification));
      case AgentNotificationsRead(:final ids):
        if (_toastNotificationId != null &&
            ids.contains(_toastNotificationId)) {
          _hideToast();
        }
      case AgentNotificationsRemoved(:final ids):
        if (_toastNotificationId != null &&
            ids.contains(_toastNotificationId)) {
          _hideToast();
        }
      default:
        break;
    }
  }

  /// A server-sourced toast (161-3): every kind but [_kNeverToastKinds],
  /// styled as an agent's or a terminal's own words once [n.source] says so
  /// — never by `kind` (a roll-up is Storm's own words about an agent,
  /// source `storm`, not an agent's).
  Future<void> _considerServer(AgentNotification n) async {
    if (n.cancelled) return;
    if (n.sessionId != null && n.sessionId == shownSessionOf(widget.location)) {
      // Already looking at it: never toasted (even a silent one — item 6
      // of the second review, checked ahead of the silent/never-toast
      // filters below, not after them), but still read, the same rule
      // `markSessionRead` gives output otherwise seen there.
      try {
        await ref.read(notificationsStateProvider)?.markRead([n.id]);
      } catch (_) {
        // Background bookkeeping, not a tap: nothing useful to surface.
      }
      return;
    }
    if (n.silent || _kNeverToastKinds.contains(n.kind)) return;
    // The three kinds `noticeOf` also raises toast from here only once the
    // inbox has proven itself present; before that, `noticeOf` alone has
    // it (item 4 — otherwise both fire during the loading window).
    if (_kOverlapsClientKinds.contains(n.kind) && !_serverCovers) return;
    final isAgent = n.source == 'agent';
    final isTerminal = n.source == 'terminal';
    final toasted = await _loadServerToasted();
    if (!mounted) return;
    if (!toasted.add(n.id)) return;
    unawaited(_rememberServer(toasted));
    _raiseServer(n, agent: isAgent, terminal: isTerminal);
  }

  Future<void> _consider(AgentNotice n, AgentSession s) async {
    final toasted = await _loadToasted();
    if (!mounted) return;
    if (s.id == shownSessionOf(widget.location)) return;
    if (n != AgentNotice.needsYou &&
        !outputUnread(ref.read(seenOutputProvider).value, s)) {
      return;
    }
    if (!toasted.add(noticeKey(n, s))) return;
    unawaited(_remember(toasted));
    _raise(n, s);
  }

  void _raise(AgentNotice n, AgentSession s) {
    final router = GoRouter.maybeOf(context);
    final id = s.id;
    final toast = showAgentToast(
      SnackBar(
        key: ValueKey('notice-toast-${n.name}-$id'),
        content: Text(switch (n) {
          AgentNotice.needsYou => '${s.displayName} needs you',
          AgentNotice.finishedTurn => '${s.displayName} finished its turn',
          AgentNotice.hitError => '${s.displayName} hit an error',
          AgentNotice.stoppedWorking => '${s.displayName} stopped working',
        }),
        persist: false,
        duration: const Duration(seconds: 10),
        action: SnackBarAction(
          key: const Key('notice-toast-open'),
          label: 'Open',
          onPressed: () => router?.go(Routes.agentSession(id)),
        ),
      ),
    );
    if (toast == null) return;
    _toast = toast;
    _toastFor = id;
    _toastKind = n;
    // A server-sourced toast this replaces left its id behind (item N6):
    // without clearing it, a later `notification_read`/`notification_
    // removed` for that stale id would hide *this* toast instead.
    _toastNotificationId = null;
    toast.closed.then((_) {
      if (_toast == toast) {
        _toast = null;
        _toastFor = null;
        _toastKind = null;
      }
    });
  }

  /// An agent's or a terminal's own text is sanitised plain text already
  /// (the server, `sanitise_agent_text`); it is never treated as Storm's
  /// own words here, and gets no action beyond Open.
  void _raiseServer(
    AgentNotification n, {
    required bool agent,
    required bool terminal,
  }) {
    final router = GoRouter.maybeOf(context);
    final sessionId = n.sessionId;
    final t = Theme.of(context);
    final tokens = context.tokens;
    final fromAgent = agent || terminal;
    final content = !fromAgent
        ? Text(n.title)
        : Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Text(
                '${n.sessionTitle ?? 'A session'} · '
                '${n.provider == null ? 'Agent' : providerLabel(n.provider!)}'
                '${terminal ? ' via its terminal' : ''}',
                style: TextStyle(
                  fontWeight: FontWeight.w600,
                  fontSize: tokens.labelSize,
                ),
              ),
              SizedBox(height: tokens.sp * 0.25),
              Text(
                n.title,
                style: TextStyle(color: t.colorScheme.onInverseSurface),
              ),
            ],
          );
    final toast = showAgentToast(
      SnackBar(
        key: ValueKey('notice-toast-server-${n.id}'),
        content: content,
        persist: false,
        duration: const Duration(seconds: 10),
        action: sessionId == null
            ? null
            : SnackBarAction(
                key: const Key('notice-toast-open'),
                label: 'Open',
                onPressed: () => router?.go(Routes.agentSession(sessionId)),
              ),
      ),
    );
    if (toast == null) return;
    _toast = toast;
    _toastFor = sessionId;
    _toastNotificationId = n.id;
    // This replaces whatever noticeOf toast might have been showing
    // (item N6): clear its kind, or a later session change could match
    // against it and mis-hide this one.
    _toastKind = null;
    toast.closed.then((_) {
      if (_toast == toast) {
        _toast = null;
        _toastFor = null;
        _toastNotificationId = null;
      }
    });
  }

  void _hideToast() {
    final toast = _toast;
    _toast = null;
    _toastFor = null;
    _toastKind = null;
    _toastNotificationId = null;
    if (toast == null) return;
    try {
      toast.close();
    } catch (_) {}
  }

  @override
  Widget build(BuildContext context) => widget.child;
}
