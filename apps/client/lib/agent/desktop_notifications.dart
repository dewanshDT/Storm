/// Decision 161, slice 161-4: desktop OS notifications, driven by the
/// server's `deliver` hint on live `notification` frames. The server
/// decides *who* gets told (the deliver hint); this device only decides
/// whether it is itself the one hinted, and how to render what it shows.
library;

import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter_local_notifications/flutter_local_notifications.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';
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
    // Nit: a tap that *launched* the app never reaches
    // `onDidReceiveNotificationResponse` — the plugin was not listening
    // yet — so a cold launch from a notification would otherwise land on
    // the ordinary start screen with no idea which session it was about.
    final launch = await _plugin.getNotificationAppLaunchDetails();
    if (launch?.didNotificationLaunchApp ?? false) {
      onTap(launch?.notificationResponse?.payload);
    }
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
      // Review item K: per-session grouping has no Windows equivalent in
      // this plugin, but muting does — `.silent()` is the only way to
      // suppress the toast's own default sound.
      windows: WindowsNotificationDetails(
        audio: sound ? null : WindowsNotificationAudio.silent(),
      ),
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

/// One session's live *fold* — only exists once its burst has actually
/// folded (review item A: an unfolded notice never joins this; it would
/// otherwise cancel an unrelated, still-unread sibling notice below the
/// fold threshold). Holds every notice id the fold absorbed, so a read or
/// removal of *any* of them cancels the one rollup they share.
class _SessionFold {
  _SessionFold(this.intId);
  final int intId;
  final Set<String> noticeIds = {};
}

/// How long [DesktopNotifier] remembers that macOS permission was asked,
/// surviving a restart (review item F) — `checkPermissions()` alone
/// cannot tell "never asked" from "asked and denied" apart, so without
/// this a denial looks like `notAsked` again on every launch.
const _kPermissionRequestedKey = 'storm.notifications.macosPermissionRequested';

/// Raises/cancels local OS notifications for the signed-in session's
/// `AgentEvents`, while this device's own prefs allow it. One instance per
/// signed-in session, alive only on desktop (161-4; amends 154 Q2 for
/// desktop only — phones and web never see this).
///
/// A [ChangeNotifier] so a page showing [unavailable] (review item G)
/// updates the moment a later `show()` fails, not only at the point it
/// first asked.
class DesktopNotifier extends ChangeNotifier {
  // A private initializing formal's named argument at the call site is
  // still its public name with the underscore stripped (`prefs:`,
  // `soundOn:`) — Dart resolves that for us, so this reads fine from
  // `desktop_notifications_test.dart` even though the fields are
  // private.
  DesktopNotifier({
    required this.deviceId,
    required this.events,
    required this._prefs,
    this._soundOn = _alwaysSound,
    OsNotifications? os,
  }) : _os = os ?? RealOsNotifications() {
    _sub = events.updates.listen(_heard);
  }

  static bool _alwaysSound() => true;

  final String deviceId;
  final AgentEvents events;
  final DeviceNotificationPrefs? Function() _prefs;
  final bool Function() _soundOn;
  final OsNotifications _os;
  final _bursts = <String, NotificationBurst>{};

  /// Session id → its live fold, only once one exists (review item A).
  final _foldBySession = <String, _SessionFold>{};

  /// Session id → the notice ids currently shown *individually* (not yet
  /// folded) for it, oldest first — exactly what a fold, the moment it
  /// starts, needs to cancel.
  final _recentIndividual = <String, List<String>>{};

  /// notice id → the plugin id it is shown under right now, for a
  /// sessionless notice or a still-individual one. A folded notice is
  /// never in here — [_foldBySession] is its only record.
  final _noticeIntId = <String, int>{};

  /// notice id → its session, for every notice this is still tracking in
  /// any form (individual or folded) — the only way [_cancelForNoticeId]
  /// knows where to look.
  final _sessionOfNotice = <String, String>{};

  StreamSubscription<AgentEventUpdate>? _sub;

  /// Null once loaded; [_permissionRequested] is the awaited value.
  bool? _permissionRequestedCache;
  Future<bool>? _permissionRequestedLoad;

  /// Guards concurrent first calls to [_show] (review nit): two notices
  /// arriving at once must never both call `requestPermission`.
  Future<void>? _permissionInFlight;

  bool _unavailable = false;

  /// The plugin failed to initialize, show, or ask permission (review
  /// item 11/G): "This device" shows "No notification service" rather
  /// than "Checking…" forever, updating live via [ChangeNotifier] the
  /// moment a later call fails too.
  bool get unavailable => _unavailable;

  void _setUnavailable() {
    if (_unavailable) return;
    _unavailable = true;
    notifyListeners();
  }

  Future<void> init() => _os.init(onTap: _onTap);

  void _onTap(String? payload) {
    notificationTapped.value = decodeTapPayload(payload);
  }

  Future<bool> _permissionRequested() async {
    if (_permissionRequestedCache != null) return _permissionRequestedCache!;
    return _permissionRequestedLoad ??= () async {
      var requested = false;
      try {
        final prefs = await SharedPreferences.getInstance();
        requested = prefs.getBool(_kPermissionRequestedKey) ?? false;
      } catch (_) {
        // Tests and previews have no preferences store; `false` (never
        // asked) is the safe default either way.
      }
      return _permissionRequestedCache = requested;
    }();
  }

  Future<void> _markPermissionRequested() async {
    _permissionRequestedCache = true;
    try {
      final prefs = await SharedPreferences.getInstance();
      await prefs.setBool(_kPermissionRequestedKey, true);
    } catch (_) {}
  }

  /// [DesktopPermissionStatus.unavailable] if the plugin itself failed —
  /// never thrown, so a page reading this to render a row never crashes
  /// on it. Reads the real OS status every time (review item F) rather
  /// than trusting any in-memory flag on its own — only "has this device
  /// ever asked" needs to survive a restart, and that part is persisted.
  Future<DesktopPermissionStatus> permissionStatus() async {
    try {
      await init();
      final granted = await _os.checkPermission();
      if (granted == null) return DesktopPermissionStatus.notApplicable;
      if (granted) return DesktopPermissionStatus.allowed;
      return await _permissionRequested()
          ? DesktopPermissionStatus.denied
          : DesktopPermissionStatus.notAsked;
    } catch (_) {
      _setUnavailable();
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
        _foldBySession.clear();
        _recentIndividual.clear();
        _noticeIntId.clear();
        _sessionOfNotice.clear();
        _bursts.clear();
        unawaited(_os.cancelAll().catchError((_) {}));
      default:
        return;
    }
  }

  /// Review item A: a read or removal only ever cancels what [noticeId]
  /// is actually part of — the whole fold if it was folded, or just its
  /// own still-individual entry otherwise — and never an unrelated
  /// sibling notice for the same session.
  void _cancelForNoticeId(String noticeId) {
    final sessionId = _sessionOfNotice.remove(noticeId);
    if (sessionId == null) {
      final intId = _noticeIntId.remove(noticeId);
      if (intId != null) unawaited(_os.cancel(intId).catchError((_) {}));
      return;
    }
    final fold = _foldBySession[sessionId];
    if (fold != null && fold.noticeIds.contains(noticeId)) {
      _foldBySession.remove(sessionId);
      _bursts.remove(
        sessionId,
      ); // review item A: prune — a later notice starts fresh
      unawaited(_os.cancel(fold.intId).catchError((_) {}));
      for (final id in fold.noticeIds) {
        _noticeIntId.remove(id);
        _sessionOfNotice.remove(id);
      }
      return;
    }
    final intId = _noticeIntId.remove(noticeId);
    _recentIndividual[sessionId]?.remove(noticeId);
    if (_recentIndividual[sessionId]?.isEmpty ?? false) {
      _recentIndividual.remove(sessionId);
    }
    if (intId != null) unawaited(_os.cancel(intId).catchError((_) {}));
  }

  Future<void> _raise(AgentNotification n) async {
    if (n.silent) return;
    // Review item E: a test notice is never raised from the live stream
    // — only `forceShow`, by the device that actually asked for it,
    // raises one. Every other device hearing the same live frame (every
    // connected device hears every frame) must ignore it here, or it
    // would show twice on the requester and once more on whichever
    // device the hint happened to name.
    if (n.kind == 'test') return;
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
      _setUnavailable();
    }
  }

  /// Bypasses [shouldRaiseOsNotification] and presence entirely — only
  /// [devicePrefsAllow] still applies — for "Send a test notification"
  /// (review item 13): the person is looking right at Settings, so the
  /// server's own hint for a notice with no session would normally be a
  /// toast or nothing, neither of which proves the OS path works. Also
  /// refreshes [permissionStatus] after (review item E), so the page's
  /// own read of it reflects whatever the attempt just revealed.
  Future<void> forceShow(AgentNotification n) async {
    final prefs = _prefs();
    if (prefs != null && devicePrefsAllow(n, prefs)) {
      try {
        await _show(n, prefs);
      } catch (_) {
        _setUnavailable();
      }
    }
    unawaited(permissionStatus());
  }

  Future<void> _show(AgentNotification n, DeviceNotificationPrefs prefs) async {
    await init();
    await (_permissionInFlight ??= _ensurePermission().whenComplete(() {
      _permissionInFlight = null;
    }));
    final plan = planOsNotification(n, prefs.preview, _bursts, DateTime.now());
    final sessionId = n.sessionId;
    if (sessionId != null) {
      if (plan.folded) {
        final isNewFold = !_foldBySession.containsKey(sessionId);
        final fold = _foldBySession.putIfAbsent(
          sessionId,
          () => _SessionFold(plan.intId),
        );
        if (isNewFold) {
          // Review item A: only *now*, the moment a burst actually
          // folds, do the individual notices it absorbed get cancelled
          // — never on every new individual notice below the threshold.
          final absorbed = _recentIndividual.remove(sessionId) ?? const [];
          for (final id in absorbed) {
            final intId = _noticeIntId.remove(id);
            if (intId != null) unawaited(_os.cancel(intId).catchError((_) {}));
          }
          fold.noticeIds.addAll(absorbed);
        }
        fold.noticeIds.add(n.id);
        _sessionOfNotice[n.id] = sessionId;
      } else {
        _noticeIntId[n.id] = plan.intId;
        _sessionOfNotice[n.id] = sessionId;
        _recentIndividual.putIfAbsent(sessionId, () => []).add(n.id);
      }
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

  Future<void> _ensurePermission() async {
    if (await _permissionRequested()) return;
    await _markPermissionRequested();
    await _os.requestPermission();
  }

  @override
  void dispose() {
    unawaited(_sub?.cancel());
    super.dispose();
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
  // Review item C: `deviceId` only, not the whole `Settings` — watching
  // the full provider rebuilt (and so recreated) this notifier, losing
  // its fold/cancel bookkeeping and permission state, on an unrelated
  // change (theme, font size, …).
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
  );
  ref.onDispose(notifier.dispose);
  return notifier;
});
