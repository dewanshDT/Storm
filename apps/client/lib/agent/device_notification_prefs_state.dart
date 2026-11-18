/// Decision 161, slice 161-4: this device's own notification settings —
/// the "This device" section's data, kept separately from the account's
/// [NotificationsState] because it lives in `auth.db`, not `agent.db`, and
/// is readable for every device but writable only by its own.
library;

import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import 'agent_api.dart';
import 'agent_state.dart' show agentApiFactoryProvider;
import 'desktop_platform.dart';
import 'notifications_models.dart';

class DeviceNotificationPrefsState extends ChangeNotifier {
  DeviceNotificationPrefsState(this._apiFactory) {
    unawaited(refresh());
  }

  final AgentApi Function() _apiFactory;

  DeviceNotificationPrefs? _prefs;
  bool _loading = true;
  Object? _error;
  bool _disposed = false;

  DeviceNotificationPrefs? get prefs => _prefs;
  bool get loading => _loading;
  Object? get error => _error;

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
    super.dispose();
  }
}

/// Null without a signed-in session. One instance per app lifetime of the
/// session, alive while Settings › Notifications is.
final deviceNotificationPrefsProvider =
    Provider.autoDispose<DeviceNotificationPrefsState?>((ref) {
      final factory = ref.watch(agentApiFactoryProvider);
      if (factory == null) return null;
      final state = DeviceNotificationPrefsState(factory);
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
