import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:storm/router.dart';

import 'shell_harness.dart';

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

      expect(find.byIcon(Icons.more_horiz), findsOneWidget);
      expect(find.byIcon(Icons.search), findsNothing);

      await tester.tap(find.byIcon(Icons.more_horiz));
      await tester.pumpAndSettle();

      expect(find.byIcon(Icons.folder_outlined), findsOneWidget);
      expect(find.byIcon(Icons.search), findsOneWidget);
      expect(find.byIcon(Icons.add), findsOneWidget);
      expect(find.byIcon(Icons.more_horiz), findsNothing);
      await disposeShell(tester, c);
    });

    testWidgets('closes again', (tester) async {
      final c = shellContainer();
      await pumpShell(tester, c);

      await tester.tap(find.byIcon(Icons.more_horiz));
      await tester.pumpAndSettle();
      await tester.tap(find.byIcon(Icons.close));
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

      await tester.tap(find.byIcon(Icons.more_horiz));
      await tester.pumpAndSettle();
      await tester.tap(find.byIcon(Icons.folder_outlined));
      await tester.pumpAndSettle();

      expect(find.text('Vault'), findsOneWidget);
      expect(find.byIcon(Icons.more_horiz), findsOneWidget);
      await disposeShell(tester, c);
    });
  });

  testWidgets('is hidden while the keyboard is up', (tester) async {
    // Stage 2 puts the formatting toolbar in this space.
    final c = shellContainer();
    await pumpShell(tester, c, keyboard: 300);

    expect(find.byIcon(Icons.more_horiz), findsNothing);
    await disposeShell(tester, c);
  });

  group('the context slot', () {
    testWidgets('offers tags outside a note', (tester) async {
      final c = shellContainer();
      await pumpShell(tester, c);

      await tester.tap(find.byIcon(Icons.more_horiz));
      await tester.pumpAndSettle();

      expect(find.byIcon(Icons.label_outline), findsOneWidget);
      expect(find.byIcon(Icons.hub_outlined), findsNothing);
      await disposeShell(tester, c);
    });

    testWidgets('offers linked mentions inside a note', (tester) async {
      final c = shellContainer();
      await pumpShell(tester, c);

      c.read(routerProvider).go(Routes.note('n0'));
      await tester.pumpAndSettle();
      await tester.tap(find.byIcon(Icons.more_horiz));
      await tester.pumpAndSettle();

      expect(find.byIcon(Icons.hub_outlined), findsOneWidget);
      expect(find.byIcon(Icons.label_outline), findsNothing);
      await disposeShell(tester, c);
    });
  });
}
