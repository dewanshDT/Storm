import 'dart:convert';

import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart' show RenderParagraph;
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:shared_preferences/shared_preferences.dart';

import 'package:storm/agent/agent_models.dart';
import 'package:storm/agent/agent_widgets.dart';
import 'package:storm/agent/crew_widgets.dart';
import 'package:storm/router.dart';
import 'package:storm/ui/accents.dart';
import 'package:storm/ui/theme.dart';
import 'package:storm/ui/tokens.dart';

import 'agent_fakes.dart';
import 'real_fonts.dart';
import 'shell_harness.dart';
import 'spawn_fakes.dart';

/// Decision 138, slice 138-3: crews in the sidebar and on the overview, the
/// crew menu, and collapse kept per device (plan §4.1–§4.3, §6.3).
void main() {
  const phone = Size(411, 900);
  const wide = Size(1400, 900);
  const long = 'Install guide and relay docs refresh, Q4'; // 40 characters

  setUpAll(loadRealFonts);
  setUp(() => SharedPreferences.setMockInitialValues({}));

  /// Gateway release: a running and an ended member, out of crew order on
  /// purpose; one loose running session and one loose ended one.
  FakeAgentServer crewServer({String crewName = 'Gateway release'}) =>
      FakeAgentServer(
        hosts: [agentHost()],
        crews: [agentCrew('crw_1', name: crewName)],
        roles: [
          {
            'vault_id': 'v-kit',
            'note_id': 'k_rev',
            'title': 'Storm Reviewer',
            'path': 'agents/Storm Reviewer.md',
            'folder': 'agents',
            'summary': 'Checks finished work; writes nothing.',
          },
        ],
        sessions: [
          agentSession(
            'ags_notes',
            name: 'release-notes',
            status: 'completed',
            crew: ('crw_1', 0),
          ),
          agentSession(
            'ags_impl',
            name: 'gateway-impl',
            crew: ('crw_1', 1),
            role: roleRef('k_coder', 'Storm Coder'),
          ),
          agentSession('ags_loose', name: 'spike-sync'),
          agentSession('ags_old', name: 'lint-fix', status: 'failed'),
        ],
      );

  Future<ProviderContainer> open(
    WidgetTester tester,
    FakeAgentServer agents, {
    Size size = wide,
  }) async {
    final c = shellContainer(agentClient: agents.client);
    await pumpShell(tester, c, size: size);
    c.read(routerProvider).go(Routes.agents);
    await tester.pumpAndSettle();
    return c;
  }

  double top(WidgetTester t, Finder f) => t.getTopLeft(f).dy;

  group('the sidebar', () {
    testWidgets('CREWS, running before ended in each, then RUNNING and '
        'ENDED with only loose sessions', (t) async {
      final c = await open(t, crewServer());
      final crews = find.byKey(const Key('crews-section'));
      final header = find.byKey(const Key('crew-crw_1'));
      final impl = find.byKey(const Key('session-ags_impl'));
      final notes = find.byKey(const Key('session-ags_notes'));
      final loose = find.byKey(const Key('session-ags_loose'));
      final old = find.byKey(const Key('session-ags_old'));
      expect(top(t, crews), lessThan(top(t, header)));
      expect(top(t, header), lessThan(top(t, impl)));
      expect(top(t, impl), lessThan(top(t, notes)), reason: 'running first');
      expect(top(t, notes), lessThan(top(t, find.text('RUNNING'))));
      expect(top(t, find.text('RUNNING')), lessThan(top(t, loose)));
      expect(top(t, loose), lessThan(top(t, find.text('ENDED'))));
      expect(top(t, find.text('ENDED')), lessThan(top(t, old)));
      expect(t.getTopLeft(notes).dy - t.getBottomLeft(impl).dy, 2);
      expect(find.text('2 crew'), findsNothing);
      expect(
        t.widget<Text>(find.byKey(const Key('crew-count-crw_1'))).data,
        '1 running',
      );
      // Each session is listed once.
      expect(impl, findsOneWidget);
      await disposeShell(t, c);
    });

    testWidgets('a crew and the CREWS section collapse, and stay collapsed '
        'after a restart', (t) async {
      var c = await open(t, crewServer());
      await t.tap(find.byKey(const Key('crew-chevron-crw_1')));
      await t.pumpAndSettle();
      expect(find.byKey(const Key('session-ags_impl')), findsNothing);
      expect(
        t.widget<Text>(find.byKey(const Key('crew-count-crw_1'))).data,
        '1 · 1',
      );
      final prefs = await SharedPreferences.getInstance();
      expect(prefs.getStringList('storm.crews.collapsed.crews'), ['crw_1']);
      await disposeShell(t, c);

      c = await open(t, crewServer());
      expect(find.byKey(const Key('crew-crw_1')), findsOneWidget);
      expect(find.byKey(const Key('session-ags_impl')), findsNothing);

      await t.tap(find.byKey(const Key('crews-section')));
      await t.pumpAndSettle();
      expect(find.text('CREWS · 1'), findsOneWidget);
      expect(find.byKey(const Key('crew-crw_1')), findsNothing);
      await disposeShell(t, c);

      c = await open(t, crewServer());
      expect(find.text('CREWS · 1'), findsOneWidget);
      await disposeShell(t, c);
    });

    testWidgets('no crews, or a server before crews: the lists as before', (
      t,
    ) async {
      for (final agents in [
        FakeAgentServer(
          hosts: [agentHost()],
          sessions: [agentSession('ags_1')],
        ),
        crewServer()..crewsSupported = false,
      ]) {
        final c = await open(t, agents);
        expect(find.byKey(const Key('crews-section')), findsNothing);
        expect(find.text('RUNNING'), findsOneWidget);
        expect(find.text('WORK'), findsOneWidget);
        expect(find.text('CREWS'), findsNothing);
        await disposeShell(t, c);
      }
    });

    testWidgets('a 40-character name ellipsizes in the header', (t) async {
      final c = await open(t, crewServer(crewName: long));
      final name = t.renderObject<RenderParagraph>(
        find.byKey(const Key('crew-name-crw_1')),
      );
      expect(name.didExceedMaxLines, isTrue);
      // The count is never what gives way.
      expect(
        t
            .renderObject<RenderParagraph>(
              find.byKey(const Key('crew-count-crw_1')),
            )
            .didExceedMaxLines,
        isFalse,
      );
      await disposeShell(t, c);
    });
  });

  group('the overview', () {
    testWidgets('CREWS cards, then NOT IN A CREW with only loose sessions', (
      t,
    ) async {
      final c = await open(t, crewServer());
      final card = find.byKey(const Key('crew-card-crw_1'));
      expect(card, findsOneWidget);
      expect(top(t, find.text('CREWS').last), lessThan(top(t, card)));
      expect(top(t, card), lessThan(top(t, find.text('NOT IN A CREW'))));
      expect(find.text('1 running · 1 ended'), findsOneWidget);
      expect(find.byKey(const Key('crew-row-ags_impl')), findsOneWidget);
      expect(find.byKey(const Key('work-ags_impl')), findsNothing);
      expect(find.byKey(const Key('work-ags_loose')), findsOneWidget);
      expect(find.byKey(const Key('work-ags_old')), findsOneWidget);
      expect(find.text('START AN AGENT'), findsOneWidget);
      expect(find.text('WORK'), findsNothing);
      // Marks: 32 in the sidebar, 28 on the card.
      expect(
        {
          for (final e in find.byKey(const Key('mark-ags_impl')).evaluate())
            (e.renderObject! as RenderBox).size.width,
        },
        {32, 28},
      );

      await t.tap(find.byKey(const Key('crew-row-ags_impl')));
      await t.pumpAndSettle();
      expect(
        c.read(routerProvider).state.uri.path,
        Routes.agentSession('ags_impl'),
      );
      await disposeShell(t, c);
    });

    testWidgets('a collapsed crew shows stacked marks and its names', (
      t,
    ) async {
      SharedPreferences.setMockInitialValues({
        'storm.crews.collapsed.crews': ['crw_1'],
      });
      final c = await open(t, crewServer());
      expect(find.byKey(const Key('crew-row-ags_impl')), findsNothing);
      expect(
        find.text('Collapsed · gateway-impl, release-notes'),
        findsOneWidget,
      );
      await disposeShell(t, c);
    });

    testWidgets('a 40-character name fits on the card', (t) async {
      final c = await open(t, crewServer(crewName: long));
      final name = t.renderObject<RenderParagraph>(
        find.descendant(
          of: find.byKey(const Key('crew-card-header-crw_1')),
          matching: find.text(long),
        ),
      );
      expect(name.didExceedMaxLines, isFalse);
      await disposeShell(t, c);
    });
  });

  group('the crew menu', () {
    Future<void> rightClick(WidgetTester t, Finder f) async {
      await t.tap(f, buttons: kSecondaryButton);
      await t.pumpAndSettle();
    }

    testWidgets('right-click and the hover ··· open the same menu', (t) async {
      final c = await open(t, crewServer());
      expect(find.byKey(const Key('crew-menu-crw_1')), findsNothing);
      await rightClick(t, find.byKey(const Key('crew-crw_1')));
      expect(find.byKey(const Key('crew-menu')), findsOneWidget);
      await t.tapAt(const Offset(1300, 850));
      await t.pumpAndSettle();
      expect(find.byKey(const Key('crew-menu')), findsNothing);

      final mouse = await t.createGesture(kind: PointerDeviceKind.mouse);
      await mouse.addPointer(location: Offset.zero);
      addTearDown(mouse.removePointer);
      await mouse.moveTo(t.getCenter(find.byKey(const Key('crew-crw_1'))));
      await t.pumpAndSettle();
      await t.tap(find.byKey(const Key('crew-menu-crw_1')));
      await t.pumpAndSettle();
      expect(find.byKey(const Key('crew-menu')), findsOneWidget);
      for (final label in [
        'Open crew page',
        'Rename',
        'Colour',
        'Collapse',
        'New session in crew',
        'Dismiss ended',
        'End all running…',
      ]) {
        expect(
          find.descendant(
            of: find.byKey(const Key('crew-menu')),
            matching: find.text(label),
          ),
          findsOneWidget,
          reason: label,
        );
      }
      await disposeShell(t, c);
    });

    testWidgets('Rename and Colour change the crew on the server', (t) async {
      final agents = crewServer();
      final c = await open(t, agents);
      await rightClick(t, find.byKey(const Key('crew-crw_1')));
      await t.tap(find.byKey(const Key('crew-action-rename')));
      await t.pumpAndSettle();
      await t.enterText(find.byKey(const Key('prompt-field')), 'Gateway 0.14');
      await t.tap(find.byKey(const Key('prompt-ok')));
      await t.pumpAndSettle();
      expect(agents.crew('crw_1')!['name'], 'Gateway 0.14');
      expect(
        find.descendant(
          of: find.byKey(const Key('crew-crw_1')),
          matching: find.text('Gateway 0.14'),
        ),
        findsOneWidget,
      );

      await rightClick(t, find.byKey(const Key('crew-crw_1')));
      await t.tap(find.byKey(const Key('crew-swatch-sky')));
      await t.pumpAndSettle();
      expect(agents.crew('crw_1')!['color'], 'sky');
      final patches = [
        for (final r in agents.requests)
          if (r.method == 'PATCH') jsonDecode(r.body),
      ];
      expect(patches, [
        {'name': 'Gateway 0.14'},
        {'color': 'sky'},
      ]);
      await disposeShell(t, c);
    });

    testWidgets('a refused rename says why', (t) async {
      final agents = crewServer()..refuseCrewChange = (404, 'no such crew');
      final c = await open(t, agents);
      await rightClick(t, find.byKey(const Key('crew-crw_1')));
      await t.tap(find.byKey(const Key('crew-action-rename')));
      await t.pumpAndSettle();
      await t.enterText(find.byKey(const Key('prompt-field')), 'Other');
      await t.tap(find.byKey(const Key('prompt-ok')));
      await t.pumpAndSettle();
      expect(
        find.textContaining("Couldn't rename Gateway release"),
        findsOneWidget,
      );
      expect(agents.crew('crw_1')!['name'], 'Gateway release');
      await disposeShell(t, c);
    });

    testWidgets('Collapse, and Dismiss ended (n)', (t) async {
      final agents = crewServer();
      final c = await open(t, agents);
      await rightClick(t, find.byKey(const Key('crew-crw_1')));
      await t.tap(find.byKey(const Key('crew-action-collapse')));
      await t.pumpAndSettle();
      expect(find.byKey(const Key('session-ags_impl')), findsNothing);

      await rightClick(t, find.byKey(const Key('crew-crw_1')));
      expect(find.text('Expand'), findsOneWidget);
      await t.tap(find.byKey(const Key('crew-action-dismissEnded')));
      await t.pumpAndSettle();
      expect(agents.session('ags_notes'), isNull);
      expect(agents.session('ags_impl'), isNotNull);
      expect(agents.session('ags_old'), isNotNull, reason: 'not in the crew');
      expect(agents.crew('crw_1'), isNotNull);
      await disposeShell(t, c);
    });

    testWidgets('dismissing the last members removes the crew and says so', (
      t,
    ) async {
      final agents = crewServer();
      agents.session('ags_impl')!['status'] = 'stopped';
      final c = await open(t, agents);
      await rightClick(t, find.byKey(const Key('crew-crw_1')));
      await t.tap(find.byKey(const Key('crew-action-dismissEnded')));
      await t.pumpAndSettle();
      expect(agents.crews, isEmpty);
      expect(
        find.text(
          'Gateway release was removed: its last session was dismissed.',
        ),
        findsOneWidget,
      );
      expect(find.byKey(const Key('crew-crw_1')), findsNothing);
      await disposeShell(t, c);
    });

    List<http.Request> dismissals(FakeAgentServer a) => [
      for (final r in a.requests)
        if (r.method == 'DELETE' && r.url.pathSegments[2] == 'sessions') r,
    ];

    testWidgets('Dismiss ended skips spawned trees and says so', (t) async {
      Map<String, dynamic> inCrew(
        Map<String, dynamic> s,
        String crew,
        int order,
      ) => s..['crew'] = {'id': crew, 'order': order};
      final agents = FakeAgentServer(
        hosts: [agentHost()],
        crews: [
          agentCrew('crw_1'),
          agentCrew('crw_2', name: 'Relay hardening', order: 1),
        ],
        sessions: [
          inCrew(
            spawnSession('ags_a', name: 'notes-a', status: 'completed'),
            'crw_1',
            0,
          ),
          inCrew(
            spawnSession('ags_b', name: 'notes-b', status: 'failed'),
            'crw_1',
            1,
          ),
          inCrew(
            spawnSession('ags_root', name: 'root', status: 'stopped'),
            'crw_1',
            2,
          ),
          inCrew(
            spawnSession(
              'ags_kid',
              name: 'kid',
              status: 'completed',
              parent: 'ags_root',
              depth: 1,
            ),
            'crw_1',
            3,
          ),
          inCrew(
            spawnSession('ags_far', name: 'far-root', status: 'stopped'),
            'crw_2',
            0,
          ),
          inCrew(
            spawnSession(
              'ags_stray',
              name: 'stray',
              status: 'completed',
              parent: 'ags_far',
              depth: 1,
            ),
            'crw_1',
            4,
          ),
        ],
      );
      FakeSpawning(agents);
      final c = await open(t, agents);
      await rightClick(t, find.byKey(const Key('crew-crw_1')));
      expect(
        find.descendant(
          of: find.byKey(const Key('crew-action-dismissEnded')),
          matching: find.text('2'),
        ),
        findsOneWidget,
      );
      await t.tap(find.byKey(const Key('crew-action-dismissEnded')));
      await t.pumpAndSettle();
      final sent = dismissals(agents);
      expect([for (final r in sent) r.url.pathSegments[3]], ['ags_a', 'ags_b']);
      for (final r in sent) {
        expect(r.url.queryParameters['tree'], isNull);
      }
      for (final id in ['ags_root', 'ags_kid', 'ags_far', 'ags_stray']) {
        expect(agents.session(id), isNotNull, reason: id);
      }
      expect(
        find.text(
          '3 are in a spawned tree and were kept: dismiss a tree from its '
          'session.',
        ),
        findsOneWidget,
      );
      expect(find.textContaining('confirm_tree'), findsNothing);
      expect(find.textContaining('tree_live'), findsNothing);
      expect(agents.crew('crw_1'), isNotNull);
      await disposeShell(t, c);
    });

    testWidgets('Dismiss ended with only standalone sessions: each, and the '
        'emptied crew offered back', (t) async {
      final agents = crewServer();
      agents.session('ags_impl')!['status'] = 'stopped';
      FakeSpawning(agents);
      final c = await open(t, agents);
      await rightClick(t, find.byKey(const Key('crew-crw_1')));
      await t.tap(find.byKey(const Key('crew-action-dismissEnded')));
      await t.pumpAndSettle();
      expect(
        [for (final r in dismissals(agents)) r.url.pathSegments[3]],
        ['ags_notes', 'ags_impl'],
      );
      expect(agents.crews, isEmpty);
      expect(
        find.text(
          'Gateway release was removed: its last session was dismissed.',
        ),
        findsOneWidget,
      );
      expect(find.textContaining('spawned tree'), findsNothing);
      await t.tap(find.text('Undo'));
      await t.pumpAndSettle();
      expect(agents.crews.single['name'], 'Gateway release');
      await disposeShell(t, c);
    });
  });

  group('both sides of 900', () {
    testWidgets('the phone list has crews and roles too (138-5), with no '
        'sidebar', (t) async {
      final c = await open(t, crewServer(), size: phone);
      expect(find.byKey(const Key('crews-section')), findsOneWidget);
      expect(find.byKey(const Key('crew-card-crw_1')), findsNothing);
      expect(find.byKey(const Key('agents-overview')), findsNothing);
      expect(find.byKey(const Key('session-ags_impl')), findsOneWidget);
      expect(find.byKey(const Key('role-tag-k_coder')), findsOneWidget);
      expect(find.text('RUNNING'), findsOneWidget);
      await disposeShell(t, c);
    });

    testWidgets('and at desk width the same session shows its role', (t) async {
      final c = await open(t, crewServer());
      expect(
        find.descendant(
          of: find.byKey(const Key('session-ags_impl')),
          matching: find.text('storm coder'),
        ),
        findsOneWidget,
      );
      await disposeShell(t, c);
    });
  });

  group('the widgets', () {
    Future<void> pumpIn(
      WidgetTester t,
      Widget child, {
      StormPreset preset = StormPreset.stormDark,
      double width = 400,
    }) async {
      await t.pumpWidget(
        MaterialApp(
          theme: StormTheme.from(preset),
          home: Scaffold(
            body: Center(
              child: SizedBox(width: width, child: child),
            ),
          ),
        ),
      );
      // A new theme animates in.
      await t.pumpAndSettle();
    }

    testWidgets('nine swatches in each theme, each its Accent.tile', (t) async {
      final seen = <String, Set<int>>{};
      for (final preset in [
        StormPreset.stormDark,
        StormPreset.stormLight,
        StormPreset.slowflowEarth,
      ]) {
        final tokens = StormTokens.from(preset);
        await pumpIn(
          t,
          CrewSwatches(selected: Accent.sage, onSelected: (_) {}),
          preset: preset,
        );
        final swatches = find.byWidgetPredicate(
          (w) =>
              w.key is ValueKey<String> &&
              (w.key! as ValueKey<String>).value.startsWith('crew-swatch-'),
        );
        expect(swatches, findsNWidgets(9), reason: preset.label);
        for (final a in kCrewPalette) {
          final box =
              t
                      .widget<Container>(
                        find.byKey(Key('crew-swatch-${a.name}')),
                      )
                      .decoration!
                  as BoxDecoration;
          expect(box.color, a.tile(tokens), reason: '${a.name} $preset');
          (seen[a.name] ??= {}).add(box.color!.toARGB32());
        }
      }
      for (final MapEntry(:key, :value) in seen.entries) {
        expect(value.length, 3, reason: '$key is one colour in every theme');
      }
    });

    testWidgets('a 40-character name ellipsizes in the chip', (t) async {
      await pumpIn(
        t,
        const Align(
          alignment: Alignment.centerLeft,
          child: CrewChip(
            crew: Crew(id: 'crw_1', name: long, color: 'sky', order: 0),
          ),
        ),
        width: 600,
      );
      final text = t.renderObject<RenderParagraph>(
        find.byKey(const Key('crew-chip-crw_1')),
      );
      expect(text.didExceedMaxLines, isTrue);
      expect(t.getSize(find.byKey(const Key('crew-chip-crw_1'))).width, 220);
    });

    testWidgets('the sub-line cuts the workspace, never the host or age', (
      t,
    ) async {
      final s = AgentSession.fromJson(
        agentSession(
          'ags_1',
          workspace: 'relay-bench-with-a-long-name',
          status: 'failed',
        ),
      );
      final style = const TextStyle(fontSize: 11);
      await pumpIn(
        t,
        SessionSubLine(session: s, host: 'mac-mini', style: style),
        width: 220,
      );
      final text = t.widget<Text>(find.byType(Text)).data!;
      expect(text, startsWith('mac-mini · relay'));
      expect(text, endsWith('… · failed'));
      await pumpIn(
        t,
        SessionSubLine(session: s, host: 'mac-mini', style: style),
        width: 900,
      );
      expect(
        find.text('mac-mini · relay-bench-with-a-long-name · failed'),
        findsOneWidget,
      );
      // Too narrow for even "…": the workspace goes, host and age stay.
      await pumpIn(
        t,
        SessionSubLine(session: s, host: 'mac-mini', style: style),
        width: 95,
      );
      expect(find.text('mac-mini · failed'), findsOneWidget);
    });
  });

  group('the models', () {
    test('crew, role and launch role parse, and are absent from an older '
        'server', () {
      final s = AgentSession.fromJson(
        agentSession(
          'ags_1',
          crew: ('crw_1', 3),
          role: roleRef('k_rev', null),
          launchRole: roleRef('k_rev', 'Storm Reviewer'),
        ),
      );
      expect(s.crew!.id, 'crw_1');
      expect(s.crew!.order, 3);
      expect(s.role!.deleted, isTrue);
      expect(s.launchRole!.title, 'Storm Reviewer');
      expect(s.withLive(s).role!.noteId, 'k_rev');

      final old = AgentSession.fromJson(agentSession('ags_2'));
      expect(old.crew, isNull);
      expect(old.role, isNull);
      expect(old.launchRole, isNull);

      expect(
        Crew.fromJson(agentCrew('crw_1', color: 'mint')).accent,
        Accent.mint,
      );
      expect(
        Crew.fromJson(agentCrew('crw_1', color: 'chartreuse')).accent,
        Accent.none,
      );
    });
  });
}
