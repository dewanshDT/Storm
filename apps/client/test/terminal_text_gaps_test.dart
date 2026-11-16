import 'dart:convert';

import 'package:flutter/gestures.dart' show PointerDeviceKind;
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:xterm2/xterm.dart';

import 'package:storm/agent/terminal_surface.dart';
import 'package:storm/ui/theme.dart';

/// Bug 2 of note 9aa73929: cells nothing wrote to, or that were erased, read
/// as spaces when text is taken from the screen.
void main() {
  Terminal terminalWith(String output, {int width = 80}) {
    final t = Terminal();
    t.resize(width, 10);
    t.write(output);
    return t;
  }

  String row(Terminal t, int y) => t.buffer.lines[y].getText();

  String select(Terminal t, BufferRange range) => t.buffer.getText(range, true);

  group('a line read as text', () {
    test('a cursor-forward gap is a space', () {
      final t = terminalWith('i\x1b[1Chave\x1b[1Cwritten\x1b[1Cstandown');
      expect(row(t, 0), 'i have written standown');
    });

    test('a CHA jump leaves spaces', () {
      final t = terminalWith('a\x1b[5Gb');
      expect(row(t, 0), 'a   b');
    });

    test('erased cells read as spaces', () {
      final ech = terminalWith('abcdef\r\x1b[1C\x1b[2X');
      expect(row(ech, 0), 'a  def');
      final el = terminalWith('abcdef\x1b[3G\x1b[1K');
      expect(row(el, 0), '   def');
    });

    test('a wide character is written once', () {
      final t = terminalWith('中文\x1b[1Cx');
      expect(row(t, 0), '中文 x');
    });

    test('nothing pads the end of the line', () {
      final t = terminalWith('abc');
      expect(row(t, 0), 'abc');
      expect(t.buffer.lines[1].getText(), '');
      expect(t.buffer.getText(), startsWith('abc\n'));
    });

    test('a range ending in a gap stops at the last written cell', () {
      final t = terminalWith('ab\x1b[3Ccd');
      expect(t.buffer.lines[0].getText(0, 4), 'ab');
      expect(t.buffer.lines[0].getText(1, 6), 'b   c');
    });
  });

  group('a selection', () {
    test('across a wrapped line keeps its gaps', () {
      final t = terminalWith('ab\x1b[3Ccdefghij', width: 10);
      expect(t.buffer.lines[1].isWrapped, isTrue);
      final range = BufferRangeLine(CellOffset(0, 0), CellOffset(10, 1));
      expect(select(t, range), 'ab   cdefghij');
    });

    test('a wide character pushed to the next row adds no space', () {
      final t = terminalWith('abcd中', width: 5);
      final range = BufferRangeLine(CellOffset(0, 0), CellOffset(5, 1));
      expect(select(t, range), 'abcd中');
    });

    test('past the text has no trailing spaces', () {
      final t = terminalWith('one\x1b[2Ctwo\r\nthree');
      final range = BufferRangeLine(CellOffset(0, 0), CellOffset(80, 1));
      expect(select(t, range), 'one  two\nthree');
    });

    test('a block keeps the gaps inside it', () {
      final t = terminalWith('a\x1b[2Cb\x1b[3Cz\r\nc\x1b[2Cd');
      final range = BufferRangeBlock(CellOffset(0, 0), CellOffset(4, 1));
      expect(select(t, range), 'a  b\nc  d');
    });
  });

  testWidgets(
    'a dragged selection copies with its spaces',
    (tester) async {
      final copied = <String>[];
      tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        SystemChannels.platform,
        (call) async {
          if (call.method == 'Clipboard.setData') {
            copied.add((call.arguments as Map)['text'] as String);
          }
          return null;
        },
      );
      final t = StormTerminal();
      addTearDown(t.dispose);
      t.write(utf8.encode('i\x1b[1Chave\x1b[1Cwritten\x1b[1Cstandown'));
      await tester.pumpWidget(
        MaterialApp(
          theme: StormTheme.dark(),
          home: Scaffold(
            body: SizedBox(
              width: 600,
              height: 400,
              child: StormTerminalView(terminal: t),
            ),
          ),
        ),
      );
      await tester.pump();
      final view = find.byType(TerminalView);
      final padding = tester.widget<TerminalView>(view).padding!;
      final start =
          tester.getTopLeft(view) + Offset(padding.left + 1, padding.top + 3);
      final end = Offset(tester.getTopRight(view).dx - 2, start.dy);

      await tester.tapAt(start, kind: PointerDeviceKind.mouse);
      await tester.pump(const Duration(milliseconds: 500));
      final drag = await tester.startGesture(
        start,
        kind: PointerDeviceKind.mouse,
      );
      await tester.pump();
      await drag.moveTo(end);
      await tester.pump();
      await drag.up();
      await tester.pump();

      await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
      await tester.sendKeyDownEvent(LogicalKeyboardKey.shiftLeft);
      await tester.sendKeyEvent(LogicalKeyboardKey.keyC);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.shiftLeft);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
      await tester.pump();

      expect(copied, ['i have written standown']);
    },
    variant: TargetPlatformVariant.only(TargetPlatform.linux),
  );
}
