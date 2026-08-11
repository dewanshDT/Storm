import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:storm/editor/markdown_theme.dart';
import 'package:storm/editor/storm_markdown_controller.dart';
import 'package:storm/keyboard/storm_activators.dart';
import 'package:storm/keyboard/storm_editor_shortcuts.dart';
import 'package:storm/keyboard/storm_global_shortcuts.dart';
import 'package:storm/keyboard/storm_intents.dart';
import 'package:storm/router.dart';
import 'package:storm/state/app_state.dart';
import 'package:storm/state/note_session.dart';
import 'package:storm/ui/shell/vault_sidebar.dart';
import 'package:storm/ui/theme.dart';

import 'fake_server.dart';
import 'shell_harness.dart';

void main() {
  tearDown(() {
    debugDefaultTargetPlatformOverride = null;
  });

  group('stormActivator', () {
    test('uses Meta on macOS', () {
      debugDefaultTargetPlatformOverride = TargetPlatform.macOS;
      final a = stormActivator(LogicalKeyboardKey.keyK);
      expect(a.meta, isTrue);
      expect(a.control, isFalse);
      expect(stormUsesMetaModifier, isTrue);
    });

    test('uses Control on Windows', () {
      debugDefaultTargetPlatformOverride = TargetPlatform.windows;
      final a = stormActivator(LogicalKeyboardKey.keyK, shift: true);
      expect(a.meta, isFalse);
      expect(a.control, isTrue);
      expect(a.shift, isTrue);
      expect(stormUsesMetaModifier, isFalse);
    });
  });

  /// A context *inside* the global shortcuts' Actions subtree.
  ///
  /// The chords live in `VaultShell`, above the pane. `Actions.invoke` walks
  /// *up* from the given context, so a context from the shortcuts widget
  /// itself — or from anywhere above them — misses the handler; a Scaffold
  /// below them is the reliable way in.
  BuildContext globalContext(WidgetTester tester) => tester.element(
    find
        .descendant(
          of: find.byType(StormGlobalShortcuts),
          matching: find.byType(Scaffold),
        )
        .first,
  );

  /// A context inside the note-level shortcuts, which wrap the editor chrome.
  BuildContext noteContext(WidgetTester tester) =>
      tester.element(find.byKey(const Key('note-actions')));

  group('sidebar toggle', () {
    const desk = Size(1280, 900);
    const phone = Size(411, 900);

    testWidgets('hides the rail at desk width', (tester) async {
      final c = shellContainer();
      await pumpShell(tester, c, size: desk);
      await openVault(tester, c);
      expect(find.byType(VaultSidebar), findsOneWidget);
      expect(c.read(sidebarCollapsedProvider), isFalse);

      final ctx = globalContext(tester);
      Actions.invoke(ctx, const StormToggleSidebarIntent());
      await tester.pumpAndSettle();

      expect(c.read(sidebarCollapsedProvider), isTrue);
      expect(find.byType(VaultSidebar), findsNothing);

      Actions.invoke(ctx, const StormToggleSidebarIntent());
      await tester.pumpAndSettle();
      expect(find.byType(VaultSidebar), findsOneWidget);

      await disposeShell(tester, c);
    });

    testWidgets('is a no-op at phone width', (tester) async {
      final c = shellContainer();
      await pumpShell(tester, c, size: phone);
      await openVault(tester, c);
      expect(find.byType(VaultSidebar), findsNothing);

      final ctx = globalContext(tester);
      Actions.invoke(ctx, const StormToggleSidebarIntent());
      await tester.pumpAndSettle();

      expect(c.read(sidebarCollapsedProvider), isFalse);
      expect(find.byType(VaultSidebar), findsNothing);
      await disposeShell(tester, c);
    });
  });

  group('global shortcuts', () {
    testWidgets('K opens vault search', (tester) async {
      final c = shellContainer();
      await pumpShell(tester, c, size: const Size(1280, 900));
      await openVault(tester, c);

      final ctx = globalContext(tester);
      Actions.invoke(ctx, const StormSearchIntent());
      await tester.pumpAndSettle();

      final path = c.read(routerProvider).state.uri.path;
      expect(path, Routes.search(FakeServer.primaryVault));
      await disposeShell(tester, c);
    });

    testWidgets('N invokes the same create path as the nav pill', (
      tester,
    ) async {
      final c = shellContainer();
      await pumpShell(tester, c, size: const Size(1280, 900));
      await openVault(tester, c);

      // The chord reaches NewNoteRequest, which is what the pill and the
      // sidebar toolbar call — the dialog appearing is the proof.
      final ctx = globalContext(tester);
      Actions.invoke(ctx, const StormNewNoteIntent());
      await tester.pumpAndSettle();
      expect(find.text('New note'), findsOneWidget);
      await disposeShell(tester, c);
    });

    testWidgets('⇧N invokes the new-folder path', (tester) async {
      final c = shellContainer();
      await pumpShell(tester, c, size: const Size(1280, 900));
      await openVault(tester, c);

      final ctx = globalContext(tester);
      Actions.invoke(ctx, const StormNewFolderIntent());
      await tester.pumpAndSettle();
      expect(find.text('New folder'), findsOneWidget);
      await disposeShell(tester, c);
    });
  });

  group('the create buttons reach the chords\' callbacks', () {
    Future<void> openNote(WidgetTester tester, ProviderContainer c) async {
      final noteId = serverOf(c).notes.keys.first;
      c.read(routerProvider).go(Routes.note(FakeServer.primaryVault, noteId));
      await tester.pumpAndSettle();
    }

    testWidgets('the desk-width sidebar New note button opens the dialog', (
      tester,
    ) async {
      // The pre-M18 bug: at desk width the browse pane is the empty pane
      // beside the sidebar and nothing provided NewNoteRequest, so this button
      // rendered but did nothing. The chords and the button share the callback
      // now — the dialog appearing is the proof.
      final c = shellContainer();
      await pumpShell(tester, c, size: const Size(1280, 900));
      await openVault(tester, c);

      await tester.tap(find.byTooltip('New note'));
      await tester.pumpAndSettle();
      expect(find.text('New note'), findsOneWidget);
      await disposeShell(tester, c);
    });

    testWidgets('the phone nav pill inside a note opens the dialog', (
      tester,
    ) async {
      // The note screen used to wrap itself in an *empty* NewNoteRequest, so
      // the pill's create button was dead exactly where a phone user makes
      // notes from. It resolves the vault shell's callback now.
      final c = shellContainer();
      await pumpShell(tester, c);
      await openVault(tester, c);
      await openNote(tester, c);

      await tester.tap(find.byTooltip('New note'));
      await tester.pumpAndSettle();
      expect(find.text('New note'), findsOneWidget);
      await disposeShell(tester, c);
    });
  });

  group('note shortcuts', () {
    Future<void> openNote(WidgetTester tester, ProviderContainer c) async {
      final noteId = serverOf(c).notes.keys.first;
      c.read(routerProvider).go(Routes.note(FakeServer.primaryVault, noteId));
      await tester.pumpAndSettle();
    }

    testWidgets('E toggles Read and Edit', (tester) async {
      final c = shellContainer();
      await pumpShell(tester, c, size: const Size(1280, 900));
      await openVault(tester, c);
      await openNote(tester, c);

      // Read Mode is the default, so the body field is not in the tree.
      expect(find.byKey(const Key('note-body')), findsNothing);

      Actions.invoke(noteContext(tester), const StormToggleReadEditIntent());
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('note-body')), findsOneWidget);

      Actions.invoke(noteContext(tester), const StormToggleReadEditIntent());
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('note-body')), findsNothing);
      await disposeShell(tester, c);
    });

    testWidgets('S saves the note now', (tester) async {
      final c = shellContainer();
      await pumpShell(tester, c, size: const Size(1280, 900));
      await openVault(tester, c);
      await openNote(tester, c);

      final session = c.read(noteSessionProvider);
      final noteId = session.noteId;
      final before = serverOf(c).notes[noteId]!;

      session.editBody('typed through the shortcut test');
      expect(session.isDirty, isTrue);

      Actions.invoke(noteContext(tester), const StormSaveNoteIntent());
      await tester.pumpAndSettle();

      expect(session.saveState, SaveState.saved);
      expect(
        serverOf(c).notes[noteId]!.content,
        contains('typed through the shortcut test'),
        reason: 'the server holds what the chord saved',
      );
      expect(serverOf(c).notes[noteId]!.version, greaterThan(before.version));
      await disposeShell(tester, c);
    });

    testWidgets('F opens the find bar and Esc closes it', (tester) async {
      final c = shellContainer();
      await pumpShell(tester, c, size: const Size(1280, 900));
      await openVault(tester, c);
      await openNote(tester, c);

      expect(find.byKey(const Key('note-find-field')), findsNothing);

      Actions.invoke(noteContext(tester), const StormFindInNoteIntent());
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('note-find-field')), findsOneWidget);

      Actions.invoke(noteContext(tester), const StormDismissIntent());
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('note-find-field')), findsNothing);
      await disposeShell(tester, c);
    });

    testWidgets('Esc with no find bar leaves the note', (tester) async {
      final c = shellContainer();
      await pumpShell(tester, c, size: const Size(1280, 900));
      await openVault(tester, c);
      await openNote(tester, c);

      expect(find.byKey(const Key('note-read')), findsOneWidget);
      Actions.invoke(noteContext(tester), const StormDismissIntent());
      await tester.pumpAndSettle();

      expect(
        find.byKey(const Key('note-read')),
        findsNothing,
        reason: 'Esc with the find bar closed leaves the note',
      );
      await disposeShell(tester, c);
    });

    testWidgets('Esc in Edit steps back to Read before leaving', (
      tester,
    ) async {
      final c = shellContainer();
      await pumpShell(tester, c, size: const Size(1280, 900));
      await openVault(tester, c);
      await openNote(tester, c);
      await enterEditMode(tester);
      expect(find.byKey(const Key('note-body')), findsOneWidget);

      // First Esc: out of writing, back to Read — the note stays put.
      Actions.invoke(noteContext(tester), const StormDismissIntent());
      await tester.pumpAndSettle();
      expect(
        find.byKey(const Key('note-read')),
        findsOneWidget,
        reason: 'the first Esc steps out of Edit, not out of the note',
      );
      expect(find.byKey(const Key('note-body')), findsNothing);

      // Second Esc: no find bar, no editor — now it leaves.
      Actions.invoke(noteContext(tester), const StormDismissIntent());
      await tester.pumpAndSettle();
      expect(
        find.byKey(const Key('note-read')),
        findsNothing,
        reason: 'the second Esc leaves the note',
      );
      await disposeShell(tester, c);
    });

    testWidgets('E is a no-op when Read Mode is disabled', (tester) async {
      final c = shellContainer();
      await c.read(settingsProvider.future);
      await c
          .read(settingsProvider.notifier)
          .save(c.read(settingsProvider).value!.copyWith(readMode: false));
      await pumpShell(tester, c, size: const Size(1280, 900));
      await openVault(tester, c);
      await openNote(tester, c);

      // readMode off → the note opens in Edit and there is no Read to return to.
      expect(find.byKey(const Key('note-body')), findsOneWidget);
      Actions.invoke(noteContext(tester), const StormToggleReadEditIntent());
      await tester.pumpAndSettle();
      expect(
        find.byKey(const Key('note-body')),
        findsOneWidget,
        reason: 'E must not switch into a Read Mode the user turned off',
      );
      await disposeShell(tester, c);
    });

    testWidgets('find matches case-insensitively, steps, and jumps to Edit', (
      tester,
    ) async {
      final c = shellContainer();
      await pumpShell(tester, c, size: const Size(1280, 900));
      await openVault(tester, c);
      await openNote(tester, c);

      Actions.invoke(noteContext(tester), const StormFindInNoteIntent());
      await tester.pumpAndSettle();

      // The note body is `# Welcome\n\nbody\n` — two e's in Welcome, and the
      // query is uppercase to prove the match is case-insensitive.
      await tester.enterText(find.byKey(const Key('note-find-field')), 'E');
      await tester.pumpAndSettle();

      expect(
        find.byKey(const Key('note-body')),
        findsOneWidget,
        reason: 'selecting the first match jumps out of Read so it is visible',
      );
      expect(find.text('1/2'), findsOneWidget, reason: 'first of two matches');

      await tester.tap(find.byKey(const Key('note-find-next')));
      await tester.pumpAndSettle();
      expect(find.text('2/2'), findsOneWidget, reason: 'stepped to the second');

      await tester.tap(find.byKey(const Key('note-find-prev')));
      await tester.pumpAndSettle();
      expect(find.text('1/2'), findsOneWidget, reason: 'previous wraps around');
      await disposeShell(tester, c);
    });
  });

  group('editor shortcuts', () {
    testWidgets('B and I wrap the selection while the body field is focused', (
      tester,
    ) async {
      final controller = StormMarkdownController(
        theme: MarkdownTheme.dark(const TextStyle()),
        text: 'hello world',
      );
      controller.selection = const TextSelection(
        baseOffset: 0,
        extentOffset: 5,
      );

      await tester.pumpWidget(
        MaterialApp(
          theme: StormTheme.dark(),
          home: Scaffold(
            body: StormEditorShortcuts(
              controller: controller,
              child: TextField(
                key: const Key('note-body'),
                controller: controller,
                autofocus: true,
              ),
            ),
          ),
        ),
      );
      await tester.pumpAndSettle();

      // Context from inside the editor's subtree, so the intent resolves.
      final ctx = tester.element(find.byKey(const Key('note-body')));
      Actions.invoke(ctx, const StormBoldIntent());
      await tester.pumpAndSettle();
      expect(controller.text, '**hello** world');

      // Italic on a fresh selection, so the two markers do not collide.
      controller.value = const TextEditingValue(
        text: 'hello world',
        selection: TextSelection(baseOffset: 0, extentOffset: 5),
      );
      Actions.invoke(ctx, const StormItalicIntent());
      await tester.pumpAndSettle();
      expect(controller.text, '*hello* world');
    });
  });
}
