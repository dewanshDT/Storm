import 'package:drift/native.dart';
import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:storm/api/storm_api.dart';
import 'package:storm/cache/cache_db.dart';
import 'package:storm/editor/markdown_theme.dart';
import 'package:storm/editor/storm_markdown_controller.dart';
import 'package:storm/state/app_state.dart';
import 'package:storm/sync/sync_engine.dart';
import 'package:storm/ui/note_editor.dart';

import 'fake_server.dart';

/// Guards against the save loop that wiped every note in the vault.
///
/// The chain was: `NoteEditor.build()` assigned a freshly constructed
/// `MarkdownTheme` on every frame; `MarkdownTheme` had no value equality, so
/// the setter always saw a change; the setter called `notifyListeners()`;
/// the editor's listener treats a notification as typing and reported the
/// still-empty controller as an edit; the empty text was saved; the server
/// bumped the version and pushed it back; the editor rebuilt and did it again.
///
/// Result: notes truncated to their frontmatter, versions climbing on their
/// own, and "Saved"/"Unsaved" flickering about twice a second.
void main() {
  late CacheDb cache;
  late FakeServer server;

  const body = '# Ideas\n\nSomeday: #maybe\n';

  ProviderContainer container() {
    cache = CacheDb(NativeDatabase.memory());
    server = FakeServer();
    server.notes['n1'] = ServerNote(
      id: 'n1',
      path: 'Projects/Ideas.md',
      content: body,
      version: 1,
    );
    final api = StormApi(
      baseUrl: 'http://test',
      token: 't',
      client: server.client,
    );
    return ProviderContainer(
      overrides: [
        cacheProvider.overrideWithValue(cache),
        apiProvider.overrideWithValue(api),
        // An engine that was never `start()`ed, so it opens no WebSocket.
        // The real one schedules a reconnect timer when the socket fails,
        // and `testWidgets` fails any test that leaves a timer pending.
        syncEngineProvider.overrideWith(
          (ref) => SyncEngine(api: api, cache: cache),
        ),
      ],
    );
  }

  Future<void> pumpEditor(WidgetTester tester, ProviderContainer c) async {
    await tester.pumpWidget(
      UncontrolledProviderScope(
        container: c,
        child: const MaterialApp(home: Scaffold(body: NoteEditor())),
      ),
    );
  }

  testWidgets('rebuilding the editor never writes an empty note', (
    tester,
  ) async {
    final c = container();
    final session = c.read(noteSessionProvider);
    await session.open('n1');
    await pumpEditor(tester, c);
    await tester.pump();

    final versionAfterOpen = server.notes['n1']!.version;

    // Frames keep coming in a real app — theme changes, status ticks,
    // animations. None of them are edits.
    for (var i = 0; i < 10; i++) {
      await tester.pump(const Duration(milliseconds: 200));
    }
    // Well past the save debounce.
    await tester.pump(const Duration(seconds: 2));

    expect(
      server.notes['n1']!.content,
      body,
      reason: 'the note must be untouched by rebuilds',
    );
    expect(
      server.notes['n1']!.version,
      versionAfterOpen,
      reason: 'no write should have happened at all',
    );
    expect(session.buffer, body);

    await tester.pumpWidget(const SizedBox());
    c.dispose();
  });

  testWidgets('the editor shows the note it was asked to open', (tester) async {
    final c = container();
    await c.read(noteSessionProvider).open('n1');
    await pumpEditor(tester, c);
    await tester.pump();

    expect(find.text(body), findsOneWidget);
    expect(find.text('Select a note to start editing'), findsNothing);

    await tester.pumpWidget(const SizedBox());
    c.dispose();
  });

  testWidgets('typing still saves', (tester) async {
    // The guard must not be so strict that real edits stop working.
    final c = container();
    final session = c.read(noteSessionProvider);
    await session.open('n1');
    await pumpEditor(tester, c);
    await tester.pump();

    await tester.enterText(find.byType(TextField), '$body\nTyped by hand.\n');
    await tester.pump(const Duration(seconds: 2));
    await session.save();

    expect(server.notes['n1']!.content, contains('Typed by hand.'));

    await tester.pumpWidget(const SizedBox());
    c.dispose();
  });

  group('the pieces that made the loop possible', () {
    test('MarkdownTheme has value equality', () {
      // Without this, every build looks like a theme change.
      const base = TextStyle(fontSize: 16);
      expect(MarkdownTheme.light(base), equals(MarkdownTheme.light(base)));
      expect(
        MarkdownTheme.light(base),
        isNot(equals(MarkdownTheme.dark(base))),
      );
      expect(
        MarkdownTheme.light(base).hashCode,
        MarkdownTheme.light(base).hashCode,
      );
    });

    test('setting the theme does not notify listeners', () {
      // A TextEditingController notification means "the value changed".
      // Saying that when only the styling changed is what fed the loop.
      const base = TextStyle(fontSize: 16);
      final controller = StormMarkdownController(
        theme: MarkdownTheme.light(base),
        text: 'hello',
      );
      addTearDown(controller.dispose);

      var notifications = 0;
      controller.addListener(() => notifications++);

      controller.theme = MarkdownTheme.dark(base);
      controller.theme = MarkdownTheme.light(base);

      expect(notifications, 0);
      expect(controller.text, 'hello', reason: 'the text never changed');
    });
  });
}
