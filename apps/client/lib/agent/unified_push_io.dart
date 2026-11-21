/// Decision 161, slice 161-8: Android closed-app push through
/// [UnifiedPush](https://unifiedpush.org) — the distributor protocol ntfy's
/// Android app implements. Android only; every call site here is gated on
/// `Platform.isAndroid`.
///
/// **Why no keypair of our own.** The plugin's native connector
/// (`org.unifiedpush.android:connector`) generates and keeps the WebPush
/// (RFC 8291) P-256 keypair and auth secret itself — this device never
/// sees a private key to store. `register(vapid: …)` hands the
/// distributor our server's VAPID public key; the distributor answers
/// [PushEndpoint.pubKeySet] with *our own* public key and auth secret (not
/// the server's), which is exactly what `POST /v1/push/endpoints` wants as
/// `keys.p256dh`/`keys.auth` — verified against the plugin's own source
/// (`unifiedpush_platform_interface`'s `PublicKeySet` doc comment) rather
/// than assumed.
///
/// **Decryption.** The connector decrypts RFC 8291 on device before
/// [PushMessage] ever reaches Dart (`PushMessage.decrypted`); there is no
/// separate decryption step here.
///
/// **Background delivery, verified against the plugin's own Kotlin
/// source** (`unifiedpush_android`'s `UnifiedPushService`, a
/// `PushService` the plugin's own manifest registers): with the app
/// process alive (foreground or merely backgrounded), a message reaches
/// this file's [UnifiedPushController] directly, the same engine as the
/// rest of the app. With the process **killed**, Android starts that
/// service in a **fresh process**, which builds a brand-new headless
/// `FlutterEngine` and runs this app's own `main()` again with the single
/// argument `--unifiedpush-bg` (`DartExecutor.DartEntrypoint.createDefault()`
/// — the default entrypoint, not a separate one). [runUnifiedPushBackgroundEntrypoint]
/// is what `main.dart` calls for that argument instead of building the
/// widget tree: it only calls [UnifiedPush.initialize] and lets
/// [handleIncomingPushMessage] show the notification. The native side
/// buffers the triggering message in a 20-deep replay buffer
/// (`MutableSharedFlow<Call>(replay = 20)`) until Dart's `initialize` call
/// subscribes, so the message that *woke* the process is never lost to
/// that race — also read from the plugin's own source, not inferred.
library;

import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'dart:math' as math;

import 'package:flutter/foundation.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';
import 'package:unifiedpush/unifiedpush.dart';

import '../state/app_state.dart' show settingsProvider;
import 'agent_api.dart';
import 'agent_notices.dart' show kServerNoticeToastedKey;
import 'agent_state.dart' show agentApiFactoryProvider;
import 'desktop_notifications.dart';
import 'device_notification_prefs_state.dart';
import 'notifications_models.dart';
import 'this_device_local_prefs.dart' show ThisDeviceLocalPrefs;

/// The one instance name this device registers under. UnifiedPush supports
/// several named "instances" per app (e.g. separate accounts); Storm has
/// exactly one signed-in account per device, so one fixed name throughout.
const kUnifiedPushInstance = 'storm';

/// "This device" Android section's state (decision 161's table, "UnifiedPush
/// (Android)" row, plus the enable flow's own steps).
enum UnifiedPushStatus {
  /// Never enabled, or explicitly disabled.
  off,
  enabling,
  on,

  /// `getDistributors()` came back empty — nothing installed that can
  /// carry the push.
  distributorMissing,

  /// `POST_NOTIFICATIONS` was asked and refused.
  permissionDenied,

  /// A registration failed for a reason other than a missing distributor
  /// or permission (see [UnifiedPushController.errorMessage]).
  error,

  /// Not Android, or the plugin itself failed to attach.
  unavailable,
}

/// The seam over the `unifiedpush` package's static [UnifiedPush] class —
/// real on Android, fake in tests. Mirrors [OsNotifications]'s pattern:
/// `flutter_local_notifications`/`unifiedpush` both talk to real platform
/// channels that don't exist under `flutter test`.
abstract class UnifiedPushBackend {
  /// Returns whether a distributor is already saved (mirrors
  /// [UnifiedPush.initialize]'s own return).
  Future<bool> initialize({
    required void Function(PushEndpoint endpoint, String instance)
    onNewEndpoint,
    required void Function(FailedReason reason, String instance)
    onRegistrationFailed,
    required void Function(String instance) onUnregistered,
    required void Function(PushMessage message, String instance) onMessage,
    required void Function(String instance) onTempUnavailable,
  });

  Future<void> register({required String instance, String? vapid});
  Future<void> unregister(String instance);
  Future<List<String>> getDistributors();
  Future<String?> getDistributor();
  Future<void> saveDistributor(String distributor);
  Future<bool> tryUseCurrentOrDefaultDistributor();
}

class RealUnifiedPushBackend implements UnifiedPushBackend {
  @override
  Future<bool> initialize({
    required void Function(PushEndpoint endpoint, String instance)
    onNewEndpoint,
    required void Function(FailedReason reason, String instance)
    onRegistrationFailed,
    required void Function(String instance) onUnregistered,
    required void Function(PushMessage message, String instance) onMessage,
    required void Function(String instance) onTempUnavailable,
  }) => UnifiedPush.initialize(
    onNewEndpoint: onNewEndpoint,
    onRegistrationFailed: onRegistrationFailed,
    onUnregistered: onUnregistered,
    onMessage: onMessage,
    onTempUnavailable: onTempUnavailable,
  );

  @override
  Future<void> register({required String instance, String? vapid}) =>
      UnifiedPush.register(instance: instance, vapid: vapid);

  @override
  Future<void> unregister(String instance) => UnifiedPush.unregister(instance);

  @override
  Future<List<String>> getDistributors() => UnifiedPush.getDistributors();

  @override
  Future<String?> getDistributor() => UnifiedPush.getDistributor();

  @override
  Future<void> saveDistributor(String distributor) =>
      UnifiedPush.saveDistributor(distributor);

  @override
  Future<bool> tryUseCurrentOrDefaultDistributor() =>
      UnifiedPush.tryUseCurrentOrDefaultDistributor();
}

/// A human word for a distributor's package id — just ntfy, the one Storm
/// tells people to install; anything else (NextPush, a vendor's embedded
/// FCM distributor, …) is shown by its own package id rather than guessed
/// at.
String friendlyDistributorName(String? packageId) => switch (packageId) {
  'io.heckel.ntfy' => 'ntfy',
  null => 'a distributor',
  _ => packageId,
};

/// [FailedReason] as a sentence for the "Delivery" row.
String describeFailedReason(FailedReason reason) => switch (reason) {
  FailedReason.network => 'No network — try again once you\'re online.',
  FailedReason.actionRequired =>
    'The distributor needs your attention (open it to sign in or reconnect), then try again.',
  FailedReason.vapidRequired =>
    'This distributor needs a server push key Storm did not send — please report this.',
  FailedReason.internalError => 'Something went wrong — try again.',
};

const _kEndpointIdKey = 'storm.unifiedpush.endpointId';

Future<String?> _loadEndpointId() async {
  try {
    final prefs = await SharedPreferences.getInstance();
    return prefs.getString(_kEndpointIdKey);
  } catch (_) {
    return null;
  }
}

Future<void> _saveEndpointId(String id) async {
  try {
    final prefs = await SharedPreferences.getInstance();
    await prefs.setString(_kEndpointIdKey, id);
  } catch (_) {}
}

Future<void> _clearEndpointId() async {
  try {
    final prefs = await SharedPreferences.getInstance();
    await prefs.remove(_kEndpointIdKey);
  } catch (_) {}
}

/// Drives the enable/disable state machine and the plugin's callbacks.
/// One instance per signed-in session, alive from app start (watched, not
/// merely read, from `main.dart` — the same reason `desktopNotifierProvider`
/// is — so [UnifiedPush.initialize] runs exactly once a process and a
/// re-registration on an endpoint-change callback is never missed because
/// nothing was listening).
class UnifiedPushController extends ChangeNotifier {
  // `this._backend` (the lint's own suggestion) would force every call
  // site — the real provider below, both test files — to use the
  // private names `_backend:`/`_apiFactory:`/`_deviceState:` as their
  // named arguments; keeping the public parameter names with a private
  // field is the better API here.
  UnifiedPushController({
    required UnifiedPushBackend backend,
    required AgentApi Function() apiFactory,
    required DeviceNotificationPrefsState deviceState,
    OsNotifications? os,
  })
    // ignore: prefer_initializing_formals
    : _backend = backend,
       // ignore: prefer_initializing_formals
       _apiFactory = apiFactory,
       // ignore: prefer_initializing_formals
       _deviceState = deviceState,
       _os = os ?? RealOsNotifications() {
    unawaited(_init());
  }

  final UnifiedPushBackend _backend;
  final AgentApi Function() _apiFactory;
  final DeviceNotificationPrefsState _deviceState;
  final OsNotifications _os;

  UnifiedPushStatus _status = UnifiedPushStatus.off;
  String? _distributorLabel;
  String? _errorMessage;
  Completer<void>? _pendingRegistration;
  bool _disposed = false;

  UnifiedPushStatus get status => _status;
  String? get distributorLabel => _distributorLabel;
  String? get errorMessage => _errorMessage;

  void _set(UnifiedPushStatus status, {String? error, String? distributor}) {
    _status = status;
    _errorMessage = error;
    if (distributor != null) _distributorLabel = distributor;
    if (!_disposed) notifyListeners();
  }

  // Deliberately no `Platform.isAndroid` guard here: that belongs to
  // whoever decides whether to *construct* this controller at all
  // (`unifiedPushControllerProvider`'s own check, below) — not to this
  // method, which a test constructs and drives directly against a fake
  // [UnifiedPushBackend] on a non-Android host. A guard here would never
  // let `_backend.initialize` run under `flutter test`, silently no-op
  // every callback the fake backend tries to invoke.
  Future<void> _init() async {
    try {
      final alreadyRegistered = await _backend.initialize(
        onNewEndpoint: (e, i) => unawaited(_onNewEndpoint(e, i)),
        onRegistrationFailed: (r, i) => _onRegistrationFailed(r, i),
        onUnregistered: (i) => unawaited(_onUnregistered(i)),
        onMessage: (m, i) => unawaited(_onMessage(m, i)),
        onTempUnavailable: (i) {},
      );
      final storedId = await _loadEndpointId();
      if (alreadyRegistered && storedId != null) {
        _set(
          UnifiedPushStatus.on,
          distributor: friendlyDistributorName(await _backend.getDistributor()),
        );
        // The package's own contract: register again at every app
        // startup with the same instance, so a distributor that rotated
        // its token while this device was closed still gets our latest
        // VAPID key.
        final vapid = await _fetchVapid();
        if (vapid != null) {
          await _backend.register(instance: kUnifiedPushInstance, vapid: vapid);
        }
      } else if (storedId != null && !alreadyRegistered) {
        // This device thought it was enabled, but the plugin disagrees —
        // the distributor was uninstalled, or the person revoked it from
        // system settings. Nothing to unregister there; just stop
        // claiming an endpoint that no longer exists.
        await _clearEndpointId();
        _set(UnifiedPushStatus.off);
      }
    } catch (e) {
      _set(UnifiedPushStatus.unavailable, error: '$e');
    }
  }

  Future<String?> _fetchVapid() async {
    final api = _apiFactory();
    try {
      return await api.pushVapidPublicKey();
    } catch (e) {
      _set(UnifiedPushStatus.error, error: '$e');
      return null;
    } finally {
      api.dispose();
    }
  }

  /// "Enable notifications" (decision 161's table): POST_NOTIFICATIONS,
  /// then a distributor, then registration. Each step's own failure sets
  /// its own [UnifiedPushStatus] rather than a generic [UnifiedPushStatus.error].
  Future<void> enable() async {
    _set(UnifiedPushStatus.enabling);
    try {
      final granted = await _requestNotificationPermission();
      if (!granted) {
        _set(UnifiedPushStatus.permissionDenied);
        return;
      }
      final usingSaved = await _backend.tryUseCurrentOrDefaultDistributor();
      if (!usingSaved) {
        final distributors = await _backend.getDistributors();
        if (distributors.isEmpty) {
          _set(UnifiedPushStatus.distributorMissing);
          return;
        }
        // Exactly one real-world choice today (ntfy); a person with more
        // than one distributor installed is a rare, deliberate case this
        // slice does not add a picker for — the first one found is used,
        // same as `tryUseCurrentOrDefaultDistributor`'s own "default"
        // half would have.
        await _backend.saveDistributor(distributors.first);
      }
      final vapid = await _fetchVapid();
      if (vapid == null) return; // _fetchVapid already set the error state.
      // A local reference, not a re-read of `_pendingRegistration` after
      // the `await` below: the distributor's answer can arrive (and
      // `_onNewEndpoint`/`_onRegistrationFailed` null the field back out)
      // before this coroutine resumes, and a `!` on the now-null field
      // would throw and overwrite the real status with "Null check
      // operator used on a null value" — found by a fake backend that
      // answers on the very next microtask, which is faster than any real
      // distributor but not a case the code may assume away.
      final pending = Completer<void>();
      _pendingRegistration = pending;
      await _backend.register(instance: kUnifiedPushInstance, vapid: vapid);
      await pending.future.timeout(
        const Duration(seconds: 25),
        onTimeout: () {
          _set(
            UnifiedPushStatus.error,
            error: 'The distributor did not answer in time — try again.',
          );
        },
      );
    } catch (e) {
      _set(UnifiedPushStatus.error, error: '$e');
    }
  }

  /// "Disable" — best-effort on every step, since the goal is "this
  /// device stops getting push" and a half-failed unregister must not
  /// leave it stuck claiming otherwise.
  Future<void> disable() async {
    final id = await _loadEndpointId();
    try {
      await _backend.unregister(kUnifiedPushInstance);
    } catch (_) {}
    if (id != null) {
      final api = _apiFactory();
      try {
        await api.deletePushEndpoint(id);
      } catch (_) {
      } finally {
        api.dispose();
      }
    }
    await _clearEndpointId();
    try {
      await _deviceState.setEnabled(false);
    } catch (_) {}
    _set(UnifiedPushStatus.off);
  }

  /// For the "Blocked in Android settings" row's "Open system settings".
  Future<bool> openSystemSettings() => _os.openSystemSettings();

  Future<bool> _requestNotificationPermission() async {
    try {
      await _os.init(onTap: (_) {});
      final granted = await _os.checkPermission();
      if (granted ?? false) return true;
      await _os.requestPermission();
      return (await _os.checkPermission()) ?? false;
    } catch (_) {
      return false;
    }
  }

  Future<void> _onNewEndpoint(PushEndpoint endpoint, String instance) async {
    final keys = endpoint.pubKeySet;
    if (keys == null) {
      _fail('This distributor does not support encrypted push.');
      return;
    }
    try {
      final api = _apiFactory();
      PushEndpointRegistration reg;
      try {
        reg = await api.registerPushEndpoint(
          kind: 'unifiedpush',
          endpoint: endpoint.url,
          p256dh: keys.pubKey,
          auth: keys.auth,
        );
      } finally {
        api.dispose();
      }
      await _saveEndpointId(reg.id);
      try {
        await _deviceState.setEnabled(true);
      } catch (_) {
        // The endpoint is registered either way; a prefs write that
        // failed here is retried the next time the page opens.
      }
      _set(
        UnifiedPushStatus.on,
        distributor: friendlyDistributorName(await _backend.getDistributor()),
      );
    } catch (e) {
      _fail('$e');
    } finally {
      _pendingRegistration?.complete();
      _pendingRegistration = null;
    }
  }

  void _onRegistrationFailed(FailedReason reason, String instance) {
    _fail(describeFailedReason(reason));
    _pendingRegistration?.complete();
    _pendingRegistration = null;
  }

  void _fail(String message) => _set(UnifiedPushStatus.error, error: message);

  Future<void> _onUnregistered(String instance) async {
    await _clearEndpointId();
    _set(UnifiedPushStatus.off);
  }

  Future<void> _onMessage(PushMessage message, String instance) =>
      handleIncomingPushMessage(message, instance, os: _os);

  /// "Send a test notification" (161-4's button, reused for Android):
  /// proves the OS-notification rendering path (channel, grouping, tap
  /// routing) directly, the same way `DesktopNotifier.forceShow` does —
  /// **not** a round trip through the distributor, which the server's own
  /// "nobody connected" gate (decision 161's delivery tiers) would refuse
  /// to queue while this page is open and this device is, by definition,
  /// connected. The real round trip is the manual phone check: send a
  /// test with the app swiped away.
  Future<void> forceShow(
    AgentNotification n,
    DeviceNotificationPrefs prefs,
  ) async {
    try {
      await _os.init(
        onTap: (payload) =>
            notificationTapped.value = decodeTapPayload(payload),
      );
      final preview = renderPreview(n, prefs.preview);
      final noteVaultId = n.noteVaultId;
      final noteId = n.noteId;
      final payload = noteVaultId != null && noteId != null
          ? 'note:$noteVaultId:$noteId'
          : (n.sessionId == null ? null : 'session:${n.sessionId}');
      await _os.show(
        id: notificationIntId(n.id),
        title: preview.title,
        body: preview.body,
        payload: payload,
        threadId: n.sessionId,
        category: osCategoryOf(n),
        sound: await _soundOn(),
        androidClass: n.kind == 'test' || n.kind == 'summary'
            ? null
            : deviceNotificationClassOf(n),
      );
    } catch (_) {
      // Best-effort, exactly like `DesktopNotifier.forceShow`: a failure
      // here is this controller's own `unavailable`/`error` state to
      // surface, never an uncaught error on a button press.
    }
  }

  @override
  void dispose() {
    _disposed = true;
    super.dispose();
  }
}

/// Bounded like `agent_notices.dart`'s own toasted sets — kept at the same
/// limit so neither grows the preferences file without bound.
const _kDedupeLimit = 200;

/// Whether [id] was already shown as a toast or an OS/push notification on
/// this device (decision 161-8 point 4: "ignore a push whose notice id was
/// already toasted"). Shares `kServerNoticeToastedKey` with
/// `agent_notices.dart`'s live-stream toasts rather than a push-only key,
/// so the two paths agree about what has already been said — the race
/// this guards is exactly a notice shown live right as the device
/// disconnected, with the push for the same id landing moments later.
Future<bool> wasAlreadyShown(String id) async {
  try {
    final prefs = await SharedPreferences.getInstance();
    final shown = prefs.getStringList(kServerNoticeToastedKey) ?? const [];
    return shown.contains(id);
  } catch (_) {
    return false;
  }
}

Future<void> markShown(String id) async {
  try {
    final prefs = await SharedPreferences.getInstance();
    final shown = <String>[
      ...prefs.getStringList(kServerNoticeToastedKey) ?? const [],
    ];
    if (shown.contains(id)) return;
    shown.add(id);
    await prefs.setStringList(
      kServerNoticeToastedKey,
      shown.sublist(math.max(0, shown.length - _kDedupeLimit)),
    );
  } catch (_) {}
}

Future<bool> _soundOn() async {
  try {
    final prefs = await SharedPreferences.getInstance();
    return prefs.getBool(ThisDeviceLocalPrefs.kSoundKey) ?? true;
  } catch (_) {
    return true;
  }
}

/// Shows (or drops) one decrypted push message — the shared path for both
/// the live app's [UnifiedPushController._onMessage] and the headless
/// background entrypoint ([runUnifiedPushBackgroundEntrypoint]), which has
/// no controller, no `AgentApi` and no Riverpod providers at all. Never
/// throws: a malformed or undecryptable payload is dropped, not crashed
/// on, since this may run on a service Android is not patient with.
///
/// Deliberately does **not** run the title/body back through
/// `renderPreview` — the server already rendered them at this device's
/// own preview level before encrypting (`push::preview_payload`); doing it
/// again here would either repeat that work harmlessly or, for a future
/// preview level this client doesn't know about, show the wrong thing.
/// Likewise there is no cross-message fold here: each push is its own
/// process in the worst (fully killed) case, so there is nothing to fold
/// across — Android's own `groupKey` stacking (`androidClass` below,
/// `threadId: n.sessionId`) is what groups several per session instead.
Future<void> handleIncomingPushMessage(
  PushMessage message,
  String instance, {
  OsNotifications? os,
}) async {
  if (!message.decrypted) {
    debugPrint('unifiedpush: message was not decrypted, dropping it');
    return;
  }
  Map<String, dynamic> json;
  try {
    final decoded = jsonDecode(utf8.decode(message.content));
    if (decoded is! Map<String, dynamic>) return;
    json = decoded;
  } catch (e) {
    debugPrint('unifiedpush: payload was not valid JSON: $e');
    return;
  }
  AgentNotification n;
  try {
    n = AgentNotification.fromPushPayload(json);
  } catch (e) {
    debugPrint('unifiedpush: payload missing required fields: $e');
    return;
  }
  if (await wasAlreadyShown(n.id)) return;
  await markShown(n.id);

  final notifier = os ?? RealOsNotifications();
  try {
    await notifier.init(
      onTap: (payload) {
        notificationTapped.value = decodeTapPayload(payload);
      },
    );
    final noteVaultId = n.noteVaultId;
    final noteId = n.noteId;
    final payload = noteVaultId != null && noteId != null
        ? 'note:$noteVaultId:$noteId'
        : (n.sessionId == null ? null : 'session:${n.sessionId}');
    await notifier.show(
      id: notificationIntId(n.id),
      title: n.title,
      body: n.body,
      payload: payload,
      threadId: n.sessionId,
      category: osCategoryOf(n),
      sound: await _soundOn(),
      androidClass: n.kind == 'test' || n.kind == 'summary'
          ? null
          : deviceNotificationClassOf(n),
    );
  } catch (e) {
    debugPrint('unifiedpush: could not show the notification: $e');
  }
}

/// `main.dart`'s entrypoint for the `--unifiedpush-bg` headless engine
/// (see this file's own doc comment for why that argument exists and
/// where it comes from). No widget tree, no `ProviderScope`, no network
/// client beyond what [handleIncomingPushMessage] itself opens — this
/// engine exists for one message and is torn down after.
Future<void> runUnifiedPushBackgroundEntrypoint() async {
  await UnifiedPush.initialize(
    onMessage: (message, instance) =>
        unawaited(handleIncomingPushMessage(message, instance)),
    // Every other callback is a no-op here on purpose: a background
    // delivery is never the moment to re-register state, dedupe (`dedupe`
    // is `handleIncomingPushMessage`'s own, not a callback) or update
    // "This device" — the foreground controller does that the next time
    // the app itself is opened and `initialize` runs again, there with
    // the current `storedId`/VAPID to compare against.
    onNewEndpoint: (endpoint, instance) {},
    onRegistrationFailed: (reason, instance) {},
    onUnregistered: (instance) {},
    onTempUnavailable: (instance) {},
  );
}

/// Alive while a signed-in Android session exists (watched, not merely
/// read, from `main.dart` — see the class doc comment on why).
final unifiedPushControllerProvider =
    Provider.autoDispose<UnifiedPushController?>((ref) {
      if (!Platform.isAndroid) return null;
      final factory = ref.watch(agentApiFactoryProvider);
      final deviceState = ref.watch(deviceNotificationPrefsProvider);
      // settingsProvider: just a liveness gate, the same as
      // `desktopNotifierProvider`'s `deviceId` read — no signed-in
      // session, no controller, so a sign-out tears this down with
      // everything else rather than leaving a stale registration flow
      // armed.
      final signedIn =
          (ref.watch(settingsProvider.select((s) => s.value?.deviceId)) ?? '')
              .isNotEmpty;
      if (factory == null || deviceState == null || !signedIn) return null;
      final controller = UnifiedPushController(
        backend: RealUnifiedPushBackend(),
        apiFactory: factory,
        deviceState: deviceState,
      );
      ref.onDispose(controller.dispose);
      return controller;
    });
