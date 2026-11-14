import 'dart:ui' show Tristate;

import 'package:drift/drift.dart' show Value;
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'package:storm/cache/cache_db.dart';
import 'package:storm/keyboard/storm_activators.dart';
import 'package:storm/router.dart';
import 'package:storm/state/app_state.dart';
import 'package:storm/ui/palette/palette_model.dart';
import 'package:storm/ui/palette/palette_rows.dart';

import 'agent_fakes.dart';
import 'fake_server.dart';
import 'shell_harness.dart';

/// The palette (decision 156-2) in the real shell, driven with real key
/// events: where it opens and in which shape, its keys, its states.
void main() {
  setUp(() => SharedPreferences.setMockInitialValues({}));
  tearDown(() => debugDefaultTargetPlatformOverride = null);

  const at899 = Size(899, 900);
  const at900 = Size(900, 900);
  const v = FakeServer.primaryVault;

  final field = find.byKey(const Key('palette-field'));
  final dialog = find.byKey(const Key('palette-dialog'));
  final screen = find.byKey(const Key('palette-screen'));
  final launcher = find.byKey(const Key('launcher-chips'));

  Future<void> frames(WidgetTester t) async {
    for (var i = 0; i < 20; i++) {
      await t.pump(const Duration(milliseconds: 50));
    }
  }

  Future<void> press(
    WidgetTester t,
    LogicalKeyboardKey key, {
    bool mod = false,
    bool shift = false,
  }) async {
    final modifier = stormUsesMetaModifier
        ? LogicalKeyboardKey.metaLeft
        : LogicalKeyboardKey.controlLeft;
    if (mod) await t.sendKeyDownEvent(modifier);
    if (shift) await t.sendKeyDownEvent(LogicalKeyboardKey.shiftLeft);
    await t.sendKeyEvent(key);
    if (shift) await t.sendKeyUpEvent(LogicalKeyboardKey.shiftLeft);
    if (mod) await t.sendKeyUpEvent(modifier);
    await frames(t);
  }

  Future<void> palette(WidgetTester t, {bool commands = false}) => press(
    t,
    commands ? LogicalKeyboardKey.keyP : LogicalKeyboardKey.keyK,
    mod: true,
    shift: commands,
  );

  String query(WidgetTester t) => t.widget<TextField>(field).controller!.text;

  Future<void> type(WidgetTester t, String text) async {
    await t.enterText(field, text);
    await frames(t);
  }

  List<PaletteRowTile> rows(WidgetTester t) =>
      t.widgetList<PaletteRowTile>(find.byType(PaletteRowTile)).toList();

  String selected(WidgetTester t) =>
      rows(t).firstWhere((r) => r.selected).row.label;

  String location(ProviderContainer c) =>
      c.read(routerProvider).state.uri.toString();

  /// A shell with titled notes in two vaults, a host and two sessions, one
  /// of which needs you.
  Future<ProviderContainer> open(
    WidgetTester t,
    Size size,
    String route, {
    TargetPlatform platform = TargetPlatform.macOS,
    bool offline = false,
    void Function(FakeServer server)? seed,
  }) async {
    debugDefaultTargetPlatformOverride = platform;
    final agents = FakeAgentServer(
      hosts: [agentHost()],
      sessions: [
        agentSession('ags_1', name: 'gateway-impl'),
        {
          ...agentSession('ags_2', name: 'release-notes'),
          'needs_attention': 'question',
        },
      ],
    );
    final c = shellContainer(agentClient: agents.client);
    final server = serverOf(c);
    server.notes['d1'] = ServerNote(
      id: 'd1',
      path: 'Projects/Design Doc.md',
      title: 'Design Doc',
      content: '# Design Doc\n',
      version: 1,
    );
    server.addVault('v-work', 'Work')['w1'] = ServerNote(
      id: 'w1',
      path: 'Q4/Design Review.md',
      title: 'Design Review',
      content: '# Design Review\n',
      version: 1,
    );
    server.notes['cfg'] = ServerNote(
      id: 'cfg',
      path: '_storm/vault.md',
      title: 'Design config',
      content: '',
      version: 1,
    );
    server.tags[v] = {'design': 2};
    seed?.call(server);
    await pumpShellFrames(t, c, size: size);
    for (final step in [Routes.agents, '/settings/ai', route]) {
      c.read(routerProvider).go(step);
      await frames(t);
    }
    // A session autofocuses its terminal at the desk; these tests start
    // from the shell itself.
    FocusManager.instance.primaryFocus?.unfocus();
    await frames(t);
    if (offline) server.unreachable = true;
    return c;
  }

  Future<void> close(WidgetTester t, ProviderContainer c) async {
    await t.pumpWidget(const SizedBox());
    c.dispose();
    debugDefaultTargetPlatformOverride = null;
  }

  group('opens with ⌘K', () {
    final screens = [
      ('Notes', Routes.browse(v)),
      ('NotesHome', Routes.notes),
      ('Agents', Routes.agents),
      ('Settings', Routes.settingsPage('device')),
    ];
    for (final platform in [TargetPlatform.macOS, TargetPlatform.linux]) {
      for (final size in [at899, at900]) {
        for (final (name, route) in screens) {
          testWidgets('${platform.name} at ${size.width.toInt()} from $name', (
            t,
          ) async {
            final c = await open(t, size, route, platform: platform);
            final at = location(c);
            await palette(t);
            expect(field, findsOneWidget);
            if (size == at900) {
              expect(dialog, findsOneWidget);
              expect(t.getSize(dialog).width, 640);
            } else {
              expect(screen, findsOneWidget, reason: 'full screen below 900');
              expect(dialog, findsNothing);
            }
            await press(t, LogicalKeyboardKey.escape);
            expect(field, findsNothing);
            expect(location(c), at);
            await close(t, c);
          });
        }
      }
    }

    testWidgets('⌘P is the same palette, ⇧⌘P is command mode', (t) async {
      final c = await open(t, at900, Routes.agents);
      await press(t, LogicalKeyboardKey.keyP, mod: true);
      expect(query(t), '');
      await press(t, LogicalKeyboardKey.escape);
      await palette(t, commands: true);
      expect(query(t), '>');
      expect(rows(t), isNotEmpty);
      expect(rows(t).every((r) => r.row.target is CommandTarget), isTrue);
      await close(t, c);
    });

    testWidgets('with no active vault too', (t) async {
      debugDefaultTargetPlatformOverride = TargetPlatform.linux;
      final c = shellContainer(
        settings: const Settings(
          baseUrl: 'http://test',
          accessToken: 'sta_test',
        ),
      );
      serverOf(c).vaults.clear();
      await pumpShellFrames(t, c, size: at900);
      c.read(routerProvider).go(Routes.settingsPage('device'));
      await frames(t);
      await palette(t);
      expect(field, findsOneWidget);
      await type(t, 'nothing here');
      expect(find.textContaining('Create note'), findsNothing);
      await close(t, c);
    });
  });

  group('keys', () {
    testWidgets('↑/↓ move and wrap; Enter opens the note', (t) async {
      final c = await open(t, at900, Routes.agents);
      await palette(t);
      await type(t, 'design');
      expect(selected(t), 'Design Doc', reason: 'the active vault first');
      expect(
        rows(t).map((r) => r.row.label),
        isNot(contains('Design config')),
        reason: '_storm/ is never a result',
      );
      await press(t, LogicalKeyboardKey.arrowDown);
      expect(selected(t), 'Design Review');
      await press(t, LogicalKeyboardKey.arrowUp);
      await press(t, LogicalKeyboardKey.arrowUp);
      expect(selected(t), rows(t).last.row.label, reason: 'wraps');
      await press(t, LogicalKeyboardKey.arrowDown);
      expect(selected(t), 'Design Doc');
      await press(t, LogicalKeyboardKey.enter);
      expect(field, findsNothing);
      expect(location(c), Routes.note(v, 'd1'));
      await close(t, c);
    });

    testWidgets('a note in another vault opens there', (t) async {
      final c = await open(t, at900, Routes.agents);
      await palette(t);
      await type(t, 'design review');
      await press(t, LogicalKeyboardKey.enter);
      expect(location(c), Routes.note('v-work', 'w1'));
      await close(t, c);
    });

    testWidgets('the selection follows its row when the list updates', (
      t,
    ) async {
      final c = await open(t, at900, Routes.agents);
      await palette(t);
      await type(t, 'design');
      await press(t, LogicalKeyboardKey.arrowDown);
      expect(selected(t), 'Design Review');
      final before = rows(t).indexWhere((r) => r.selected);

      serverOf(c).notes['d0'] = ServerNote(
        id: 'd0',
        path: 'Design.md',
        title: 'Design',
        content: '',
        version: 1,
      );
      c.read(vaultRevisionProvider.notifier).state++;
      await frames(t);
      expect(rows(t).first.row.label, 'Design', reason: 'a new row above');
      expect(selected(t), 'Design Review');
      expect(rows(t).indexWhere((r) => r.selected), before + 1);
      await close(t, c);
    });

    testWidgets('Enter and the arrows wait while an IME composes', (t) async {
      final c = await open(t, at900, Routes.agents);
      await palette(t);
      await type(t, 'design');
      final controller = t.widget<TextField>(field).controller!;
      controller.value = const TextEditingValue(
        text: 'design',
        selection: TextSelection.collapsed(offset: 6),
        composing: TextRange(start: 0, end: 6),
      );
      await frames(t);
      await press(t, LogicalKeyboardKey.arrowDown);
      expect(selected(t), 'Design Doc');
      await press(t, LogicalKeyboardKey.enter);
      expect(field, findsOneWidget, reason: 'Enter commits the composition');
      expect(location(c), Routes.agents);

      controller.value = controller.value.copyWith(composing: TextRange.empty);
      await frames(t);
      await press(t, LogicalKeyboardKey.arrowDown);
      expect(selected(t), 'Design Review');
      await close(t, c);
    });

    testWidgets('Esc clears the query, then closes', (t) async {
      final c = await open(t, at900, Routes.agents);
      await palette(t);
      await type(t, 'design');
      await press(t, LogicalKeyboardKey.escape);
      expect(field, findsOneWidget);
      expect(query(t), '');
      await press(t, LogicalKeyboardKey.escape);
      expect(field, findsNothing);
      await close(t, c);
    });

    testWidgets('⌘Enter on a note opens the launcher with it as context', (
      t,
    ) async {
      final c = await open(t, at900, Routes.browse(v));
      await palette(t);
      await type(t, 'design doc');
      await press(t, LogicalKeyboardKey.enter, mod: true);
      expect(field, findsNothing);
      expect(launcher, findsOneWidget, reason: 'never a spawn: the launcher');
      expect(find.byKey(const Key('launcher-context')), findsOneWidget);
      expect(
        find.descendant(
          of: find.byKey(const Key('launcher-context')),
          matching: find.textContaining('Design Doc'),
        ),
        findsWidgets,
      );
      expect(location(c), Routes.browse(v));
      await close(t, c);
    });

    testWidgets('Ctrl+Enter is the same on Linux', (t) async {
      final c = await open(
        t,
        at900,
        Routes.browse(v),
        platform: TargetPlatform.linux,
      );
      await palette(t);
      await type(t, 'design doc');
      await press(t, LogicalKeyboardKey.enter, mod: true);
      expect(launcher, findsOneWidget);
      await close(t, c);
    });

    testWidgets('a session opens; a host goes to Agents & hosts', (t) async {
      final c = await open(t, at900, Routes.browse(v));
      await palette(t);
      await type(t, '@gateway');
      await press(t, LogicalKeyboardKey.enter);
      expect(location(c), startsWith(Routes.agentSession('ags_1')));

      await palette(t);
      await type(t, 'build-vm');
      final host = rows(t).indexWhere((r) => r.row.label == 'build-vm');
      for (var i = 0; i < host; i++) {
        await press(t, LogicalKeyboardKey.arrowDown);
      }
      expect(selected(t), 'build-vm');
      await press(t, LogicalKeyboardKey.enter);
      expect(location(c), Routes.settingsPage('hosts'));
      await close(t, c);
    });

    testWidgets('a command runs, with its chord shown', (t) async {
      final c = await open(t, at900, Routes.agents);
      await palette(t, commands: true);
      await type(t, '>toggle side');
      final row = rows(t).single;
      expect(row.row.hint, '⌘\\');
      expect(find.text('⌘\\'), findsOneWidget);
      await press(t, LogicalKeyboardKey.enter);
      expect(c.read(sidebarCollapsedProvider), isTrue);
      await close(t, c);
    });
  });

  group('create', () {
    testWidgets('the create row is pinned last and opens New note named', (
      t,
    ) async {
      final c = await open(t, at900, Routes.browse(v));
      await palette(t);
      await type(t, 'Design plan');
      expect(rows(t).last.row.label, 'Create note “Design plan” in Primary');
      await press(t, LogicalKeyboardKey.arrowUp);
      expect(selected(t), rows(t).last.row.label);
      await press(t, LogicalKeyboardKey.enter);
      expect(find.widgetWithText(AlertDialog, 'New note'), findsOneWidget);
      expect(find.widgetWithText(TextField, 'Design plan'), findsOneWidget);
      await close(t, c);
    });

    testWidgets('⇧Enter creates from the query whatever is selected', (
      t,
    ) async {
      final c = await open(t, at900, Routes.browse(v));
      await palette(t);
      await type(t, 'design');
      expect(selected(t), 'Design Doc');
      await press(t, LogicalKeyboardKey.enter, shift: true);
      expect(find.widgetWithText(AlertDialog, 'New note'), findsOneWidget);
      expect(find.widgetWithText(TextField, 'design'), findsOneWidget);
      await close(t, c);
    });

    testWidgets('an exact title leaves no create row', (t) async {
      final c = await open(t, at900, Routes.browse(v));
      await palette(t);
      await type(t, 'design doc');
      expect(find.textContaining('Create note'), findsNothing);
      await close(t, c);
    });
  });

  group('states', () {
    testWidgets('the empty query: needs you, recent, suggested', (t) async {
      final c = await open(
        t,
        at900,
        Routes.agents,
        seed: (s) => s.markOpened('v-work', 'w1', '2026-11-13T09:00:00Z'),
      );
      await palette(t);
      for (final heading in ['NEEDS YOU', 'RECENT', 'SUGGESTED']) {
        expect(find.text(heading), findsOneWidget, reason: heading);
      }
      expect(
        t.getTopLeft(find.text('NEEDS YOU')).dy,
        lessThan(t.getTopLeft(find.text('RECENT')).dy),
      );
      expect(
        t.getTopLeft(find.text('RECENT')).dy,
        lessThan(t.getTopLeft(find.text('SUGGESTED')).dy),
      );
      expect(selected(t), 'release-notes');
      expect(find.text('Design Review'), findsOneWidget);
      expect(find.text('New note'), findsOneWidget);
      await close(t, c);
    });

    testWidgets('offline searches this device, and says so', (t) async {
      final c = await open(t, at900, Routes.agents, offline: true);
      await c
          .read(cacheProvider)
          .putNote(
            CachedNotesCompanion.insert(
              vaultId: const Value(v),
              id: 'held',
              path: 'Held design.md',
              title: const Value('Held design'),
              version: 1,
              content: 'body',
              cachedAt: DateTime.now(),
            ),
          );
      await palette(t);
      await type(t, 'held');
      expect(find.text('Offline · searching this device'), findsOneWidget);
      expect(find.text('Held design'), findsOneWidget);
      await type(t, 'settings');
      expect(find.text('Offline · searching this device'), findsOneWidget);
      await close(t, c);
    });

    testWidgets('the result count is announced, politely', (t) async {
      final handle = t.ensureSemantics();
      final c = await open(t, at900, Routes.agents);
      await palette(t);
      await type(t, 'design review');
      await t.pump(const Duration(milliseconds: 500));
      final live = find.bySemanticsLabel(RegExp(r'^\d+ results?$'));
      expect(live, findsOneWidget);
      expect(t.getSemantics(live).flagsCollection.isLiveRegion, isTrue);
      final row = t.getSemantics(
        find.bySemanticsLabel(RegExp('^Design Review')),
      );
      expect(row.flagsCollection.isButton, isTrue);
      expect(row.flagsCollection.isSelected, Tristate.isTrue);
      expect(row.hint, isEmpty, reason: 'a note has no chord');
      handle.dispose();
      await close(t, c);
    });
  });

  group('focus', () {
    testWidgets('closing gives focus back to where it was', (t) async {
      final c = await open(t, at900, Routes.browse(v));
      c.read(routerProvider).go(Routes.note(v, 'd1'));
      await frames(t);
      await t.tap(find.byKey(const Key('mode-edit')));
      await frames(t);
      await t.tap(find.byKey(const Key('note-body')));
      await frames(t);
      final editing = FocusManager.instance.primaryFocus;
      expect(
        editing?.context?.findAncestorWidgetOfExactType<EditableText>(),
        isNotNull,
      );
      await palette(t);
      expect(FocusManager.instance.primaryFocus, isNot(editing));
      await press(t, LogicalKeyboardKey.escape);
      expect(field, findsNothing);
      expect(FocusManager.instance.primaryFocus, same(editing));
      await close(t, c);
    });

    testWidgets('after a jump, the chords still work at the destination', (
      t,
    ) async {
      final c = await open(t, at900, Routes.agents);
      await palette(t);
      await type(t, 'design doc');
      await press(t, LogicalKeyboardKey.enter);
      expect(location(c), Routes.note(v, 'd1'));
      await palette(t);
      expect(field, findsOneWidget);
      await press(t, LogicalKeyboardKey.escape);
      await close(t, c);
    });
  });
}
