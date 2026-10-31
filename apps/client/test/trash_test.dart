import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';

import 'package:storm/api/storm_api.dart';
import 'package:storm/router.dart';
import 'package:storm/state/app_state.dart';
import 'package:storm/ui/browse_screen.dart';
import 'package:storm/ui/trash_screen.dart';

import 'fake_server.dart';
import 'shell_harness.dart';

/// Decision 150: the Trash view and the folder-delete confirm.
void main() {
  const vault = 'v1';
  late List<String> requests;
  late List<Map<String, dynamic>> items;

  Map<String, dynamic> item(
    String id,
    String kind,
    String name,
    String path, {
    Map<String, dynamic> by = const {'kind': 'person', 'name': 'Laptop'},
    int notes = 1,
    String? session,
  }) => {
    'id': id,
    'kind': kind,
    'name': name,
    'original_path': path,
    'deleted_at': DateTime.now()
        .subtract(const Duration(hours: 3))
        .toUtc()
        .toIso8601String(),
    'deleted_by': by,
    'note_count': notes,
    'session_name': ?session,
  };

  StormApi api() => StormApi(
    baseUrl: 'http://test',
    token: 't',
    client: MockClient((request) async {
      final path = request.url.path;
      requests.add(
        '${request.method} $path'
        '${request.url.hasQuery ? '?${request.url.query}' : ''}',
      );
      final prefix = '/v1/vaults/$vault';
      Object body = {};
      if (request.method == 'GET' && path == '$prefix/trash') {
        body = {'items': items, 'retention_days': 30};
      } else if (path.endsWith('/restore')) {
        items.removeWhere((i) => path.contains('/trash/${i['id']}/'));
        body = {'notes': [], 'attachments': []};
      } else if (request.method == 'DELETE' && path == '$prefix/trash') {
        body = {'purged': items.length};
        items.clear();
      } else if (request.method == 'DELETE' && path.contains('/trash/')) {
        items.removeWhere((i) => path.endsWith('/trash/${i['id']}'));
        body = {'purged': path.split('/').last};
      } else if (request.method == 'DELETE' && path.contains('/folders/')) {
        body = {'deleted': path, 'trash_id': 'x', 'notes': 3};
      }
      return http.Response(
        jsonEncode(body),
        200,
        headers: {'content-type': 'application/json'},
      );
    }),
  );

  setUp(() {
    requests = [];
    items = [
      item('t1', 'note', 'Ideas', 'Notes/Ideas.md'),
      item(
        't2',
        'folder',
        'Projects',
        'Projects',
        notes: 12,
        by: {'kind': 'agent', 'session_id': 'ags_1', 'reason': '**not** md'},
        session: 'Research',
      ),
    ];
  });

  Widget host(Widget child) => ProviderScope(
    overrides: [apiProvider.overrideWithValue(api())],
    child: MaterialApp(home: Scaffold(body: child)),
  );

  group('the Trash view', () {
    testWidgets('lists each item with its path, when, who and note count', (
      tester,
    ) async {
      await tester.pumpWidget(host(const TrashView(vaultId: vault)));
      await tester.pumpAndSettle();

      expect(find.text('Ideas'), findsOneWidget);
      expect(find.text('Notes/Ideas.md'), findsOneWidget);
      expect(find.text('Deleted 3h ago by Laptop'), findsOneWidget);
      expect(find.text('Projects'), findsWidgets);
      expect(
        find.text('Deleted 3h ago by an agent (Research) · 12 notes'),
        findsOneWidget,
      );
      // The agent's reason is shown as it was written, not rendered.
      expect(find.text('“**not** md”'), findsOneWidget);
      expect(find.text('30 days'), findsOneWidget);
    });

    testWidgets('restore puts an item back without asking', (tester) async {
      await tester.pumpWidget(host(const TrashView(vaultId: vault)));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const Key('restore:t1')));
      await tester.pumpAndSettle();

      expect(requests, contains('POST /v1/vaults/$vault/trash/t1/restore'));
      expect(find.byKey(const Key('trash:t1')), findsNothing);
    });

    testWidgets('delete forever needs the confirm', (tester) async {
      await tester.pumpWidget(host(const TrashView(vaultId: vault)));
      await tester.pumpAndSettle();

      await tester.tap(find.byKey(const Key('purge:t2')));
      await tester.pumpAndSettle();
      expect(find.text('Delete “Projects” forever?'), findsOneWidget);
      expect(find.textContaining('its 12 notes'), findsOneWidget);
      await tester.tap(find.byKey(const Key('confirm-cancel')));
      await tester.pumpAndSettle();
      expect(requests.where((r) => r.startsWith('DELETE')), isEmpty);

      await tester.tap(find.byKey(const Key('purge:t2')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const Key('confirm-action')));
      await tester.pumpAndSettle();
      expect(requests, contains('DELETE /v1/vaults/$vault/trash/t2'));
      expect(find.byKey(const Key('trash:t2')), findsNothing);
    });

    testWidgets('empty trash needs the confirm', (tester) async {
      await tester.pumpWidget(host(const TrashView(vaultId: vault)));
      await tester.pumpAndSettle();

      await tester.tap(find.byKey(const Key('trash-empty')));
      await tester.pumpAndSettle();
      expect(find.text('Empty trash?'), findsOneWidget);
      await tester.tap(find.byKey(const Key('confirm-cancel')));
      await tester.pumpAndSettle();
      expect(requests, isNot(contains('DELETE /v1/vaults/$vault/trash')));

      await tester.tap(find.byKey(const Key('trash-empty')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const Key('confirm-action')));
      await tester.pumpAndSettle();
      expect(requests, contains('DELETE /v1/vaults/$vault/trash'));
      expect(find.text('Trash is empty'), findsOneWidget);
    });
  });

  group('deleting a folder', () {
    Widget deleter(BrowseEntry entry) => host(
      Consumer(
        builder: (context, ref, _) => TextButton(
          onPressed: () => deleteFolderToTrash(context, ref, vault, entry),
          child: const Text('delete'),
        ),
      ),
    );

    testWidgets('a folder with notes goes only after the confirm', (
      tester,
    ) async {
      await tester.pumpWidget(
        deleter(const BrowseEntry.folderWith('Projects', 'Work/Projects', 3)),
      );
      await tester.tap(find.text('delete'));
      await tester.pumpAndSettle();
      expect(
        find.text('Delete folder Projects and its 3 notes? They go to Trash.'),
        findsOneWidget,
      );
      await tester.tap(find.byKey(const Key('confirm-cancel')));
      await tester.pumpAndSettle();
      expect(requests, isEmpty);

      await tester.tap(find.text('delete'));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const Key('confirm-action')));
      await tester.pumpAndSettle();
      expect(requests, [
        'DELETE /v1/vaults/$vault/folders/Work/Projects?trash=true',
      ]);
    });

    testWidgets('one note is said in the singular', (tester) async {
      await tester.pumpWidget(
        deleter(const BrowseEntry.folderWith('Inbox', 'Inbox', 1)),
      );
      await tester.tap(find.text('delete'));
      await tester.pumpAndSettle();
      expect(
        find.text('Delete folder Inbox and its 1 note? They go to Trash.'),
        findsOneWidget,
      );
    });

    testWidgets('an empty folder is deleted without a question', (
      tester,
    ) async {
      await tester.pumpWidget(
        deleter(const BrowseEntry.folderWith('Empty', 'Empty', 0)),
      );
      await tester.tap(find.text('delete'));
      await tester.pumpAndSettle();
      expect(find.byType(AlertDialog), findsNothing);
      expect(requests, ['DELETE /v1/vaults/$vault/folders/Empty?trash=true']);
    });
  });

  group('the Trash entry', () {
    const primary = FakeServer.primaryVault;

    for (final (label, size) in [
      ('in the desktop sidebar', const Size(1280, 900)),
      ('at the phone vault root', const Size(411, 900)),
    ]) {
      testWidgets('is $label and opens the vault\'s trash', (tester) async {
        final c = shellContainer();
        await pumpShell(tester, c, size: size);
        await openVault(tester, c);

        final entry = find.byKey(const Key('sidebar-trash'));
        await tester.scrollUntilVisible(
          entry,
          200,
          scrollable: find.byType(Scrollable).first,
        );
        await tester.tap(entry);
        await tester.pumpAndSettle();
        expect(c.read(routerProvider).state.uri.path, Routes.trash(primary));
        expect(find.byType(TrashView), findsOneWidget);
        await disposeShell(tester, c);
      });
    }
  });
}
