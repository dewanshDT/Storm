import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:storm/router.dart';

import 'shell_harness.dart';

/// The MCP switch on the server settings screen.
///
/// The thing worth testing here is not that a `Switch` renders. It is that the
/// switch reflects and changes *the server's* state: MCP is a way into every
/// vault, so a control that looked local, or that showed "on" while the server
/// had it off, would be worse than no control at all.
void main() {
  Future<void> openSettings(WidgetTester tester, container) async {
    container.read(routerProvider).go(Routes.serverSettings);
    await tester.pumpAndSettle();
  }

  final switchFinder = find.byType(SwitchListTile);
  // The first switch is the endpoint, the second is write access.
  Finder readSwitch() => switchFinder.at(0);
  Finder writeSwitch() => switchFinder.at(1);
  bool valueOf(WidgetTester t, Finder f) => t.widget<SwitchListTile>(f).value;

  testWidgets('shows the server as off, and says what off means', (
    tester,
  ) async {
    final c = shellContainer();
    await pumpShell(tester, c);
    await openSettings(tester, c);

    expect(serverOf(c).mcpEnabled, isFalse, reason: 'precondition');
    expect(valueOf(tester, readSwitch()), isFalse);
    expect(find.textContaining('refuses every request'), findsOneWidget);
    await disposeShell(tester, c);
  });

  testWidgets('shows the server as on when it is', (tester) async {
    final c = shellContainer();
    serverOf(c).mcpEnabled = true;
    await pumpShell(tester, c);
    await openSettings(tester, c);

    expect(valueOf(tester, readSwitch()), isTrue);
    expect(find.textContaining('Serving at /mcp'), findsOneWidget);
    await disposeShell(tester, c);
  });

  testWidgets('turning it on reaches the server, not just the widget', (
    tester,
  ) async {
    // The failure this guards: a switch that flips locally and never sends the
    // request looks identical on screen, and the vault stays unreachable — or,
    // worse in the other direction, stays reachable after being switched off.
    final c = shellContainer();
    await pumpShell(tester, c);
    await openSettings(tester, c);

    await tester.tap(readSwitch());
    await tester.pumpAndSettle();

    expect(serverOf(c).mcpEnabled, isTrue, reason: 'the server was told');
    expect(valueOf(tester, readSwitch()), isTrue);
    await disposeShell(tester, c);
  });

  testWidgets('turning it off reaches the server too', (tester) async {
    final c = shellContainer();
    serverOf(c).mcpEnabled = true;
    await pumpShell(tester, c);
    await openSettings(tester, c);

    await tester.tap(readSwitch());
    await tester.pumpAndSettle();

    expect(serverOf(c).mcpEnabled, isFalse);
    expect(valueOf(tester, readSwitch()), isFalse);
    await disposeShell(tester, c);
  });

  testWidgets('a server that fails the request does not lie about the state', (
    tester,
  ) async {
    // Showing "on" after a failed request is the dangerous direction: the user
    // believes AI access is off while the server is still serving it.
    final c = shellContainer();
    final server = serverOf(c);
    await pumpShell(tester, c);
    await openSettings(tester, c);
    // Set after the screen has loaded: `failWith` refuses every request, and
    // the settings screen has to be able to read the config first.
    server.failWith = 500;

    await tester.tap(readSwitch());
    await tester.pumpAndSettle();

    expect(server.mcpEnabled, isFalse, reason: 'the write failed');
    expect(
      valueOf(tester, readSwitch()),
      isFalse,
      reason: 'so the switch must not claim otherwise',
    );
    await disposeShell(tester, c);
  });

  testWidgets('an older server without the field reads as off', (tester) async {
    // `mcp_enabled` is absent from a server built before this existed. Reading
    // that as "on" would offer a switch that cannot work.
    final c = shellContainer();
    serverOf(c).omitMcpField = true;
    await pumpShell(tester, c);
    await openSettings(tester, c);

    expect(valueOf(tester, readSwitch()), isFalse);
    await disposeShell(tester, c);
  });

  group('write access', () {
    testWidgets('is off, and disabled, while the endpoint is off', (
      tester,
    ) async {
      final c = shellContainer();
      await pumpShell(tester, c);
      await openSettings(tester, c);

      expect(valueOf(tester, writeSwitch()), isFalse);
      expect(
        tester.widget<SwitchListTile>(writeSwitch()).onChanged,
        isNull,
        reason: 'write access without the endpoint means nothing',
      );
      await disposeShell(tester, c);
    });

    testWidgets('turning it on reaches the server', (tester) async {
      final c = shellContainer();
      serverOf(c).mcpEnabled = true;
      await pumpShell(tester, c);
      await openSettings(tester, c);

      await tester.tap(writeSwitch());
      await tester.pumpAndSettle();

      expect(serverOf(c).mcpWritable, isTrue);
      expect(valueOf(tester, writeSwitch()), isTrue);
      await disposeShell(tester, c);
    });

    testWidgets('switching the endpoint off disarms writes', (tester) async {
      // Otherwise turning MCP back on later would silently restore write
      // access the user believed they had revoked.
      final c = shellContainer();
      final server = serverOf(c);
      server.mcpEnabled = true;
      server.mcpWritable = true;
      await pumpShell(tester, c);
      await openSettings(tester, c);
      expect(valueOf(tester, writeSwitch()), isTrue, reason: 'precondition');

      await tester.tap(readSwitch());
      await tester.pumpAndSettle();

      expect(server.mcpEnabled, isFalse);
      expect(server.mcpWritable, isFalse, reason: 'disarmed with the endpoint');
      expect(valueOf(tester, writeSwitch()), isFalse);
      await disposeShell(tester, c);
    });

    testWidgets('says a deleted note is not recoverable', (tester) async {
      // Storm has no trash. The screen is the only place that says so before
      // an agent is given permission to delete.
      final c = shellContainer();
      final server = serverOf(c);
      server.mcpEnabled = true;
      server.mcpWritable = true;
      await pumpShell(tester, c);
      await openSettings(tester, c);

      expect(find.textContaining('no trash'), findsOneWidget);
      await disposeShell(tester, c);
    });
  });
}
