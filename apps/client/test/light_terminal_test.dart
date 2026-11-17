import 'dart:convert';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:xterm2/xterm.dart' show TerminalView;

import 'package:storm/agent/terminal_surface.dart';
import 'package:storm/state/terminal_prefs.dart';
import 'package:storm/ui/oklch.dart';
import 'package:storm/ui/theme.dart';
import 'package:storm/ui/tokens.dart';

/// Decision 157 (AM117): the light terminal.
void main() {
  group('the light palette (157b)', () {
    for (final preset in StormPreset.values.where((p) => !p.inputs.dark)) {
      test(preset.label, () {
        final t = StormTokens.from(preset);
        final bg = lightTerminalBackground(t);
        expect(t.terminal.background, bg);
        for (var i = 0; i < 16; i++) {
          if (i % 8 == 7) continue;
          final ratio = contrastRatio(t.terminal.ansi[i], bg);
          expect(
            ratio,
            greaterThanOrEqualTo(i < 8 ? 4.5 : 3.0),
            reason: 'colour $i is ${ratio.toStringAsFixed(2)}:1',
          );
        }
        expect(contrastRatio(t.text, bg), greaterThanOrEqualTo(4.5));
      });
    }
  });

  String hex(Color c) {
    String part(double v) =>
        ((v * 255).round() * 0x101).toRadixString(16).padLeft(4, '0');
    return 'rgb:${part(c.r)}/${part(c.g)}/${part(c.b)}';
  }

  final lightT = StormTokens.from(StormPreset.stormLight);
  final darkT = StormTokens.from(StormPreset.stormDark);

  ({StormTerminal t, List<String> sent, Future<void> Function(ThemeData) pump})
  harness(
    WidgetTester tester, {
    bool shell = false,
    TerminalPrefs prefs = const TerminalPrefs(),
  }) {
    final t = StormTerminal();
    addTearDown(t.dispose);
    final sent = <String>[];
    t.onInput = (b) => sent.add(latin1.decode(b));
    Future<void> pump(ThemeData theme) async {
      await tester.pumpWidget(
        MaterialApp(
          theme: theme,
          home: Scaffold(
            body: StormTerminalView(terminal: t, shell: shell, prefs: prefs),
          ),
        ),
      );
      await tester.pumpAndSettle();
    }

    return (t: t, sent: sent, pump: pump);
  }

  TerminalView view(WidgetTester tester) =>
      tester.widget<TerminalView>(find.byType(TerminalView));

  void write(StormTerminal t, String s) =>
      t.write(Uint8List.fromList(utf8.encode(s)));

  testWidgets('a shell view in a light theme is light, and follows the theme '
      'live', (tester) async {
    final h = harness(tester, shell: true);
    await h.pump(StormTheme.light());
    expect(view(tester).theme.background, lightTerminalBackground(lightT));
    expect(view(tester).theme.foreground, lightT.text);
    expect(view(tester).theme.red, lightT.terminal.ansi[1]);
    expect(view(tester).theme.brightWhite, lightT.terminal.ansi[15]);

    await h.pump(StormTheme.dark());
    expect(view(tester).theme.background, darkT.bg);
    expect(view(tester).theme.red, darkT.terminal.ansi[1]);

    await h.pump(StormTheme.light());
    expect(view(tester).theme.background, lightTerminalBackground(lightT));
  });

  testWidgets('an agent view in a light theme is dark until it asks, then '
      'answers light and shows light', (tester) async {
    final h = harness(tester);
    await h.pump(StormTheme.light());
    expect(view(tester).theme.background, darkT.bg, reason: "Storm dark's");
    expect(view(tester).theme.red, darkT.terminal.ansi[1]);

    write(h.t, '\x1b]11;?\x1b\\');
    expect(h.sent, ['\x1b]11;${hex(lightTerminalBackground(lightT))}\x1b\\']);
    await tester.pumpAndSettle();
    expect(h.t.answered.value, Brightness.light);
    expect(view(tester).theme.background, lightTerminalBackground(lightT));
    expect(view(tester).theme.red, lightT.terminal.ansi[1]);
  });

  testWidgets('the answered brightness outlives a remount', (tester) async {
    final h = harness(tester);
    await h.pump(StormTheme.light());
    write(h.t, '\x1b]10;?\x1b\\');
    await tester.pumpWidget(const SizedBox());
    await h.pump(StormTheme.light());
    expect(view(tester).theme.background, lightTerminalBackground(lightT));
  });

  testWidgets('with 2031 on, a switch to light is reported, but the view '
      'stays dark until the program asks again', (tester) async {
    final h = harness(tester);
    await h.pump(StormTheme.dark());
    write(h.t, '\x1b]11;?\x1b\\\x1b[?2031h');
    h.sent.clear();

    await h.pump(StormTheme.light());
    expect(h.sent, ['\x1b[?997;2n']);
    expect(view(tester).theme.background, darkT.bg, reason: 'still dark');

    h.sent.clear();
    write(h.t, '\x1b]11;?\x1b\\');
    await tester.pumpAndSettle();
    expect(view(tester).theme.background, lightTerminalBackground(lightT));
    expect(h.sent, hasLength(1), reason: 'the answer, and no second report');
  });

  testWidgets('a program that never asks keeps its start brightness', (
    tester,
  ) async {
    final h = harness(tester);
    await h.pump(StormTheme.light());
    write(h.t, '\x1b]11;?\x1b\\');
    await h.pump(StormTheme.dark());
    expect(h.t.answered.value, Brightness.light);
    expect(view(tester).theme.red, lightT.terminal.ansi[1]);
  });

  testWidgets('Always dark keeps everything dark and answers dark', (
    tester,
  ) async {
    const prefs = TerminalPrefs(colours: TerminalColours.alwaysDark);
    for (final shell in [false, true]) {
      final h = harness(tester, shell: shell, prefs: prefs);
      await h.pump(StormTheme.light());
      write(h.t, '\x1b]11;?\x1b\\\x1b[?996n');
      await tester.pumpAndSettle();
      expect(h.sent, ['\x1b]11;${hex(darkT.bg)}\x1b\\', '\x1b[?997;1n']);
      expect(h.t.answered.value, Brightness.dark);
      expect(view(tester).theme.background, darkT.bg);
      expect(view(tester).theme.red, darkT.terminal.ansi[1]);
    }
  });

  testWidgets('a rebuild with an unchanged theme reports nothing', (
    tester,
  ) async {
    for (final shell in [false, true]) {
      final h = harness(tester, shell: shell);
      await h.pump(StormTheme.light());
      write(h.t, '\x1b]11;?\x1b\\\x1b[?2031h');
      await tester.pumpAndSettle();
      h.sent.clear();
      await h.pump(StormTheme.light());
      await h.pump(StormTheme.light());
      expect(h.sent, isEmpty);
    }
  });

  testWidgets('bold is not drawn bright in a light view', (tester) async {
    final h = harness(tester, shell: true);
    await h.pump(StormTheme.light());
    expect(view(tester).textStyle.drawBoldTextWithBrightColors, isFalse);
    await h.pump(StormTheme.dark());
    expect(view(tester).textStyle.drawBoldTextWithBrightColors, isTrue);
  });
}
