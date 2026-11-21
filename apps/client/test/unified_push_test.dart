import 'dart:async';
import 'dart:convert';
import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:shared_preferences/shared_preferences.dart';
import 'package:unifiedpush/unifiedpush.dart';
// `PublicKeySet` is the one type `package:unifiedpush` does not re-export
// (verified against its own `lib/unifiedpush.dart`, which exports
// `FailedReason`/`PushEndpoint`/`PushMessage` only) — a direct import of
// its platform-interface home is the only way to construct one in a test.
import 'package:unifiedpush_platform_interface/data/public_key_set.dart';

import 'package:storm/agent/agent_api.dart';
import 'package:storm/agent/desktop_notifications.dart' show OsNotifications;
import 'package:storm/agent/device_notification_prefs_state.dart';
import 'package:storm/agent/unified_push.dart';

/// A fake [UnifiedPushBackend]: no real platform channel exists under
/// `flutter test`, and the real one would need an actual Android OS and
/// distributor installed anyway.
class _FakeBackend implements UnifiedPushBackend {
  void Function(PushEndpoint, String)? onNewEndpoint;
  void Function(FailedReason, String)? onRegistrationFailed;
  void Function(String)? onUnregistered;
  void Function(PushMessage, String)? onMessage;

  bool alreadyRegistered = false;
  List<String> distributors = const ['io.heckel.ntfy'];
  bool tryUseCurrentSucceeds = false;
  String? currentDistributor = 'io.heckel.ntfy';
  String? savedDistributor;
  final registerCalls = <String>[];
  final unregisterCalls = <String>[];

  /// What `register()` simulates happening, asynchronously — a real
  /// distributor answers out of band too.
  PushEndpoint? respondWithEndpoint;
  FailedReason? respondWithFailure;
  bool hangForever = false;

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
    this.onMessage = onMessage;
    return alreadyRegistered;
  }

  @override
  Future<void> register({required String instance, String? vapid}) async {
    registerCalls.add(instance);
    if (hangForever) return;
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
    savedDistributor = distributor;
    currentDistributor = distributor;
  }

  @override
  Future<bool> tryUseCurrentOrDefaultDistributor() async =>
      tryUseCurrentSucceeds;
}

class _FakeOs implements OsNotifications {
  bool? permission = true;
  var requested = 0;
  final shown = <int, ({String title, String? body, String? androidClass})>{};

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
  }) async {
    shown[id] = (title: title, body: body, androidClass: androidClass);
  }

  @override
  Future<void> cancel(int id) async {}

  @override
  Future<void> cancelAll() async {}

  @override
  Future<bool?> checkPermission() async => permission;

  @override
  Future<void> requestPermission() async {
    requested++;
  }

  @override
  Future<bool> openSystemSettings() async => true;
}

/// An `AgentApi` over a scripted `MockClient`, so [UnifiedPushController]'s
/// real HTTP calls (`GET /v1/push/vapid`, `POST`/`DELETE
/// /v1/push/endpoints*`, `PUT /v1/notifications/device`) go somewhere real
/// rather than being faked at the method level — the same approach
/// `this_device_section_test.dart` takes for `DeviceNotificationPrefsState`.
AgentApi Function() apiFactory({
  String vapid = 'server-vapid-key',
  void Function(Map<String, dynamic> body)? onRegister,
  void Function(String id)? onDelete,
  void Function(bool enabled)? onSetEnabled,
  int vapidStatus = 200,
}) {
  final client = MockClient((req) async {
    if (req.url.path == '/v1/push/vapid') {
      if (vapidStatus != 200) return http.Response('', vapidStatus);
      return http.Response(jsonEncode({'public_key': vapid}), 200);
    }
    if (req.url.path == '/v1/push/endpoints' && req.method == 'POST') {
      final body = jsonDecode(req.body) as Map<String, dynamic>;
      onRegister?.call(body);
      return http.Response(
        jsonEncode({'id': 'ep_1', 'kind': body['kind'], 'created': 'now'}),
        200,
      );
    }
    if (req.url.path.startsWith('/v1/push/endpoints/') &&
        req.method == 'DELETE') {
      onDelete?.call(req.url.path.split('/').last);
      return http.Response('', 200);
    }
    if (req.url.path == '/v1/notifications/device') {
      if (req.method == 'PUT') {
        final body = jsonDecode(req.body) as Map<String, dynamic>;
        if (body.containsKey('enabled')) {
          onSetEnabled?.call(body['enabled'] as bool);
        }
        return http.Response(
          jsonEncode({
            'device_id': 'dev_a',
            'enabled': body['enabled'] ?? true,
            'classes': <String>[],
            'preview': 'full',
            'updated_at': 'now',
          }),
          200,
        );
      }
      return http.Response(
        jsonEncode({
          'device_id': 'dev_a',
          'enabled': false,
          'classes': <String>[],
          'preview': 'full',
          'updated_at': 'now',
        }),
        200,
      );
    }
    return http.Response('', 404);
  });
  return () => AgentApi(baseUrl: 'http://test', token: 't', client: client);
}

DeviceNotificationPrefsState deviceState(AgentApi Function() factory) =>
    DeviceNotificationPrefsState(factory);

PushEndpoint endpoint({String url = 'https://push.example/ep/1'}) =>
    PushEndpoint(url, PublicKeySet('pub-key-b64u', 'auth-secret-b64u'));

Future<void> settle() async {
  // Lets the fake backend's `scheduleMicrotask` callback and the
  // controller's own awaited chain both run before an assertion.
  for (var i = 0; i < 5; i++) {
    await Future<void>.delayed(Duration.zero);
  }
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  setUp(() {
    SharedPreferences.setMockInitialValues({});
  });

  group('friendlyDistributorName', () {
    test('ntfy by its real package id', () {
      expect(friendlyDistributorName('io.heckel.ntfy'), 'ntfy');
    });
    test('an unknown distributor by its raw package id', () {
      expect(
        friendlyDistributorName('org.unifiedpush.distributor.nextpush'),
        'org.unifiedpush.distributor.nextpush',
      );
    });
    test('none at all', () {
      expect(friendlyDistributorName(null), 'a distributor');
    });
  });

  group('describeFailedReason', () {
    test('every reason has a non-empty sentence', () {
      for (final r in FailedReason.values) {
        expect(describeFailedReason(r), isNotEmpty);
      }
    });
  });

  group('UnifiedPushController.enable', () {
    test(
      'happy path: permission, distributor, vapid, register, endpoint',
      () async {
        final backend = _FakeBackend()..respondWithEndpoint = endpoint();
        Map<String, dynamic>? registered;
        bool? enabledSet;
        final api = apiFactory(
          onRegister: (b) => registered = b,
          onSetEnabled: (v) => enabledSet = v,
        );
        final os = _FakeOs();
        final c = UnifiedPushController(
          backend: backend,
          apiFactory: api,
          deviceState: deviceState(api),
          os: os,
        );
        await settle();

        await c.enable();
        await settle();

        expect(c.status, UnifiedPushStatus.on);
        expect(c.distributorLabel, 'ntfy');
        expect(backend.registerCalls, [kUnifiedPushInstance]);
        expect(registered, isNotNull);
        expect(registered!['kind'], 'unifiedpush');
        expect(registered!['endpoint'], 'https://push.example/ep/1');
        expect(registered!['keys'], {
          'p256dh': 'pub-key-b64u',
          'auth': 'auth-secret-b64u',
        });
        expect(enabledSet, true);
      },
    );

    test('no distributor installed', () async {
      final backend = _FakeBackend()..distributors = const [];
      final api = apiFactory();
      final c = UnifiedPushController(
        backend: backend,
        apiFactory: api,
        deviceState: deviceState(api),
        os: _FakeOs(),
      );
      await settle();
      await c.enable();
      expect(c.status, UnifiedPushStatus.distributorMissing);
      // Never reached registration without a distributor to register with.
      expect(backend.registerCalls, isEmpty);
    });

    test('POST_NOTIFICATIONS refused', () async {
      final backend = _FakeBackend();
      final api = apiFactory();
      final os = _FakeOs()..permission = false;
      final c = UnifiedPushController(
        backend: backend,
        apiFactory: api,
        deviceState: deviceState(api),
        os: os,
      );
      await settle();
      await c.enable();
      expect(c.status, UnifiedPushStatus.permissionDenied);
      expect(os.requested, 1);
      expect(backend.registerCalls, isEmpty);
    });

    test('registration failed (network)', () async {
      final backend = _FakeBackend()..respondWithFailure = FailedReason.network;
      final api = apiFactory();
      final c = UnifiedPushController(
        backend: backend,
        apiFactory: api,
        deviceState: deviceState(api),
        os: _FakeOs(),
      );
      await settle();
      await c.enable();
      await settle();
      expect(c.status, UnifiedPushStatus.error);
      expect(c.errorMessage, describeFailedReason(FailedReason.network));
    });

    test('the server has no VAPID key to give', () async {
      final backend = _FakeBackend();
      final api = apiFactory(vapidStatus: 503);
      final c = UnifiedPushController(
        backend: backend,
        apiFactory: api,
        deviceState: deviceState(api),
        os: _FakeOs(),
      );
      await settle();
      await c.enable();
      expect(c.status, UnifiedPushStatus.error);
      expect(backend.registerCalls, isEmpty);
    });

    test(
      'an endpoint change while already on re-registers, not just once',
      () async {
        final backend = _FakeBackend()..respondWithEndpoint = endpoint();
        var registrations = 0;
        final api = apiFactory(onRegister: (_) => registrations++);
        final c = UnifiedPushController(
          backend: backend,
          apiFactory: api,
          deviceState: deviceState(api),
          os: _FakeOs(),
        );
        await settle();
        await c.enable();
        await settle();
        expect(c.status, UnifiedPushStatus.on);
        expect(registrations, 1);

        // The distributor rotates the endpoint on its own, outside any
        // `enable()` call this controller made.
        backend.onNewEndpoint?.call(
          endpoint(url: 'https://push.example/ep/2'),
          kUnifiedPushInstance,
        );
        await settle();
        expect(registrations, 2);
        expect(c.status, UnifiedPushStatus.on);
      },
    );
  });

  group('UnifiedPushController.disable', () {
    test(
      'unregisters, deletes the endpoint, and turns the device off',
      () async {
        final backend = _FakeBackend()..respondWithEndpoint = endpoint();
        String? deletedId;
        bool? enabledSet;
        final api = apiFactory(
          onDelete: (id) => deletedId = id,
          onSetEnabled: (v) => enabledSet = v,
        );
        final c = UnifiedPushController(
          backend: backend,
          apiFactory: api,
          deviceState: deviceState(api),
          os: _FakeOs(),
        );
        await settle();
        await c.enable();
        await settle();
        expect(c.status, UnifiedPushStatus.on);

        await c.disable();
        expect(c.status, UnifiedPushStatus.off);
        expect(backend.unregisterCalls, [kUnifiedPushInstance]);
        expect(deletedId, 'ep_1');
        expect(enabledSet, false);
      },
    );
  });

  group('handleIncomingPushMessage', () {
    Uint8List payload(Map<String, dynamic> j) =>
        Uint8List.fromList(utf8.encode(jsonEncode(j)));

    test(
      'shows a decrypted notice with its own already-rendered text',
      () async {
        final os = _FakeOs();
        await handleIncomingPushMessage(
          PushMessage(
            payload({
              'id': 'ntf_1',
              'kind': 'needs_you',
              'urgency': 'high',
              'session_id': 'ses_1',
              'title': 'tidier',
              'body': 'needs you',
            }),
            true,
          ),
          kUnifiedPushInstance,
          os: os,
        );
        expect(os.shown.values.single.title, 'tidier');
        expect(os.shown.values.single.body, 'needs you');
        expect(os.shown.values.single.androidClass, 'needs_you');
      },
    );

    test('an undecrypted message is dropped, never shown', () async {
      final os = _FakeOs();
      await handleIncomingPushMessage(
        PushMessage(payload({'id': 'ntf_2', 'title': 'x'}), false),
        kUnifiedPushInstance,
        os: os,
      );
      expect(os.shown, isEmpty);
    });

    test('malformed JSON is dropped, never throws', () async {
      final os = _FakeOs();
      await handleIncomingPushMessage(
        PushMessage(Uint8List.fromList(utf8.encode('not json')), true),
        kUnifiedPushInstance,
        os: os,
      );
      expect(os.shown, isEmpty);
    });

    test('a test/summary notice gets no android channel class', () async {
      final os = _FakeOs();
      await handleIncomingPushMessage(
        PushMessage(
          payload({'id': 'ntf_3', 'kind': 'test', 'title': 'Test'}),
          true,
        ),
        kUnifiedPushInstance,
        os: os,
      );
      expect(os.shown.values.single.androidClass, isNull);
    });

    test('the same notice id is never shown twice', () async {
      final os = _FakeOs();
      final msg = payload({
        'id': 'ntf_dup',
        'kind': 'finished',
        'title': 'tidier finished',
      });
      await handleIncomingPushMessage(
        PushMessage(msg, true),
        kUnifiedPushInstance,
        os: os,
      );
      await handleIncomingPushMessage(
        PushMessage(msg, true),
        kUnifiedPushInstance,
        os: os,
      );
      expect(os.shown.length, 1);
    });
  });
}
