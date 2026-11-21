import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import 'agent_api.dart';
import 'agent_events.dart';
import 'agent_state.dart';
import 'notifications_models.dart';

/// Decision 161: the account's inbox, prefs and mutes, kept across every
/// screen that shows the bell, the inbox panel or Settings › Notifications.
///
/// Subscribes to the shared [AgentEvents] stream immediately (never waiting
/// on a `GET`), and backfills `GET /v1/notifications` by `cursor` on every
/// fresh connection (every [AgentSnapshot] — the server's own stream resumes
/// from "now" on reconnect, so anything that happened while this device was
/// disconnected only reaches it through another backfill). Live frames
/// heard while a backfill is in flight are queued and replayed once it
/// lands, so none are lost to the race between subscribing and the `GET`
/// landing. A server before decision 161 answers the first backfill 404:
/// [unsupported] is then true forever, and nothing here is shown.
class NotificationsState extends ChangeNotifier {
  NotificationsState(this._apiFactory, this._events) {
    _sub = _events?.updates.listen(_heard);
    unawaited(_backfill());
  }

  final AgentApi Function() _apiFactory;
  final AgentEvents? _events;

  final _byId = <String, AgentNotification>{};
  NotificationPrefs? _prefs;
  List<NotificationMute> _mutes = const [];

  bool _loading = true;
  bool _unsupported = false;

  /// True once one backfill has completed and was not refused 404 (item 4
  /// of the 161-3 review): while false, a toast watcher must not yet decide
  /// either way whether the server's inbox "covers" a kind.
  bool _supported = false;

  /// The most recent backfill threw something other than a 404 (offline, a
  /// 5xx): distinct from [unsupported], which is a permanent fact about the
  /// server, not a transient one about the network.
  bool _backfillFailed = false;

  StreamSubscription<AgentEventUpdate>? _sub;
  bool _disposed = false;
  bool _backfilling = false;
  Timer? _retry;

  /// Live frames heard mid-backfill, replayed once it lands.
  final _pending = <AgentEvent>[];

  List<AgentNotification>? _sortedCache;

  /// Newest first.
  List<AgentNotification> get items =>
      _sortedCache ??= _byId.values.toList()
        ..sort((a, b) => b.seq.compareTo(a.seq));

  int get unreadCount => _byId.values.where((n) => n.unread).length;

  NotificationPrefs? get prefs => _prefs;
  List<NotificationMute> get mutes => _mutes;

  /// Backfilling, or waiting on the first prefs/mutes read.
  bool get loading => _loading;

  /// A server before decision 161: the bell, the inbox and Settings ›
  /// Notifications must all hide.
  bool get unsupported => _unsupported;

  /// One backfill has succeeded: the inbox's own notices for
  /// `needs_you`/`finished`/`error` are authoritative, so a toast watcher
  /// may stop relying on [noticeOf] for them. Distinct from `!loading &&
  /// !unsupported`, which is also (wrongly) true while a transient failure
  /// has merely stopped loading without ever succeeding.
  bool get supported => _supported;

  /// The inbox couldn't be refreshed just now (item N4 of the 161-3
  /// re-review): distinct from "nothing yet", which is a real empty inbox.
  bool get backfillFailed => _backfillFailed;

  /// Retries the backfill on request (a "Retry" button); harmless to call
  /// while one is already running.
  Future<void> refresh() => _backfill();

  void _invalidate() => _sortedCache = null;

  void _heard(AgentEventUpdate u) {
    if (_backfilling) {
      _pending.add(u.event);
      return;
    }
    applyEvent(u.event);
    // A fresh snapshot is a new or resumed connection: the live stream
    // resumes from "now", so catching up needs another backfill (item 1).
    if (u.event is AgentSnapshot) unawaited(_backfill());
  }

  /// Rebuilds the inbox from scratch (a blocker of the second review): a
  /// merge-by-id never drops a row the server stopped returning (cancelled
  /// elsewhere, cleared or pruned by retention while this device was
  /// offline), so a full page set is first collected in [fresh] and only
  /// then swapped in — never written into [_byId] piecemeal.
  Future<void> _backfill() async {
    if (_backfilling) return;
    _backfilling = true;
    _retry?.cancel();
    _retry = null;
    final api = _apiFactory();
    final fresh = <String, AgentNotification>{};
    var failed = false;
    try {
      var after = 0;
      while (true) {
        final page = await api.notifications(after: after, limit: 200);
        for (final n in page.items) {
          if (!n.cancelled) fresh[n.id] = n;
        }
        if (page.items.length < 200 || page.cursor == after) break;
        after = page.cursor;
      }
      // Prefs/mutes failing (a transient error) must not stop the inbox
      // itself from being usable, and must not be mistaken for "no server
      // support" (item 4) — only a 404 on the inbox list itself means that.
      try {
        _prefs = await api.notificationPrefs();
      } catch (_) {}
      try {
        _mutes = await api.notificationMutes();
      } catch (_) {}
      _byId
        ..clear()
        ..addAll(fresh);
      _supported = true;
      _backfillFailed = false;
    } catch (e) {
      failed = true;
      if (e is AgentRefusal && e.statusCode == 404) {
        _unsupported = true;
      } else {
        // Offline, a 5xx: retried below, never mistaken for "no support".
        _backfillFailed = true;
      }
    } finally {
      api.dispose();
      _loading = false;
      _backfilling = false;
      _invalidate();
      // Replayed strictly after the swap above, so each buffered push (the
      // server's own more recent word) simply overwrites it — this is the
      // only ordering signal available: `updated_seq` exists only inside
      // the server's own cursor logic and is never on the wire (checked
      // against the current server source, including #416's amendments).
      final pending = _pending.toList();
      _pending.clear();
      var sawSnapshot = false;
      for (final e in pending) {
        if (e is AgentSnapshot) sawSnapshot = true;
        applyEvent(e);
      }
      if (!_disposed) notifyListeners();
      // A snapshot buffered mid-backfill is a reconnect `applyEvent` does
      // not itself act on (it carries no notifications of its own); and a
      // failed attempt (anything but a confirmed 404) must not wait for
      // the next reconnect to try again (item N3/1).
      if (sawSnapshot || (failed && !_unsupported)) {
        _retry = Timer(const Duration(seconds: 3), () {
          _retry = null;
          unawaited(_backfill());
        });
      }
    }
  }

  /// Applies one live frame. Public so a unit test can feed frames directly
  /// without a real stream.
  void applyEvent(AgentEvent e) {
    switch (e) {
      case AgentNotificationPushed(:final notification):
        if (notification.cancelled) {
          _byId.remove(notification.id);
        } else {
          _byId[notification.id] = notification;
        }
      case AgentNotificationsRead(:final ids):
        for (final id in ids) {
          final held = _byId[id];
          if (held != null) {
            _byId[id] = held.withReadAt(
              DateTime.now().toUtc().toIso8601String(),
            );
          }
        }
      case AgentNotificationsRemoved(:final ids):
        for (final id in ids) {
          _byId.remove(id);
        }
      case AgentPrefsChanged(:final prefs):
        _prefs = prefs;
      case AgentMutesChanged(:final mutes):
        _mutes = mutes;
      case AgentNotificationsCleared():
        _byId.clear();
      default:
        return;
    }
    _invalidate();
    if (!_disposed) notifyListeners();
  }

  Future<void> markRead(List<String> ids) async {
    final api = _apiFactory();
    try {
      await api.markNotificationsRead(ids: ids);
      final at = DateTime.now().toUtc().toIso8601String();
      for (final id in ids) {
        final held = _byId[id];
        if (held != null) _byId[id] = held.withReadAt(at);
      }
      _invalidate();
      if (!_disposed) notifyListeners();
    } finally {
      api.dispose();
    }
  }

  /// Mark-as-unread. Optimistic, like the rest of this class's mutations
  /// are not — this one is, because [AgentNotification.withUnread]'s
  /// effect (the row jumping into the unread filter, the badge ticking
  /// up) is otherwise visibly delayed by a round trip. Rolled back to
  /// exactly what was held before on failure, never a guessed `read_at`.
  Future<void> markUnread(List<String> ids) async {
    final before = {
      for (final id in ids)
        if (_byId[id] != null) id: _byId[id]!,
    };
    if (before.isEmpty) return;
    for (final entry in before.entries) {
      _byId[entry.key] = entry.value.withUnread();
    }
    _invalidate();
    if (!_disposed) notifyListeners();
    final api = _apiFactory();
    try {
      await api.markNotificationsUnread(ids);
    } catch (e) {
      for (final entry in before.entries) {
        _byId[entry.key] = entry.value;
      }
      _invalidate();
      if (!_disposed) notifyListeners();
      rethrow;
    } finally {
      api.dispose();
    }
  }

  Future<void> markAllRead() async {
    final unread = [
      for (final n in _byId.values)
        if (n.unread) n.id,
    ];
    if (unread.isNotEmpty) await markRead(unread);
  }

  /// Best-effort: opening a session calls this alongside the output
  /// watermark regardless of whether the server has this inbox at all, so
  /// a refusal (an older server, briefly offline) is swallowed rather than
  /// surfaced — there is nothing useful to tell the person opening a
  /// session about their notifications sync.
  Future<void> markSessionRead(String sessionId) async {
    final api = _apiFactory();
    try {
      await api.markNotificationsRead(sessionId: sessionId);
      final at = DateTime.now().toUtc().toIso8601String();
      for (final entry in _byId.entries.toList()) {
        if (entry.value.sessionId == sessionId && entry.value.unread) {
          _byId[entry.key] = entry.value.withReadAt(at);
        }
      }
      _invalidate();
      if (!_disposed) notifyListeners();
    } catch (_) {
      // See doc comment: never surfaced.
    } finally {
      api.dispose();
    }
  }

  Future<void> clear() async {
    final api = _apiFactory();
    try {
      await api.clearNotifications();
      _byId.clear();
      _invalidate();
      if (!_disposed) notifyListeners();
    } finally {
      api.dispose();
    }
  }

  Future<NotificationPrefs> putPrefs(Map<String, dynamic> patch) async {
    final current = _prefs;
    if (current == null) throw StateError('prefs not loaded yet');
    final api = _apiFactory();
    try {
      final next = await api.putNotificationPrefs(current.version, patch);
      _prefs = next;
      if (!_disposed) notifyListeners();
      return next;
    } finally {
      api.dispose();
    }
  }

  /// Re-`GET`s prefs after a 409 `prefs_version` conflict, so the next edit
  /// starts from the current row rather than clobbering it.
  Future<NotificationPrefs> reloadPrefs() async {
    final api = _apiFactory();
    try {
      final p = await api.notificationPrefs();
      _prefs = p;
      if (!_disposed) notifyListeners();
      return p;
    } finally {
      api.dispose();
    }
  }

  Future<NotificationMute> createMute(
    String scope,
    String target, {
    String? until,
  }) async {
    final api = _apiFactory();
    try {
      final m = await api.createNotificationMute(scope, target, until: until);
      _mutes = [..._mutes.where((e) => e.id != m.id), m];
      if (!_disposed) notifyListeners();
      return m;
    } finally {
      api.dispose();
    }
  }

  Future<void> deleteMute(String id) async {
    final api = _apiFactory();
    try {
      await api.deleteNotificationMute(id);
      _mutes = _mutes.where((m) => m.id != id).toList();
      if (!_disposed) notifyListeners();
    } finally {
      api.dispose();
    }
  }

  Future<NotificationPrefs> pause({String? until}) async {
    final api = _apiFactory();
    try {
      final p = await api.pauseNotifications(until: until);
      _prefs = p;
      if (!_disposed) notifyListeners();
      return p;
    } finally {
      api.dispose();
    }
  }

  Future<NotificationPrefs> unpause() async {
    final api = _apiFactory();
    try {
      final p = await api.unpauseNotifications();
      _prefs = p;
      if (!_disposed) notifyListeners();
      return p;
    } finally {
      api.dispose();
    }
  }

  Future<AgentNotification> sendTest() async {
    final api = _apiFactory();
    try {
      return await api.sendTestNotification();
    } finally {
      api.dispose();
    }
  }

  /// 161-9: the ntfy sink's own test.
  Future<NtfyTestResult> sendTestNtfy() async {
    final api = _apiFactory();
    try {
      return await api.sendTestNtfy();
    } finally {
      api.dispose();
    }
  }

  @override
  void dispose() {
    _disposed = true;
    _sub?.cancel();
    _retry?.cancel();
    super.dispose();
  }
}

/// Null without a signed-in session. One instance per app lifetime of the
/// session, alive while the bell, the inbox or Settings › Notifications is.
final notificationsStateProvider = Provider.autoDispose<NotificationsState?>((
  ref,
) {
  final factory = ref.watch(agentApiFactoryProvider);
  if (factory == null) return null;
  final events = ref.watch(agentEventsProvider);
  final state = NotificationsState(factory, events);
  ref.onDispose(state.dispose);
  return state;
});
