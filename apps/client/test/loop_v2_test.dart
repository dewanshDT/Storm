import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'package:storm/router.dart';
import 'package:storm/state/agent_writes.dart';
import 'package:storm/ui/shell/sidebar_rows.dart';

import 'agent_fakes.dart';
import 'fake_server.dart';
import 'shell_harness.dart';

/// Storm v2 slice 8, the loop (handoff §4): provenance on the version line,
/// unseen dots, "by this session" in the panel, and the way back.
void main() {
  const phone = Size(411, 900);
  const desk = Size(1280, 900);
  const primary = FakeServer.primaryVault;

  // A device that has already looked at the primary vault's agent writes
  // once, so the writes these tests make are new to it.
  setUp(
    () => SharedPreferences.setMockInitialValues({
      SeenVersions.key: '{"$primary/":1}',
    }),
  );

  Uri where(ProviderContainer c) => c.read(routerProvider).state.uri;

  Future<ProviderContainer> at(
    WidgetTester tester,
    String location, {
    Size size = desk,
    FakeAgentServer? agents,
    void Function(FakeServer)? seed,
  }) async {
    final c = shellContainer(agentClient: agents?.client);
    seed?.call(serverOf(c));
    await pumpShell(tester, c, size: size);
    c.read(routerProvider).go(location);
    await tester.pumpAndSettle();
    return c;
  }

  bool dotOn(String key) => find
      .descendant(
        of: find.byKey(ValueKey(key)),
        matching: find.byType(UnseenDot),
      )
      .evaluate()
      .isNotEmpty;

  group('provenance on the version line', () {
    for (final size in [desk, phone]) {
      final side = size == desk ? 'desk' : 'phone';

      testWidgets('$side: an edit names its session and opens its Wrote', (
        tester,
      ) async {
        final c = await at(
          tester,
          Routes.note(primary, 'n1'),
          size: size,
          seed: (s) => s.agentWrote('n1', sessionId: 'ags_1', version: 1),
        );
        expect(
          find.text('Edited by session gateway-spec, just now ›'),
          findsOneWidget,
        );
        await tester.tap(find.byKey(const Key('provenance-link')));
        await tester.pumpAndSettle();
        expect(where(c).path, '/agents/s/ags_1');
        expect(where(c).queryParameters['tab'], 'wrote');
        await disposeShell(tester, c);
      });
    }

    testWidgets('a note the session made says Created', (tester) async {
      final c = await at(
        tester,
        Routes.note(primary, 'n1'),
        seed: (s) => s.agentWrote(
          'n1',
          sessionId: 'ags_1',
          version: 1,
          kind: 'created',
          sessionName: 'test-sweep',
          at: DateTime.now()
              .subtract(const Duration(hours: 2))
              .toUtc()
              .toIso8601String(),
        ),
      );
      expect(
        find.text('Created by session test-sweep, 2h ago ›'),
        findsOneWidget,
      );
      await disposeShell(tester, c);
    });

    testWidgets('a dismissed session keeps its name, as plain text', (
      tester,
    ) async {
      final c = await at(
        tester,
        Routes.note(primary, 'n1'),
        seed: (s) => s.agentWrote(
          'n1',
          sessionId: 'ags_gone',
          version: 1,
          dismissed: true,
        ),
      );
      expect(
        find.text('Edited by session gateway-spec, just now'),
        findsOneWidget,
      );
      expect(find.byKey(const Key('provenance-link')), findsNothing);
      await disposeShell(tester, c);
    });

    testWidgets('a later human edit does not clear it (Q13)', (tester) async {
      final c = await at(
        tester,
        Routes.note(primary, 'n1'),
        seed: (s) {
          s.notes['n1'] = s.notes['n1']!.copyWith(version: 3);
          s.agentWrote('n1', sessionId: 'ags_1', version: 2);
        },
      );
      expect(find.byKey(const Key('provenance-link')), findsOneWidget);
      await disposeShell(tester, c);
    });

    testWidgets('a note no agent wrote has none, and asks for none', (
      tester,
    ) async {
      final c = await at(tester, Routes.note(primary, 'n1'));
      expect(find.byKey(const Key('provenance')), findsNothing);
      await disposeShell(tester, c);
    });
  });

  group('unseen dots', () {
    testWidgets('desk: a collapsed folder rolls the dot up; opening the note '
        'clears it', (tester) async {
      final c = await at(
        tester,
        Routes.browse(primary),
        seed: (s) => s.agentWrote('n3', sessionId: 'ags_1', version: 1),
      );
      expect(dotOn('folder:Projects'), isTrue);
      expect(dotOn('folder:Daily'), isFalse);
      expect(dotOn('note:n0'), isFalse);

      await tester.tap(find.byKey(const ValueKey('folder:Projects')));
      await tester.pumpAndSettle();
      expect(dotOn('folder:Projects'), isFalse, reason: 'open, so not rolled');
      expect(dotOn('folder:Projects/Storm'), isTrue);
      await tester.tap(find.byKey(const ValueKey('folder:Projects/Storm')));
      await tester.pumpAndSettle();
      expect(dotOn('note:n3'), isTrue);
      expect(dotOn('note:n4'), isFalse);

      await tester.tap(find.byKey(const ValueKey('note:n3')));
      await tester.pumpAndSettle();
      expect(dotOn('note:n3'), isFalse);
      final prefs = await SharedPreferences.getInstance();
      expect(prefs.getString(SeenVersions.key), contains('"$primary/n3":1'));
      await disposeShell(tester, c);
    });

    testWidgets('phone: folder and note rows carry it', (tester) async {
      final c = await at(
        tester,
        Routes.browse(primary),
        size: phone,
        seed: (s) => s.agentWrote('n3', sessionId: 'ags_1', version: 1),
      );
      expect(dotOn('folder:Projects'), isTrue);
      expect(dotOn('folder:Daily'), isFalse);
      c.read(routerProvider).go(Routes.folder(primary, 'Projects/Storm'));
      await tester.pumpAndSettle();
      expect(dotOn('note:n3'), isTrue);
      await disposeShell(tester, c);
    });

    testWidgets('a note opened here at the agent’s version has no dot; a '
        'newer agent write brings it back', (tester) async {
      SharedPreferences.setMockInitialValues({
        SeenVersions.key: '{"$primary/":1,"$primary/n0":2}',
      });
      final c = await at(
        tester,
        Routes.browse(primary),
        seed: (s) => s.agentWrote('n0', sessionId: 'ags_1', version: 2),
      );
      expect(dotOn('note:n0'), isFalse, reason: 'seen on this device');

      serverOf(c).agentWrote('n0', sessionId: 'ags_1', version: 3);
      c.invalidate(agentWritesProvider);
      await tester.pumpAndSettle();
      expect(dotOn('note:n0'), isTrue);
      await disposeShell(tester, c);
    });

    testWidgets('a fresh device baselines: no dots for what agents wrote '
        'before it looked, a dot for a later write', (tester) async {
      SharedPreferences.setMockInitialValues({});
      final c = await at(
        tester,
        Routes.browse(primary),
        seed: (s) {
          s.agentWrote('n0', sessionId: 'ags_1', version: 2);
          s.agentWrote('n3', sessionId: 'ags_1', version: 1);
        },
      );
      expect(dotOn('note:n0'), isFalse, reason: 'written before this device');
      expect(dotOn('folder:Projects'), isFalse);
      final prefs = await SharedPreferences.getInstance();
      expect(prefs.getString(SeenVersions.key), contains('"$primary/":1'));

      serverOf(c).agentWrote('n3', sessionId: 'ags_1', version: 2);
      c.invalidate(agentWritesProvider);
      await tester.pumpAndSettle();
      expect(dotOn('folder:Projects'), isTrue, reason: 'a later agent write');
      expect(dotOn('note:n0'), isFalse);
      await disposeShell(tester, c);
    });

    testWidgets('the baseline is taken once, and survives a restart', (
      tester,
    ) async {
      SharedPreferences.setMockInitialValues({});
      var c = await at(tester, Routes.browse(primary));
      await disposeShell(tester, c);

      c = await at(
        tester,
        Routes.browse(primary),
        seed: (s) => s.agentWrote('n0', sessionId: 'ags_1', version: 1),
      );
      expect(dotOn('note:n0'), isTrue, reason: 'written after the baseline');
      await disposeShell(tester, c);
    });

    test('a long reading history never evicts a baseline', () async {
      SharedPreferences.setMockInitialValues({});
      final c = ProviderContainer();
      addTearDown(c.dispose);
      final seen = c.read(seenVersionsProvider.notifier);
      await c.read(seenVersionsProvider.future);
      await seen.baseline(primary, const {});
      for (var i = 0; i <= SeenVersions.limit; i++) {
        await seen.markSeen(primary, 'n$i', 1);
      }
      final map = c.read(seenVersionsProvider).value!;
      expect(map.containsKey('$primary/'), isTrue);
      expect(map.containsKey('$primary/n0'), isFalse, reason: 'oldest out');
      expect(map.length, SeenVersions.limit + 1);
    });

    testWidgets('what this device has seen survives a restart', (tester) async {
      var c = await at(
        tester,
        Routes.note(primary, 'n0'),
        seed: (s) => s.agentWrote('n0', sessionId: 'ags_1', version: 1),
      );
      await disposeShell(tester, c);

      c = await at(
        tester,
        Routes.browse(primary),
        seed: (s) => s.agentWrote('n0', sessionId: 'ags_1', version: 1),
      );
      expect(dotOn('note:n0'), isFalse);
      await disposeShell(tester, c);
    });
  });

  group('the session panel', () {
    Map<String, dynamic> design() => {
      'vault_id': primary,
      'note_id': 'n3',
      'title': 'Design',
    };

    testWidgets('the context note says when this session wrote it', (
      tester,
    ) async {
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        sessions: [agentSession('ags_1', context: design(), wroteCount: 1)],
      );
      final c = await at(
        tester,
        Routes.agentSession('ags_1'),
        agents: agents,
        seed: (s) =>
            s.agentWrote('n3', sessionId: 'ags_1', version: 1, kind: 'created'),
      );
      expect(find.text('v1 · created by this session'), findsOneWidget);
      await disposeShell(tester, c);
    });

    testWidgets('…and not when another session did', (tester) async {
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        sessions: [agentSession('ags_1', context: design())],
      );
      final c = await at(
        tester,
        Routes.agentSession('ags_1'),
        agents: agents,
        seed: (s) => s.agentWrote('n3', sessionId: 'ags_other', version: 1),
      );
      final line = tester.widget<Text>(
        find.byKey(const Key('panel-version-line')),
      );
      expect(line.data, 'v1');
      await disposeShell(tester, c);
    });

    testWidgets('a Wrote note opened in the panel says edited', (tester) async {
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        sessions: [agentSession('ags_1', wroteCount: 1)],
        writes: {
          'ags_1': [
            {
              'vault_id': primary,
              'note_id': 'n4',
              'title': 'Ideas',
              'path': 'Projects/Ideas.md',
              'kind': 'edited',
              'version': 1,
              'at': '2026-10-08T10:00:00Z',
            },
          ],
        },
      );
      final c = await at(
        tester,
        Routes.agentSession('ags_1', tab: 'wrote'),
        agents: agents,
        seed: (s) => s.agentWrote('n4', sessionId: 'ags_1', version: 1),
      );
      await tester.tap(find.text('Ideas'));
      await tester.pumpAndSettle();
      expect(find.text('v1 · edited by this session'), findsOneWidget);
      await disposeShell(tester, c);
    });

    testWidgets('About lists the integrations granted at launch', (
      tester,
    ) async {
      for (final size in [desk, phone]) {
        final agents = FakeAgentServer(
          hosts: [agentHost()],
          sessions: [
            agentSession(
              'ags_1',
              integrations: [
                {'id': 'mcc_1', 'slug': 'linear', 'display_name': 'Linear'},
              ],
            ),
          ],
        );
        final c = await at(
          tester,
          Routes.agentSession('ags_1', tab: 'about'),
          size: size,
          agents: agents,
        );
        if (size == phone) {
          await tester.tap(find.byKey(const Key('chip-details')));
          await tester.pumpAndSettle();
        }
        expect(
          find.text('Linear. Fixed when the session started.'),
          findsOneWidget,
        );
        await disposeShell(tester, c);
      }
    });

    testWidgets('phone: a pushed context note backs out as the session', (
      tester,
    ) async {
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        sessions: [agentSession('ags_1', context: design())],
      );
      final c = await at(
        tester,
        Routes.agentSession('ags_1'),
        size: phone,
        agents: agents,
      );
      await tester.tap(find.byKey(const Key('chip-context')));
      await tester.pumpAndSettle();
      expect(where(c).path, '/v/$primary/note/n3');
      expect(find.text('‹ gateway-spec'), findsOneWidget);
      await tester.tap(find.byKey(const Key('back-link')));
      await tester.pumpAndSettle();
      expect(where(c).path, '/agents/s/ags_1');
      await disposeShell(tester, c);
    });

    for (final width in [360.0, 390.0, 430.0, 860.0]) {
      testWidgets('phone ${width.toInt()}: the session name is never cut', (
        tester,
      ) async {
        final agents = FakeAgentServer(
          hosts: [agentHost()],
          sessions: [agentSession('ags_1', context: design())],
        );
        final c = await at(
          tester,
          Routes.note(primary, 'n3', session: 'ags_1'),
          size: Size(width, 844),
          agents: agents,
        );
        final link = tester.renderObject<RenderParagraph>(
          find.text('‹ gateway-spec'),
        );
        expect(link.didExceedMaxLines, isFalse);
        expect(link.size.width, closeTo(link.getMaxIntrinsicWidth(0), 0.5));
        expect(find.byTooltip('Properties'), findsOneWidget);
        final oneLine =
            tester.getTopLeft(find.byKey(const Key('start-session'))).dy <
            tester.getBottomLeft(find.byKey(const Key('back-link'))).dy;
        expect(oneLine, width > 800, reason: 'controls wrap only when cut');
        expect(tester.takeException(), isNull);
        await disposeShell(tester, c);
      });
    }

    testWidgets('desk: the same note keeps its crumb', (tester) async {
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        sessions: [agentSession('ags_1', context: design())],
      );
      final c = await at(
        tester,
        Routes.note(primary, 'n3', session: 'ags_1'),
        agents: agents,
      );
      expect(find.byKey(const Key('note-crumb')), findsOneWidget);
      expect(find.text('‹ gateway-spec'), findsNothing);
      await disposeShell(tester, c);
    });
  });

  testWidgets('the loop: note → session → the agent writes → Wrote, the dot '
      'and the provenance link → back to the session', (tester) async {
    final agents = FakeAgentServer(hosts: [agentHost()]);
    final c = await at(tester, Routes.note(primary, 'n0'), agents: agents);
    final server = serverOf(c);

    await tester.tap(find.byKey(const Key('start-session')));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('launch')));
    await tester.pumpAndSettle();
    final id = agents.sessions.single['id'] as String;
    expect(where(c).path, '/agents/s/$id');
    expect(find.text('Wrote 0'), findsOneWidget);

    // The agent edits Design through the gateway.
    server.notes['n3'] = server.notes['n3']!.copyWith(
      content: '# Design\n\nby the agent\n',
      version: 2,
    );
    server.agentWrote('n3', sessionId: id, version: 2, sessionName: 'launched');
    agents.session(id)!['wrote_count'] = 1;
    agents.writes[id] = [
      {
        'vault_id': primary,
        'note_id': 'n3',
        'title': 'Design',
        'path': 'Projects/Storm/Design.md',
        'kind': 'edited',
        'version': 2,
        'at': DateTime.now().toUtc().toIso8601String(),
      },
    ];
    final listed = agents.count('GET', '/v1/agent/sessions');
    await tester.pump(const Duration(seconds: 5));
    await tester.pumpAndSettle();
    expect(find.text('Wrote 1'), findsOneWidget);
    expect(
      agents.count('GET', '/v1/agent/sessions'),
      greaterThan(listed),
      reason: 'the lists follow the count without waiting for their poll',
    );

    c.read(routerProvider).go(Routes.browse(primary));
    await tester.pumpAndSettle();
    expect(dotOn('folder:Projects'), isTrue);

    c.read(routerProvider).go(Routes.note(primary, 'n3'));
    await tester.pumpAndSettle();
    expect(dotOn('note:n3'), isFalse, reason: 'opened, so seen');
    expect(find.text('Edited by session launched, just now ›'), findsOneWidget);

    await tester.tap(find.byKey(const Key('provenance-link')));
    await tester.pumpAndSettle();
    expect(where(c).path, '/agents/s/$id');
    expect(where(c).queryParameters['tab'], 'wrote');
    expect(find.text('Design'), findsWidgets);
    await disposeShell(tester, c);
  });
}
