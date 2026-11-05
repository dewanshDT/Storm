import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:storm/agent/terminal_surface.dart';
import 'package:storm/ui/theme.dart';

/// A-147-2 (decision 147b): while Storm's terminal has focus, Tab and
/// Shift+Tab reach the agent, once each, whatever keyboard protocol it
/// asked for; Escape, then Tab, moves focus out. No database or shell, so
/// it also runs with `--platform chrome`, on the web's own text input.
void main() {
  /// A focused terminal between two buttons; what it sent, as text.
  Future<({StormTerminal term, List<String> sent, FocusNode node})> terminal(
    WidgetTester t, {
    String modes = '',
  }) async {
    final term = StormTerminal();
    addTearDown(term.dispose);
    final sent = <String>[];
    term.onInput = (b) => sent.add(latin1.decode(b));
    final node = FocusNode();
    addTearDown(node.dispose);
    await t.pumpWidget(
      MaterialApp(
        theme: StormTheme.dark(),
        home: Scaffold(
          body: Column(
            children: [
              TextButton(
                key: const Key('before'),
                onPressed: () {},
                child: const Text('before'),
              ),
              Expanded(
                child: StormTerminalView(
                  terminal: term,
                  focusNode: node,
                  autofocus: true,
                ),
              ),
              TextButton(
                key: const Key('after'),
                onPressed: () {},
                child: const Text('after'),
              ),
            ],
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
    return (term: term, sent: sent, node: node);
  }

  Future<void> shiftTab(WidgetTester t) async {
    await t.sendKeyDownEvent(LogicalKeyboardKey.shiftLeft);
    await t.sendKeyEvent(LogicalKeyboardKey.tab);
    await t.sendKeyUpEvent(LogicalKeyboardKey.shiftLeft);
    await t.pump();
  }

  bool focused(WidgetTester t, String key) {
    final f = FocusManager.instance.primaryFocus?.context;
    if (f == null) return false;
    return f.widget.key == Key(key) ||
        find
            .descendant(
              of: find.byKey(Key(key)),
              matching: find.byWidget(f.widget),
            )
            .evaluate()
            .isNotEmpty;
  }

  group('A-147-2: Tab reaches the agent', () {
    // Claude Code turns on the kitty protocol (`>5u`) and modifyOtherKeys
    // once the terminal says it is xterm2.
    for (final (name, modes) in [
      ('legacy', ''),
      ('with the kitty protocol on', '\x1b[>5u\x1b[>4;2m'),
    ]) {
      testWidgets('Tab is \\t and Shift+Tab is ESC [ Z, once each ($name)', (
        t,
      ) async {
        final r = await terminal(t, modes: modes);
        await t.sendKeyEvent(LogicalKeyboardKey.tab);
        await t.pump();
        expect(r.sent, ['\t']);
        expect(r.node.hasFocus, isTrue, reason: 'focus stays');
        r.sent.clear();
        await shiftTab(t);
        expect(r.sent, ['\x1b[Z'], reason: 'one press, one mode step');
        expect(r.node.hasFocus, isTrue);
      });
    }

    testWidgets('an Escape is sent once with the kitty protocol on', (t) async {
      final r = await terminal(t, modes: '\x1b[>5u');
      await t.sendKeyEvent(LogicalKeyboardKey.escape);
      await t.pump();
      expect(r.sent, ['\x1b[27u'], reason: 'a release is no second Escape');
      r.sent.clear();
      await t.sendKeyDownEvent(LogicalKeyboardKey.shiftLeft);
      await t.sendKeyUpEvent(LogicalKeyboardKey.shiftLeft);
      expect(
        r.sent,
        isEmpty,
        reason: 'Shift alone is not a key the agent asked for',
      );
    });

    testWidgets(
      'Escape then Tab moves focus out; Escape still reaches the agent',
      (t) async {
        final r = await terminal(t);
        await t.sendKeyEvent(LogicalKeyboardKey.escape);
        await t.sendKeyEvent(LogicalKeyboardKey.tab);
        await t.pump();
        expect(r.sent, ['\x1b'], reason: 'no Tab was sent');
        expect(r.node.hasFocus, isFalse);
        expect(focused(t, 'after'), isTrue);

        // Back in, Escape then Shift+Tab goes the other way.
        r.node.requestFocus();
        await t.pump();
        r.sent.clear();
        await t.sendKeyEvent(LogicalKeyboardKey.escape);
        await shiftTab(t);
        expect(r.sent, ['\x1b']);
        expect(focused(t, 'before'), isTrue);
      },
    );

    testWidgets(
      'a Tab long after an Escape, or after another key, is the agent’s',
      (t) async {
        final r = await terminal(t);
        var now = DateTime(2026, 10, 30, 12);
        r.term.clock = () => now;
        await t.sendKeyEvent(LogicalKeyboardKey.escape);
        now = now.add(StormTerminal.escapeThenTab + const Duration(seconds: 1));
        await t.sendKeyEvent(LogicalKeyboardKey.tab);
        await t.pump();
        expect(r.sent, ['\x1b', '\t']);
        expect(r.node.hasFocus, isTrue);

        r.sent.clear();
        await t.sendKeyEvent(LogicalKeyboardKey.escape);
        await t.sendKeyEvent(LogicalKeyboardKey.keyA);
        await t.sendKeyEvent(LogicalKeyboardKey.tab);
        await t.pump();
        expect(r.sent.last, '\t');
        expect(r.node.hasFocus, isTrue);
      },
    );

    testWidgets('the terminal’s name says how to leave', (t) async {
      final handle = t.ensureSemantics();
      await terminal(t);
      expect(find.bySemanticsLabel(RegExp('Escape, then Tab')), findsOneWidget);
      handle.dispose();
    });
  });
}
