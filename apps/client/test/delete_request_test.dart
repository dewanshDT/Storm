import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:shared_preferences/shared_preferences.dart';

import 'package:storm/router.dart';
import 'package:storm/ui/controls.dart';

import 'agent_fakes.dart';
import 'shell_harness.dart';

const _reason =
    'Finished in 2024 - see [the plan](http://example.com) and **this**.';

Map<String, dynamic> _request({
  String id = 'adr_1',
  String session = 'ags_d',
  String kind = 'folder',
  String state = 'pending',
  String? decidedBy,
}) => {
  'id': id,
  'session_id': session,
  'vault_id': 'v1',
  'kind': kind,
  if (kind == 'note') 'note_id': 'n1',
  'path': kind == 'note' ? 'Ideas/Ideas.md' : 'Projects/Old',
  if (kind == 'note') 'title': 'Ideas',
  'reason': _reason,
  'notes': kind == 'note' ? 1 : 12,
  'size': 2048,
  'modified': '2026-10-29T09:30:00Z',
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

/// A server holding agents' deletions (decision 149): its list, every
/// answer it was sent, and what answered first.
http.Response _json(Object body, [int status = 200]) => http.Response.bytes(
  utf8.encode(jsonEncode(body)),
  status,
  headers: {'content-type': 'application/json; charset=utf-8'},
);

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
                'name': 'tidier',
                'provider': 'claude-code',
                'model': 'sonnet',
                'context': {
                  'vault_id': 'v1',
                  'note_id': 'n9',
                  'title': 'Spring cleaning',
                },
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
      version++;
      return _json(r);
    }
    return null;
  }

  /// Another device answers first.
  void answeredElsewhere(String by) {
    requests.first['state'] = 'deleted';
    requests.first['decided_by'] = by;
    version++;
  }
}

Future<(ProviderContainer, _Deletes)> _open(
  WidgetTester t, {
  List<Map<String, dynamic>>? requests,
  List<Map<String, dynamic>>? sessions,
  Size size = const Size(1400, 900),
}) async {
  final agents = FakeAgentServer(
    hosts: [agentHost()],
    sessions: sessions ?? [agentSession('ags_d', name: 'tidier')],
  );
  final deletes = _Deletes(agents, requests: requests);
  final c = shellContainer(agentClient: agents.client);
  await pumpShell(t, c, size: size);
  await t.pumpAndSettle();
  return (c, deletes);
}

Finder _dialog() => find.byKey(const Key('delete-request-dialog'));

/// Decision 149c in the client: the dialog that every deletion an agent
/// asks for waits on, on whichever screen is open, and its Needs you entry.
void main() {
  setUp(() => SharedPreferences.setMockInitialValues({}));

  group('A-149-4: the dialog', () {
    testWidgets('names the agent, the session, the target, its count and '
        'the reason as plain text', (t) async {
      final (c, _) = await _open(t);
      expect(_dialog(), findsOneWidget);
      final title = t.widget<Text>(
        find.byKey(const Key('delete-request-title')),
      );
      expect(
        title.textSpan!.toPlainText(),
        'Delete folder Projects/Old and its 12 notes?',
      );
      expect(
        (title.textSpan! as TextSpan).children![1].style!.fontStyle,
        FontStyle.italic,
        reason: 'the target is emphasised',
      );
      expect(find.text('tidier'), findsOneWidget);
      expect(find.text('Claude Code · sonnet'), findsOneWidget);
      expect(find.text('Started from Spring cleaning'), findsOneWidget);
      expect(
        find.byKey(const Key('delete-request-open-session')),
        findsOneWidget,
      );
      expect(find.text('TIDIER SAYS:'), findsOneWidget);
      expect(
        t.widget<Text>(find.byKey(const Key('delete-request-reason'))).data,
        _reason,
        reason: 'the agent’s words, unrendered',
      );
      expect(find.text(_reason), findsOneWidget);
      expect(find.text('12 notes'), findsOneWidget);
      for (var i = 0; i < 5; i++) {
        expect(find.text('Projects/Old/N0$i.md'), findsOneWidget);
      }
      expect(find.text('Projects/Old/N05.md'), findsNothing);
      await t.tap(find.byKey(const Key('delete-request-more')));
      await t.pumpAndSettle();
      expect(find.text('Projects/Old/N11.md'), findsOneWidget);
      expect(
        find.text('It goes to Trash, which keeps it for 30 days.'),
        findsOneWidget,
      );
      final approve = t.widget<StormButton>(
        find.byKey(const Key('delete-request-approve')),
      );
      expect(approve.kind, StormButtonKind.danger, reason: 'the main CTA');
      expect(approve.label, 'Approve');
      expect(find.byKey(const Key('delete-request-decline')), findsOneWidget);
      expect(
        find.textContaining(RegExp('always', caseSensitive: false)),
        findsNothing,
      );
      await disposeShell(t, c);
    });

    testWidgets('a note shows its path, size and last change', (t) async {
      final (c, _) = await _open(t, requests: [_request(kind: 'note')]);
      final title = t.widget<Text>(
        find.byKey(const Key('delete-request-title')),
      );
      expect(title.textSpan!.toPlainText(), 'Delete Ideas.md?');
      final what = t
          .widget<Text>(find.byKey(const Key('delete-request-what')))
          .data!;
      expect(what, startsWith('Ideas/Ideas.md · 2.0 KB · modified '));
      await disposeShell(t, c);
    });

    for (final (how, answer) in [
      ('Approve', 'approve'),
      ('Decline', 'decline'),
      ('Esc', 'decline'),
      ('dismissing it', 'decline'),
    ]) {
      testWidgets('$how answers $answer', (t) async {
        final (c, deletes) = await _open(t);
        switch (how) {
          case 'Approve':
            await t.tap(find.byKey(const Key('delete-request-approve')));
          case 'Decline':
            await t.tap(find.byKey(const Key('delete-request-decline')));
          case 'Esc':
            // ignore: avoid_print
            await t.sendKeyEvent(LogicalKeyboardKey.escape);
          default:
            await t.tapAt(const Offset(10, 10));
        }
        await t.pumpAndSettle();
        expect(_dialog(), findsNothing);
        expect(deletes.answers, [
          {'decision': answer},
        ]);
        await t.pump(const Duration(seconds: 2));
        await t.pumpAndSettle();
        expect(_dialog(), findsNothing, reason: 'asked once');
        await disposeShell(t, c);
      });
    }

    testWidgets('answered on another device first: this one closes and '
        'says where, sending nothing', (t) async {
      final (c, deletes) = await _open(t);
      expect(_dialog(), findsOneWidget);
      deletes.answeredElsewhere('Laptop');
      await t.pump(const Duration(seconds: 2));
      await t.pumpAndSettle();
      expect(_dialog(), findsNothing);
      expect(find.text('Answered on Laptop'), findsOneWidget);
      expect(deletes.answers, isEmpty);
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

  group('149c: Needs you', () {
    testWidgets('the session shows the request under Needs you, and Review '
        'opens the dialog', (t) async {
      final pending = _request(kind: 'note');
      final session = {
        ...agentSession('ags_d', name: 'tidier'),
        'needs_attention': 'delete_approval',
        'delete_request': pending,
      };
      final (c, deletes) = await _open(
        t,
        sessions: [session],
        size: const Size(411, 900),
      );
      await t.tap(find.byKey(const Key('delete-request-decline')));
      await t.pumpAndSettle();
      deletes.answers.clear();
      deletes.requests.first['state'] = 'pending';

      c.read(routerProvider).go(Routes.agents);
      await t.pumpAndSettle();
      expect(find.byKey(const Key('attention-delete_approval')), findsWidgets);
      expect(find.text('Needs you'), findsWidgets);
      c.read(routerProvider).go('/agents/s/ags_d');
      await t.pumpAndSettle();
      expect(find.byKey(const Key('delete-request-card')), findsOneWidget);
      await t.tap(find.byKey(const Key('delete-request-review')));
      await t.pumpAndSettle();
      expect(_dialog(), findsOneWidget);
      await t.tap(find.byKey(const Key('delete-request-approve')));
      await t.pumpAndSettle();
      expect(deletes.answers, [
        {'decision': 'approve'},
      ]);
      await disposeShell(t, c);
    });
  });
}
