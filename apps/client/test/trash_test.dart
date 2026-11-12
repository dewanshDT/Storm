import 'dart:convert';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';

import 'package:storm/api/storm_api.dart';
import 'package:storm/router.dart';
import 'package:storm/state/app_state.dart';
import 'package:storm/ui/controls.dart';
import 'package:storm/ui/browse_screen.dart';
import 'package:storm/ui/shell/nav_bubble.dart';
import 'package:storm/ui/trash_screen.dart';

import 'fake_server.dart';
import 'shell_harness.dart';

/// Decision 150: the Trash view and the folder-delete confirm.
void main() {
  const vault = 'v1';
  late List<String> requests;
  late List<Map<String, dynamic>> items;

  late Object? retention;
  late Set<String> failing;

  Map<String, dynamic> item(
    String id,
    String kind,
    String name,
    String path, {
    Map<String, dynamic> by = const {'kind': 'person', 'name': 'Laptop'},
    int notes = 1,
    String? session,
    Duration ago = const Duration(hours: 3),
  }) => {
    'id': id,
    'kind': kind,
    'name': name,
    'original_path': path,
    'deleted_at': DateTime.now().subtract(ago).toUtc().toIso8601String(),
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
      if (failing.any((id) => path.contains('/trash/$id'))) {
        return http.Response(
          jsonEncode({'error': 'locked'}),
          500,
          headers: {'content-type': 'application/json'},
        );
      }
      if (request.method == 'GET' && path == '/v1/vaults') {
        body = {
          'vaults': [
            {'id': vault, 'name': 'personal', 'dir': 'personal'},
            // Two, so the list skips the one-vault cache adoption.
            {'id': 'v2', 'name': 'work', 'dir': 'work'},
          ],
        };
      } else if (request.method == 'GET' && path == '$prefix/trash') {
        body = {'items': items, 'retention_days': retention};
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
    retention = 30;
    failing = {};
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
      item(
        't3',
        'note',
        'Old',
        'Old.md',
        ago: const Duration(days: 29, hours: 23),
      ),
    ];
  });

  Widget host(Widget child) => ProviderScope(
    overrides: [apiProvider.overrideWithValue(api())],
    child: MaterialApp(home: Scaffold(body: child)),
  );

  Future<void> pumpTrash(WidgetTester tester, {Size? size}) async {
    if (size != null) {
      tester.view.physicalSize = size;
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.reset);
    }
    await tester.pumpWidget(host(const TrashView(vaultId: vault)));
    await tester.pumpAndSettle();
  }

  const desk = Size(1280, 800);
  const phone = Size(390, 844);

  String intro(WidgetTester tester) =>
      tester.widget<Text>(find.byKey(const Key('trash-intro'))).data!;

  Future<void> tapKey(WidgetTester tester, String key) async {
    await tester.ensureVisible(find.byKey(Key(key)));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(Key(key)));
    await tester.pumpAndSettle();
  }

  group('the Trash view', () {
    for (final (days, said) in [
      (30, 'Deleted notes are kept for 30 days, then removed for good.'),
      (7, 'Deleted notes are kept for 7 days, then removed for good.'),
      (null, 'Deleted notes are kept until you empty the trash.'),
      (14, 'Deleted notes are kept for 14 days, then removed for good.'),
    ]) {
      testWidgets('the title names the vault; the intro says $days', (
        tester,
      ) async {
        retention = days;
        await pumpTrash(tester, size: desk);
        expect(find.text('Trash · personal'), findsOneWidget);
        expect(intro(tester), said);
        expect(find.byKey(const Key('trash-retention-link')), findsOneWidget);
        expect(find.text('Change in Settings › Vaults ›'), findsOneWidget);
        // Retention moved to Settings › Vaults (153l).
        expect(find.text('Keep deleted items for'), findsNothing);
        expect(find.byType(ChoiceChips<int?>), findsNothing);
        expect(find.text('Never delete'), findsNothing);
      });
    }

    testWidgets('each card has its time left, who deleted it and why', (
      tester,
    ) async {
      await pumpTrash(tester, size: desk);

      expect(find.text('Ideas'), findsOneWidget);
      expect(find.text('Notes/Ideas.md · 29 days left'), findsOneWidget);
      expect(find.text('Deleted 3h ago by Laptop'), findsOneWidget);
      expect(find.text('Projects · 12 notes · 29 days left'), findsOneWidget);
      expect(
        find.text('Deleted 3h ago by Research (approved by you)'),
        findsOneWidget,
      );
      expect(find.text('Old.md · less than a day left'), findsOneWidget);
      // The agent's reason is shown as it was written, not rendered.
      expect(find.text('“**not** md”'), findsOneWidget);
      // No per-item Delete forever: the frame has only Restore.
      expect(find.byKey(const Key('purge:t1')), findsNothing);
      expect(find.byKey(const Key('restore:t1')), findsOneWidget);
    });

    testWidgets('kept until emptied: no countdown', (tester) async {
      retention = null;
      await pumpTrash(tester, size: desk);
      expect(find.text('Notes/Ideas.md'), findsOneWidget);
      expect(find.textContaining('left'), findsNothing);
    });

    test('time left counts from the deletion and the retention', () {
      final now = DateTime.utc(2026, 11, 12, 12);
      String? left(int? days, Duration ago) => retentionOf(
        days,
      ).timeLeft(now.subtract(ago).toIso8601String(), now: now);
      expect(left(30, const Duration(days: 3)), '27 days left');
      expect(left(30, const Duration(days: 28, hours: 1)), '1 day left');
      expect(
        left(7, const Duration(days: 6, hours: 23)),
        'less than a day left',
      );
      expect(left(7, const Duration(days: 9)), 'less than a day left');
      expect(left(null, const Duration(days: 3)), isNull);
      expect(
        const TrashRetention.unknown().timeLeft(now.toIso8601String()),
        isNull,
      );
    });

    testWidgets('restore puts one item back without asking', (tester) async {
      await pumpTrash(tester, size: desk);
      await tapKey(tester, 'restore:t1');

      expect(find.byKey(const Key('trash-confirm')), findsNothing);
      expect(requests, contains('POST /v1/vaults/$vault/trash/t1/restore'));
      expect(find.byKey(const Key('trash:t1')), findsNothing);
      expect(find.text('Restored “Ideas”'), findsOneWidget);
    });

    testWidgets('checkboxes select; the bar counts them and Esc clears', (
      tester,
    ) async {
      await pumpTrash(tester, size: desk);
      expect(find.byKey(const Key('trash-selection')), findsNothing);

      await tapKey(tester, 'trash-check:t1');
      await tapKey(tester, 'trash-check:t2');
      expect(find.byKey(const Key('trash-selection')), findsOneWidget);
      expect(find.text('2 selected'), findsOneWidget);
      expect(find.byKey(const Key('selection-restore')), findsOneWidget);
      expect(find.text('Delete forever…'), findsOneWidget);
      expect(find.text('Esc'), findsOneWidget);
      // At desk width the bar sits above the cards.
      expect(
        tester.getTopLeft(find.byKey(const Key('trash-selection'))).dy,
        lessThan(tester.getTopLeft(find.byKey(const Key('trash:t1'))).dy),
      );

      await tapKey(tester, 'trash-check:t2');
      expect(find.text('1 selected'), findsOneWidget);

      await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('trash-selection')), findsNothing);

      await tapKey(tester, 'trash-check:t3');
      await tapKey(tester, 'selection-clear');
      expect(find.byKey(const Key('trash-selection')), findsNothing);
      expect(requests.where((r) => r.contains('/trash/')), isEmpty);
    });

    testWidgets('restoring a selection asks nothing and loops per item', (
      tester,
    ) async {
      await pumpTrash(tester, size: desk);
      await tapKey(tester, 'trash-check:t1');
      await tapKey(tester, 'trash-check:t3');
      await tapKey(tester, 'selection-restore');

      expect(find.byKey(const Key('trash-confirm')), findsNothing);
      expect(requests.where((r) => r.endsWith('/restore')), [
        'POST /v1/vaults/$vault/trash/t1/restore',
        'POST /v1/vaults/$vault/trash/t3/restore',
      ]);
      expect(find.byKey(const Key('trash:t1')), findsNothing);
      expect(find.byKey(const Key('trash:t3')), findsNothing);
      expect(find.byKey(const Key('trash-selection')), findsNothing);
      expect(find.text('Restored 2 items'), findsOneWidget);
    });

    testWidgets('a partial failure says so and keeps what failed selected', (
      tester,
    ) async {
      failing = {'t3'};
      await pumpTrash(tester, size: desk);
      await tapKey(tester, 'trash-check:t1');
      await tapKey(tester, 'trash-check:t3');
      await tapKey(tester, 'selection-restore');

      expect(find.byKey(const Key('trash:t1')), findsNothing);
      expect(find.byKey(const Key('trash:t3')), findsOneWidget);
      expect(
        find.textContaining('Couldn’t restore 1 of 2 items'),
        findsOneWidget,
      );
      expect(find.text('1 selected'), findsOneWidget);
    });

    testWidgets('Delete forever… confirms the count in the strip', (
      tester,
    ) async {
      await pumpTrash(tester, size: desk);
      await tapKey(tester, 'trash-check:t1');
      await tapKey(tester, 'trash-check:t2');
      await tapKey(tester, 'selection-purge');

      // In the strip at the top of Trash, never a dialog (153g).
      expect(find.byType(AlertDialog), findsNothing);
      expect(find.byKey(const Key('trash-confirm')), findsOneWidget);
      expect(find.text('CONFIRM'), findsOneWidget);
      expect(find.text('Delete 2 items forever?'), findsOneWidget);
      expect(
        tester.getTopLeft(find.byKey(const Key('trash-confirm'))).dy,
        lessThan(tester.getTopLeft(find.byKey(const Key('trash-title'))).dy),
      );
      final action = tester.widget<StormButton>(
        find.byKey(const Key('confirm-action')),
      );
      expect(action.label, 'Delete forever');
      expect(action.kind, StormButtonKind.danger);

      // Esc is the strip's Cancel; the selection waits.
      await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('trash-confirm')), findsNothing);
      expect(find.text('2 selected'), findsOneWidget);
      expect(requests.where((r) => r.startsWith('DELETE')), isEmpty);

      await tapKey(tester, 'selection-purge');
      await tester.tap(find.byKey(const Key('confirm-action')));
      await tester.pumpAndSettle();
      expect(requests.where((r) => r.startsWith('DELETE')), [
        'DELETE /v1/vaults/$vault/trash/t1',
        'DELETE /v1/vaults/$vault/trash/t2',
      ]);
      expect(find.byKey(const Key('trash:t1')), findsNothing);
      expect(find.byKey(const Key('trash:t2')), findsNothing);
      expect(find.byKey(const Key('trash:t3')), findsOneWidget);
      expect(find.text('Deleted 2 items forever'), findsOneWidget);
    });

    testWidgets('one selected folder is named, with its notes', (tester) async {
      await pumpTrash(tester, size: desk);
      await tapKey(tester, 'trash-check:t2');
      await tapKey(tester, 'selection-purge');
      expect(find.text('Delete “Projects” forever?'), findsOneWidget);
      expect(find.textContaining('its 12 notes'), findsOneWidget);
    });

    testWidgets('empty trash needs the confirm', (tester) async {
      await pumpTrash(tester, size: desk);

      await tester.tap(find.byKey(const Key('trash-empty')));
      await tester.pumpAndSettle();
      expect(find.byType(AlertDialog), findsNothing);
      expect(find.byKey(const Key('trash-confirm')), findsOneWidget);
      expect(find.text('Empty trash?'), findsOneWidget);
      expect(
        tester
            .widget<StormButton>(find.byKey(const Key('confirm-action')))
            .label,
        'Empty trash',
      );
      await tester.tap(find.byKey(const Key('confirm-cancel')));
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('trash-confirm')), findsNothing);
      expect(requests, isNot(contains('DELETE /v1/vaults/$vault/trash')));

      // Esc is Cancel.
      await tester.tap(find.byKey(const Key('trash-empty')));
      await tester.pumpAndSettle();
      await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('trash-confirm')), findsNothing);
      expect(requests, isNot(contains('DELETE /v1/vaults/$vault/trash')));

      await tester.tap(find.byKey(const Key('trash-empty')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const Key('confirm-action')));
      await tester.pumpAndSettle();
      expect(requests, contains('DELETE /v1/vaults/$vault/trash'));
      expect(find.text('Trash is empty'), findsOneWidget);
    });

    testWidgets('phone: a long-press selects and the bar sits at the bottom', (
      tester,
    ) async {
      await pumpTrash(tester, size: phone);
      // No checkboxes until something is chosen.
      expect(find.byKey(const Key('trash-check:t1')), findsNothing);

      await tester.longPress(find.byKey(const Key('trash:t1')));
      await tester.pumpAndSettle();
      expect(find.text('1 selected'), findsOneWidget);
      expect(find.byKey(const Key('trash-check:t2')), findsOneWidget);
      final bar = tester.getRect(find.byKey(const Key('trash-selection')));
      expect(bar.bottom, phone.height);
      expect(
        bar.top,
        greaterThan(tester.getRect(find.byKey(const Key('trash:t1'))).bottom),
      );

      // While selecting, a tap on a card toggles it.
      await tester.tap(find.text('Old'));
      await tester.pumpAndSettle();
      expect(find.text('2 selected'), findsOneWidget);

      await tapKey(tester, 'selection-restore');
      expect(requests.where((r) => r.endsWith('/restore')), hasLength(2));
      expect(find.byKey(const Key('trash-selection')), findsNothing);
    });
  });

  test('no hard-coded "30 days" in lib/: retention is per vault', () {
    final offenders = <String>[];
    final literal = RegExp(r"""['"][^'"\n]*\b30 days""");
    for (final f in Directory('lib').listSync(recursive: true)) {
      if (f is! File || !f.path.endsWith('.dart')) continue;
      final lines = f.readAsLinesSync();
      for (var i = 0; i < lines.length; i++) {
        final line = lines[i].trimLeft();
        if (line.startsWith('//')) continue;
        if (literal.hasMatch(line)) offenders.add('${f.path}:${i + 1}');
      }
    }
    expect(offenders, isEmpty);
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

    testWidgets('phone: a selection bar takes the nav pill\'s place', (
      tester,
    ) async {
      final c = shellContainer();
      serverOf(c)
        ..trashRetention[primary] = 30
        ..trashItems[primary] = [item('t1', 'note', 'Ideas', 'Ideas.md')];
      await pumpShell(tester, c, size: const Size(411, 900));
      await openVault(tester, c);
      c.read(routerProvider).go(Routes.trash(primary));
      await tester.pumpAndSettle();
      expect(find.byType(NavBubble), findsOneWidget);

      await tester.longPress(find.byKey(const Key('trash:t1')));
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('trash-selection')), findsOneWidget);
      expect(find.byType(NavBubble), findsNothing);

      await tester.tap(find.byKey(const Key('selection-clear')));
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('trash-selection')), findsNothing);
      expect(find.byType(NavBubble), findsOneWidget);
      await disposeShell(tester, c);
    });
  });
}
