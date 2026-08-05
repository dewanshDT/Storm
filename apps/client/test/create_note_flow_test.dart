import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:storm/router.dart';
import 'package:storm/ui/shell/storm_scaffold.dart';

import 'shell_harness.dart';

/// The "new note" flow, end to end through the real screens.
///
/// Creating a note showed a red error screen even though the note was created
/// — the dialog's `TextEditingController` was disposed the moment
/// `showDialog` returned, while the route was still animating out and the
/// `TextField` was still mounted. The next rebuild of that field threw
/// "A TextEditingController was used after being disposed".
///
/// These tests pump *past* the dialog's exit animation, which is what makes
/// the failure visible. The shell changed underneath them; the bug they guard
/// against did not, so they follow the flow to where it now lives — behind the
/// nav bubble rather than an app-bar button.
void main() {
  /// Opens the new-note dialog the way a user reaches it.
  Future<void> openDialog(WidgetTester tester) async {
    await tester.tap(find.byIcon(Icons.more_horiz));
    await tester.pumpAndSettle();
    await tester.tap(find.byIcon(Icons.add));
    await tester.pumpAndSettle();
  }

  testWidgets('creating a note does not throw', (tester) async {
    final c = shellContainer();
    await pumpShell(tester, c);

    await openDialog(tester);
    expect(find.text('New note'), findsOneWidget, reason: 'dialog should open');

    await tester.enterText(find.byType(TextField).last, 'Notes/Fresh.md');
    await tester.tap(find.text('OK'));

    // Pump through the dialog's exit animation — the window where the
    // disposed controller was still being used.
    await tester.pumpAndSettle();

    expect(
      tester.takeException(),
      isNull,
      reason: 'the red error screen came from here',
    );
    expect(
      serverOf(c).notes.values.any((n) => n.path == 'Notes/Fresh.md'),
      isTrue,
      reason: 'and the note should actually be created',
    );

    await disposeShell(tester, c);
  });

  testWidgets('creating a note opens it', (tester) async {
    // New behaviour worth pinning down: the route now decides which note is
    // open, so creating one has to navigate rather than mutate a flag.
    final c = shellContainer();
    await pumpShell(tester, c);

    await openDialog(tester);
    await tester.enterText(find.byType(TextField).last, 'Notes/Fresh.md');
    await tester.tap(find.text('OK'));
    await tester.pumpAndSettle();

    final created = serverOf(c).notes.values.firstWhere(
          (n) => n.path == 'Notes/Fresh.md',
        );
    expect(c.read(routerProvider).state.uri.path, Routes.note(created.id));
    await disposeShell(tester, c);
  });

  testWidgets('cancelling the dialog does not throw either', (tester) async {
    final c = shellContainer();
    await pumpShell(tester, c);

    await openDialog(tester);
    await tester.tap(find.text('Cancel'));
    await tester.pumpAndSettle();

    expect(tester.takeException(), isNull);
    await disposeShell(tester, c);
  });

  testWidgets('an invalid path is rejected without closing the dialog',
      (tester) async {
    final c = shellContainer();
    await pumpShell(tester, c);

    await openDialog(tester);
    await tester.enterText(find.byType(TextField).last, '../escape.md');
    await tester.tap(find.text('OK'));
    await tester.pumpAndSettle();

    expect(tester.takeException(), isNull);
    expect(
      find.text('New note'),
      findsOneWidget,
      reason: 'the dialog stays open so the name can be corrected',
    );
    expect(
      serverOf(c).notes.values.any((n) => n.path.contains('escape')),
      isFalse,
    );

    await disposeShell(tester, c);
  });

  group('path validation mirrors the server rules', () {
    test('accepts ordinary paths', () {
      expect(validateVaultPath('Note.md'), isNull);
      expect(validateVaultPath('Daily/2026/08/05.md'), isNull);
      expect(validateVaultPath('A note with spaces.md'), isNull);
      expect(validateVaultPath('日本語/ノート.md'), isNull);
    });

    test('rejects what the server would reject', () {
      // Caught here so the user gets a clear message instead of a 400.
      expect(validateVaultPath(''), isNotNull);
      expect(validateVaultPath('/absolute.md'), isNotNull);
      expect(validateVaultPath('../escape.md'), isNotNull);
      expect(validateVaultPath('Notes/../../escape.md'), isNotNull);
      expect(validateVaultPath('.hidden/x.md'), isNotNull);
      expect(validateVaultPath('Notes/.obsidian/x.md'), isNotNull);
      expect(validateVaultPath('Notes//double.md'), isNotNull);
    });
  });
}
