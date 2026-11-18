import 'package:flutter_test/flutter_test.dart';
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
  deliver: deliver,
);

void main() {
  group('shouldRaiseOsNotification (161-4 deliver hint)', () {
    test('no hint at all never raises (an older server, or skipped)', () {
      expect(shouldRaiseOsNotification(null, 'dev_a'), isFalse);
    });

    test('"os" raises on every device', () {
      expect(shouldRaiseOsNotification('os', 'dev_a'), isTrue);
      expect(shouldRaiseOsNotification('os', 'dev_b'), isTrue);
    });

    test('"toast:<id>" raises only on the named device', () {
      expect(shouldRaiseOsNotification('toast:dev_a', 'dev_a'), isTrue);
      expect(shouldRaiseOsNotification('toast:dev_a', 'dev_b'), isFalse);
    });
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
      expect(
        deviceNotificationClassOf(_notif(kind: 'child_finished')),
        'agent_messages',
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

    test('disabled on this device blocks everything', () {
      expect(devicePrefsAllow(_notif(), prefs(enabled: false)), isFalse);
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

    test('session is "{session} · {kind}", never the agent\'s own words', () {
      final r = renderPreview(
        _notif(kind: 'needs_you', title: 'secret', body: 'also secret'),
        'session',
      );
      expect(r.title, 'tidier · needs_you');
      expect(r.body, isNull);
    });

    test('storm is "Storm · {kind}" — not even the session name', () {
      final r = renderPreview(_notif(kind: 'finished'), 'storm');
      expect(r.title, 'Storm · finished');
      expect(r.body, isNull);
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
      expect(third.title, 'tidier · 3 updates');
      expect(third.body, isNull);
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
      expect(third.title, isNot(contains('updates')));
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
        expect(r.title, isNot(contains('updates')));
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
      expect(osCategoryOf(_notif(source: 'storm')), 'storm');
      expect(osCategoryOf(_notif(source: 'agent')), 'agent-messages');
      expect(osCategoryOf(_notif(source: 'terminal')), 'agent-messages');
    });
  });

  group('decodeTapPayload', () {
    test('null and empty decode to nothing', () {
      expect(decodeTapPayload(null), isNull);
      expect(decodeTapPayload(''), isNull);
    });

    test('a session payload decodes to its id', () {
      final target = decodeTapPayload('session:ags_1');
      expect(target, isA<NotificationTapSession>());
      expect((target as NotificationTapSession).sessionId, 'ags_1');
    });

    test('garbage decodes to nothing rather than throwing', () {
      expect(decodeTapPayload('not a real payload'), isNull);
    });
  });
}
