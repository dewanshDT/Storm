import 'package:drift/native.dart';
import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:storm/api/storm_api.dart';
import 'package:storm/cache/cache_db.dart';
import 'package:storm/editor/markdown_theme.dart';
import 'package:storm/editor/storm_markdown_controller.dart';
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

  const frontmatter = '---\nid: n1\ntags: [homelab]\n# hand comment\n---\n';
  const bodyOnly = '\n# Ideas\n\nSomeday: #maybe\n';
  const body = frontmatter + bodyOnly;

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

  testWidgets('the editor shows the body, not the raw frontmatter', (
    tester,
  ) async {
    final c = container();
    await c.read(noteSessionProvider).open('n1');
    await pumpEditor(tester, c);
    await tester.pump();

    expect(
      find.text(bodyOnly),
      findsOneWidget,
      reason: 'the text field holds the body only',
    );
    expect(
      find.text(body),
      findsNothing,
      reason: 'the raw --- block must not be in the editor',
    );
    expect(find.text('Select a note to start editing'), findsNothing);

    // …and the metadata is rendered as properties instead.
    expect(find.text('tags'), findsOneWidget);
    expect(find.text('#homelab'), findsOneWidget);

    await tester.pumpWidget(const SizedBox());
    c.dispose();
  });

  testWidgets('editing the body preserves the frontmatter exactly', (
    tester,
  ) async {
    // The editor never shows the frontmatter, so it must be incapable of
    // losing it — including the hand-written comment.
    final c = container();
    final session = c.read(noteSessionProvider);
    await session.open('n1');
    await pumpEditor(tester, c);
    await tester.pump();

    await tester.enterText(find.byType(TextField), '\n# Ideas\n\nEdited.\n');
    await tester.pump(const Duration(seconds: 2));
    await session.save();

    final saved = server.notes['n1']!.content;
    expect(
      saved,
      startsWith(frontmatter),
      reason: 'frontmatter must survive byte for byte',
    );
    expect(saved, contains('# hand comment'));
    expect(saved, contains('Edited.'));
    expect(saved, isNot(contains('Someday')));

    await tester.pumpWidget(const SizedBox());
    c.dispose();
  });

  testWidgets('a note with no frontmatter shows no properties panel', (
    tester,
  ) async {
    final c = container();
    // After container(), which builds a fresh FakeServer.
    server.notes['n2'] = ServerNote(
      id: 'n2',
      path: 'Plain.md',
      content: '# Plain\n\nNo metadata.\n',
      version: 1,
    );
    await c.read(noteSessionProvider).open('n2');
    await pumpEditor(tester, c);
    await tester.pump();

    expect(find.text('# Plain\n\nNo metadata.\n'), findsOneWidget);
    expect(find.text('Details'), findsNothing);

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

  testWidgets('a huge note explains why formatting is off', (tester) async {
    // The styling drops above maxStyledLines to keep typing responsive. Left
    // unexplained that reads as a bug — the badge that used to show it only
    // ever existed in the retired M0 spike, not in the app.
    final c = container();
    final huge = List.filled(
      StormMarkdownController.maxStyledLines + 50,
      '# heading',
    ).join('\n');
    server.notes['big'] = ServerNote(
      id: 'big',
      path: 'Big.md',
      content: huge,
      version: 1,
    );

    await c.read(noteSessionProvider).open('big');
    await pumpEditor(tester, c);
    await tester.pump();

    expect(find.textContaining('Formatting is off above'), findsOneWidget);

    await tester.pumpWidget(const SizedBox());
    c.dispose();
  });

  testWidgets('an ordinary note shows no such notice', (tester) async {
    final c = container();
    await c.read(noteSessionProvider).open('n1');
    await pumpEditor(tester, c);
    await tester.pump();

    expect(find.textContaining('Formatting is off'), findsNothing);

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
