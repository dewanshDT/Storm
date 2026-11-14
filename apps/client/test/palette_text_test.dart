import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'package:storm/keyboard/storm_activators.dart';
import 'package:storm/router.dart';
import 'package:storm/ui/palette/palette_model.dart';
import 'package:storm/ui/palette/palette_rows.dart';
import 'package:storm/ui/tokens.dart';

import 'fake_server.dart';
import 'shell_harness.dart';

/// The palette's In note text slot (decision 156l, m), the phone's ways in
/// (156n) and the retired search page (156o), in the real shell.
void main() {
  setUp(() => SharedPreferences.setMockInitialValues({}));
  tearDown(() => debugDefaultTargetPlatformOverride = null);

  const at900 = Size(900, 900);
  const phone = Size(411, 900);
  const v = FakeServer.primaryVault;

  final field = find.byKey(const Key('palette-field'));
  final screen = find.byKey(const Key('palette-screen'));
  final pending = find.byKey(const Key('palette-text-pending'));
  final heading = find.text('IN NOTE TEXT');

  Future<void> frames(WidgetTester t, [int n = 20]) async {
    for (var i = 0; i < n; i++) {
      await t.pump(const Duration(milliseconds: 50));
    }
  }

  Future<void> press(WidgetTester t, LogicalKeyboardKey key) async {
    await t.sendKeyEvent(key);
    await frames(t);
  }

  Future<void> palette(WidgetTester t) async {
    final modifier = stormUsesMetaModifier
        ? LogicalKeyboardKey.metaLeft
        : LogicalKeyboardKey.controlLeft;
    await t.sendKeyDownEvent(modifier);
    await t.sendKeyEvent(LogicalKeyboardKey.keyK);
    await t.sendKeyUpEvent(modifier);
    await frames(t);
  }

  List<PaletteRowTile> rows(WidgetTester t) =>
      t.widgetList<PaletteRowTile>(find.byType(PaletteRowTile)).toList();

  List<String> labels(WidgetTester t) => [for (final r in rows(t)) r.row.label];

  String selected(WidgetTester t) =>
      rows(t).firstWhere((r) => r.selected).row.label;

  List<String> lines(WidgetTester t) => [
    for (final l in t.widgetList<PaletteQuietLine>(
      find.byType(PaletteQuietLine),
    ))
      l.text,
  ];

  String location(ProviderContainer c) =>
      c.read(routerProvider).state.uri.toString();

  /// Titled notes in two vaults; "Meeting notes" has "design" only in its
  /// body.
  Future<ProviderContainer> open(
    WidgetTester t, {
    Size size = at900,
    String? route,
    bool offline = false,
    TargetPlatform platform = TargetPlatform.macOS,
    void Function(FakeServer server)? seed,
  }) async {
    debugDefaultTargetPlatformOverride = platform;
    final c = shellContainer();
    final server = serverOf(c);
    server.notes['d1'] = ServerNote(
      id: 'd1',
      path: 'Projects/Design Doc.md',
      title: 'Design Doc',
      content: '# Design Doc\n',
      version: 1,
    );
    server.notes['m1'] = ServerNote(
      id: 'm1',
      path: 'Meeting notes.md',
      title: 'Meeting notes',
      content: 'We went over the design system.',
      version: 1,
    );
    server.addVault('v-work', 'Work')['w1'] = ServerNote(
      id: 'w1',
      path: 'Q4/Design Review.md',
      title: 'Design Review',
      content: '# Design Review\n',
      version: 1,
    );
    seed?.call(server);
    await pumpShellFrames(t, c, size: size);
    c.read(routerProvider).go(route ?? Routes.agents);
    await frames(t);
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

  group('in note text', () {
    testWidgets('a skeleton while pending; hits land below the titles '
        'without moving the selection', (t) async {
      final hold = Completer<void>();
      final c = await open(
        t,
        seed: (s) => s.crossSearchHold = (_) => hold.future,
      );
      await palette(t);
      await t.enterText(field, 'design');
      await t.pump();
      expect(pending, findsOneWidget, reason: 'from the first keystroke');
      await frames(t);
      expect(pending, findsOneWidget, reason: 'while the request is out');
      expect(serverOf(c).crossSearches, ['design']);

      await press(t, LogicalKeyboardKey.arrowDown);
      expect(selected(t), 'Design Review');
      final before = labels(t);
      final at = rows(t).indexWhere((r) => r.selected);

      hold.complete();
      await frames(t);
      expect(pending, findsNothing);
      expect(heading, findsOneWidget);
      expect(selected(t), 'Design Review');
      expect(rows(t).indexWhere((r) => r.selected), at);
      final after = labels(t);
      expect(
        after.take(before.length - 1),
        before.take(before.length - 1),
        reason: 'the groups above are untouched; only the create row moves',
      );
      final text = [
        for (final r in rows(t))
          if (r.row.group == PaletteGroup.text) r.row,
      ];
      expect(text.map((r) => r.label), ['Meeting notes'], reason: 'deduped');
      expect(text.single.vault, 'Primary');
      expect(
        t.getTopLeft(find.text('Meeting notes')).dy,
        greaterThan(t.getTopLeft(find.text('Design Review')).dy),
      );

      final snippet = t.widget<PaletteSnippet>(find.byType(PaletteSnippet));
      expect(snippet.raw, 'We went over the <<design>> system.');
      final rich = t.widget<RichText>(
        find.descendant(
          of: find.byType(PaletteSnippet),
          matching: find.byType(RichText),
        ),
      );
      final spans = <TextSpan>[];
      rich.text.visitChildren((span) {
        if (span is TextSpan && span.text != null) spans.add(span);
        return true;
      });
      final tokens = t.element(field).tokens;
      final marked = spans.firstWhere((s) => s.text == 'design');
      expect(marked.style!.color, tokens.accent);
      expect(marked.style!.fontWeight, FontWeight.w600);

      // Down to it, then Enter opens the note.
      while (selected(t) != 'Meeting notes') {
        await press(t, LogicalKeyboardKey.arrowDown);
      }
      await press(t, LogicalKeyboardKey.enter);
      expect(field, findsNothing);
      expect(location(c), Routes.note(v, 'm1'));
      await close(t, c);
    });

    testWidgets('debounced: typing fast asks once', (t) async {
      final c = await open(t);
      await palette(t);
      for (final q in ['d', 'de', 'des', 'desi', 'design']) {
        await t.enterText(field, q);
        await t.pump(const Duration(milliseconds: 40));
      }
      await frames(t);
      expect(serverOf(c).crossSearches, ['design']);
      await close(t, c);
    });

    testWidgets('an answer to an older query is dropped', (t) async {
      final holds = <String, Completer<void>>{};
      final c = await open(
        t,
        seed: (s) {
          s.notes['z1'] = ServerNote(
            id: 'z1',
            path: 'Zeta.md',
            title: 'Zeta',
            content: 'a deal maker',
            version: 1,
          );
          s.crossSearchHold = (q) => (holds[q] = Completer<void>()).future;
        },
      );
      await palette(t);
      await t.enterText(field, 'de');
      await frames(t);
      await t.enterText(field, 'des');
      await frames(t);
      expect(serverOf(c).crossSearches, ['de', 'des']);

      holds['des']!.complete();
      await frames(t);
      expect(labels(t), contains('Meeting notes'));
      holds['de']!.complete();
      await frames(t);
      expect(labels(t), isNot(contains('Zeta')), reason: 'only "de" has it');
      expect(labels(t), contains('Meeting notes'));
      await close(t, c);
    });

    testWidgets('a prefix or one letter never asks the server', (t) async {
      final c = await open(t);
      await palette(t);
      for (final q in ['d', '>design', '@design', '#design']) {
        await t.enterText(field, q);
        await frames(t);
        expect(pending, findsNothing, reason: q);
      }
      expect(serverOf(c).crossSearches, isEmpty);
      await close(t, c);
    });

    testWidgets('a skipped vault is one line', (t) async {
      final c = await open(t, seed: (s) => s.searchSkipped.add('v-work'));
      await palette(t);
      await t.enterText(field, 'design');
      await frames(t);
      expect(labels(t), contains('Meeting notes'));
      expect(lines(t), ['Couldn’t search Work']);
      await close(t, c);
    });

    testWidgets('an older server (404) is searched vault by vault', (t) async {
      final c = await open(t, seed: (s) => s.noCrossSearch = true);
      await palette(t);
      await t.enterText(field, 'design');
      await frames(t);
      expect(labels(t), contains('Meeting notes'));
      expect(lines(t), isEmpty);
      expect(serverOf(c).vaultSearches.toSet(), {v, 'v-work'});
      await close(t, c);
    });

    testWidgets('a server error keeps the local rows, plus one quiet line', (
      t,
    ) async {
      final c = await open(t, seed: (s) => s.crossSearchStatus = 500);
      await palette(t);
      await t.enterText(field, 'design');
      await frames(t);
      expect(labels(t), containsAll(['Design Doc', 'Design Review']));
      expect(labels(t), isNot(contains('Meeting notes')));
      expect(heading, findsOneWidget);
      expect(lines(t), ['Couldn’t search note text']);
      await close(t, c);
    });

    testWidgets('offline, the slot is hidden', (t) async {
      final c = await open(t, offline: true);
      await palette(t);
      await t.enterText(field, 'design');
      await frames(t);
      expect(find.byKey(const Key('palette-offline')), findsOneWidget);
      expect(heading, findsNothing);
      expect(pending, findsNothing);
      expect(lines(t), isEmpty);
      await close(t, c);
    });
  });

  group('the phone (156n)', () {
    testWidgets("the pill's Search slot opens the palette full screen", (
      t,
    ) async {
      final c = await open(
        t,
        size: phone,
        route: Routes.browse(v),
        platform: TargetPlatform.android,
      );
      await t.tap(find.byTooltip('Search'));
      await frames(t);
      expect(screen, findsOneWidget);
      expect(field, findsOneWidget);
      expect(location(c), Routes.browse(v), reason: 'no search page');
      await close(t, c);
    });

    for (final (name, route) in [
      ('Agents', Routes.agents),
      ('Settings', Routes.settings),
    ]) {
      testWidgets('$name has a search bubble that opens it full screen', (
        t,
      ) async {
        final c = await open(
          t,
          size: phone,
          route: route,
          platform: TargetPlatform.android,
        );
        final bubble = find.byKey(const Key('search-bubble'));
        expect(bubble, findsOneWidget);
        expect(
          t.getRect(bubble).right,
          lessThan(t.getRect(find.byKey(const Key('settings-bubble'))).left),
        );
        await t.tap(bubble);
        await frames(t);
        expect(screen, findsOneWidget);
        await close(t, c);
      });
    }

    testWidgets('a vault has the pill instead, so no bubble', (t) async {
      final c = await open(
        t,
        size: phone,
        route: Routes.browse(v),
        platform: TargetPlatform.android,
      );
      expect(find.byKey(const Key('search-bubble')), findsNothing);
      await close(t, c);
    });
  });

  group('the old search page (156o)', () {
    testWidgets('/v/x/search goes to the vault root', (t) async {
      final c = await open(t);
      c.read(routerProvider).go('/v/$v/search');
      await frames(t);
      expect(location(c), Routes.browse(v));
      c.read(routerProvider).go('/v/v-work/search?q=plan');
      await frames(t);
      expect(location(c), Routes.browse('v-work'));
      await close(t, c);
    });
  });
}
