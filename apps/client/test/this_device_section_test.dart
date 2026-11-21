import 'dart:async';
import 'dart:convert';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:shared_preferences/shared_preferences.dart';
import 'package:unifiedpush/unifiedpush.dart';
import 'package:unifiedpush_platform_interface/data/public_key_set.dart';

import 'package:storm/agent/agent_api.dart';
import 'package:storm/agent/desktop_notifications.dart' show OsNotifications;
import 'package:storm/agent/device_notification_prefs_state.dart';
import 'package:storm/agent/unified_push.dart';
import 'package:storm/router.dart';

import 'shell_harness.dart';

/// Decision 161, slice 161-4: Settings › Notifications' "This device"
/// section, on its real `/v1/notifications/device` endpoint. Everything
/// else the page reads (`prefs`, `mutes`, the inbox) is stubbed to its
/// quietest possible shape — these tests are about the device section
/// alone.
void main() {
  setUp(() {
    SharedPreferences.setMockInitialValues({});
  });

  const desk = Size(1280, 800);

  Map<String, dynamic> device({
    bool enabled = true,
    List<String> classes = const [],
    String preview = 'full',
  }) => {
    'device_id': 'dev_a',
    'enabled': enabled,
    'classes': classes,
    'preview': preview,
    'updated_at': '2026-11-18T00:00:00Z',
  };

  Map<String, dynamic> accountPrefs() => {
    'version': 1,
    'events': <String, dynamic>{},
    'quiet_on': false,
    'quiet_start': '22:00',
    'quiet_end': '08:00',
    'quiet_tz': 'UTC',
    'quiet_allow_delete_requests': false,
    'summary_on': true,
    'paused_until': null,
    'agent_default_on': true,
    'children_default_on': false,
    'per_session_limit': 20,
    'retention_days': 30,
    'retention_rows': 1000,
  };

  /// [devicePrefs] is read fresh on every GET, so a test can mutate it
  /// between a toggle and the next pump if it wants to simulate the
  /// server's own echo; [onPut] sees every PUT body decoded.
  http.Client client(
    Map<String, dynamic> Function() devicePrefs,
    void Function(Map<String, dynamic> body) onPut,
  ) => MockClient((req) async {
    if (req.url.path == '/v1/notifications/device') {
      if (req.method == 'PUT') {
        final body = jsonDecode(req.body) as Map<String, dynamic>;
        onPut(body);
        return http.Response(jsonEncode({...devicePrefs(), ...body}), 200);
      }
      return http.Response(jsonEncode(devicePrefs()), 200);
    }
    if (req.url.path == '/v1/notifications') {
      return http.Response(jsonEncode({'items': [], 'cursor': 0}), 200);
    }
    if (req.url.path == '/v1/notifications/prefs') {
      return http.Response(jsonEncode(accountPrefs()), 200);
    }
    if (req.url.path == '/v1/notifications/mutes') {
      return http.Response(jsonEncode([]), 200);
    }
    // 161-8: the android UnifiedPush group below reuses this same client
    // for its own `AgentApi`, so it needs these two answered too.
    if (req.url.path == '/v1/push/vapid') {
      return http.Response(jsonEncode({'public_key': 'test-vapid-key'}), 200);
    }
    if (req.url.path == '/v1/push/endpoints' && req.method == 'POST') {
      final body = jsonDecode(req.body) as Map<String, dynamic>;
      return http.Response(
        jsonEncode({'id': 'ep_test', 'kind': body['kind'], 'created': 'now'}),
        200,
      );
    }
    if (req.url.path.startsWith('/v1/push/endpoints/') &&
        req.method == 'DELETE') {
      return http.Response('', 200);
    }
    return http.Response('', 404);
  });

  Future<void> open(WidgetTester tester, c) async {
    c.read(routerProvider).go(Routes.settingsPage('notifications'));
    await tester.pumpAndSettle();
  }

  testWidgets(
    'android with no push controller (the real one never resolves off a '
    'real Android OS, which this harness is not) still shows the section, '
    'offering Enable rather than hiding entirely (decision 161, 161-8)',
    (tester) async {
      debugDefaultTargetPlatformOverride = TargetPlatform.android;
      try {
        final c = shellContainer(agentClient: client(device, (_) {}));
        await pumpShell(tester, c, size: desk);
        await open(tester, c);
        expect(find.byKey(const Key('device-notify-enabled')), findsOneWidget);
        expect(
          find.byKey(const Key('android-enable-notifications')),
          findsOneWidget,
        );
        await disposeShell(tester, c);
      } finally {
        debugDefaultTargetPlatformOverride = null;
      }
    },
  );

  testWidgets('shown on a desktop platform (linux), with the platform '
      "default's enabled state", (tester) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.linux;
    try {
      final c = shellContainer(agentClient: client(device, (_) {}));
      await pumpShell(tester, c, size: desk);
      await open(tester, c);
      expect(find.byKey(const Key('device-notify-enabled')), findsOneWidget);
      expect(find.text('This device'), findsOneWidget);
      await disposeShell(tester, c);
    } finally {
      debugDefaultTargetPlatformOverride = null;
    }
  });

  testWidgets(
    'review item G: with no real plugin channel (this harness), both the '
    'OS permission row and the Delivery row say "No notification service"',
    (tester) async {
      debugDefaultTargetPlatformOverride = TargetPlatform.linux;
      try {
        final c = shellContainer(
          agentClient: client(device, (_) {}),
          // desktopNotifierProvider needs both a paired device id and a
          // live events stream to resolve at all — see fakeDeviceId's doc.
          deviceId: fakeDeviceId,
          deviceSecret: fakeDeviceSecret,
          agentEvents: (_) => const Stream.empty(),
        );
        await pumpShell(tester, c, size: desk);
        await open(tester, c);
        expect(find.text('No notification service'), findsNWidgets(2));
        await disposeShell(tester, c);
      } finally {
        debugDefaultTargetPlatformOverride = null;
      }
    },
  );

  testWidgets('toggling "Notifications on this device" PUTs enabled:false', (
    tester,
  ) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.linux;
    Map<String, dynamic>? lastPut;
    try {
      final c = shellContainer(agentClient: client(device, (b) => lastPut = b));
      await pumpShell(tester, c, size: desk);
      await open(tester, c);

      await tester.tap(find.byKey(const Key('device-notify-enabled')));
      await tester.pumpAndSettle();

      expect(lastPut, {'enabled': false});
      await disposeShell(tester, c);
    } finally {
      debugDefaultTargetPlatformOverride = null;
    }
  });

  testWidgets('picking "Session name only" PUTs preview:session', (
    tester,
  ) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.linux;
    Map<String, dynamic>? lastPut;
    try {
      final c = shellContainer(agentClient: client(device, (b) => lastPut = b));
      await pumpShell(tester, c, size: desk);
      await open(tester, c);

      await tester.tap(find.text('Session name only'));
      await tester.pumpAndSettle();

      expect(lastPut, {'preview': 'session'});
      await disposeShell(tester, c);
    } finally {
      debugDefaultTargetPlatformOverride = null;
    }
  });

  testWidgets('turning one class off from "every class" ([]) sends the rest '
      'explicitly', (tester) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.linux;
    Map<String, dynamic>? lastPut;
    try {
      final c = shellContainer(agentClient: client(device, (b) => lastPut = b));
      await pumpShell(tester, c, size: desk);
      await open(tester, c);

      await tester.tap(find.byKey(const Key('device-class-needs_you')));
      await tester.pumpAndSettle();

      expect(lastPut, isNotNull);
      final classes = (lastPut!['classes'] as List).cast<String>();
      expect(classes, isNot(contains('needs_you')));
      expect(
        classes,
        containsAll(['delete_requests', 'finished', 'errors', 'hosts']),
      );
      await disposeShell(tester, c);
    } finally {
      debugDefaultTargetPlatformOverride = null;
    }
  });

  testWidgets(
    'the macOS "keep running" toggle only shows on macOS, never linux',
    (tester) async {
      debugDefaultTargetPlatformOverride = TargetPlatform.linux;
      try {
        final c = shellContainer(agentClient: client(device, (_) {}));
        await pumpShell(tester, c, size: desk);
        await open(tester, c);
        expect(find.byKey(const Key('device-keep-running')), findsNothing);
        await disposeShell(tester, c);
      } finally {
        debugDefaultTargetPlatformOverride = null;
      }
    },
  );

  testWidgets('the macOS "keep running" toggle shows on macOS', (tester) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.macOS;
    try {
      final c = shellContainer(agentClient: client(device, (_) {}));
      await pumpShell(tester, c, size: desk);
      await open(tester, c);
      expect(find.byKey(const Key('device-keep-running')), findsOneWidget);
      await disposeShell(tester, c);
    } finally {
      debugDefaultTargetPlatformOverride = null;
    }
  });

  group('android: UnifiedPush states (decision 161, slice 161-8)', () {
    UnifiedPushController controller(
      _FakeUpBackend backend, {
      OsNotifications? os,
    }) {
      AgentApi factory() => AgentApi(
        baseUrl: 'http://test',
        token: 't',
        client: client(device, (_) {}),
      );
      return UnifiedPushController(
        backend: backend,
        apiFactory: factory,
        deviceState: DeviceNotificationPrefsState(factory),
        os: os,
      );
    }

    Future<void> pumpWith(
      WidgetTester tester,
      UnifiedPushController push,
    ) async {
      debugDefaultTargetPlatformOverride = TargetPlatform.android;
      final c = shellContainer(
        agentClient: client(device, (_) {}),
        unifiedPush: push,
      );
      await pumpShell(tester, c, size: desk);
      await open(tester, c);
      addTearDown(() async {
        await disposeShell(tester, c);
        debugDefaultTargetPlatformOverride = null;
      });
    }

    testWidgets('distributorMissing: offers Play Store and F-Droid', (
      tester,
    ) async {
      final backend = _FakeUpBackend()..distributors = const [];
      final push = controller(backend);
      await push.enable();
      await pumpWith(tester, push);

      expect(find.text('No push app installed'), findsOneWidget);
      expect(
        find.byKey(const Key('android-install-ntfy-play')),
        findsOneWidget,
      );
      expect(
        find.byKey(const Key('android-install-ntfy-fdroid')),
        findsOneWidget,
      );
    });

    testWidgets('permissionDenied: offers Open system settings', (
      tester,
    ) async {
      final backend = _FakeUpBackend();
      final push = controller(backend, os: _FakeUpOs()..osPermission = false);
      await push.enable();
      await pumpWith(tester, push);

      expect(find.text('Notifications are blocked'), findsOneWidget);
      expect(
        find.byKey(const Key('android-open-system-settings')),
        findsOneWidget,
      );
    });

    testWidgets('error: shows the failure reason', (tester) async {
      final backend = _FakeUpBackend()
        ..respondWithFailure = FailedReason.network;
      final push = controller(backend);
      await push.enable();
      await Future<void>.delayed(Duration.zero);
      await pumpWith(tester, push);

      expect(find.text('Could not enable notifications'), findsOneWidget);
      expect(
        find.text(describeFailedReason(FailedReason.network)),
        findsOneWidget,
      );
    });

    testWidgets('on: shows the delivery row, classes and a disable toggle', (
      tester,
    ) async {
      final backend = _FakeUpBackend()
        ..respondWithEndpoint = PushEndpoint(
          'https://push.example/x',
          PublicKeySet('pub', 'auth'),
        );
      final push = controller(backend);
      await push.enable();
      await Future<void>.delayed(Duration.zero);
      await pumpWith(tester, push);

      expect(push.status, UnifiedPushStatus.on);
      expect(find.byKey(const Key('device-notify-enabled')), findsOneWidget);
      expect(find.textContaining('Push via ntfy'), findsOneWidget);
      expect(find.byKey(const Key('device-class-needs_you')), findsOneWidget);
      expect(find.byKey(const Key('device-send-test')), findsOneWidget);

      await tester.tap(find.byKey(const Key('device-notify-enabled')));
      await tester.pumpAndSettle();
      expect(backend.unregisterCalls, [kUnifiedPushInstance]);
    });
  });
}

/// A fake [UnifiedPushBackend] for the widget-level "This device" states —
/// the unit-level state machine itself is `unified_push_test.dart`'s.
class _FakeUpBackend implements UnifiedPushBackend {
  void Function(PushEndpoint, String)? onNewEndpoint;
  void Function(FailedReason, String)? onRegistrationFailed;
  void Function(String)? onUnregistered;

  List<String> distributors = const ['io.heckel.ntfy'];
  String? currentDistributor = 'io.heckel.ntfy';
  PushEndpoint? respondWithEndpoint;
  FailedReason? respondWithFailure;
  final unregisterCalls = <String>[];

  @override
  Future<bool> initialize({
    required void Function(PushEndpoint endpoint, String instance)
    onNewEndpoint,
    required void Function(FailedReason reason, String instance)
    onRegistrationFailed,
    required void Function(String instance) onUnregistered,
    required void Function(PushMessage message, String instance) onMessage,
    required void Function(String instance) onTempUnavailable,
  }) async {
    this.onNewEndpoint = onNewEndpoint;
    this.onRegistrationFailed = onRegistrationFailed;
    this.onUnregistered = onUnregistered;
    return false;
  }

  @override
  Future<void> register({required String instance, String? vapid}) async {
    if (respondWithEndpoint != null) {
      scheduleMicrotask(
        () => onNewEndpoint?.call(respondWithEndpoint!, instance),
      );
    } else if (respondWithFailure != null) {
      scheduleMicrotask(
        () => onRegistrationFailed?.call(respondWithFailure!, instance),
      );
    }
  }

  @override
  Future<void> unregister(String instance) async {
    unregisterCalls.add(instance);
  }

  @override
  Future<List<String>> getDistributors() async => distributors;

  @override
  Future<String?> getDistributor() async => currentDistributor;

  @override
  Future<void> saveDistributor(String distributor) async {
    currentDistributor = distributor;
  }

  @override
  Future<bool> tryUseCurrentOrDefaultDistributor() async => false;
}

class _FakeUpOs implements OsNotifications {
  bool? osPermission = true;

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
    String? androidClass,
  }) async {}

  @override
  Future<void> cancel(int id) async {}

  @override
  Future<void> cancelAll() async {}

  @override
  Future<bool?> checkPermission() async => osPermission;

  @override
  Future<void> requestPermission() async {}

  @override
  Future<bool> openSystemSettings() async => true;
}
