/// Decision 161, slice 161-4: desktop OS notifications, driven by the
/// server's `deliver` hint on live `notification` frames. The server
/// decides *who* gets told (the deliver hint); this device only decides
/// whether it is itself the one hinted, and how to render what it shows.
library;

import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter_local_notifications/flutter_local_notifications.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:url_launcher/url_launcher.dart';

import '../state/app_state.dart' show settingsProvider;
import 'agent_events.dart';
import 'agent_models.dart' show providerLabel;
import 'desktop_platform.dart';
import 'device_notification_prefs_state.dart';
import 'notifications_models.dart';
import 'this_device_local_prefs.dart';

/// Whether [deliver] (161-4's hint on a live frame) means *this* device,
/// identified by [deviceId], should raise a local OS notification.
/// `null` (an older server, or "an active device already has it open")
/// never raises one.
bool shouldRaiseOsNotification(String? deliver, String deviceId) {
  if (deliver == null) return false;
  return deliver == 'os' || deliver == 'toast:$deviceId';
}

/// Whether this device's own prefs let [n] through at all (enabled, and
/// its class — mapped from [AgentNotification.kind]/[source] — is one
/// [prefs] sends here; `classes: []` means every class).
bool devicePrefsAllow(AgentNotification n, DeviceNotificationPrefs prefs) {
  if (!prefs.enabled) return false;
  return prefs.classOn(deviceNotificationClassOf(n));
}

/// Maps a notice onto 161-4's "Send to this device" classes, which are
/// coarser than [kNotificationEventKinds] (that table is the account-level
/// *event* toggles; this one is per-device *channels*).
String deviceNotificationClassOf(AgentNotification n) {
  if (n.source != 'storm') return 'agent_messages';
  return switch (n.kind) {
    'needs_you' => 'needs_you',
    'delete_request' => 'delete_requests',
    'finished' => 'finished',
    'error' || 'usage_limit' => 'errors',
    'host_offline' => 'hosts',
    _ => 'agent_messages',
  };
}

/// What a notification actually shows, by preview level (161-4's table):
/// `full` is "{session} · {Agent}" plus the real text; `session` is
/// "{session} · {kind}"; `storm` is "Storm · {kind}" — never the agent's
/// own words.
({String title, String? body}) renderPreview(
  AgentNotification n,
  String previewLevel,
) {
  final session = n.sessionTitle ?? 'Storm';
  switch (previewLevel) {
    case 'session':
      return (title: '$session · ${n.kind}', body: null);
    case 'storm':
      return (title: 'Storm · ${n.kind}', body: null);
    default: // 'full'
      final agent = n.provider == null
          ? session
          : '$session · ${providerLabel(n.provider!)}';
      final text = n.body == null || n.body!.isEmpty
          ? n.title
          : '${n.title}\n${n.body}';
      return (title: agent, body: text);
  }
}

/// A session's recent notices, for the 60 s/≥3 fold rule. Pure and
/// clock-injectable so the rule is unit-testable without waiting real
/// seconds.
class NotificationBurst {
  final _at = <DateTime>[];

  /// Records one more notice now, prunes anything outside [window], and
  /// returns the count still within it (including this one).
  int record(DateTime now, Duration window) {
    _at.add(now);
    _at.removeWhere((t) => now.difference(t) > window);
    return _at.length;
  }
}

const _burstWindow = Duration(seconds: 60);
const _burstThreshold = 3;

/// A stable 31-bit id for the OS notification plugin (which wants an
/// `int`), so showing the same logical notice again — a `key` replace, a
/// burst's folded entry — replaces it instead of stacking a duplicate.
int notificationIntId(String key) => key.hashCode & 0x7fffffff;

int sessionBurstIntId(String sessionId) =>
    notificationIntId('burst:$sessionId');

/// Builds the one thing worth raising for [n], already past
/// [shouldRaiseOsNotification] and [devicePrefsAllow] — either the notice
/// itself, rendered at [previewLevel], or, past the burst threshold, a
/// fold for its whole session. `intId` is what the plugin cancels by
/// later (a read, a clear, or the next notice in the same burst replacing
/// this one).
({int intId, String title, String? body}) planOsNotification(
  AgentNotification n,
  String previewLevel,
  Map<String, NotificationBurst> bursts,
  DateTime now,
) {
  final sessionId = n.sessionId;
  if (sessionId != null) {
    final burst = bursts.putIfAbsent(sessionId, NotificationBurst.new);
    final count = burst.record(now, _burstWindow);
    if (count >= _burstThreshold) {
      final session = n.sessionTitle ?? 'Storm';
      return (
        intId: sessionBurstIntId(sessionId),
        title: '$session · $count updates',
        body: null,
      );
    }
  }
  final preview = renderPreview(n, previewLevel);
  return (
    intId: notificationIntId(n.id),
    title: preview.title,
    body: preview.body,
  );
}

/// `agent`/`terminal` notices get their own OS category, distinct from
/// Storm's own automatic ones (161-4's "Agent messages" channel) — an
/// agent can never dress its text up as a Storm system notice.
String osCategoryOf(AgentNotification n) =>
    n.source == 'storm' ? 'storm' : 'agent-messages';

/// Where a tap should go: the session (default) or, were a note ever
/// added to a notice's payload, a note. Only `session` exists today — no
/// notice currently carries a note reference — kept as a sum type so a
/// later payload shape has somewhere to land without redesigning this.
sealed class NotificationTapTarget {}

class NotificationTapSession extends NotificationTapTarget {
  NotificationTapSession(this.sessionId);
  final String sessionId;
}

/// Fired when the person taps a Storm-raised OS notification from outside
/// the widget tree — the same pattern `OAuthLinks.instance.orphans` uses
/// for a deep link the app was not yet listening for. `_StormAppState`
/// listens and routes, after bringing the window forward.
final notificationTapped = ValueNotifier<NotificationTapTarget?>(null);

NotificationTapTarget? decodeTapPayload(String? payload) {
  if (payload == null || payload.isEmpty) return null;
  // `session:<id>` is the only shape ever written (see `_raise` below);
  // kept this explicit, rather than `jsonDecode`, so a stray payload from
  // a notification shown before an upgrade never throws decoding it.
  const prefix = 'session:';
  if (payload.startsWith(prefix)) {
    return NotificationTapSession(payload.substring(prefix.length));
  }
  return null;
}

/// Raises/cancels local OS notifications for the signed-in session's
/// `AgentEvents`, while this device's own prefs allow it. One instance per
/// signed-in session, alive only on desktop (161-4; amends 154 Q2 for
/// desktop only — phones and web never see this).
class DesktopNotifier {
  DesktopNotifier({
    required this.deviceId,
    required this.events,
    required DeviceNotificationPrefs? Function() prefs,
    bool Function() soundOn = _alwaysSound,
  }) : _prefs = prefs,
       _soundOn = soundOn {
    _sub = events.updates.listen(_heard);
  }

  static bool _alwaysSound() => true;

  final String deviceId;
  final AgentEvents events;
  final DeviceNotificationPrefs? Function() _prefs;
  final bool Function() _soundOn;
  final _plugin = FlutterLocalNotificationsPlugin();
  final _bursts = <String, NotificationBurst>{};
  StreamSubscription<AgentEventUpdate>? _sub;
  bool _ready = false;

  Future<void> init() async {
    if (_ready) return;
    await _plugin.initialize(
      settings: const InitializationSettings(
        macOS: DarwinInitializationSettings(
          requestAlertPermission: false,
          requestBadgePermission: false,
          requestSoundPermission: false,
        ),
        linux: LinuxInitializationSettings(defaultActionName: 'Open'),
        windows: WindowsInitializationSettings(
          appName: 'Storm',
          appUserModelId: 'dev.storm.Storm',
          guid: '6f1b1c0e-6b7b-4a9e-9b0a-2a9b7c7a0f1a',
        ),
      ),
      onDidReceiveNotificationResponse: _onTap,
    );
    _ready = true;
  }

  void _onTap(NotificationResponse response) {
    final target = decodeTapPayload(response.payload);
    if (target != null) notificationTapped.value = target;
  }

  /// `true`/`false` on macOS (the only desktop platform with a real OS
  /// permission for this); `null` on Windows and Linux, which show a
  /// toast unconditionally — "This device" reads `null` as "Allowed".
  Future<bool?> permissionGranted() async {
    await init();
    if (defaultTargetPlatform != TargetPlatform.macOS) return null;
    final mac = _plugin
        .resolvePlatformSpecificImplementation<
          MacOSFlutterLocalNotificationsPlugin
        >();
    final options = await mac?.checkPermissions();
    return options?.isEnabled;
  }

  /// Best-effort: there is no cross-platform API for this, so each branch
  /// is a platform's own settings URI. A platform with neither (Linux has
  /// none standard) leaves the button hidden rather than open nothing.
  static Future<bool> openSystemNotificationSettings() {
    final uri = switch (defaultTargetPlatform) {
      TargetPlatform.macOS => Uri.parse(
        'x-apple.systempreferences:com.apple.preference.notifications',
      ),
      TargetPlatform.windows => Uri.parse('ms-settings:notifications'),
      _ => null,
    };
    if (uri == null) return Future.value(false);
    return launchUrl(uri);
  }

  void _heard(AgentEventUpdate u) {
    switch (u.event) {
      case AgentNotificationPushed(:final notification):
        unawaited(_raise(notification));
      case AgentNotificationsRead(:final ids):
        for (final id in ids) {
          unawaited(_plugin.cancel(id: notificationIntId(id)));
        }
      case AgentNotificationsRemoved(:final ids):
        for (final id in ids) {
          unawaited(_plugin.cancel(id: notificationIntId(id)));
        }
      case AgentNotificationsCleared():
        unawaited(_plugin.cancelAll());
      default:
        return;
    }
  }

  Future<void> _raise(AgentNotification n) async {
    if (n.silent) return;
    if (!shouldRaiseOsNotification(n.deliver, deviceId)) return;
    final prefs = _prefs();
    if (prefs == null || !devicePrefsAllow(n, prefs)) return;
    await init();
    final plan = planOsNotification(n, prefs.preview, _bursts, DateTime.now());
    final category = osCategoryOf(n);
    final sound = _soundOn();
    await _plugin.show(
      id: plan.intId,
      title: plan.title,
      body: plan.body,
      payload: n.sessionId == null ? null : 'session:${n.sessionId}',
      notificationDetails: NotificationDetails(
        macOS: DarwinNotificationDetails(
          threadIdentifier: n.sessionId,
          categoryIdentifier: category,
          presentSound: sound,
        ),
        linux: LinuxNotificationDetails(suppressSound: !sound),
        windows: const WindowsNotificationDetails(),
      ),
    );
  }

  void dispose() {
    unawaited(_sub?.cancel());
  }
}

/// Alive while a desktop app has a signed-in session with a live events
/// stream — nothing on phones or web, where there is no OS notification
/// to raise and no background stream to raise it from. Watch this (not
/// read it) somewhere that is always built, so Riverpod keeps it alive
/// rather than tearing the plugin registration down between frames.
final desktopNotifierProvider = Provider.autoDispose<DesktopNotifier?>((ref) {
  if (!isDesktopPlatform) return null;
  final events = ref.watch(agentEventsProvider);
  final deviceId = ref.watch(settingsProvider).value?.deviceId ?? '';
  if (events == null || deviceId.isEmpty) return null;
  final prefsState = ref.watch(deviceNotificationPrefsProvider);
  final local = ref.watch(thisDeviceLocalPrefsProvider);
  final notifier = DesktopNotifier(
    deviceId: deviceId,
    events: events,
    prefs: () => prefsState?.prefs,
    soundOn: () => local.soundOn,
  );
  ref.onDispose(notifier.dispose);
  return notifier;
});
