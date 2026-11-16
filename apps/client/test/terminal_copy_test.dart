import 'dart:convert';

import 'package:flutter/gestures.dart' show PointerDeviceKind;
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:xterm2/xterm.dart' show TerminalView;

import 'package:storm/agent/terminal_surface.dart';
import 'package:storm/ui/theme.dart';

/// Bug 1 of note 9aa73929: copying from the terminal (AM116).
void main() {
  late List<String> copied;
  late int reads;

  setUp(() {
    copied = [];
    reads = 0;
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(SystemChannels.platform, (call) async {
          switch (call.method) {
            case 'Clipboard.setData':
              copied.add((call.arguments as Map)['text'] as String);
            case 'Clipboard.getData':
              reads++;
              return {'text': 'pasted'};
          }
          return null;
        });
  });

  StormTerminal terminalWith(String output) {
    final t = StormTerminal();
    addTearDown(t.dispose);
    t.write(utf8.encode(output));
    return t;
  }

  Widget app(Widget child) => MaterialApp(
    theme: StormTheme.dark(),
    home: Scaffold(body: SizedBox(width: 600, height: 400, child: child)),
  );

  /// The first cell of row 0, and a point past the end of that row.
  (Offset, Offset) firstRow(WidgetTester tester, {Finder? view}) {
    view ??= find.byType(TerminalView);
    final padding = tester.widget<TerminalView>(view).padding!;
    final start =
        tester.getTopLeft(view) + Offset(padding.left + 1, padding.top + 3);
    return (start, Offset(tester.getTopRight(view).dx - 2, start.dy));
  }

  /// A press that moves at once, so xterm2's tap never fires.
  Future<void> drag(WidgetTester tester, Offset from, Offset to) async {
    final gesture = await tester.startGesture(
      from,
      kind: PointerDeviceKind.mouse,
    );
    await gesture.moveTo(to);
    await tester.pump();
    await gesture.up();
    await tester.pump();
  }

  Future<void> chord(
    WidgetTester tester,
    List<LogicalKeyboardKey> modifiers,
    LogicalKeyboardKey key,
  ) async {
    for (final m in modifiers) {
      await tester.sendKeyDownEvent(m);
    }
    await tester.sendKeyEvent(key);
    for (final m in modifiers.reversed) {
      await tester.sendKeyUpEvent(m);
    }
    await tester.pump();
  }

  const mac = [LogicalKeyboardKey.metaLeft];
  const linux = [LogicalKeyboardKey.controlLeft, LogicalKeyboardKey.shiftLeft];

  testWidgets(
    'a drag with no tap focuses the terminal, and ⌘C copies it on a Mac',
    (tester) async {
      final focus = FocusNode();
      addTearDown(focus.dispose);
      final t = terminalWith('hello world');
      await tester.pumpWidget(
        app(StormTerminalView(terminal: t, focusNode: focus)),
      );
      await tester.pump();
      expect(focus.hasFocus, isFalse);
      final (start, end) = firstRow(tester);
      await drag(tester, start, end);
      expect(focus.hasFocus, isTrue);
      await chord(tester, mac, LogicalKeyboardKey.keyC);
      expect(copied, ['hello world']);
    },
    variant: TargetPlatformVariant.only(TargetPlatform.macOS),
  );

  testWidgets(
    'a drag with no tap focuses the terminal, and Ctrl+Shift+C copies it',
    (tester) async {
      final focus = FocusNode();
      addTearDown(focus.dispose);
      final t = terminalWith('hello world');
      await tester.pumpWidget(
        app(StormTerminalView(terminal: t, focusNode: focus)),
      );
      await tester.pump();
      final (start, end) = firstRow(tester);
      await drag(tester, start, end);
      expect(focus.hasFocus, isTrue);
      await chord(tester, linux, LogicalKeyboardKey.keyC);
      expect(copied, ['hello world']);
    },
    variant: TargetPlatformVariant.only(TargetPlatform.linux),
  );

  testWidgets(
    'an ended session copies too',
    (tester) async {
      final t = terminalWith('hello world');
      await tester.pumpWidget(
        app(StormTerminalView(terminal: t, readOnly: true)),
      );
      await tester.pump();
      final (start, end) = firstRow(tester);
      await drag(tester, start, end);
      await chord(tester, linux, LogicalKeyboardKey.keyC);
      expect(copied, ['hello world']);
    },
    variant: TargetPlatformVariant.only(TargetPlatform.linux),
  );

  testWidgets(
    'a touch does not focus on the press',
    (tester) async {
      final focus = FocusNode();
      addTearDown(focus.dispose);
      final t = terminalWith('hello world');
      await tester.pumpWidget(
        app(StormTerminalView(terminal: t, focusNode: focus, phone: true)),
      );
      await tester.pump();
      final (start, end) = firstRow(tester);
      final gesture = await tester.startGesture(start);
      await gesture.moveTo(end);
      await tester.pump();
      expect(focus.hasFocus, isFalse, reason: 'a scroll opens no keyboard');
      await gesture.up();
    },
    variant: TargetPlatformVariant.only(TargetPlatform.android),
  );

  testWidgets(
    '⌘A selects the screen for ⌘C on a Mac',
    (tester) async {
      final t = terminalWith('one\r\ntwo');
      await tester.pumpWidget(
        app(StormTerminalView(terminal: t, autofocus: true)),
      );
      await tester.pump();
      await chord(tester, mac, LogicalKeyboardKey.keyA);
      await chord(tester, mac, LogicalKeyboardKey.keyC);
      expect(copied.single, startsWith('one\ntwo'));
    },
    variant: TargetPlatformVariant.only(TargetPlatform.macOS),
  );

  group('the agent keeps its chords off a Mac', () {
    Future<List<List<int>>> press(
      WidgetTester tester,
      LogicalKeyboardKey key,
    ) async {
      final t = terminalWith('hello world');
      final sent = <List<int>>[];
      t.onInput = sent.add;
      await tester.pumpWidget(
        app(StormTerminalView(terminal: t, autofocus: true)),
      );
      await tester.pump();
      final (start, end) = firstRow(tester);
      await drag(tester, start, end);
      await chord(tester, [LogicalKeyboardKey.controlLeft], key);
      return sent;
    }

    testWidgets(
      'Ctrl+A is ^A, not select all',
      (tester) async {
        final sent = await press(tester, LogicalKeyboardKey.keyA);
        expect(sent, [
          [0x01],
        ]);
      },
      variant: TargetPlatformVariant.only(TargetPlatform.linux),
    );

    testWidgets(
      'Ctrl+V is ^V, not a paste',
      (tester) async {
        final sent = await press(tester, LogicalKeyboardKey.keyV);
        expect(sent, [
          [0x16],
        ]);
        expect(reads, 0);
      },
      variant: TargetPlatformVariant.only(TargetPlatform.linux),
    );

    testWidgets(
      'Ctrl+K and Ctrl+P go to the agent',
      (tester) async {
        final t = terminalWith('');
        final sent = <List<int>>[];
        t.onInput = sent.add;
        await tester.pumpWidget(
          app(StormTerminalView(terminal: t, autofocus: true)),
        );
        await tester.pump();
        await chord(tester, [
          LogicalKeyboardKey.controlLeft,
        ], LogicalKeyboardKey.keyK);
        await chord(tester, [
          LogicalKeyboardKey.controlLeft,
        ], LogicalKeyboardKey.keyP);
        expect(sent, [
          [0x0b],
          [0x10],
        ]);
      },
      variant: TargetPlatformVariant.only(TargetPlatform.linux),
    );
  });

  group('a paste is made once', () {
    Future<List<String>> paste(
      WidgetTester tester,
      List<LogicalKeyboardKey> modifiers, {
      required bool storm,
    }) async {
      final t = terminalWith('');
      final sent = <String>[];
      t.onInput = (b) => sent.add(utf8.decode(b));
      if (storm) t.onImagePaste = () async => false;
      await tester.pumpWidget(
        app(StormTerminalView(terminal: t, autofocus: true)),
      );
      await tester.pump();
      await chord(tester, modifiers, LogicalKeyboardKey.keyV);
      await tester.pump();
      return sent;
    }

    testWidgets(
      'by Storm, where it pastes itself',
      (tester) async {
        final sent = await paste(tester, linux, storm: true);
        expect(sent.join(), contains('pasted'));
        expect(reads, 1);
      },
      variant: TargetPlatformVariant.only(TargetPlatform.linux),
    );

    testWidgets(
      'by Storm on a Mac',
      (tester) async {
        final sent = await paste(tester, mac, storm: true);
        expect(sent.join(), contains('pasted'));
        expect(reads, 1);
      },
      variant: TargetPlatformVariant.only(TargetPlatform.macOS),
    );

    testWidgets(
      'by the terminal, where Storm does not',
      (tester) async {
        final sent = await paste(tester, linux, storm: false);
        expect(sent.join(), contains('pasted'));
        expect(reads, 1);
      },
      variant: TargetPlatformVariant.only(TargetPlatform.linux),
    );
  });

  testWidgets(
    'a composition left open by a lost focus does not swallow the copy',
    (tester) async {
      final focus = FocusNode();
      addTearDown(focus.dispose);
      final t = terminalWith('hello world');
      await tester.pumpWidget(
        app(StormTerminalView(terminal: t, focusNode: focus, autofocus: true)),
      );
      await tester.pump();
      tester.testTextInput.updateEditingValue(
        const TextEditingValue(
          text: '´',
          selection: TextSelection.collapsed(offset: 1),
          composing: TextRange(start: 0, end: 1),
        ),
      );
      await tester.pump();
      focus.unfocus();
      await tester.pump();
      final (start, end) = firstRow(tester);
      await drag(tester, start, end);
      expect(focus.hasFocus, isTrue);
      await chord(tester, linux, LogicalKeyboardKey.keyC);
      expect(copied, ['hello world']);
    },
    variant: TargetPlatformVariant.only(TargetPlatform.linux),
  );

  testWidgets('OSC 52 still copies after the view is remounted', (
    tester,
  ) async {
    final t = terminalWith('');
    final focus = FocusNode();
    addTearDown(focus.dispose);
    Widget views(List<String> keys) => app(
      Column(
        children: [
          for (final k in keys)
            Expanded(
              child: StormTerminalView(
                key: ValueKey(k),
                terminal: t,
                focusNode: k == 'new' ? focus : null,
              ),
            ),
        ],
      ),
    );

    await tester.pumpWidget(views(['old']));
    await tester.pumpWidget(views(['old', 'new']));
    await tester.pumpWidget(views(['new']));
    focus.requestFocus();
    await tester.pump();

    t.write(utf8.encode('\x1b]52;c;${base64.encode(utf8.encode('osc'))}\x07'));
    await tester.pump();
    expect(copied, ['osc']);
  });
}
