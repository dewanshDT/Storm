import 'dart:async';
import 'dart:convert';
import 'dart:ui';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shared_preferences/shared_preferences.dart';
import 'package:xterm2/xterm.dart';

import 'package:storm/agent/agent_models.dart';
import 'package:storm/agent/session_keep_alive.dart';
import 'package:storm/agent/terminal_events.dart';
import 'package:storm/router.dart';

import 'agent_fakes.dart';
import 'shell_harness.dart';

/// 171-12: at desk width the session shown has the keyboard, however it was
/// reached, and a hidden kept view never does.
void main() {
  setUp(() => SharedPreferences.setMockInitialValues({}));

  Uint8List bytes(String s) => Uint8List.fromList(utf8.encode(s));

  Stream<TerminalEvent> open(String id, int offset, bool _) {
    final s = StreamController<TerminalEvent>();
    s.add(
      StatusEvent(AgentSession.fromJson({...agentSession(id), 'cols': 80})),
    );
    final text = bytes('$id history');
    if (offset == 0) s.add(OutputEvent(text.length, text));
    s.add(LiveEvent());
    return s.stream;
  }

  Future<ProviderContainer> mount(WidgetTester tester, Size size) async {
    final c = shellContainer(
      agentClient: FakeAgentServer(
        hosts: [agentHost()],
        sessions: [
          agentSession('ags_a', name: 'alpha'),
          agentSession('ags_b', name: 'bravo'),
        ],
      ).client,
      terminalStream: open,
      keepAlive: const KeepAliveBudget(
        hidden: 3,
        bytes: 48 << 20,
        ttl: null,
        grace: Duration(seconds: 2),
      ),
    );
    await pumpShell(tester, c, size: size);
    return c;
  }

  Iterable<FocusNode> focusedViews() => find
      .byType(TerminalView, skipOffstage: false)
      .evaluate()
      .map((e) => (e.widget as TerminalView).focusNode)
      .whereType<FocusNode>()
      .where((n) => n.hasFocus);

  bool shownHasFocus(WidgetTester tester) {
    final shown = find.byType(TerminalView);
    expect(shown, findsOneWidget);
    return tester.widget<TerminalView>(shown).focusNode!.hasPrimaryFocus;
  }

  Future<void> clickRow(WidgetTester tester, String name) async {
    await tester.tap(find.text(name).first, kind: PointerDeviceKind.mouse);
    await tester.pumpAndSettle();
  }

  Future<void> leaveWindowAndComeBack(WidgetTester tester) async {
    for (final state in [ViewFocusState.unfocused, ViewFocusState.focused]) {
      tester.binding.handleViewFocusChanged(
        ViewFocusEvent(
          viewId: tester.view.viewId,
          state: state,
          direction: ViewFocusDirection.forward,
        ),
      );
      await tester.pumpAndSettle();
    }
  }

  final desk = TargetPlatformVariant({
    TargetPlatform.macOS,
    TargetPlatform.linux,
  });

  testWidgets('desk: the session opened from the sidebar has the keyboard, '
      'a kept one too when it is back', (tester) async {
    final c = await mount(tester, const Size(1400, 900));
    c.read(routerProvider).go('/agents');
    await tester.pumpAndSettle();

    await clickRow(tester, 'alpha');
    expect(shownHasFocus(tester), isTrue, reason: 'first open');

    await clickRow(tester, 'bravo');
    expect(shownHasFocus(tester), isTrue, reason: 'switched to B');

    await clickRow(tester, 'alpha');
    expect(shownHasFocus(tester), isTrue, reason: 'back to the kept A');
    expect(focusedViews(), hasLength(1), reason: 'no hidden view has focus');
    await disposeShell(tester, c);
  }, variant: desk);

  testWidgets('desk: with the window left and come back, the session '
      'clicked takes the keyboard back', (tester) async {
    final c = await mount(tester, const Size(1400, 900));
    c.read(routerProvider).go('/agents');
    await tester.pumpAndSettle();
    await clickRow(tester, 'alpha');

    await leaveWindowAndComeBack(tester);
    expect(shownHasFocus(tester), isFalse, reason: 'the shell took it');
    await clickRow(tester, 'bravo');
    expect(shownHasFocus(tester), isTrue, reason: 'B');

    await leaveWindowAndComeBack(tester);
    await clickRow(tester, 'alpha');
    expect(shownHasFocus(tester), isTrue, reason: 'the kept A');

    await leaveWindowAndComeBack(tester);
    await clickRow(tester, 'alpha');
    expect(shownHasFocus(tester), isTrue, reason: 'A again, already shown');
    expect(focusedViews(), hasLength(1));
    await disposeShell(tester, c);
  }, variant: desk);

  testWidgets(
    'desk: a switch leaves a focused field alone',
    (tester) async {
      final c = await mount(tester, const Size(1400, 900));
      c.read(routerProvider).go('/agents/s/ags_a');
      await tester.pumpAndSettle();
      expect(shownHasFocus(tester), isTrue);

      await tester.sendKeyDownEvent(LogicalKeyboardKey.metaLeft);
      await tester.sendKeyEvent(LogicalKeyboardKey.keyK);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.metaLeft);
      await tester.pumpAndSettle();
      final field = find.byKey(const Key('palette-field'));
      expect(field, findsOneWidget);
      c.read(routerProvider).go('/agents/s/ags_b');
      await tester.pumpAndSettle();
      final editable = tester.widget<EditableText>(
        find.descendant(of: field, matching: find.byType(EditableText)),
      );
      expect(editable.focusNode.hasPrimaryFocus, isTrue);
      expect(focusedViews(), isEmpty);

      await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      await tester.pumpAndSettle();
      await disposeShell(tester, c);
    },
    variant: TargetPlatformVariant.only(TargetPlatform.macOS),
  );

  testWidgets('desk: the first open from the overview has the keyboard', (
    tester,
  ) async {
    final c = await mount(tester, const Size(1400, 900));
    c.read(routerProvider).go('/agents');
    await tester.pumpAndSettle();
    await tester.tap(find.text('bravo').last, kind: PointerDeviceKind.mouse);
    await tester.pumpAndSettle();
    expect(shownHasFocus(tester), isTrue);
    await disposeShell(tester, c);
  }, variant: desk);

  testWidgets('phone: a session opened or switched to is not focused, so no '
      'soft keyboard comes up', (tester) async {
    final c = await mount(tester, const Size(411, 900));
    c.read(routerProvider).go('/agents/s/ags_a');
    await tester.pumpAndSettle();
    expect(focusedViews(), isEmpty);
    c.read(routerProvider).go('/agents');
    await tester.pumpAndSettle();
    c.read(routerProvider).go('/agents/s/ags_b');
    await tester.pumpAndSettle();
    expect(focusedViews(), isEmpty);
    await disposeShell(tester, c);
  });
}
