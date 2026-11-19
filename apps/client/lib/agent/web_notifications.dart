/// Web notifications while a Storm tab is open (not Web Push, which stays
/// 161-6/7 — this is the plain
/// [Notification](https://developer.mozilla.org/en-US/docs/Web/API/Notification)
/// API, raised from the page itself, no service worker).
///
/// Reuses [DesktopNotifier] wholesale: its fold/burst/preview/cancel
/// engine is already platform-agnostic, needing only an [OsNotifications]
/// implementation to drive — [WebOsNotifications] below is that
/// implementation, so `cancel`/`cancelAll`/a `key` replace and the
/// read/removed/cleared cases all come for free exactly as desktop has
/// them, with no duplicated bookkeeping.
library;

import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../state/app_state.dart' show settingsProvider;
import 'agent_events.dart';
import 'desktop_notifications.dart';
import 'device_notification_prefs_state.dart';
import 'this_device_local_prefs.dart';
import 'web_notification_raw.dart';

/// The Settings page's own three-or-four-way state (the task's own
/// wording: "Allowed / Blocked in browser settings / Not asked", plus
/// "Notifications need HTTPS" for a plain-`http://` origin). Richer than
/// [DesktopPermissionStatus] on purpose: the browser already tells apart
/// "never asked" from "denied" (`Notification.permission`'s own
/// `"default"`), so there is no need for [DesktopNotifier]'s
/// persisted-flag workaround — that workaround exists only because the
/// native platforms collapse to a bare boolean.
enum WebNotificationPermission { insecureOrigin, notAsked, allowed, denied }

/// Reads the browser's answer right now — synchronous, since
/// `Notification.permission` is. `insecureOrigin` off the web build too,
/// though nothing here is ever read there: every caller gates on `kIsWeb`
/// first.
WebNotificationPermission webNotificationPermission() {
  if (!kIsWeb) return WebNotificationPermission.insecureOrigin;
  return mapWebNotificationPermission(
    isSecureContext: rawIsSecureContext(),
    rawPermission: rawPermission(),
  );
}

/// The pure half of [webNotificationPermission] — everything but the two
/// real DOM reads, so the state mapping itself is unit-testable on the
/// VM, where `kIsWeb` is always false and the real `Notification` global
/// does not exist at all.
WebNotificationPermission mapWebNotificationPermission({
  required bool isSecureContext,
  required String rawPermission,
}) {
  if (!isSecureContext) return WebNotificationPermission.insecureOrigin;
  return switch (rawPermission) {
    'granted' => WebNotificationPermission.allowed,
    'denied' => WebNotificationPermission.denied,
    _ => WebNotificationPermission.notAsked, // 'default', or 'unsupported'
  };
}

/// Asks the browser — only ever call this from inside the click that
/// triggered it (the Settings page's "Enable notifications" button):
/// `Notification.requestPermission()` needs a user gesture, and asking
/// from anywhere else risks the browser treating it as refused outright.
Future<WebNotificationPermission> requestWebNotificationPermission() async {
  if (!kIsWeb || !rawIsSecureContext()) return webNotificationPermission();
  await rawRequestPermission();
  return webNotificationPermission();
}

/// Served at the web root by `manifest.json`'s own PWA icon — reused
/// rather than adding a new asset.
const _kWebNotificationIcon = '/icons/Icon-192.png';

/// [OsNotifications] over the plain web `Notification` API, so
/// [DesktopNotifier] can drive it exactly as it drives the native
/// plugins.
class WebOsNotifications implements OsNotifications {
  void Function(String? payload)? _onTap;

  @override
  Future<void> init({required void Function(String? payload) onTap}) async {
    _onTap = onTap;
  }

  @override
  Future<void> show({
    required int id,
    required String title,
    String? body,
    String? payload,
    String? threadId,
    required String category,
    required bool sound,
  }) async {
    rawShow(
      id: id,
      title: title,
      body: body,
      icon: _kWebNotificationIcon,
      payload: payload,
      silent: !sound,
      onClick: (p) => _onTap?.call(p),
    );
  }

  @override
  Future<void> cancel(int id) async => rawClose(id);

  @override
  Future<void> cancelAll() async => rawCloseAll();

  @override
  Future<bool?> checkPermission() async =>
      webNotificationPermission() == WebNotificationPermission.allowed;

  /// Never actually asks (the task's own constraint): [DesktopNotifier]'s
  /// own background `_show` call site has no user gesture to spend, and
  /// asking there risks a permanent browser-side refusal. The only real
  /// ask is [requestWebNotificationPermission], from the Settings page's
  /// button.
  @override
  Future<void> requestPermission() async {}

  /// No web equivalent of "open system settings" (review-pattern default
  /// for a platform with neither).
  @override
  Future<bool> openSystemSettings() async => false;
}

/// Alive while a signed-in web session has a live events stream — nothing
/// on desktop or phones, where [desktopNotifierProvider] or nothing at all
/// (native OS push is 161-6 onward) already covers it. Watch this (not
/// read it) from `main.dart`, the same way `desktopNotifierProvider` is
/// watched, so Riverpod keeps it alive — and so keeps
/// [AgentEvents.setWebNotificationsEnabled] current — for the whole
/// session, not only while Settings › Notifications happens to be open.
final webNotifierProvider = Provider.autoDispose<DesktopNotifier?>((ref) {
  if (!kIsWeb) return null;
  final events = ref.watch(agentEventsProvider);
  final deviceId =
      ref.watch(settingsProvider.select((s) => s.value?.deviceId)) ?? '';
  if (events == null || deviceId.isEmpty) return null;
  final prefsState = ref.watch(deviceNotificationPrefsProvider);
  final local = ref.watch(thisDeviceLocalPrefsProvider);
  final notifier = DesktopNotifier(
    deviceId: deviceId,
    events: events,
    prefs: () => prefsState?.prefs,
    soundOn: () => local.soundOn,
    os: WebOsNotifications(),
  );
  ref.onDispose(notifier.dispose);
  unawaited(notifier.init().catchError((_) {}));

  // 154 Q2, amended for web: a hidden tab keeps its stream open only once
  // this device's own toggle is on *and* the browser has actually granted
  // permission — enabling one without the other would keep a socket open
  // for nothing a tab in the background could ever show. Reacts to every
  // later prefs change (the toggle, or `setEnabled(true)` from the
  // "Enable notifications" button once permission is granted), not only
  // the state this provider happened to build with.
  void pushKeepAlive() {
    final enabled =
        (prefsState?.prefs.enabled ?? false) &&
        webNotificationPermission() == WebNotificationPermission.allowed;
    events.setWebNotificationsEnabled(enabled);
  }

  pushKeepAlive();
  prefsState?.addListener(pushKeepAlive);
  ref.onDispose(() {
    prefsState?.removeListener(pushKeepAlive);
    events.setWebNotificationsEnabled(false);
  });

  return notifier;
});
