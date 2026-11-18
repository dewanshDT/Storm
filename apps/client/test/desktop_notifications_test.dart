import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:storm/agent/agent_events.dart';
import 'package:storm/agent/desktop_notifications.dart';
import 'package:storm/agent/notifications_models.dart';

AgentNotification _notif({
  String id = 'ntf_1',
  String source = 'storm',
  String kind = 'needs_you',
  String? sessionId = 'ags_1',
  String? sessionTitle = 'tidier',
  String? provider = 'claude-code',
  String title = 'Needs you',
  String? body,
  bool silent = false,
  String? deliver,
}) => AgentNotification(
  id: id,
  seq: 1,
  created: '2026-11-18T00:00:00Z',
  source: source,
  kind: kind,
  sessionId: sessionId,
  sessionTitle: sessionTitle,
  provider: provider,
  title: title,
  body: body,
  urgency: 'normal',
  silent: silent,
  hasDeliverHint: true,
  deliver: deliver,
);

void main() {
  group('shouldRaiseOsNotification (161-4 deliver hint, review item 1)', () {
    test('no hint at all never raises (an older server, or skipped)', () {
      expect(shouldRaiseOsNotification(null, thisDeviceActive: false), isFalse);
    });

    test('"os" raises only while this device is backgrounded', () {
      expect(shouldRaiseOsNotification('os', thisDeviceActive: false), isTrue);
      expect(shouldRaiseOsNotification('os', thisDeviceActive: true), isFalse);
    });

    test(
      '"toast:<id>" never raises an OS notification on any device, '
      'active or not — that is an in-app toast\'s job, never this one\'s',
      () {
        expect(
          shouldRaiseOsNotification('toast:dev_a', thisDeviceActive: false),
          isFalse,
        );
        expect(
          shouldRaiseOsNotification('toast:dev_a', thisDeviceActive: true),
          isFalse,
        );
      },
    );
  });

  group('deviceNotificationClassOf', () {
    test('an agent or terminal notice is always agent_messages', () {
      expect(
        deviceNotificationClassOf(_notif(source: 'agent', kind: 'needs_you')),
        'agent_messages',
      );
      expect(
        deviceNotificationClassOf(_notif(source: 'terminal', kind: 'error')),
        'agent_messages',
      );
    });

    test('a storm notice maps by kind', () {
      expect(deviceNotificationClassOf(_notif(kind: 'needs_you')), 'needs_you');
      expect(
        deviceNotificationClassOf(_notif(kind: 'delete_request')),
        'delete_requests',
      );
      expect(deviceNotificationClassOf(_notif(kind: 'finished')), 'finished');
      expect(deviceNotificationClassOf(_notif(kind: 'error')), 'errors');
      expect(deviceNotificationClassOf(_notif(kind: 'usage_limit')), 'errors');
      expect(deviceNotificationClassOf(_notif(kind: 'host_offline')), 'hosts');
    });

    test('review item 10: stopped_working and child_finished map to finished, '
        'never the catch-all', () {
      expect(
        deviceNotificationClassOf(_notif(kind: 'stopped_working')),
        'finished',
      );
      expect(
        deviceNotificationClassOf(_notif(kind: 'child_finished')),
        'finished',
      );
    });
  });

  group('devicePrefsAllow', () {
    DeviceNotificationPrefs prefs({
      bool enabled = true,
      List<String> classes = const [],
    }) => DeviceNotificationPrefs(
      deviceId: 'dev_a',
      enabled: enabled,
      classes: classes,
      preview: 'full',
      updatedAt: '2026-11-18T00:00:00Z',
    );

    test('disabled on this device blocks everything, test and summary '
        'included', () {
      expect(devicePrefsAllow(_notif(), prefs(enabled: false)), isFalse);
      expect(
        devicePrefsAllow(_notif(kind: 'test'), prefs(enabled: false)),
        isFalse,
      );
    });

    test('an empty classes list is every class', () {
      expect(devicePrefsAllow(_notif(kind: 'needs_you'), prefs()), isTrue);
      expect(devicePrefsAllow(_notif(source: 'agent'), prefs()), isTrue);
    });

    test('a non-empty classes list only lets its own class through', () {
      final p = prefs(classes: const ['needs_you']);
      expect(devicePrefsAllow(_notif(kind: 'needs_you'), p), isTrue);
      expect(devicePrefsAllow(_notif(kind: 'error'), p), isFalse);
    });

    test('review item 10: test and summary bypass the class filter entirely '
        '— only "enabled" gates them', () {
      final p = prefs(classes: const ['needs_you']);
      expect(devicePrefsAllow(_notif(kind: 'test'), p), isTrue);
      expect(devicePrefsAllow(_notif(kind: 'summary'), p), isTrue);
    });
  });

  group('renderPreview', () {
    test('full is "{session} · {Agent}" plus the real text', () {
      final r = renderPreview(_notif(title: 't', body: 'b'), 'full');
      expect(r.title, 'tidier · Claude Code');
      expect(r.body, 't\nb');
    });

    test('full with no body falls back to the title alone', () {
      final r = renderPreview(_notif(title: 'only a title'), 'full');
      expect(r.body, 'only a title');
    });

    test('full with no provider (no session, e.g.) is the session alone', () {
      final r = renderPreview(_notif(provider: null), 'full');
      expect(r.title, 'tidier');
    });

    test('session is "{session} · {human label}" (review item 9), never the '
        "agent's own words or the raw kind code", () {
      final r = renderPreview(
        _notif(kind: 'needs_you', title: 'secret', body: 'also secret'),
        'session',
      );
      expect(r.title, 'tidier · Needs you');
      expect(r.body, isNull);
    });

    test('storm is "Storm · {human label}" — not even the session name', () {
      final r = renderPreview(_notif(kind: 'finished'), 'storm');
      expect(r.title, 'Storm · Finished its turn');
      expect(r.body, isNull);
    });

    test('an unlabelled kind falls back to its own code, not a crash', () {
      final r = renderPreview(_notif(kind: 'summary'), 'storm');
      expect(r.title, 'Storm · summary');
    });
  });

  group('planOsNotification: the 60s/>=3 fold', () {
    test('fewer than 3 in the window shows each on its own', () {
      final bursts = <String, NotificationBurst>{};
      final t0 = DateTime(2026, 11, 18, 12, 0, 0);
      final a = planOsNotification(_notif(id: 'n1'), 'full', bursts, t0);
      final b = planOsNotification(
        _notif(id: 'n2'),
        'full',
        bursts,
        t0.add(const Duration(seconds: 10)),
      );
      expect(a.folded, isFalse);
      expect(b.folded, isFalse);
      expect(a.title, isNot(contains('updates')));
      expect(b.title, isNot(contains('updates')));
      expect(a.intId, isNot(b.intId));
    });

    test('the third within 60s folds into one "N updates" entry', () {
      final bursts = <String, NotificationBurst>{};
      final t0 = DateTime(2026, 11, 18, 12, 0, 0);
      planOsNotification(_notif(id: 'n1'), 'full', bursts, t0);
      planOsNotification(
        _notif(id: 'n2'),
        'full',
        bursts,
        t0.add(const Duration(seconds: 5)),
      );
      final third = planOsNotification(
        _notif(id: 'n3'),
        'full',
        bursts,
        t0.add(const Duration(seconds: 10)),
      );
      expect(third.folded, isTrue);
      expect(third.title, 'tidier · 3 updates');
      expect(third.body, isNull);
    });

    test('review item 8: a fold at the storm preview level never names the '
        'session', () {
      final bursts = <String, NotificationBurst>{};
      final t0 = DateTime(2026, 11, 18, 12, 0, 0);
      planOsNotification(_notif(id: 'n1'), 'storm', bursts, t0);
      planOsNotification(_notif(id: 'n2'), 'storm', bursts, t0);
      final third = planOsNotification(_notif(id: 'n3'), 'storm', bursts, t0);
      expect(third.title, 'Storm · 3 updates');
    });

    test('a fourth replaces the same folded entry, not a new one', () {
      final bursts = <String, NotificationBurst>{};
      final t0 = DateTime(2026, 11, 18, 12, 0, 0);
      planOsNotification(_notif(id: 'n1'), 'full', bursts, t0);
      planOsNotification(_notif(id: 'n2'), 'full', bursts, t0);
      final third = planOsNotification(_notif(id: 'n3'), 'full', bursts, t0);
      final fourth = planOsNotification(_notif(id: 'n4'), 'full', bursts, t0);
      expect(fourth.title, 'tidier · 4 updates');
      expect(fourth.intId, third.intId);
    });

    test('one outside the 60s window does not count toward the fold', () {
      final bursts = <String, NotificationBurst>{};
      final t0 = DateTime(2026, 11, 18, 12, 0, 0);
      planOsNotification(_notif(id: 'n1'), 'full', bursts, t0);
      planOsNotification(
        _notif(id: 'n2'),
        'full',
        bursts,
        t0.add(const Duration(seconds: 61)),
      );
      final third = planOsNotification(
        _notif(id: 'n3'),
        'full',
        bursts,
        t0.add(const Duration(seconds: 62)),
      );
      // Only n2 and n3 are within 60s of each other at this point.
      expect(third.folded, isFalse);
    });

    test('a notice with no session never folds', () {
      final bursts = <String, NotificationBurst>{};
      final t0 = DateTime(2026, 11, 18, 12, 0, 0);
      for (var i = 0; i < 5; i++) {
        final r = planOsNotification(
          _notif(id: 'n$i', sessionId: null),
          'full',
          bursts,
          t0,
        );
        expect(r.folded, isFalse);
      }
    });
  });

  group('notificationIntId / sessionBurstIntId', () {
    test('the same key always gives the same id', () {
      expect(notificationIntId('ntf_abc'), notificationIntId('ntf_abc'));
    });

    test('different keys give different ids (not guaranteed, but true for '
        'these)', () {
      expect(notificationIntId('ntf_abc'), isNot(notificationIntId('ntf_xyz')));
    });

    test('a burst id never collides with a plain notice id for the same '
        'string', () {
      expect(sessionBurstIntId('ags_1'), isNot(notificationIntId('ags_1')));
    });

    test('ids are always non-negative (the plugin wants a plain int)', () {
      for (final key in ['a', 'ntf_1', 'burst:ags_1', '']) {
        expect(notificationIntId(key), greaterThanOrEqualTo(0));
      }
    });
  });

  group('osCategoryOf', () {
    test('storm is its own category, distinct from an agent\'s', () {
      expect(osCategoryOf(_notif(source: 'storm')), kStormNotificationCategory);
      expect(
        osCategoryOf(_notif(source: 'agent')),
        kAgentMessagesNotificationCategory,
      );
      expect(
        osCategoryOf(_notif(source: 'terminal')),
        kAgentMessagesNotificationCategory,
      );
    });
  });

  group('decodeTapPayload', () {
    test('null, empty and garbage all still bring the window forward (review '
        'item 18) — there is just nowhere further to route', () {
      expect(decodeTapPayload(null), isA<NotificationTapNone>());
      expect(decodeTapPayload(''), isA<NotificationTapNone>());
      expect(
        decodeTapPayload('not a real payload'),
        isA<NotificationTapNone>(),
      );
    });

    test('a session payload decodes to its id', () {
      final target = decodeTapPayload('session:ags_1');
      expect(target, isA<NotificationTapSession>());
      expect((target as NotificationTapSession).sessionId, 'ags_1');
    });
  });

  group('DesktopNotifier with a fake OsNotifications', () {
    late _FakeOs os;
    late _Harness harness;

    DeviceNotificationPrefs allowAll() => const DeviceNotificationPrefs(
      deviceId: 'dev_a',
      enabled: true,
      classes: [],
      preview: 'full',
      updatedAt: '',
    );

    setUpAll(() => debugDefaultTargetPlatformOverride = TargetPlatform.linux);
    tearDownAll(() => debugDefaultTargetPlatformOverride = null);

    setUp(() {
      os = _FakeOs();
      harness = _Harness();
    });

    tearDown(() => harness.dispose());

    test('"os" shows while backgrounded; cancel on read removes it', () async {
      harness.setActive(false);
      final notifier = DesktopNotifier(
        deviceId: 'dev_a',
        events: harness.events,
        prefs: allowAll,
        os: os,
      );
      harness.push(_notif(id: 'n1', deliver: 'os'));
      await pumpEventQueue();
      expect(os.shown.keys, contains(notificationIntId('n1')));

      harness.read(['n1']);
      await pumpEventQueue();
      expect(os.shown.keys, isNot(contains(notificationIntId('n1'))));
      notifier.dispose();
    });

    test('"toast:<id>" never shows an OS notification here', () async {
      harness.setActive(false);
      final notifier = DesktopNotifier(
        deviceId: 'dev_a',
        events: harness.events,
        prefs: allowAll,
        os: os,
      );
      harness.push(_notif(id: 'n1', deliver: 'toast:dev_a'));
      await pumpEventQueue();
      expect(os.shown, isEmpty);
      notifier.dispose();
    });

    test('review item 8: reading one notice in a fold cancels the whole '
        'rollup, and its id stops tracking anything', () async {
      harness.setActive(false);
      final notifier = DesktopNotifier(
        deviceId: 'dev_a',
        events: harness.events,
        prefs: allowAll,
        os: os,
      );
      for (final id in ['n1', 'n2', 'n3']) {
        harness.push(_notif(id: id, deliver: 'os'));
      }
      await pumpEventQueue();
      final foldId = sessionBurstIntId('ags_1');
      expect(os.shown.keys, contains(foldId));

      harness.read(['n2']);
      await pumpEventQueue();
      expect(os.shown.keys, isNot(contains(foldId)));
      notifier.dispose();
    });

    test('cancelAll on a cleared inbox', () async {
      harness.setActive(false);
      final notifier = DesktopNotifier(
        deviceId: 'dev_a',
        events: harness.events,
        prefs: allowAll,
        os: os,
      );
      harness.push(_notif(id: 'n1', deliver: 'os'));
      await pumpEventQueue();
      expect(os.shown, isNotEmpty);

      harness.clearAll();
      await pumpEventQueue();
      expect(os.cancelledAll, isTrue);
      notifier.dispose();
    });

    test('review item 11: an init failure marks the notifier unavailable '
        'rather than throwing', () async {
      os.failInit = true;
      harness.setActive(false);
      final notifier = DesktopNotifier(
        deviceId: 'dev_a',
        events: harness.events,
        prefs: allowAll,
        os: os,
      );
      harness.push(_notif(id: 'n1', deliver: 'os'));
      await pumpEventQueue();
      expect(notifier.unavailable, isTrue);
      notifier.dispose();
    });

    test('review item 13: forceShow bypasses the deliver hint and presence '
        'entirely', () async {
      // `harness.events` starts active by default — the normal path
      // (`deliver: null`, this device active) would show nothing at all.
      final notifier = DesktopNotifier(
        deviceId: 'dev_a',
        events: harness.events,
        prefs: allowAll,
        os: os,
      );
      await notifier.forceShow(_notif(id: 'n1', kind: 'test', sessionId: null));
      expect(os.shown.keys, contains(notificationIntId('n1')));
      notifier.dispose();
    });

    test(
      'a permission request happens lazily, once, before the first show',
      () async {
        harness.setActive(false);
        final notifier = DesktopNotifier(
          deviceId: 'dev_a',
          events: harness.events,
          prefs: allowAll,
          os: os,
        );
        expect(os.permissionRequests, 0);
        harness.push(_notif(id: 'n1', deliver: 'os'));
        await pumpEventQueue();
        expect(os.permissionRequests, 1);
        harness.push(_notif(id: 'n2', deliver: 'os'));
        await pumpEventQueue();
        expect(os.permissionRequests, 1, reason: 'only ever asked once');
        notifier.dispose();
      },
    );
  });
}

/// A fake [OsNotifications] that just records what would have happened —
/// no real platform channel exists under `flutter test`.
class _FakeOs implements OsNotifications {
  final shown = <int, String>{};
  var cancelledAll = false;
  var permissionRequests = 0;
  var failInit = false;
  bool? permission = true;

  @override
  Future<void> init({required void Function(String? payload) onTap}) async {
    if (failInit) throw StateError('no notification service');
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
    shown[id] = title;
  }

  @override
  Future<void> cancel(int id) async {
    shown.remove(id);
  }

  @override
  Future<void> cancelAll() async {
    shown.clear();
    cancelledAll = true;
  }

  @override
  Future<bool?> checkPermission() async => permission;

  @override
  Future<void> requestPermission() async {
    permissionRequests++;
  }

  @override
  Future<bool> openSystemSettings() async => false;
}

/// Drives a real [AgentEvents] over a controller this test owns, so
/// `DesktopNotifier`'s own stream subscription sees exactly the frames a
/// test pushes. Requires a desktop `debugDefaultTargetPlatformOverride`
/// (set by the group around these tests) — [setActive] goes through
/// `AgentEvents.background`/`.foreground`, which only touch presence
/// without dropping the stream on desktop.
class _Harness {
  _Harness() {
    events = AgentEvents((_) => _controller.stream)
      ..start(followLifecycle: false);
  }

  final _controller = StreamController<AgentEvent>();
  late final AgentEvents events;

  /// Active by default (matches [AgentPresence.initial]); call this only
  /// to go background, or to return to active after doing so.
  void setActive(bool active) {
    if (active) {
      events.foreground();
    } else {
      events.background();
    }
  }

  void push(AgentNotification n) => _controller.add(AgentNotificationPushed(n));
  void read(List<String> ids) => _controller.add(AgentNotificationsRead(ids));
  void clearAll() => _controller.add(AgentNotificationsCleared());

  void dispose() {
    events.dispose();
    unawaited(_controller.close());
  }
}
