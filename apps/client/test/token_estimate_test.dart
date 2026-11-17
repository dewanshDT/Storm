import 'dart:io';

import 'package:flutter/gestures.dart' show PointerDeviceKind;
import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'package:storm/editor/token_estimate.dart';
import 'package:storm/router.dart';
import 'package:storm/state/app_state.dart';
import 'package:storm/state/vault_config.dart';
import 'package:storm/ui/note_size.dart';
import 'package:storm/ui/properties_panel.dart';
import 'package:storm/ui/theme.dart';

import 'agent_fakes.dart';
import 'fake_server.dart';
import 'shell_harness.dart';

/// Decision 158: the token estimate, its formatting, the Properties Size row,
/// the selection count and the launcher's context chip.
void main() {
  setUp(() => SharedPreferences.setMockInitialValues({}));

  group('accuracy', () {
    // cl100k_base counts from Python tiktoken 0.12, over each file's bytes.
    const reference = {
      'english_prose.md': 323,
      'note_frontmatter.md': 315,
      'code_heavy.md': 413,
      'cjk.md': 250,
      'mixed.md': 228,
    };
    for (final MapEntry(key: name, value: want) in reference.entries) {
      test('$name is within ±15% of cl100k ($want)', () {
        final text = File(
          'test/fixtures/token_estimate/$name',
        ).readAsStringSync();
        final got = estimateText(text).tokens;
        expect(
          (got - want).abs() / want,
          lessThanOrEqualTo(0.15),
          reason: '$name: estimated $got, cl100k $want',
        );
      });
    }

    test('empty text is nothing', () {
      expect(estimateText(''), TokenEstimate.zero);
    });

    test('CJK counts a token and a word per character', () {
      final e = estimateText('会議メモ한국어');
      expect(e.words, 7);
      expect(e.chars, 7);
      expect(e.tokens, inInclusiveRange(7, 8));
    });

    test('code inside a fence is estimated at the code rate', () {
      const line = 'final debounce = Debouncer(const Duration(ms: 300));';
      final prose = estimateText(line).tokens;
      final code = estimateText('```\n$line\n```').tokens;
      expect(code, isNot(prose));
    });

    test(
      'a note: tokens over the whole file, words and chars over the body',
      () {
        const file = '---\ntitle: Weekly\ntags: [a, b]\n---\n\nTwo words\n';
        final note = estimateNote(file);
        expect(note.tokens, estimateText(file).tokens);
        expect(note.words, 2);
        expect(note.chars, 'Two words\n'.length + 1);
        expect(estimateNote('Just a body\n'), estimateText('Just a body\n'));
      },
    );
  });

  group('formatting', () {
    test('tokens', () {
      expect(formatTokens(0), '~0');
      expect(formatTokens(240), '~240');
      expect(formatTokens(999), '~999');
      expect(formatTokens(1000), '~1k');
      expect(formatTokens(2400), '~2.4k');
      expect(formatTokens(2449), '~2.4k');
      expect(formatTokens(10240), '~10.2k');
      expect(formatTokens(240000), '~240k');
      expect(formatTokens(999499), '~999k');
      expect(formatTokens(999500), '~1M');
      expect(formatTokens(1200000), '~1.2M');
    });

    test('words and chars', () {
      expect(groupedCount(7), '7');
      expect(groupedCount(1812), '1,812');
      expect(groupedCount(1234567), '1,234,567');
      expect(compactCount(812), '812');
      expect(compactCount(10200), '10.2k');
    });

    test('the row and the selection', () {
      const e = TokenEstimate(tokens: 2400, words: 1812, chars: 10200);
      expect(sizeLabel(e), '~2.4k tokens · 1,812 words · 10.2k chars');
      const s = TokenEstimate(tokens: 240, words: 180, chars: 900);
      expect(selectionLabel(s), 'Selection: ~240 tokens · 180 words');
    });
  });

  group('the Properties Size row', () {
    String label(WidgetTester t, String key) =>
        t.widget<Text>(find.byKey(Key(key))).data!;

    testWidgets('renders, carries the tooltip, and follows an edit after '
        'the debounce', (t) async {
      var content = '---\ntitle: A\n---\n\nOne two three\n';
      late StateSetter set;
      await t.pumpWidget(
        ProviderScope(
          overrides: [
            vaultConfigProvider.overrideWith(
              (ref) async => const VaultConfig(),
            ),
          ],
          child: MaterialApp(
            theme: StormTheme.dark(),
            home: Scaffold(
              body: StatefulBuilder(
                builder: (context, setState) {
                  set = setState;
                  return PropertiesPanel(content: content, onChanged: (_) {});
                },
              ),
            ),
          ),
        ),
      );
      await t.pumpAndSettle();

      expect(label(t, 'note-size'), sizeLabel(estimateNote(content)));
      expect(label(t, 'note-size'), contains('3 words'));
      expect(find.byTooltip(kSizeTooltip), findsOneWidget);
      expect(find.byKey(const Key('note-size-selection')), findsNothing);

      set(() => content = '$content\nfour five six seven\n');
      await t.pump();
      await t.pump(const Duration(milliseconds: 100));
      expect(label(t, 'note-size'), contains('3 words'));
      await t.pump(NoteSizeRow.debounce);
      expect(label(t, 'note-size'), contains('7 words'));
      expect(label(t, 'note-size'), sizeLabel(estimateNote(content)));
    });

    testWidgets('a note past the large size is estimated off the UI thread', (
      t,
    ) async {
      final big = 'word ' * (kLargeNoteChars ~/ 5 + 10);
      await t.pumpWidget(
        ProviderScope(
          child: MaterialApp(
            theme: StormTheme.dark(),
            home: Scaffold(body: NoteSizeRow(content: big)),
          ),
        ),
      );
      expect(label(t, 'note-size'), '—');
      await t.runAsync(
        () => Future<void>.delayed(const Duration(milliseconds: 500)),
      );
      await t.pump();
      expect(label(t, 'note-size'), sizeLabel(estimateNote(big)));
    });
  });

  group('selection and the launcher', () {
    const desk = Size(1280, 900);
    const primary = FakeServer.primaryVault;
    const body = 'Alpha bravo charlie delta echo foxtrot.\n';
    const file = '---\nstatus: active\n---\n\n$body';

    ProviderContainer withNote() {
      final c = shellContainer();
      serverOf(c).notes['p1'] = ServerNote(
        id: 'p1',
        path: 'Projects/BOARD.md',
        content: file,
        version: 3,
      );
      return c;
    }

    Future<void> openNote(WidgetTester t, ProviderContainer c) async {
      await pumpShell(t, c, size: desk);
      c.read(propertiesOpenProvider.notifier).state = true;
      c.read(routerProvider).go(Routes.note(primary, 'p1'));
      await t.pumpAndSettle();
    }

    String text(WidgetTester t, String key) =>
        t.widget<Text>(find.byKey(Key(key))).data!;

    testWidgets('the editor: a selection shows its own count beside the '
        'totals, and clears with it', (t) async {
      final c = withNote();
      await openNote(t, c);
      expect(text(t, 'note-size'), sizeLabel(estimateNote(file)));

      await t.tap(find.byKey(const Key('mode-edit')));
      await t.pumpAndSettle();
      final field = t.widget<TextField>(find.byKey(const Key('note-body')));
      final start = field.controller!.text.indexOf('Alpha bravo');
      field.controller!.selection = TextSelection(
        baseOffset: start,
        extentOffset: start + 'Alpha bravo'.length,
      );
      await t.pump();
      expect(
        text(t, 'note-size-selection'),
        selectionLabel(estimateText('Alpha bravo')),
      );
      expect(text(t, 'note-size-selection'), contains('2 words'));
      expect(find.byKey(const Key('note-size')), findsOneWidget);

      field.controller!.selection = const TextSelection.collapsed(offset: 3);
      await t.pump();
      expect(find.byKey(const Key('note-size-selection')), findsNothing);
      await disposeShell(t, c);
    });

    testWidgets('the reader: a selected word is counted', (t) async {
      final c = withNote();
      await openNote(t, c);
      expect(find.byKey(const Key('note-read')), findsOneWidget);

      final word = find.textContaining('Alpha bravo', findRichText: true);
      final at = t.getTopLeft(word.first) + const Offset(4, 8);
      final gesture = await t.startGesture(at, kind: PointerDeviceKind.mouse);
      await gesture.moveTo(at + const Offset(30, 0));
      await gesture.up();
      await t.pumpAndSettle();
      final shown = text(t, 'note-size-selection');
      expect(shown, startsWith('Selection: ~'));
      expect(shown, matches(RegExp(r' · \d+ words?$')));
      await disposeShell(t, c);
    });

    testWidgets("the launcher's context chip shows the note's estimate", (
      t,
    ) async {
      final agents = FakeAgentServer(
        hosts: [
          agentHost(
            id: 'hst_1',
            name: 'build-vm',
            providers: ['claude-code'],
            workspaces: ['storm'],
          ),
        ],
      );
      final c = shellContainer(agentClient: agents.client);
      serverOf(c).notes['p1'] = ServerNote(
        id: 'p1',
        path: 'Projects/BOARD.md',
        content: file,
        version: 3,
      );
      await pumpShell(t, c, size: desk);
      c.read(routerProvider).go(Routes.note(primary, 'p1'));
      await t.pumpAndSettle();
      await t.tap(find.byKey(const Key('start-session')));
      await t.pumpAndSettle();

      expect(find.byKey(const Key('launcher-context')), findsOneWidget);
      expect(
        text(t, 'launcher-context-tokens'),
        ' · ${formatTokens(estimateNote(file).tokens)} tokens',
      );
      await disposeShell(t, c);
    });
  });
}
