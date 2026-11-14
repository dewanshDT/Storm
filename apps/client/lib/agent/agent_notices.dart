import 'dart:async';
import 'dart:math' as math;

import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';
import 'package:shared_preferences/shared_preferences.dart';

import '../router.dart';
import 'agent_events.dart';
import 'agent_models.dart';
import 'delete_request.dart' show shownSessionOf, showAgentToast;
import 'output_seen.dart';
import 'sidebar_rules.dart';

/// What a live change of a session's says to someone not looking at it
/// (decision 154, Notices). In-app only: Storm has no OS notifications.
enum AgentNotice {
  needsYou,

  /// Never "done": a top-level agent at a prompt can read as idle (134 A3).
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
  final wasWorking = before.status == 'running' && before.working;
  final stopped = after.ended || (after.status == 'running' && !after.working);
  return wasWorking && stopped ? AgentNotice.stoppedWorking : null;
}

/// One transition, as this device remembers it toasted: the same prompt or
/// the same stop seen again (a repeated view, a flap) is not said twice.
String noticeKey(AgentNotice n, AgentSession s) => switch (n) {
  AgentNotice.needsYou =>
    '${s.id} needs ${attentionKindOf(s) ?? 'decision'}@'
        '${s.pendingDecisions.firstOrNull?.id ?? s.spawn.attention?.since?.toIso8601String() ?? s.outputEnd ?? ''}',
  AgentNotice.stoppedWorking => '${s.id} stopped@${s.outputEnd ?? ''}',
};

/// The transitions this device has toasted, kept so nothing is said twice
/// across a restart. Device-local, like what it has seen (Q12).
const kNoticeToastedKey = 'storm.agents.notice_toasted';
const _toastedLimit = 200;

/// Raises "{name} needs you · Open" and "{name} stopped working · Open"
/// from live session changes on every screen while signed in (decision 154,
/// slice 154-4). Never from a snapshot or a resync, so a reconnect or a
/// return to the app says nothing; never for the session on screen; a stop
/// only while its output is unread here, the same rule as the row's dot.
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

  ScaffoldFeatureController<SnackBar, SnackBarClosedReason>? _toast;
  String? _toastFor;
  AgentNotice? _toastKind;

  @override
  void initState() {
    super.initState();
    ref.listenManual(seenOutputProvider, (_, _) {});
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
          unawaited(_consider(n, session));
        }
      case AgentSessionRemoved(:final id) when id == _toastFor:
        _hideToast();
      default:
        break;
    }
  }

  Future<void> _consider(AgentNotice n, AgentSession s) async {
    final toasted = await _loadToasted();
    if (!mounted) return;
    if (s.id == shownSessionOf(widget.location)) return;
    if (n == AgentNotice.stoppedWorking &&
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
    toast.closed.then((_) {
      if (_toast == toast) {
        _toast = null;
        _toastFor = null;
        _toastKind = null;
      }
    });
  }

  void _hideToast() {
    final toast = _toast;
    _toast = null;
    _toastFor = null;
    _toastKind = null;
    if (toast == null) return;
    try {
      toast.close();
    } catch (_) {}
  }

  @override
  Widget build(BuildContext context) => widget.child;
}
