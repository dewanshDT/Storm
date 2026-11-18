/// Decision 161, slice 161-4: this device's own notification settings —
/// the "This device" section's data, kept separately from the account's
/// [NotificationsState] because it lives in `auth.db`, not `agent.db`, and
/// is readable for every device but writable only by its own.
library;

import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import 'agent_api.dart';
import 'agent_events.dart';
import 'agent_state.dart' show agentApiFactoryProvider;
import 'desktop_platform.dart';
import 'notifications_models.dart';

class DeviceNotificationPrefsState extends ChangeNotifier {
  DeviceNotificationPrefsState(this._apiFactory, {AgentEvents? events})
    : _prefs = _placeholder() {
    unawaited(refresh());
    _sub = events?.updates.listen(_onEvent);
  }

  final AgentApi Function() _apiFactory;
  StreamSubscription<AgentEventUpdate>? _sub;

  /// Review item 12: shown the moment this is constructed, before the
  /// real row ever answers — this device's own platform default, exactly
  /// what the server would hand back on a first `ensure_device_prefs`
  /// anyway (Q19), so the page is usable at once rather than blank.
  static DeviceNotificationPrefs _placeholder() => DeviceNotificationPrefs(
    deviceId: '',
    enabled: isDesktopPlatform,
    classes: const [],
    preview: 'full',
    updatedAt: '',
  );

  DeviceNotificationPrefs _prefs;
  bool _loading = true;
  Object? _error;
  bool _disposed = false;

  DeviceNotificationPrefs get prefs => _prefs;
  bool get loading => _loading;
  Object? get error => _error;

  /// Review item 12: a fresh connection or reconnect retries a backfill
  /// that failed — the same reason `NotificationsState._backfill` retries
  /// on a snapshot — so a network blip at launch doesn't strand this
  /// device on the placeholder forever.
  void _onEvent(AgentEventUpdate u) {
    if (u.event is AgentSnapshot && _error != null) {
      unawaited(refresh());
    }
  }

  Future<void> refresh() async {
    final api = _apiFactory();
    try {
      _prefs = await api.deviceNotificationPrefs(
        platform: currentPlatformName(),
      );
      _error = null;
    } catch (e) {
      _error = e;
    } finally {
      api.dispose();
      _loading = false;
      if (!_disposed) notifyListeners();
    }
  }

  Future<DeviceNotificationPrefs> patch(Map<String, dynamic> body) async {
    final api = _apiFactory();
    try {
      final next = await api.putDeviceNotificationPrefs(
        body,
        platform: currentPlatformName(),
      );
      _prefs = next;
      _error = null;
      if (!_disposed) notifyListeners();
      return next;
    } finally {
      api.dispose();
    }
  }

  Future<void> setEnabled(bool v) => patch({'enabled': v});

  Future<void> setClasses(List<String> classes) => patch({'classes': classes});

  Future<void> setPreview(String level) => patch({'preview': level});

  /// Sends a real notice through the whole server pipeline to this
  /// device, reusing 161-1's test endpoint — proving delivery, not only
  /// permission.
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
    unawaited(_sub?.cancel());
    super.dispose();
  }
}

/// Null without a signed-in session. One instance per app lifetime of the
/// session, alive while Settings › Notifications is.
final deviceNotificationPrefsProvider =
    Provider.autoDispose<DeviceNotificationPrefsState?>((ref) {
      final factory = ref.watch(agentApiFactoryProvider);
      if (factory == null) return null;
      final events = ref.watch(agentEventsProvider);
      final state = DeviceNotificationPrefsState(factory, events: events);
      ref.onDispose(state.dispose);
      return state;
    });

/// `GET /v1/notifications/devices`: every paired device's row, for the
/// future "Other devices" list. Read-only, so a plain `FutureProvider`
/// (no live stream event drives this — AM124 names only the account-level
/// events) is enough; the Notifications page re-reads it on open.
final notificationDevicesProvider =
    FutureProvider.autoDispose<List<NotificationDeviceRow>>((ref) async {
      final factory = ref.watch(agentApiFactoryProvider);
      if (factory == null) return const [];
      final api = factory();
      try {
        return await api.notificationDevices();
      } finally {
        api.dispose();
      }
    });
