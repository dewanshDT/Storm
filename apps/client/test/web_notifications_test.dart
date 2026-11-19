import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:storm/agent/agent_events.dart';
import 'package:storm/agent/desktop_notifications.dart';
import 'package:storm/agent/notifications_models.dart';
import 'package:storm/agent/web_notifications.dart';

AgentNotification _notif({
  String id = 'ntf_1',
  String source = 'storm',
  String kind = 'needs_you',
  String? sessionId = 'ags_1',
  String? sessionTitle = 'tidier',
  String? provider = 'claude-code',
  bool silent = false,
  String? deliver,
}) => AgentNotification(
  id: id,
  seq: 1,
  created: '2026-11-19T00:00:00Z',
  source: source,
  kind: kind,
  sessionId: sessionId,
  sessionTitle: sessionTitle,
  provider: provider,
  title: 'Needs you',
  urgency: 'normal',
  silent: silent,
  hasDeliverHint: true,
  deliver: deliver,
);

DeviceNotificationPrefs _prefs({bool enabled = true}) =>
    DeviceNotificationPrefs(
      deviceId: 'dev_1',
      enabled: enabled,
      classes: const [],
      preview: 'full',
      updatedAt: '2026-11-19T00:00:00Z',
    );

/// A minimal stand-in for [WebOsNotifications] — same contract
/// ([OsNotifications]), so it proves [DesktopNotifier]'s deliver → raise/
/// close logic still works exactly as the web notifications round wires
/// it, without reaching the real, unreachable-on-the-VM `Notification`
/// DOM API that the real implementation would call.
class _FakeOs implements OsNotifications {
  final shown = <int, String>{};
  final payloads = <int, String?>{};
  var cancelledAll = false;

  @override
  Future<void> init({required void Function(String? payload) onTap}) async {}

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
    shown[id] = title;
    payloads[id] = payload;
  }

  @override
  Future<void> cancel(int id) async => shown.remove(id);

  @override
  Future<void> cancelAll() async {
    shown.clear();
    cancelledAll = true;
  }

  @override
  Future<bool?> checkPermission() async => true;

  @override
  Future<void> requestPermission() async {}

  @override
  Future<bool> openSystemSettings() async => false;
}

void main() {
  group('mapWebNotificationPermission (the secure/insecure state mapping)', () {
    test('an insecure origin is "insecureOrigin" whatever the browser last '
        'answered', () {
      for (final raw in ['granted', 'denied', 'default', 'unsupported']) {
        expect(
          mapWebNotificationPermission(
            isSecureContext: false,
            rawPermission: raw,
          ),
          WebNotificationPermission.insecureOrigin,
        );
      }
    });

    test('a secure origin maps the browser\'s three real answers', () {
      expect(
        mapWebNotificationPermission(
          isSecureContext: true,
          rawPermission: 'granted',
        ),
        WebNotificationPermission.allowed,
      );
      expect(
        mapWebNotificationPermission(
          isSecureContext: true,
          rawPermission: 'denied',
        ),
        WebNotificationPermission.denied,
      );
      expect(
        mapWebNotificationPermission(
          isSecureContext: true,
          rawPermission: 'default',
        ),
        WebNotificationPermission.notAsked,
      );
    });

    test('an unrecognised answer on a secure origin is "not asked" rather '
        'than denied — never a false "blocked"', () {
      expect(
        mapWebNotificationPermission(
          isSecureContext: true,
          rawPermission: 'unsupported',
        ),
        WebNotificationPermission.notAsked,
      );
    });
  });

  group('WebOsNotifications', () {
    test('requestPermission never calls the browser (needs a user gesture '
        'only the Settings button has)', () async {
      // Nothing to assert beyond "completes without touching the real
      // DOM" — the whole point is that this is a no-op; a VM test can't
      // observe the real API either way, but it does prove the method
      // doesn't throw or try to synchronously do something it shouldn't.
      await WebOsNotifications().requestPermission();
    });

    test('openSystemSettings is false: no web equivalent', () async {
      expect(await WebOsNotifications().openSystemSettings(), isFalse);
    });
  });

  group('deliver → raise/close logic through the web notifications round\'s '
      'own DesktopNotifier wiring', () {
    late StreamController<AgentEvent> controller;
    late AgentEvents events;
    late _FakeOs os;
    late DesktopNotifier notifier;

    // `background()` only keeps the connection alive and flips `isActive`
    // (what this group is testing against) rather than dropping it
    // outright when `AgentEvents._keepsStreamWhileHidden` is true. A
    // desktop platform override is the simplest way to get that for a
    // plain `AgentEvents` in a test — the mechanism is identical to the
    // web notifications round's own `setWebNotificationsEnabled`, which
    // has its own dedicated tests in `agent_events_test.dart`; this group
    // is only about what `DesktopNotifier` + an [OsNotifications] like
    // [WebOsNotifications] does once a device is inactive, not about why.
    setUp(() {
      debugDefaultTargetPlatformOverride = TargetPlatform.linux;
      controller = StreamController<AgentEvent>();
      events = AgentEvents((_) => controller.stream)
        ..start(followLifecycle: false);
      os = _FakeOs();
      notifier = DesktopNotifier(
        deviceId: 'dev_1',
        events: events,
        prefs: () => _prefs(),
        os: os,
      );
    });

    tearDown(() {
      notifier.dispose();
      events.dispose();
      unawaited(controller.close());
      debugDefaultTargetPlatformOverride = null;
    });

    Future<void> settle() => Future<void>.delayed(Duration.zero);

    test('a backgrounded device raises "os"-hinted notices', () async {
      controller.add(AgentSnapshot('run_1', const []));
      await settle();
      events.background();
      await settle();

      controller.add(AgentNotificationPushed(_notif(deliver: 'os')));
      await settle();
      expect(os.shown, isNotEmpty);
      expect(os.payloads.values.single, 'session:ags_1');
    });

    test('an active device never raises one, even "os"-hinted', () async {
      controller.add(AgentSnapshot('run_1', const []));
      await settle();

      controller.add(AgentNotificationPushed(_notif(deliver: 'os')));
      await settle();
      expect(os.shown, isEmpty);
    });

    test('a read closes the one it was shown under', () async {
      controller.add(AgentSnapshot('run_1', const []));
      await settle();
      events.background();
      await settle();

      controller.add(
        AgentNotificationPushed(_notif(id: 'ntf_a', deliver: 'os')),
      );
      await settle();
      expect(os.shown, isNotEmpty);

      controller.add(AgentNotificationsRead(['ntf_a']));
      await settle();
      expect(os.shown, isEmpty);
    });

    test('clearing the inbox closes everything', () async {
      controller.add(AgentSnapshot('run_1', const []));
      await settle();
      events.background();
      await settle();

      controller.add(
        AgentNotificationPushed(_notif(id: 'ntf_a', deliver: 'os')),
      );
      await settle();
      expect(os.shown, isNotEmpty);

      controller.add(AgentNotificationsCleared());
      await settle();
      expect(os.shown, isEmpty);
      expect(os.cancelledAll, isTrue);
    });
  });
}
