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

/// Whether [deliver] (161-4's hint on a live frame) means this device
/// should raise a local OS notification right now, given whether it is
/// itself [thisDeviceActive] (review item 1):
///
/// - `"toast:<id>"` is never this notifier's job, on *any* device — it
///   means that one device shows an in-app toast (`agent_notices.dart`),
///   not an OS notification.
/// - `"os"` raises on every backgrounded desktop. If this device somehow
///   became active after the server computed the hint, it shows nothing
///   — an OS popup while the person is already looking at Storm is
///   exactly what the toast path exists to avoid.
/// - `null` (key present, delivered nowhere) or no key at all (an older
///   server) never raises one either.
bool shouldRaiseOsNotification(
  String? deliver, {
  required bool thisDeviceActive,
}) => deliver == 'os' && !thisDeviceActive;

/// Whether this device's own prefs let [n] through at all. `test` and
/// `summary` bypass the class filter (review item 10): a self-requested
/// test has to prove delivery regardless of what classes are on, and a
/// quiet-hours summary has no device-level channel of its own — only
/// [DeviceNotificationPrefs.enabled] gates either.
bool devicePrefsAllow(AgentNotification n, DeviceNotificationPrefs prefs) {
  if (!prefs.enabled) return false;
  if (n.kind == 'test' || n.kind == 'summary') return true;
  return prefs.classOn(deviceNotificationClassOf(n));
}

/// Maps a notice onto 161-4's "Send to this device" classes, which are
/// coarser than [kNotificationEventKinds] (that table is the account-level
/// *event* toggles; this one is per-device *channels*). Every kind Storm
/// can emit is named here or falls into `agent_messages` only because it
/// is agent- or terminal-sourced — never because it was merely
/// unrecognised (review item 10: `test`/`summary` never reach this at
/// all, gated in [devicePrefsAllow] first).
String deviceNotificationClassOf(AgentNotification n) {
  if (n.source != 'storm') return 'agent_messages';
  return switch (n.kind) {
    'needs_you' => 'needs_you',
    'delete_request' => 'delete_requests',
    'finished' || 'stopped_working' || 'child_finished' => 'finished',
    'error' || 'usage_limit' => 'errors',
    'host_offline' => 'hosts',
    _ => 'agent_messages',
  };
}

/// What a notification actually shows, by preview level (161-4's table):
/// `full` is "{session} · {Agent}" plus the real text; `session` is
/// "{session} · {kind}"; `storm` is "Storm · {kind}" — never the agent's
/// own words, and `{kind}` is always a human label (review item 9), never
/// the wire code.
({String title, String? body}) renderPreview(
  AgentNotification n,
  String previewLevel,
) {
  final session = n.sessionTitle ?? 'Storm';
  final label = noticeKindLabel(n.kind);
  switch (previewLevel) {
    case 'session':
      return (title: '$session · $label', body: null);
    case 'storm':
      return (title: 'Storm · $label', body: null);
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
/// fold for its whole session, itself rendered at [previewLevel] (review
/// item 8: `"Storm · 3 updates"` at the `storm` level, never the session
/// name). `intId` is what the plugin cancels by later (a read, a clear,
/// or the next notice in the same burst replacing this one); `folded`
/// says whether this call folded, so the caller can track which notice
/// ids the one shown `intId` now represents.
({int intId, String title, String? body, bool folded}) planOsNotification(
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
      final session = previewLevel == 'storm'
          ? 'Storm'
          : (n.sessionTitle ?? 'Storm');
      return (
        intId: sessionBurstIntId(sessionId),
        title: '$session · $count updates',
        body: null,
        folded: true,
      );
    }
  }
  final preview = renderPreview(n, previewLevel);
  return (
    intId: notificationIntId(n.id),
    title: preview.title,
    body: preview.body,
    folded: false,
  );
}

/// `agent`/`terminal` notices get their own OS category, distinct from
/// Storm's own automatic ones (161-4's "Agent messages" channel) — an
/// agent can never dress its text up as a Storm system notice. These are
/// the macOS category identifiers [OsNotifications.init] registers
/// (review item 17) and the Linux category hint `_RealOsNotifications`
/// maps them to.
const kStormNotificationCategory = 'storm';
const kAgentMessagesNotificationCategory = 'agent-messages';

String osCategoryOf(AgentNotification n) => n.source == 'storm'
    ? kStormNotificationCategory
    : kAgentMessagesNotificationCategory;

/// Where a tap should go. [NotificationTapNone] is still a real target —
/// review item 18: even a notice with no session brings the window
/// forward, it just has nowhere further to route.
sealed class NotificationTapTarget {}

class NotificationTapSession extends NotificationTapTarget {
  NotificationTapSession(this.sessionId);
  final String sessionId;
}

class NotificationTapNone extends NotificationTapTarget {}

/// Fired when the person taps a Storm-raised OS notification from outside
/// the widget tree — the same pattern `OAuthLinks.instance.orphans` uses
/// for a deep link the app was not yet listening for. `_StormAppState`
/// listens and routes, after bringing the window forward.
final notificationTapped = ValueNotifier<NotificationTapTarget?>(null);

NotificationTapTarget decodeTapPayload(String? payload) {
  // `session:<id>` is the only shape ever written (see `_raise` below);
  // kept this explicit, rather than `jsonDecode`, so a stray payload from
  // a notification shown before an upgrade never throws decoding it.
  const prefix = 'session:';
  if (payload != null && payload.startsWith(prefix)) {
    return NotificationTapSession(payload.substring(prefix.length));
  }
  return NotificationTapNone();
}

/// macOS only: `allowed`/`denied` (asked, and the answer); `notAsked`
/// (macOS, never asked yet — review item 2: asking happens lazily, at
/// the first notification, never at launch); `notApplicable` elsewhere,
/// which have no such permission; `unavailable` when the plugin itself
/// failed (review item 11).
enum DesktopPermissionStatus {
  notApplicable,
  notAsked,
  allowed,
  denied,
  unavailable,
}

/// The plugin calls [DesktopNotifier] needs, behind a seam tests can fake
/// — `flutter_local_notifications` talks to real platform channels, which
/// don't exist under `flutter test`.
abstract class OsNotifications {
  Future<void> init({required void Function(String? payload) onTap});
  Future<void> show({
    required int id,
    required String title,
    String? body,
    String? payload,
    String? threadId,
    required String category,
    required bool sound,
  });
  Future<void> cancel(int id);
  Future<void> cancelAll();

  /// `true`/`false` on macOS; `null` elsewhere (no such concept).
  Future<bool?> checkPermission();

  /// Asks once; a no-op everywhere but macOS.
  Future<void> requestPermission();

  /// Best-effort: there is no cross-platform API for this, so each
  /// platform is its own settings URI. `false` on a platform with
  /// neither (Linux has none standard).
  Future<bool> openSystemSettings();
}

class RealOsNotifications implements OsNotifications {
  final _plugin = FlutterLocalNotificationsPlugin();
  bool _initialized = false;

  @override
  Future<void> init({required void Function(String? payload) onTap}) async {
    if (_initialized) return;
    await _plugin.initialize(
      settings: InitializationSettings(
        // Alert/sound/badge permission is asked lazily by
        // `requestPermission` (review item 2: at the first notification,
        // not at launch); categories registered up front cost nothing
        // and have no actions (138p-7).
        macOS: const DarwinInitializationSettings(
          requestAlertPermission: false,
          requestBadgePermission: false,
          requestSoundPermission: false,
          notificationCategories: [
            DarwinNotificationCategory(kStormNotificationCategory),
            DarwinNotificationCategory(kAgentMessagesNotificationCategory),
          ],
        ),
        linux: const LinuxInitializationSettings(defaultActionName: 'Open'),
        windows: WindowsInitializationSettings(
          appName: 'Storm',
          appUserModelId: 'dev.storm.Storm',
          guid: '6f1b1c0e-6b7b-4a9e-9b0a-2a9b7c7a0f1a',
        ),
      ),
      onDidReceiveNotificationResponse: (r) => onTap(r.payload),
    );
    _initialized = true;
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
  }) => _plugin.show(
    id: id,
    title: title,
    body: body,
    payload: payload,
    notificationDetails: NotificationDetails(
      macOS: DarwinNotificationDetails(
        threadIdentifier: threadId,
        categoryIdentifier: category,
        presentSound: sound,
      ),
      linux: LinuxNotificationDetails(
        suppressSound: !sound,
        category: category == kStormNotificationCategory
            ? LinuxNotificationCategory.im
            : LinuxNotificationCategory.imReceived,
      ),
      windows: const WindowsNotificationDetails(),
    ),
  );

  @override
  Future<void> cancel(int id) => _plugin.cancel(id: id);

  @override
  Future<void> cancelAll() => _plugin.cancelAll();

  @override
  Future<bool?> checkPermission() async {
    if (defaultTargetPlatform != TargetPlatform.macOS) return null;
    final mac = _plugin
        .resolvePlatformSpecificImplementation<
          MacOSFlutterLocalNotificationsPlugin
        >();
    final options = await mac?.checkPermissions();
    return options?.isEnabled;
  }

  @override
  Future<void> requestPermission() async {
    if (defaultTargetPlatform != TargetPlatform.macOS) return;
    final mac = _plugin
        .resolvePlatformSpecificImplementation<
          MacOSFlutterLocalNotificationsPlugin
        >();
    await mac?.requestPermissions(alert: true, sound: true);
  }

  @override
  Future<bool> openSystemSettings() {
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
}

/// One session's live OS notification: its plugin id, and every notice id
/// it currently represents — one while shown individually, every folded
/// notice's id once the 60 s/≥3 fold kicks in (review item 8) — so a read
/// or a removal of *any* of them cancels what is actually on screen.
class _SessionNotice {
  _SessionNotice(this.intId);
  int intId;
  final Set<String> noticeIds = {};
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
    OsNotifications? os,
    // Not `this._prefs`/`this._soundOn` above: an initializing formal's
    // label is the field's own name, and a private name can't be used
    // as a named argument from another library — this constructor is
    // called from `desktop_notifications_test.dart` too.
    // ignore: prefer_initializing_formals
  }) : _prefs = prefs,
       // ignore: prefer_initializing_formals
       _soundOn = soundOn,
       _os = os ?? RealOsNotifications() {
    _sub = events.updates.listen(_heard);
  }

  static bool _alwaysSound() => true;

  final String deviceId;
  final AgentEvents events;
  final DeviceNotificationPrefs? Function() _prefs;
  final bool Function() _soundOn;
  final OsNotifications _os;
  final _bursts = <String, NotificationBurst>{};

  /// Session id → its live OS entry. A notice with no session (a test,
  /// `host_offline`) is tracked only in [_noticeIntId] below and never
  /// folds.
  final _bySession = <String, _SessionNotice>{};

  /// notice id → the plugin id it was shown under, for a sessionless
  /// notice (1:1, never folded) or to find which session's
  /// [_SessionNotice] a given notice id belongs to.
  final _noticeIntId = <String, int>{};
  final _sessionOfNotice = <String, String>{};

  StreamSubscription<AgentEventUpdate>? _sub;
  bool _permissionRequested = false;

  /// The plugin failed to initialize, show, or ask permission (review
  /// item 11): "This device" shows "No notification service" rather than
  /// "Checking…" forever.
  bool unavailable = false;

  Future<void> init() => _os.init(onTap: _onTap);

  void _onTap(String? payload) {
    notificationTapped.value = decodeTapPayload(payload);
  }

  /// [DesktopPermissionStatus.unavailable] if the plugin itself failed —
  /// never thrown, so a page reading this to render a row never crashes
  /// on it.
  Future<DesktopPermissionStatus> permissionStatus() async {
    try {
      await init();
      final granted = await _os.checkPermission();
      if (granted == null) return DesktopPermissionStatus.notApplicable;
      if (granted) return DesktopPermissionStatus.allowed;
      return _permissionRequested
          ? DesktopPermissionStatus.denied
          : DesktopPermissionStatus.notAsked;
    } catch (_) {
      unavailable = true;
      return DesktopPermissionStatus.unavailable;
    }
  }

  Future<bool> openSystemNotificationSettings() => _os.openSystemSettings();

  void _heard(AgentEventUpdate u) {
    switch (u.event) {
      case AgentNotificationPushed(:final notification):
        unawaited(_raise(notification));
      case AgentNotificationsRead(:final ids):
        for (final id in ids) {
          _cancelForNoticeId(id);
        }
      case AgentNotificationsRemoved(:final ids):
        for (final id in ids) {
          _cancelForNoticeId(id);
        }
      case AgentNotificationsCleared():
        _bySession.clear();
        _noticeIntId.clear();
        _sessionOfNotice.clear();
        unawaited(_os.cancelAll().catchError((_) {}));
      default:
        return;
    }
  }

  /// Review item 8: a read or removal of any notice in a fold (or the
  /// lone notice behind an unfolded entry) cancels whatever is actually
  /// shown for its session, and drops the bookkeeping for the rest of
  /// that fold with it — a stale rollup never outlives every notice it
  /// once counted.
  void _cancelForNoticeId(String noticeId) {
    final intId = _noticeIntId.remove(noticeId);
    final sessionId = _sessionOfNotice.remove(noticeId);
    if (sessionId == null) {
      if (intId != null) unawaited(_os.cancel(intId).catchError((_) {}));
      return;
    }
    final tracked = _bySession.remove(sessionId);
    if (tracked == null) return;
    unawaited(_os.cancel(tracked.intId).catchError((_) {}));
    for (final id in tracked.noticeIds) {
      _noticeIntId.remove(id);
      _sessionOfNotice.remove(id);
    }
  }

  Future<void> _raise(AgentNotification n) async {
    if (n.silent) return;
    if (!shouldRaiseOsNotification(
      n.deliver,
      thisDeviceActive: events.isActive,
    )) {
      return;
    }
    final prefs = _prefs();
    if (prefs == null || !devicePrefsAllow(n, prefs)) return;
    try {
      await _show(n, prefs);
    } catch (_) {
      unavailable = true;
    }
  }

  /// Bypasses [shouldRaiseOsNotification] and presence entirely — only
  /// [devicePrefsAllow] still applies — for "Send a test notification"
  /// (review item 13): the person is looking right at Settings, so the
  /// server's own hint for a notice with no session would normally be a
  /// toast or nothing, neither of which proves the OS path works.
  Future<void> forceShow(AgentNotification n) async {
    final prefs = _prefs();
    if (prefs == null || !devicePrefsAllow(n, prefs)) return;
    try {
      await _show(n, prefs);
    } catch (_) {
      unavailable = true;
    }
  }

  Future<void> _show(AgentNotification n, DeviceNotificationPrefs prefs) async {
    await init();
    if (!_permissionRequested) {
      _permissionRequested = true;
      await _os.requestPermission();
    }
    final plan = planOsNotification(n, prefs.preview, _bursts, DateTime.now());
    final sessionId = n.sessionId;
    if (sessionId != null) {
      // One OS entry per session, always (review item 8): a new notice
      // replaces whatever was shown for this session rather than
      // stacking beside it, so a fold's own arrival is just one more
      // replacement — the individual notices it absorbs are exactly the
      // ones already superseded on the way there, cancelled as they go
      // rather than left behind for the rollup to join. [tracked]
      // accumulates every notice id that is this way "covered" by
      // whatever is currently shown, so a read or removal of *any* of
      // them (even an already-superseded one) cancels the current entry.
      final existing = _bySession[sessionId];
      if (existing != null && existing.intId != plan.intId) {
        unawaited(_os.cancel(existing.intId).catchError((_) {}));
      }
      final tracked = existing ?? _SessionNotice(plan.intId);
      tracked.intId = plan.intId;
      tracked.noticeIds.add(n.id);
      _bySession[sessionId] = tracked;
      _sessionOfNotice[n.id] = sessionId;
    } else {
      _noticeIntId[n.id] = plan.intId;
    }
    await _os.show(
      id: plan.intId,
      title: plan.title,
      body: plan.body,
      payload: sessionId == null ? null : 'session:$sessionId',
      threadId: sessionId,
      category: osCategoryOf(n),
      sound: _soundOn(),
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
