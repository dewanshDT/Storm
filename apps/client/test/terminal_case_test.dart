import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:storm/agent/terminal_surface.dart';
import 'package:storm/ui/theme.dart';

/// Round 2: capitals typed into Storm's terminal reached Claude Code
/// lowercased. Claude Code turns on the kitty protocol (`>5u`) and
/// modifyOtherKeys (`>4;2m`); an unmodified "T" went out as
/// `CSI 27;1;84~`, which it reads as "t". Whatever the modes, a capital
/// is a capital: typed with Shift, arriving as an uppercase character with
/// no Shift (Caps Lock, a browser's automation, a soft keyboard), or
/// pasted. Runs with `--platform chrome` too.
void main() {
  Future<({StormTerminal term, List<String> sent})> terminal(
    WidgetTester t,
    String modes,
  ) async {
    final term = StormTerminal();
    addTearDown(term.dispose);
    final sent = <String>[];
    term.onInput = (b) => sent.add(utf8.decode(b));
    final node = FocusNode();
    addTearDown(node.dispose);
    await t.pumpWidget(
      MaterialApp(
        theme: StormTheme.dark(),
        home: Scaffold(
          body: StormTerminalView(
            terminal: term,
            focusNode: node,
            autofocus: true,
          ),
        ),
      ),
    );
    await t.pump();
    if (modes.isNotEmpty) {
      term.write(Uint8List.fromList(modes.codeUnits));
      await t.pump();
    }
    sent.clear();
    return (term: term, sent: sent);
  }

  Future<void> shifted(WidgetTester t, LogicalKeyboardKey key, String c) async {
    await t.sendKeyDownEvent(LogicalKeyboardKey.shiftLeft);
    await t.sendKeyEvent(key, character: c);
    await t.sendKeyUpEvent(LogicalKeyboardKey.shiftLeft);
    await t.pump();
  }

  for (final (name, modes) in [
    ('legacy', ''),
    ('modifyOtherKeys', '\x1b[>4;2m'),
    ('the kitty protocol', '\x1b[>5u'),
    ('as Claude Code sets them', '\x1b[>5u\x1b[>4;2m'),
  ]) {
    group(name, () {
      testWidgets('Shift and a letter is the capital', (t) async {
        final r = await terminal(t, modes);
        await shifted(t, LogicalKeyboardKey.keyT, 'T');
        await t.sendKeyEvent(LogicalKeyboardKey.keyE, character: 'e');
        await t.pump();
        expect(r.sent.join(), 'Te');
      });

      testWidgets('an uppercase character with no Shift is the capital', (
        t,
      ) async {
        final r = await terminal(t, modes);
        await t.sendKeyEvent(LogicalKeyboardKey.keyT, character: 'T');
        await t.sendKeyEvent(LogicalKeyboardKey.keyS, character: 'S');
        await t.pump();
        expect(r.sent.join(), 'TS');
      });

      testWidgets('text the keyboard inserts keeps its case', (t) async {
        final r = await terminal(t, modes);
        for (final c in 'Test Sandbox'.split('')) {
          t.testTextInput.updateEditingValue(
            TextEditingValue(
              text: c,
              selection: const TextSelection.collapsed(offset: 1),
            ),
          );
          await t.pump();
        }
        expect(r.sent.join(), 'Test Sandbox');
      });

      testWidgets('a paste keeps its case', (t) async {
        final r = await terminal(t, modes);
        r.term.paste('Test Sandbox');
        await t.pump();
        expect(r.sent.join(), contains('Test Sandbox'));
      });
    });
  }

  testWidgets('modified keys are still encoded for the agent', (t) async {
    final r = await terminal(t, '\x1b[>4;2m');
    await t.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
    await t.sendKeyEvent(LogicalKeyboardKey.keyT, character: 't');
    await t.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
    await t.pump();
    expect(r.sent.join(), '\x1b[27;5;116~');
  });

  testWidgets('with the kitty protocol on, its encoding wins', (t) async {
    final k = await terminal(t, '\x1b[>5u\x1b[>4;2m');
    await t.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
    await t.sendKeyEvent(LogicalKeyboardKey.keyT, character: 't');
    await t.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
    await t.pump();
    expect(k.sent.join(), '\x1b[116;5u');
  });

  testWidgets('with every key as an escape code, Shift is reported', (t) async {
    final r = await terminal(t, '\x1b[>13u');
    await shifted(t, LogicalKeyboardKey.keyT, 'T');
    expect(r.sent.last, '\x1b[116:84;2u');
  });
}
