import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:shared_preferences/shared_preferences.dart';

import 'package:storm/agent/delete_request.dart'
    show DeleteAsker, kDeleteToastedKey;
import 'package:storm/router.dart';
import 'package:storm/state/nav_memory.dart';
import 'package:storm/ui/controls.dart';
import 'package:storm/ui/trash_screen.dart' show TrashRetention;

import 'agent_fakes.dart';
import 'fake_server.dart';
import 'shell_harness.dart';

const _reason =
    'Finished in 2024 - see [the plan](http://example.com) and **this**.';

const _vault = FakeServer.primaryVault;

Map<String, dynamic> _request({
  String id = 'adr_1',
  String session = 'ags_d',
  String kind = 'note',
  String state = 'pending',
  String? decidedBy,
}) => {
  'id': id,
  'session_id': session,
  'vault_id': _vault,
  'kind': kind,
  if (kind == 'note') 'note_id': 'n1',
  'path': kind == 'note' ? 'Ideas/Ideas.md' : 'Projects/Old',
  if (kind == 'note') 'title': 'Ideas',
  'reason': _reason,
  'notes': kind == 'note' ? 1 : 12,
  'size': 2048,
  'modified': DateTime.now()
      .subtract(const Duration(hours: 2))
      .toUtc()
      .toIso8601String(),
  'paths': kind == 'note'
      ? ['Ideas/Ideas.md']
      : [
          for (var i = 0; i < 12; i++)
            'Projects/Old/N${i.toString().padLeft(2, '0')}.md',
        ],
  'created_at': '2026-10-30T09:00:00Z',
  'expires_at': '2026-10-30T09:10:00Z',
  'state': state,
  'decided_by': ?decidedBy,
};

/// A session waiting on [request], as the server's view carries it.
Map<String, dynamic> _asking(
  String id,
  Map<String, dynamic> request, {
  String name = 'tidier',
  List<Map<String, dynamic>>? decisions,
}) => {
  ...agentSession(id, name: name),
  'needs_attention': 'delete_approval',
  'delete_request': request,
  'pending_decisions':
      decisions ??
      [
        {
          'kind': 'delete_request',
          'id': request['id'],
          'created_at': request['created_at'],
          'expires_at': request['expires_at'],
        },
      ],
};

http.Response _json(Object body, [int status = 200]) => http.Response.bytes(
  utf8.encode(jsonEncode(body)),
  status,
  headers: {'content-type': 'application/json; charset=utf-8'},
);

/// A server holding agents' deletions (decision 149): its list, every
/// answer it was sent, and what answered first. An answer also clears the
/// session's own view, as the real server's does.
class _Deletes {
  _Deletes(this.agents, {List<Map<String, dynamic>>? requests})
    : requests = requests ?? [_request()] {
    agents.extra = _handle;
  }

  final FakeAgentServer agents;
  final List<Map<String, dynamic>> requests;
  final answers = <Map<String, dynamic>>[];
  int version = 1;

  Future<http.Response?> _handle(http.Request req) async {
    final seg = req.url.pathSegments;
    if (req.method == 'GET' && req.url.path == '/v1/agent/delete-requests') {
      return _json({
        'version': version,
        'requests': [
          for (final r in requests)
            {
              ...r,
              'session': {
                'id': r['session_id'],
                'name': agents.session(r['session_id'] as String)?['name'],
                'provider': 'claude-code',
                'model': 'sonnet',
              },
            },
        ],
      });
    }
    if (req.method == 'POST' &&
        seg.length == 6 &&
        seg[4] == 'delete-requests') {
      final body = jsonDecode(req.body) as Map<String, dynamic>;
      answers.add(body);
      final r = requests.firstWhere((r) => r['id'] == seg[5]);
      if (r['state'] != 'pending') {
        return _json({'error': 'request_${r['state']}'}, 409);
      }
      r['state'] = body['decision'] == 'approve' ? 'deleted' : 'declined';
      r['decided_by'] = 'This phone';
      _settle(r);
      version++;
      return _json(r);
    }
    return null;
  }

  void _settle(Map<String, dynamic> r) {
    final s = agents.session(r['session_id'] as String);
    if (s == null) return;
    s.remove('delete_request');
    s.remove('needs_attention');
    s['pending_decisions'] = const [];
  }

  /// Another device answers first.
  void answeredElsewhere(String by) {
    requests.first['state'] = 'deleted';
    requests.first['decided_by'] = by;
    _settle(requests.first);
    version++;
  }
}

Future<(ProviderContainer, _Deletes)> _open(
  WidgetTester t, {
  List<Map<String, dynamic>>? requests,
  List<Map<String, dynamic>>? sessions,
  Size size = const Size(1400, 900),
  String? at,
  Object? retention = 30,
}) async {
  final pending = requests ?? [_request()];
  final agents = FakeAgentServer(
    hosts: [agentHost()],
    sessions:
        sessions ??
        [
          for (final r in pending)
            if (r['state'] == 'pending')
              _asking(r['session_id'] as String, r)
            else
              agentSession(r['session_id'] as String, name: 'tidier'),
        ],
  );
  final deletes = _Deletes(agents, requests: pending);
  // Opened there, so the watcher's first look already knows the screen.
  final c = shellContainer(
    agentClient: agents.client,
    nav: at == null
        ? const NavMemory()
        : NavMemory(activity: Activity.agents, agents: at),
  );
  if (retention != #unknown) {
    serverOf(c).trashRetention[_vault] = retention as int?;
  }
  await pumpShell(t, c, size: size);
  await t.pumpAndSettle();
  return (c, deletes);
}

Finder _strip() => find.byKey(const Key('delete-request-strip'));

String _text(WidgetTester t, Finder f) => t.widget<Text>(f).data!;

Finder _row(String key) => find.byKey(Key('delete-request-$key'));

/// The value beside a strip row's key.
String _value(WidgetTester t, String key) => t
    .widgetList<Text>(
      find.descendant(of: _row(key), matching: find.byType(Text)),
    )
    .elementAt(1)
    .data!;

/// Decisions 149c and 153g/h in the client: an agent's deletion waits in a
/// strip under its session's header, never in a dialog; anywhere else a
/// toast says so, once.
void main() {
  setUp(() => SharedPreferences.setMockInitialValues({}));

  group('153g: the strip', () {
    testWidgets('says who asks, their words, what goes and where, with '
        'Later, Decline and Move to Trash', (t) async {
      final (c, deletes) = await _open(t, at: Routes.agentSession('ags_d'));
      expect(find.byType(AlertDialog), findsNothing);
      expect(_strip(), findsOneWidget);
      expect(
        _text(t, find.byKey(const Key('decision-label'))),
        'NEEDS YOU · 1 OF 1',
      );
      expect(
        _text(t, find.byKey(const Key('decision-title'))),
        'tidier wants to delete Ideas.md',
      );
      expect(find.text('CLAUDE CODE SAYS'), findsOneWidget);
      expect(_value(t, 'says'), '“$_reason”', reason: 'word for word');
      expect(find.text('WHAT'), findsOneWidget);
      expect(_value(t, 'what'), 'Ideas/Ideas.md · 2.0 KB · modified 2h ago');
      expect(find.text('IF YOU APPROVE'), findsOneWidget);
      expect(
        _value(t, 'if-approved'),
        'It moves to Primary’s Trash and is kept for 30 days.',
      );
      final buttons = [
        for (final k in ['later', 'decline', 'approve'])
          t.widget<StormButton>(find.byKey(Key('delete-request-$k'))),
      ];
      expect(
        [for (final b in buttons) b.label],
        ['Later', 'Decline', 'Move to Trash'],
      );
      expect(
        [for (final b in buttons) b.kind],
        [
          StormButtonKind.text,
          StormButtonKind.outline,
          StormButtonKind.primary,
        ],
        reason: 'Move to Trash can be undone, so it is not danger',
      );
      final later = t.getCenter(find.byKey(const Key('delete-request-later')));
      final approve = t.getCenter(
        find.byKey(const Key('delete-request-approve')),
      );
      expect(later.dx, lessThan(approve.dx));
      // The title and the rows start in the one column.
      expect(
        t.getTopLeft(find.byKey(const Key('decision-title'))).dx,
        t.getTopLeft(find.text('“$_reason”')).dx,
      );
      expect(deletes.answers, isEmpty);
      await disposeShell(t, c);
    });

    for (final (how, key, answer) in [
      ('Move to Trash', 'approve', 'approve'),
      ('Decline', 'decline', 'decline'),
    ]) {
      testWidgets('$how answers $answer and the strip goes', (t) async {
        final (c, deletes) = await _open(t, at: Routes.agentSession('ags_d'));
        await t.tap(find.byKey(Key('delete-request-$key')));
        await t.pumpAndSettle();
        expect(deletes.answers, [
          {'decision': answer},
        ]);
        expect(_strip(), findsNothing);
        await t.pump(const Duration(seconds: 2));
        await t.pumpAndSettle();
        expect(
          find.byKey(const Key('delete-answered-elsewhere')),
          findsNothing,
        );
        await disposeShell(t, c);
      });
    }

    for (final how in ['Later', 'Esc']) {
      testWidgets('$how puts it aside without answering; Review brings it '
          'back', (t) async {
        final (c, deletes) = await _open(t, at: Routes.agentSession('ags_d'));
        if (how == 'Later') {
          await t.tap(find.byKey(const Key('delete-request-later')));
        } else {
          // Esc is the strip's once the terminal no longer has the key.
          await t.tap(find.byKey(const Key('decision-title')));
          await t.pump();
          await t.sendKeyEvent(LogicalKeyboardKey.escape);
        }
        await t.pumpAndSettle();
        expect(deletes.answers, isEmpty, reason: 'Later is not Decline');
        expect(deletes.requests.first['state'], 'pending');
        expect(_strip(), findsOneWidget, reason: 'folded, not gone');
        expect(find.byKey(const Key('delete-request-approve')), findsNothing);
        expect(c.read(routerProvider).state.uri.path, '/agents/s/ags_d');
        await t.tap(find.byKey(const Key('delete-request-review')));
        await t.pumpAndSettle();
        expect(find.byKey(const Key('delete-request-approve')), findsOneWidget);
        await disposeShell(t, c);
      });
    }

    testWidgets('a terminal with focus keeps its own Esc', (t) async {
      final (c, deletes) = await _open(t, at: Routes.agentSession('ags_d'));
      final terminal = FocusManager.instance.primaryFocus;
      expect(terminal, isNotNull);
      await t.sendKeyEvent(LogicalKeyboardKey.escape);
      await t.pumpAndSettle();
      expect(find.byKey(const Key('delete-request-approve')), findsOneWidget);
      expect(deletes.answers, isEmpty);
      await disposeShell(t, c);
    });

    testWidgets('"1 OF 2" when another decision is waiting', (t) async {
      final r = _request();
      final (c, _) = await _open(
        t,
        at: Routes.agentSession('ags_d'),
        sessions: [
          _asking(
            'ags_d',
            r,
            decisions: [
              {'kind': 'delete_request', 'id': 'adr_1'},
              {'kind': 'delete_request', 'id': 'adr_9'},
            ],
          ),
        ],
      );
      expect(
        _text(t, find.byKey(const Key('decision-label'))),
        'NEEDS YOU · 1 OF 2',
      );
      await disposeShell(t, c);
    });

    testWidgets('a folder names its notes and its first paths', (t) async {
      final (c, _) = await _open(
        t,
        at: Routes.agentSession('ags_d'),
        requests: [_request(kind: 'folder')],
      );
      expect(
        _text(t, find.byKey(const Key('decision-title'))),
        'tidier wants to delete folder Projects/Old and its 12 notes',
      );
      expect(find.text('Projects/Old/N00.md'), findsOneWidget);
      expect(find.text('Projects/Old/N02.md'), findsOneWidget);
      expect(find.text('Projects/Old/N03.md'), findsNothing);
      expect(find.text('…and 9 more'), findsOneWidget);
      await disposeShell(t, c);
    });

    for (final (retention, said) in [
      (7, 'It moves to Primary’s Trash and is kept for 7 days.'),
      (null, 'It moves to Primary’s Trash and is kept until you empty it.'),
      (
        #unknown,
        'It moves to Primary’s Trash and is kept for the vault’s retention '
            'period.',
      ),
    ]) {
      testWidgets('the vault’s own retention: $retention', (t) async {
        final (c, _) = await _open(
          t,
          at: Routes.agentSession('ags_d'),
          retention: retention,
        );
        expect(_value(t, 'if-approved'), said);
        await disposeShell(t, c);
      });
    }

    test('kept for, in words', () {
      expect(const TrashRetention.days(1).keptFor, 'kept for 1 day');
      expect(const TrashRetention.days(90).keptFor, 'kept for 90 days');
      expect(const TrashRetention.never().keptFor, 'kept until you empty it');
      expect(
        const TrashRetention.unknown().keptFor,
        'kept for the vault’s retention period',
      );
    });

    testWidgets('answered on another device: the strip goes and says where', (
      t,
    ) async {
      final (c, deletes) = await _open(t, at: Routes.agentSession('ags_d'));
      expect(_strip(), findsOneWidget);
      deletes.answeredElsewhere('Laptop');
      await t.pump(const Duration(seconds: 2));
      await t.pumpAndSettle();
      expect(_strip(), findsNothing);
      expect(find.text('Answered on Laptop'), findsOneWidget);
      expect(deletes.answers, isEmpty);
      await disposeShell(t, c);
    });
  });

  group('153h: elsewhere, a toast and never a dialog', () {
    testWidgets('a request for another session: one toast, and Review goes '
        'to the session', (t) async {
      final (c, deletes) = await _open(t, at: Routes.agents);
      expect(find.byType(AlertDialog), findsNothing);
      expect(find.text('tidier wants to delete Ideas.md'), findsOneWidget);
      expect(find.byType(SnackBar), findsOneWidget);
      // Later polls do not raise it again.
      deletes.version++;
      await t.pump(const Duration(seconds: 2));
      await t.pumpAndSettle();
      expect(find.byType(SnackBar), findsOneWidget);
      await t.tap(find.byKey(const Key('delete-toast-review')));
      await t.pumpAndSettle();
      expect(c.read(routerProvider).state.uri.path, '/agents/s/ags_d');
      expect(_strip(), findsOneWidget);
      expect(find.byType(AlertDialog), findsNothing);
      expect(deletes.answers, isEmpty);
      await disposeShell(t, c);
    });

    testWidgets('on a notes screen too, and once per device across a '
        'restart', (t) async {
      final (c, _) = await _open(t);
      expect(find.byType(AlertDialog), findsNothing);
      expect(find.byType(SnackBar), findsOneWidget);
      final prefs = await SharedPreferences.getInstance();
      expect(prefs.getStringList(kDeleteToastedKey), ['adr_1']);
      await disposeShell(t, c);

      final (again, _) = await _open(t);
      expect(find.byType(SnackBar), findsNothing, reason: 'shown once');
      await disposeShell(t, again);
    });

    testWidgets('no toast while its session is open', (t) async {
      final (c, _) = await _open(t, at: Routes.agentSession('ags_d'));
      expect(find.byType(SnackBar), findsNothing);
      expect(_strip(), findsOneWidget);
      // Nor once the person leaves it: they have seen it.
      c.read(routerProvider).go(Routes.agents);
      await t.pump(const Duration(seconds: 2));
      await t.pumpAndSettle();
      expect(find.byType(SnackBar), findsNothing);
      await disposeShell(t, c);
    });

    testWidgets('a newer request replaces the toast showing', (t) async {
      final (c, deletes) = await _open(t, at: Routes.agents);
      expect(find.byType(SnackBar), findsOneWidget);
      final other = _request(id: 'adr_2', session: 'ags_e');
      deletes.agents.sessions.add(_asking('ags_e', other, name: 'sweeper'));
      deletes.requests.add(other);
      deletes.version++;
      await t.pump(const Duration(seconds: 2));
      await t.pumpAndSettle();
      expect(find.byType(SnackBar), findsOneWidget);
      expect(find.text('sweeper wants to delete Ideas.md'), findsOneWidget);
      expect(find.text('tidier wants to delete Ideas.md'), findsNothing);
      await disposeShell(t, c);
    });

    testWidgets('a server before decision 149 is not asked again', (t) async {
      final agents = FakeAgentServer(hosts: [agentHost()]);
      final c = shellContainer(agentClient: agents.client);
      await pumpShell(t, c, size: const Size(1400, 900));
      await t.pump(const Duration(seconds: 10));
      await t.pumpAndSettle();
      expect(agents.count('GET', '/v1/agent/delete-requests'), 1);
      await disposeShell(t, c);
    });
  });

  group('§7: the phone', () {
    testWidgets('a compact strip under the header; Review raises the sheet '
        'with Move to Trash, Decline and Later stacked', (t) async {
      final (c, deletes) = await _open(
        t,
        size: const Size(390, 844),
        at: Routes.agentSession('ags_d'),
      );
      expect(find.byType(AlertDialog), findsNothing);
      final compact = find.byKey(const Key('delete-request-compact'));
      expect(compact, findsOneWidget);
      expect(
        find.descendant(of: compact, matching: find.text('NEEDS YOU')),
        findsOneWidget,
      );
      expect(
        find.descendant(of: compact, matching: find.text('Delete Ideas.md?')),
        findsOneWidget,
      );
      expect(_strip(), findsNothing, reason: 'the full strip is the sheet');

      await t.tap(find.byKey(const Key('decision-review')));
      await t.pumpAndSettle();
      expect(find.byType(AlertDialog), findsNothing);
      expect(find.byType(BottomSheet), findsOneWidget);
      expect(find.text('tidier wants to delete Ideas.md'), findsOneWidget);
      expect(find.text('CLAUDE CODE SAYS'), findsOneWidget);
      expect(find.text(_reason), findsOneWidget, reason: 'a quote block');
      expect(find.text('WHAT'), findsOneWidget);
      expect(
        find.text('It moves to Primary’s Trash and is kept for 30 days.'),
        findsOneWidget,
      );
      final ys = [
        for (final k in ['approve', 'decline', 'later'])
          t.getCenter(find.byKey(Key('delete-request-$k'))).dy,
      ];
      expect(ys[0], lessThan(ys[1]));
      expect(ys[1], lessThan(ys[2]));
      final width = t.getSize(find.byKey(const Key('delete-request-approve')));
      expect(
        t.getSize(find.byKey(const Key('delete-request-later'))).width,
        width.width,
        reason: 'full width, all three',
      );

      await t.tap(find.byKey(const Key('delete-request-later')));
      await t.pumpAndSettle();
      expect(find.byType(BottomSheet), findsNothing);
      expect(deletes.answers, isEmpty);
      expect(compact, findsOneWidget);

      await t.tap(find.byKey(const Key('decision-review')));
      await t.pumpAndSettle();
      await t.tap(find.byKey(const Key('delete-request-approve')));
      await t.pumpAndSettle();
      expect(deletes.answers, [
        {'decision': 'approve'},
      ]);
      expect(find.byType(BottomSheet), findsNothing);
      expect(compact, findsNothing);
      await disposeShell(t, c);
    });
  });

  test('the reason is said by the agent, or a child by its given name', () {
    const titled = DeleteAsker(
      sessionId: 'ags_a',
      name: 'Words-child agent test',
      provider: 'opencode',
    );
    expect(titled.speaker, 'OpenCode');
    const child = DeleteAsker(
      sessionId: 'ags_b',
      name: 'Words-child agent test',
      provider: 'claude-code',
      given: 'words-child',
    );
    expect(child.speaker, 'words-child');
    expect(
      DeleteAsker.fromJson(const {'title': 'A title', 'provider': ''}).speaker,
      'The agent',
    );
  });
}
