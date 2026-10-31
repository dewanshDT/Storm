import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:storm/router.dart';
import 'package:storm/ui/shell/storm_scaffold.dart';

import 'shell_harness.dart';
import 'fake_server.dart';

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
    await tester.tap(find.byTooltip('New note'));
    await tester.pumpAndSettle();
  }

  testWidgets('creating a note does not throw', (tester) async {
    final c = shellContainer();
    await pumpShell(tester, c);
    await openVault(tester, c);

    await openDialog(tester);
    expect(find.text('New note'), findsOneWidget, reason: 'dialog should open');

    // A *name*, not a path: the folder comes from where you are and `.md`
    // is added internally.
    await tester.enterText(find.byType(TextField).last, 'Fresh');
    await tester.tap(find.text('Create'));

    // Pump through the dialog's exit animation — the window where the
    // disposed controller was still being used.
    await tester.pumpAndSettle();

    expect(
      tester.takeException(),
      isNull,
      reason: 'the red error screen came from here',
    );
    expect(
      serverOf(c).notes.values.any((n) => n.path == 'Fresh.md'),
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
    await openVault(tester, c);

    await openDialog(tester);
    await tester.enterText(find.byType(TextField).last, 'Fresh');
    await tester.tap(find.text('Create'));
    await tester.pumpAndSettle();

    final created = serverOf(
      c,
    ).notes.values.firstWhere((n) => n.path == 'Fresh.md');
    expect(
      c.read(routerProvider).state.uri.path,
      Routes.note(FakeServer.primaryVault, created.id),
    );
    await disposeShell(tester, c);
  });

  testWidgets('cancelling the dialog does not throw either', (tester) async {
    final c = shellContainer();
    await pumpShell(tester, c);
    await openVault(tester, c);

    await openDialog(tester);
    await tester.tap(find.text('Cancel'));
    await tester.pumpAndSettle();

    expect(tester.takeException(), isNull);
    await disposeShell(tester, c);
  });

  Future<List<String>> create(
    WidgetTester tester,
    String name, {
    String folder = '',
  }) async {
    final c = shellContainer();
    await pumpShell(tester, c);
    await openVault(tester, c);
    if (folder.isNotEmpty) {
      c.read(routerProvider).go(Routes.folder(FakeServer.primaryVault, folder));
      await tester.pumpAndSettle();
    }
    await openDialog(tester);
    await tester.enterText(find.byType(TextField).last, name);
    await tester.pump();
    await tester.tap(find.text('Create'));
    await tester.pumpAndSettle();
    expect(tester.takeException(), isNull);
    final paths = serverOf(c).notes.values.map((n) => n.path).toList();
    await disposeShell(tester, c);
    return paths;
  }

  testWidgets('a / in the name makes the note in that folder', (tester) async {
    expect(
      await create(tester, 'Test Sandbox 2026-10-30/Folder A/One'),
      contains('Test Sandbox 2026-10-30/Folder A/One.md'),
    );
  });

  testWidgets('from inside a folder, a / is relative to it and a leading / '
      'starts at the top', (tester) async {
    expect(
      await create(tester, 'Ideas/Plan', folder: 'Projects'),
      contains('Projects/Ideas/Plan.md'),
    );
    expect(
      await create(tester, '/Ideas/Plan', folder: 'Projects'),
      contains('Ideas/Plan.md'),
    );
  });

  testWidgets('the dialog says where the note will land', (tester) async {
    final c = shellContainer();
    await pumpShell(tester, c);
    await openVault(tester, c);
    c.read(routerProvider).go(Routes.folder(FakeServer.primaryVault, 'Work'));
    await tester.pumpAndSettle();
    await openDialog(tester);
    String where() =>
        tester.widget<Text>(find.byKey(const Key('new-note-where'))).data!;
    expect(where(), 'in Work');
    expect(find.byKey(const Key('new-note-hint')), findsOneWidget);
    await tester.enterText(find.byType(TextField).last, 'Q1/Plan');
    await tester.pump();
    expect(where(), 'in Work/Q1');
    await tester.enterText(find.byType(TextField).last, '/Plan');
    await tester.pump();
    expect(where(), 'at the top of the vault');
    await tester.tap(find.text('Cancel'));
    await tester.pumpAndSettle();
    await disposeShell(tester, c);
  });

  for (final (name, why) in [
    ('../Q1/Plan', 'can\'t use'),
    ('Q1/', 'after the last /'),
  ]) {
    testWidgets('"$name" is refused, says why, and makes nothing', (
      tester,
    ) async {
      final c = shellContainer();
      await pumpShell(tester, c);
      await openVault(tester, c);
      final before = serverOf(c).notes.length;
      await openDialog(tester);
      await tester.enterText(find.byType(TextField).last, name);
      await tester.tap(find.text('Create'));
      await tester.pumpAndSettle();
      expect(find.text('New note'), findsOneWidget, reason: 'still open');
      expect(find.textContaining(why), findsOneWidget);
      expect(serverOf(c).notes.length, before);
      await tester.tap(find.text('Cancel'));
      await tester.pumpAndSettle();
      await disposeShell(tester, c);
    });
  }

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
