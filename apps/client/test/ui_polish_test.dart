import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'package:storm/agent/session_inspector.dart';
import 'package:storm/agent/terminal_surface.dart';
import 'package:storm/router.dart';
import 'package:storm/ui/breakpoints.dart';

import 'agent_fakes.dart';
import 'fake_server.dart';
import 'shell_harness.dart';

/// The Storm v2 layout pass: the session workspace (header, terminal,
/// inspector), the phone keyboard accessory, the rail's foot and the note's
/// editor surface.
void main() {
  const phone = Size(411, 900);
  const wide = Size(1600, 900);
  const primary = FakeServer.primaryVault;

  setUp(() => SharedPreferences.setMockInitialValues({}));

  Future<ProviderContainer> session(
    WidgetTester tester, {
    Size size = wide,
  }) async {
    final agents = FakeAgentServer(
      hosts: [agentHost()],
      sessions: [agentSession('ags_1')],
    );
    final c = shellContainer(agentClient: agents.client);
    await pumpShell(tester, c, size: size);
    c.read(routerProvider).go(Routes.agentSession('ags_1'));
    await tester.pumpAndSettle();
    return c;
  }

  Rect rect(WidgetTester tester, String key) =>
      tester.getRect(find.byKey(Key(key)));

  double terminalWidth(WidgetTester tester) =>
      tester.getSize(find.byType(StormTerminalView)).width;

  group('the session workspace (desk)', () {
    testWidgets('the header spans the terminal and the inspector', (
      tester,
    ) async {
      final c = await session(tester);
      final inspector = rect(tester, 'session-inspector');
      final terminal = tester.getRect(find.byType(StormTerminalView));
      final header = tester.getRect(
        find
            .ancestor(
              of: find.byKey(const Key('session-name')),
              matching: find.byType(Container),
            )
            .first,
      );
      expect(header.left, closeTo(terminal.left, 1));
      expect(header.right, closeTo(inspector.right, 1));
      expect(inspector.width, closeTo(kInspectorWidth + 1, 1));
      // The actions sit at the workspace's far edge, past the terminal.
      expect(
        rect(tester, 'inspector-toggle').left,
        greaterThan(terminal.right - 40),
      );
      await disposeShell(tester, c);
    });

    testWidgets('closing the inspector gives its width to the terminal', (
      tester,
    ) async {
      final c = await session(tester);
      final open = terminalWidth(tester);
      await tester.tap(find.byKey(const Key('inspector-close')));
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('session-inspector')), findsNothing);
      expect(terminalWidth(tester), greaterThan(open + kInspectorWidth - 2));

      await tester.tap(find.byKey(const Key('inspector-toggle')));
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('session-inspector')), findsOneWidget);
      expect(terminalWidth(tester), closeTo(open, 1));
      await disposeShell(tester, c);
    });

    testWidgets('dragging the edge resizes between the bounds', (tester) async {
      final c = await session(tester);
      final handle = find.byKey(const Key('inspector-resize'));
      await tester.drag(handle, const Offset(-400, 0));
      await tester.pumpAndSettle();
      expect(
        rect(tester, 'session-inspector').width,
        closeTo(kInspectorMaxWidth + 1, 1),
      );
      await tester.drag(handle, const Offset(600, 0));
      await tester.pumpAndSettle();
      expect(
        rect(tester, 'session-inspector').width,
        closeTo(kInspectorMinWidth + 1, 1),
      );
      expect(terminalWidth(tester), greaterThan(kSessionMinWidth));
      await disposeShell(tester, c);
    });

    testWidgets('the terminal keeps its minimum on a narrow desk', (
      tester,
    ) async {
      final c = await session(tester, size: const Size(1000, 800));
      c.read(sessionInspectorOpenProvider.notifier).state = true;
      await tester.pumpAndSettle();
      await tester.drag(
        find.byKey(const Key('inspector-resize')),
        const Offset(-400, 0),
      );
      await tester.pumpAndSettle();
      expect(terminalWidth(tester), greaterThanOrEqualTo(kSessionMinWidth - 1));
      expect(tester.takeException(), isNull);
      await disposeShell(tester, c);
    });

    testWidgets('the terminal sits 10 px in', (tester) async {
      final c = await session(tester);
      final view = tester.widget<StormTerminalView>(
        find.byType(StormTerminalView),
      );
      expect(view.padding!.left, 10);
      expect(view.padding!.right, 10);
      await disposeShell(tester, c);
    });

    testWidgets('below the design frame the inspector starts closed', (
      tester,
    ) async {
      final c = await session(tester, size: const Size(1100, 800));
      expect(find.byKey(const Key('session-inspector')), findsNothing);
      expect(find.byKey(const Key('inspector-toggle')), findsOneWidget);
      await disposeShell(tester, c);
    });
  });

  group('the session on a phone', () {
    testWidgets('no third column; a 10 px terminal inset', (tester) async {
      final c = await session(tester, size: phone);
      expect(find.byKey(const Key('session-inspector')), findsNothing);
      final view = tester.widget<StormTerminalView>(
        find.byType(StormTerminalView),
      );
      expect(view.padding!.left, 10);
      await disposeShell(tester, c);
    });

    testWidgets('the extra keys come and go with the keyboard', (tester) async {
      final c = await session(tester, size: phone);
      final bottom = tester.getRect(find.byType(StormTerminalView)).bottom;
      expect(find.byKey(const Key('key-esc')), findsNothing);

      tester.view.viewInsets = const FakeViewPadding(bottom: 300);
      await tester.pumpAndSettle();
      final keys = rect(tester, 'key-esc');
      expect(keys.bottom, lessThanOrEqualTo(900 - 300 + 1));

      tester.view.resetViewInsets();
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('key-esc')), findsNothing);
      expect(
        tester.getRect(find.byType(StormTerminalView)).bottom,
        closeTo(bottom, 1),
      );
      await disposeShell(tester, c);
    });
  });

  group('the rail foot', () {
    testWidgets('status and Settings share one square on the rail axis', (
      tester,
    ) async {
      final c = shellContainer();
      await pumpShell(tester, c, size: wide);
      c.read(routerProvider).go(Routes.browse(primary));
      await tester.pumpAndSettle();
      final status = rect(tester, 'rail-status');
      final settings = tester.getRect(
        find.descendant(
          of: find.byKey(const Key('rail-settings')),
          matching: find.byType(InkWell),
        ),
      );
      final notes = rect(tester, 'rail-notes');
      expect(status.size, settings.size);
      expect(status.width, status.height);
      expect(status.center.dx, closeTo(notes.center.dx, 0.5));
      expect(settings.center.dx, closeTo(notes.center.dx, 0.5));
      expect(settings.top - status.bottom, closeTo(8, 0.5));
      expect(900 - settings.bottom, closeTo(8, 0.5));
      await disposeShell(tester, c);
    });
  });

  group('the note editor surface', () {
    Future<ProviderContainer> note(WidgetTester tester, Size size) async {
      final c = shellContainer();
      await pumpShell(tester, c, size: size);
      c.read(routerProvider).go(Routes.note(primary, 'n1'));
      await tester.pumpAndSettle();
      return c;
    }

    Rect headerRow(WidgetTester tester) =>
        tester.getRect(find.byKey(const Key('note-header-row')));

    testWidgets('wide: the header row spans the surface, prose ≤ the measure', (
      tester,
    ) async {
      final c = await note(tester, const Size(2400, 1000));
      final header = headerRow(tester);
      final drawer = tester.getRect(find.text('PROPERTIES')).left;
      expect(header.width, greaterThan(kEditorMeasure));
      expect(drawer - header.right, lessThan(kEditorInset + 30));
      final prose = tester.getRect(
        find.byKey(const Key('storm-markdown-body')),
      );
      expect(prose.width, lessThanOrEqualTo(kEditorMeasure));
      await disposeShell(tester, c);
    });

    testWidgets('closing Properties gives the surface the room', (
      tester,
    ) async {
      final c = await note(tester, const Size(1600, 1000));
      final before = headerRow(tester).width;
      await tester.tap(find.byKey(const Key('drawer-toggle')));
      await tester.pumpAndSettle();
      expect(headerRow(tester).width, greaterThan(before + 200));
      await disposeShell(tester, c);
    });

    testWidgets('a desk below the measure uses the whole surface', (
      tester,
    ) async {
      final c = await note(tester, const Size(1100, 900));
      final header = headerRow(tester);
      final prose = tester.getRect(
        find.byKey(const Key('storm-markdown-body')),
      );
      expect(prose.width, closeTo(header.width, 1));
      await disposeShell(tester, c);
    });

    testWidgets('the phone note is unchanged: one column at the inset', (
      tester,
    ) async {
      final c = await note(tester, phone);
      final prose = tester.getRect(
        find.byKey(const Key('storm-markdown-body')),
      );
      expect(prose.left, closeTo(20, 1));
      expect(prose.right, closeTo(phone.width - 20, 1));
      expect(tester.takeException(), isNull);
      await disposeShell(tester, c);
    });
  });
}
