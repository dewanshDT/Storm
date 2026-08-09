import 'package:flutter_lucide/flutter_lucide.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:storm/router.dart';
import 'package:storm/ui/shell/nav_bubble.dart';
import 'package:storm/ui/editor_toolbar.dart';

import 'shell_harness.dart';
import 'fake_server.dart';

/// The navigation bubble.
///
/// Its whole design rests on two claims: expansion is widget-local, and the
/// Context slot derives from the route rather than a parallel flag. Both are
/// easy to break silently, so both are asserted here.
void main() {
  group('the bar is always open', () {
    testWidgets('every slot is reachable without a tap first', (tester) async {
      // It used to collapse to a single `…`, costing a tap before every
      // navigation and hiding where you could go.
      final c = shellContainer();
      await pumpShell(tester, c);
      await openVault(tester, c);

      expect(find.byTooltip('Directory'), findsOneWidget);
      expect(find.byTooltip('Search'), findsOneWidget);
      expect(find.byTooltip('New note'), findsOneWidget);
      // No `…` to tap: nothing collapses any more.
      expect(find.byIcon(LucideIcons.ellipsis), findsNothing);
      await disposeShell(tester, c);
    });

    testWidgets('stays open after navigating', (tester) async {
      final c = shellContainer();
      await pumpShell(tester, c);
      await openVault(tester, c);

      await tester.tap(find.byTooltip('Directory'));
      await tester.pumpAndSettle();

      expect(find.text('Vaults'), findsOneWidget);
      expect(find.byTooltip('Directory'), findsOneWidget);
      await disposeShell(tester, c);
    });
  });

  testWidgets('is hidden while the keyboard is up', (tester) async {
    final c = shellContainer();
    await pumpShell(tester, c, keyboard: 300);

    expect(find.byType(NavBubble), findsNothing);
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
    expect(find.byType(NavBubble), findsOneWidget);

    await pumpShell(tester, c, keyboard: 320);
    await tester.pumpAndSettle();

    expect(find.byType(EditorToolbar), findsOneWidget);
    expect(find.byType(NavBubble), findsNothing);
    await disposeShell(tester, c);
  });

  group('the context slot', () {
    testWidgets('keeps mentions and tags visible outside a note', (
      tester,
    ) async {
      final c = shellContainer();
      await pumpShell(tester, c);
      await openVault(tester, c);

      expect(find.byTooltip('Mentions'), findsOneWidget);
      expect(find.byTooltip('Tags'), findsOneWidget);
      await disposeShell(tester, c);
    });

    testWidgets('shows the linked-mentions count inside a note', (
      tester,
    ) async {
      final c = shellContainer();
      await pumpShell(tester, c);

      c.read(routerProvider).go(Routes.note(FakeServer.primaryVault, 'n0'));
      await tester.pumpAndSettle();
      expect(find.byTooltip('0 linked mentions'), findsOneWidget);
      expect(find.byTooltip('Tags'), findsOneWidget);
      await disposeShell(tester, c);
    });
  });
}
