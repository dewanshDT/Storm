import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'package:storm/agent/agent_api.dart';
import 'package:storm/agent/agent_events.dart';
import 'package:storm/agent/launcher.dart';
import 'package:storm/agent/notifications_models.dart';
import 'package:storm/agent/notifications_state.dart';
import 'package:storm/router.dart';

import 'agent_fakes.dart';
import 'fake_server.dart';
import 'shell_harness.dart';

/// Decision 161, slice 161-3: the account's inbox state, its settings page,
/// the inbox panel and the launcher's "Notifies you" chip.
void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  const wide = Size(1400, 900);
  const primary = FakeServer.primaryVault;

  setUp(() => SharedPreferences.setMockInitialValues({}));

  Map<String, dynamic> host() => agentHost(
    id: 'hst_1',
    name: 'build-vm',
    providers: ['claude-code', 'opencode', 'shell', 'codex'],
    identity: 'verified',
    isolation: 'verified',
    execution: {'account': 'dewansh', 'isolation': 'per-session'},
    limits: {'max_children': 3, 'max_tree': 6},
  );

  group('NotificationsState', () {
    AgentApi fakeApi([Future<http.Response?> Function(http.Request)? extra]) =>
        AgentApi(
          baseUrl: 'http://x',
          token: 't',
          client: MockClient((req) async {
            final answered = await extra?.call(req);
            if (answered != null) return answered;
            return http.Response('', 404);
          }),
        );

    AgentNotification notif(
      String id, {
      String title = 't',
      bool read = false,
    }) => AgentNotification(
      id: id,
      seq: 1,
      created: DateTime.now().toUtc().toIso8601String(),
      source: 'storm',
      kind: 'needs_you',
      title: title,
      urgency: 'high',
      readAt: read ? DateTime.now().toUtc().toIso8601String() : null,
    );

    test('a pushed notification replaces an existing id, never duplicates', () {
      final state = NotificationsState(fakeApi, null);
      state.applyEvent(AgentNotificationPushed(notif('n1', title: 'first')));
      state.applyEvent(AgentNotificationPushed(notif('n1', title: 'second')));
      expect(state.items.length, 1);
      expect(state.items.single.title, 'second');
    });

    test('notification_read marks held ids read without a round trip', () {
      final state = NotificationsState(fakeApi, null);
      state.applyEvent(AgentNotificationPushed(notif('n1')));
      state.applyEvent(AgentNotificationPushed(notif('n2')));
      expect(state.unreadCount, 2);
      state.applyEvent(AgentNotificationsRead(['n1']));
      expect(state.unreadCount, 1);
      expect(state.items.where((n) => n.id == 'n1').single.unread, isFalse);
    });

    test('notifications_cleared empties the held inbox', () {
      final state = NotificationsState(fakeApi, null);
      state.applyEvent(AgentNotificationPushed(notif('n1')));
      state.applyEvent(AgentNotificationsCleared());
      expect(state.items, isEmpty);
    });

    test('a 404 on the first backfill marks the server unsupported', () async {
      final state = NotificationsState(
        () => fakeApi((req) async => null),
        null,
      );
      await Future<void>.delayed(Duration.zero);
      await Future<void>.delayed(Duration.zero);
      expect(state.unsupported, isTrue);
    });
  });

  group('Settings › Notifications and the inbox', () {
    testWidgets('loading, then loaded; a toggle writes prefs', (tester) async {
      final notif = FakeNotifications();
      final agents = FakeAgentServer(hosts: [host()]);
      agents.extra = notif.handle;
      final c = shellContainer(agentClient: agents.client);
      await pumpShell(tester, c, size: wide);
      c.read(routerProvider).go(Routes.settingsPage('notifications'));
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('notify-event-needs_you')), findsOneWidget);
      await tester.tap(find.byKey(const Key('notify-event-child_finished')));
      await tester.pumpAndSettle();
      expect((notif.prefs['events'] as Map)['child_finished']['on'], isTrue);
      await disposeShell(tester, c);
    });

    testWidgets('a stale version conflict reloads and tells the person', (
      tester,
    ) async {
      final notif = FakeNotifications();
      final agents = FakeAgentServer(hosts: [host()]);
      agents.extra = notif.handle;
      final c = shellContainer(agentClient: agents.client);
      await pumpShell(tester, c, size: wide);
      c.read(routerProvider).go(Routes.settingsPage('notifications'));
      await tester.pumpAndSettle();
      notif.refusePrefsPutOnce = (409, 'prefs_version');
      await tester.tap(find.byKey(const Key('notify-event-error')));
      await tester.pumpAndSettle();
      expect(
        find.text('Someone changed this elsewhere — reloaded.'),
        findsOneWidget,
      );
      await disposeShell(tester, c);
    });

    testWidgets('an older server hides the nav entry and settles quietly', (
      tester,
    ) async {
      final agents = FakeAgentServer(hosts: [host()]);
      final c = shellContainer(agentClient: agents.client);
      await pumpShell(tester, c, size: wide);
      c.read(routerProvider).go(Routes.settingsPage('device'));
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('settings-nav-notifications')), findsNothing);
      expect(find.byKey(const Key('bar-notifications')), findsNothing);
      await disposeShell(tester, c);
    });

    testWidgets('turning off new-session notify asks, then turns off '
        'running sessions too', (tester) async {
      final notif = FakeNotifications();
      final agents = FakeAgentServer(hosts: [host()]);
      agents.extra = notif.handle;
      final c = shellContainer(agentClient: agents.client);
      await pumpShell(tester, c, size: wide);
      c.read(routerProvider).go(Routes.settingsPage('notifications'));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const Key('notify-agent-default')));
      await tester.pumpAndSettle();
      expect(find.text('Also turn off for running sessions?'), findsOneWidget);
      await tester.tap(find.byKey(const Key('notify-default-off-confirm')));
      await tester.pumpAndSettle();
      expect(notif.notifyOffCalls, ['all']);
      expect(notif.prefs['agent_default_on'], isFalse);
      await disposeShell(tester, c);
    });

    testWidgets('unread badge, source labels, held reasons, mark all read', (
      tester,
    ) async {
      final notif = FakeNotifications()
        ..push(
          id: 'n1',
          source: 'storm',
          kind: 'needs_you',
          title: 'alpha needs you',
          sessionId: 'ags_1',
          sessionTitle: 'alpha',
        )
        ..push(
          id: 'n2',
          source: 'agent',
          kind: 'agent_message',
          title: 'build finished',
          sessionId: 'ags_1',
          sessionTitle: 'alpha',
          provider: 'claude-code',
        )
        ..push(
          id: 'n3',
          source: 'terminal',
          kind: 'terminal_notice',
          title: 'tests failed',
          sessionId: 'ags_1',
          sessionTitle: 'alpha',
          provider: 'codex',
        )
        ..push(
          id: 'n4',
          source: 'storm',
          kind: 'finished',
          title: 'beta finished its turn',
          silent: true,
          silentReason: 'quiet',
        );
      final agents = FakeAgentServer(hosts: [host()]);
      agents.extra = notif.handle;
      final c = shellContainer(agentClient: agents.client);
      await pumpShell(tester, c, size: wide);
      await tester.pumpAndSettle();
      expect(
        find.descendant(
          of: find.byKey(const Key('bar-notifications')),
          matching: find.text('4'),
        ),
        findsOneWidget,
      );
      await tester.tap(find.byKey(const Key('bar-notifications')));
      await tester.pumpAndSettle();
      expect(find.textContaining('alpha (Claude Code)'), findsOneWidget);
      expect(
        find.textContaining('alpha (Codex) via its terminal'),
        findsOneWidget,
      );
      expect(find.text('Held: quiet hours'), findsOneWidget);
      await tester.tap(find.byKey(const Key('notify-mark-all-read')));
      await tester.pumpAndSettle();
      expect(notif.items.every((n) => n['read_at'] != null), isTrue);
      await disposeShell(tester, c);
    });
  });

  group('the launcher chip', () {
    testWidgets('reflects agent_default_on until the person touches it', (
      tester,
    ) async {
      for (final on in [true, false]) {
        final notif = FakeNotifications()..prefs['agent_default_on'] = on;
        final agents = FakeAgentServer(hosts: [host()]);
        agents.extra = notif.handle;
        final c = shellContainer(agentClient: agents.client);
        await pumpShell(tester, c, size: wide);
        c.read(routerProvider).go(Routes.note(primary, 'n3'));
        await tester.pumpAndSettle();
        await tester.tap(find.byKey(const Key('start-session')));
        await tester.pumpAndSettle();
        final chip = tester.widget<LauncherChip>(
          find.byKey(const Key('launcher-chip-notify')),
        );
        expect(
          chip.state,
          on ? LauncherChipState.on : LauncherChipState.off,
          reason: 'agent_default_on=$on',
        );
        await disposeShell(tester, c);
      }
    });
  });
}

/// A minimal `/v1/notifications*` backend for `FakeAgentServer.extra`.
class FakeNotifications {
  FakeNotifications() : prefs = _defaults();

  Map<String, dynamic> prefs;
  final mutes = <Map<String, dynamic>>[];
  final items = <Map<String, dynamic>>[];
  final notifyOffCalls = <String>[];
  (int, String)? refusePrefsPutOnce;
  int _seq = 0;

  static Map<String, dynamic> _defaults() => {
    'version': 1,
    'events': {
      for (final k in kNotificationEventKinds)
        k: {
          'on': !{'child_finished', 'stopped_working'}.contains(k),
          if (k == 'finished') 'threshold_s': 30,
          if (k == 'host_offline') 'threshold_s': 120,
        },
    },
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

  void push({
    required String id,
    required String source,
    required String kind,
    required String title,
    String? sessionId,
    String? sessionTitle,
    String? provider,
    bool silent = false,
    String? silentReason,
  }) {
    items.add({
      'id': id,
      'seq': ++_seq,
      'created': DateTime.now().toUtc().toIso8601String(),
      'source': source,
      'kind': kind,
      'title': title,
      'urgency': 'normal',
      'silent': silent,
      'silent_reason': silentReason,
      'session_id': sessionId,
      'session_title': sessionTitle,
      'provider': provider,
      'read_at': null,
      'cancelled': false,
    });
  }

  http.Response _json(Object body, [int status = 200]) =>
      http.Response(jsonEncode(body), status);

  Future<http.Response?> handle(http.Request req) async {
    final path = req.url.path;
    if (path == '/v1/notifications' && req.method == 'GET') {
      final after = int.tryParse(req.url.queryParameters['after'] ?? '0') ?? 0;
      final page = items.where((n) => (n['seq'] as int) > after).toList();
      return _json({
        'items': page,
        'cursor': page.isEmpty ? after : page.last['seq'],
      });
    }
    if (path == '/v1/notifications' && req.method == 'DELETE') {
      items.clear();
      return http.Response('', 204);
    }
    if (path == '/v1/notifications/read' && req.method == 'POST') {
      final body = jsonDecode(req.body) as Map;
      final ids = (body['ids'] as List?)?.cast<String>();
      final sessionId = body['session_id'] as String?;
      final changed = <String>[];
      for (final n in items) {
        final match = ids != null
            ? ids.contains(n['id'])
            : n['session_id'] == sessionId;
        if (match && n['read_at'] == null) {
          n['read_at'] = DateTime.now().toUtc().toIso8601String();
          changed.add(n['id'] as String);
        }
      }
      return _json({'read': changed});
    }
    if (path == '/v1/notifications/prefs' && req.method == 'GET') {
      return _json(prefs);
    }
    if (path == '/v1/notifications/prefs' && req.method == 'PUT') {
      final body = jsonDecode(req.body) as Map<String, dynamic>;
      if (refusePrefsPutOnce case (final status, final code)?) {
        refusePrefsPutOnce = null;
        return _json({'error': code}, status);
      }
      if (body['version'] != prefs['version']) {
        return _json({'error': 'prefs_version'}, 409);
      }
      final next = Map<String, dynamic>.from(prefs);
      for (final key in body.keys) {
        if (key == 'version') continue;
        if (key == 'events' && body['events'] is Map) {
          final events = Map<String, dynamic>.from(next['events'] as Map);
          for (final e in (body['events'] as Map).entries) {
            events['${e.key}'] = e.value;
          }
          next['events'] = events;
        } else {
          next[key] = body[key];
        }
      }
      next['version'] = (prefs['version'] as int) + 1;
      prefs = next;
      return _json(prefs);
    }
    if (path == '/v1/notifications/mutes' && req.method == 'GET') {
      return _json(mutes);
    }
    if (path == '/v1/notifications/mutes' && req.method == 'POST') {
      final body = jsonDecode(req.body) as Map<String, dynamic>;
      final m = {
        'id': 'ntm_${mutes.length + 1}',
        'scope': body['scope'],
        'target': body['target'],
        'until': body['until'],
        'created_at': DateTime.now().toUtc().toIso8601String(),
      };
      mutes.removeWhere(
        (e) => e['scope'] == m['scope'] && e['target'] == m['target'],
      );
      mutes.add(m);
      return _json(m);
    }
    if (path.startsWith('/v1/notifications/mutes/') && req.method == 'DELETE') {
      final id = path.substring('/v1/notifications/mutes/'.length);
      final existed = mutes.any((m) => m['id'] == id);
      mutes.removeWhere((m) => m['id'] == id);
      return existed
          ? http.Response('', 204)
          : _json({'error': 'no such mute'}, 404);
    }
    if (path == '/v1/notifications/pause' && req.method == 'POST') {
      final body = jsonDecode(req.body) as Map<String, dynamic>;
      prefs = {
        ...prefs,
        'paused_until': body['until'] ?? '9999-12-31T23:59:59Z',
        'version': (prefs['version'] as int) + 1,
      };
      return _json(prefs);
    }
    if (path == '/v1/notifications/pause' && req.method == 'DELETE') {
      prefs = {
        ...prefs,
        'paused_until': null,
        'version': (prefs['version'] as int) + 1,
      };
      return _json(prefs);
    }
    if (path == '/v1/notifications/test' && req.method == 'POST') {
      final n = {
        'id': 'ntf_test',
        'seq': ++_seq,
        'created': DateTime.now().toUtc().toIso8601String(),
        'source': 'storm',
        'kind': 'test',
        'title': 'Test notification',
        'body': 'If this reached you, delivery to this device works.',
        'urgency': 'normal',
        'silent': false,
      };
      items.add(n);
      return _json(n);
    }
    if (path == '/v1/agent/sessions/notify-off' && req.method == 'POST') {
      final body = jsonDecode(req.body) as Map<String, dynamic>;
      notifyOffCalls.add(body['scope'] as String);
      return _json({'changed': 0});
    }
    return null;
  }
}
