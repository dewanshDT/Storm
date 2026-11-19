import 'dart:convert';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';

import 'package:storm/router.dart';

import 'shell_harness.dart';

/// Decision 161, slice 161-4: Settings › Notifications' "This device"
/// section, on its real `/v1/notifications/device` endpoint. Everything
/// else the page reads (`prefs`, `mutes`, the inbox) is stubbed to its
/// quietest possible shape — these tests are about the device section
/// alone.
void main() {
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
    return http.Response('', 404);
  });

  Future<void> open(WidgetTester tester, c) async {
    c.read(routerProvider).go(Routes.settingsPage('notifications'));
    await tester.pumpAndSettle();
  }

  testWidgets('hidden entirely on a phone platform (android)', (tester) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.android;
    try {
      final c = shellContainer(agentClient: client(device, (_) {}));
      await pumpShell(tester, c, size: desk);
      await open(tester, c);
      expect(find.byKey(const Key('device-notify-enabled')), findsNothing);
      await disposeShell(tester, c);
    } finally {
      debugDefaultTargetPlatformOverride = null;
    }
  });

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
}
