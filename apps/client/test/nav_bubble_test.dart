import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:storm/router.dart';
import 'package:storm/ui/editor_toolbar.dart';

import 'shell_harness.dart';
import 'fake_server.dart';

/// The navigation bubble.
///
/// Its whole design rests on two claims: expansion is widget-local, and the
/// Context slot derives from the route rather than a parallel flag. Both are
/// easy to break silently, so both are asserted here.
void main() {
  group('expansion', () {
    testWidgets('starts collapsed and opens on tap', (tester) async {
      final c = shellContainer();
      await pumpShell(tester, c);
      // Inside a vault: the browse/search/new-note slots only exist there.
      // On the dashboard the bubble offers vault-level actions instead.
      await openVault(tester, c);

      expect(find.byIcon(Icons.more_horiz), findsOneWidget);
      expect(find.byIcon(Icons.search), findsNothing);

      await tester.tap(find.byIcon(Icons.more_horiz));
      await tester.pumpAndSettle();

      expect(find.byTooltip('Directory'), findsOneWidget);
      expect(find.byTooltip('Search'), findsOneWidget);
      expect(find.byTooltip('New note'), findsOneWidget);
      expect(find.byIcon(Icons.more_horiz), findsNothing);
      await disposeShell(tester, c);
    });

    testWidgets('closes again', (tester) async {
      final c = shellContainer();
      await pumpShell(tester, c);
      await openVault(tester, c);

      await tester.tap(find.byIcon(Icons.more_horiz));
      await tester.pumpAndSettle();
      await tester.tap(find.byTooltip('Close'));
      await tester.pumpAndSettle();

      expect(find.byIcon(Icons.more_horiz), findsOneWidget);
      expect(find.byIcon(Icons.search), findsNothing);
      await disposeShell(tester, c);
    });

    testWidgets('collapses when it navigates', (tester) async {
      // Leaving it open would land the next screen with a menu already
      // covering its content.
      final c = shellContainer();
      await pumpShell(tester, c);
      await openVault(tester, c);

      await tester.tap(find.byIcon(Icons.more_horiz));
      await tester.pumpAndSettle();
      await tester.tap(find.byTooltip('Directory'));
      await tester.pumpAndSettle();

      expect(find.text('Vault'), findsOneWidget);
      expect(find.byIcon(Icons.more_horiz), findsOneWidget);
      await disposeShell(tester, c);
    });
  });

  testWidgets('is hidden while the keyboard is up', (tester) async {
    final c = shellContainer();
    await pumpShell(tester, c, keyboard: 300);

    expect(find.byIcon(Icons.more_horiz), findsNothing);
    await disposeShell(tester, c);
  });

  testWidgets('trades places with the formatting toolbar in a note', (
    tester,
  ) async {
    // Both read the same signal, so they must never be on screen together and
    // never both absent.
    final c = shellContainer();
    await pumpShell(tester, c);
    c.read(routerProvider).go(Routes.note(FakeServer.primaryVault, 'n0'));
    await tester.pumpAndSettle();

    expect(find.byType(EditorToolbar), findsNothing);
    expect(find.byIcon(Icons.more_horiz), findsOneWidget);

    await pumpShell(tester, c, keyboard: 320);
    await tester.pumpAndSettle();

    expect(find.byType(EditorToolbar), findsOneWidget);
    expect(find.byIcon(Icons.more_horiz), findsNothing);
    await disposeShell(tester, c);
  });

  group('the context slot', () {
    testWidgets('offers tags outside a note', (tester) async {
      final c = shellContainer();
      await pumpShell(tester, c);
      await openVault(tester, c);

      await tester.tap(find.byIcon(Icons.more_horiz));
      await tester.pumpAndSettle();

      expect(find.byTooltip('Tags'), findsOneWidget);
      expect(find.byIcon(Icons.hub_outlined), findsNothing);
      await disposeShell(tester, c);
    });

    testWidgets('offers linked mentions inside a note', (tester) async {
      final c = shellContainer();
      await pumpShell(tester, c);

      c.read(routerProvider).go(Routes.note(FakeServer.primaryVault, 'n0'));
      await tester.pumpAndSettle();
      await tester.tap(find.byIcon(Icons.more_horiz));
      await tester.pumpAndSettle();

      expect(find.byIcon(Icons.hub_outlined), findsOneWidget);
      expect(find.byTooltip('Tags'), findsNothing);
      await disposeShell(tester, c);
    });
  });
}
