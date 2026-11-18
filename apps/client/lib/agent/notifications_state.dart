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
/// Backfills `GET /v1/notifications` by `cursor` on connect, then applies
/// `notification`/`notification_read`/`prefs_changed`/`mutes_changed`/
/// `notifications_cleared` frames from the one shared [AgentEvents] stream
/// (161-3 reuses it rather than opening a second connection). A server
/// before decision 161 answers the first backfill 404: [unsupported] is
/// then true forever, and nothing here is shown.
class NotificationsState extends ChangeNotifier {
  NotificationsState(this._apiFactory, this._events) {
    unawaited(_start());
  }

  final AgentApi Function() _apiFactory;
  final AgentEvents? _events;

  final _byId = <String, AgentNotification>{};
  NotificationPrefs? _prefs;
  List<NotificationMute> _mutes = const [];

  bool _loading = true;
  bool _unsupported = false;
  StreamSubscription<AgentEventUpdate>? _sub;
  bool _disposed = false;

  /// Newest first.
  List<AgentNotification> get items =>
      _byId.values.toList()..sort((a, b) => b.seq.compareTo(a.seq));

  int get unreadCount => _byId.values.where((n) => n.unread).length;

  NotificationPrefs? get prefs => _prefs;
  List<NotificationMute> get mutes => _mutes;

  /// Backfilling, or waiting on the first prefs/mutes read.
  bool get loading => _loading;

  /// A server before decision 161: the bell, the inbox and Settings ›
  /// Notifications must all hide.
  bool get unsupported => _unsupported;

  Future<void> _start() async {
    final api = _apiFactory();
    try {
      var after = 0;
      while (true) {
        final page = await api.notifications(after: after, limit: 200);
        for (final n in page.items) {
          _byId[n.id] = n;
        }
        if (page.items.length < 200 || page.cursor == after) break;
        after = page.cursor;
      }
      _prefs = await api.notificationPrefs();
      _mutes = await api.notificationMutes();
      _sub = _events?.updates.listen((u) => applyEvent(u.event));
    } catch (e) {
      if (e is AgentRefusal && e.statusCode == 404) {
        _unsupported = true;
      }
      // Any other failure (offline, a transient error) leaves the inbox
      // empty rather than claiming the server has none of this; the next
      // reconnect's backfill tries again.
    } finally {
      api.dispose();
      _loading = false;
      if (!_disposed) notifyListeners();
    }
  }

  /// Applies one live frame. Public so a unit test can feed frames directly
  /// without a real stream.
  void applyEvent(AgentEvent e) {
    switch (e) {
      case AgentNotificationPushed(:final notification):
        _byId[notification.id] = notification;
      case AgentNotificationsRead(:final ids):
        for (final id in ids) {
          final held = _byId[id];
          if (held != null) {
            _byId[id] = held.withReadAt(
              DateTime.now().toUtc().toIso8601String(),
            );
          }
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
      if (!_disposed) notifyListeners();
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

  @override
  void dispose() {
    _disposed = true;
    _sub?.cancel();
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
