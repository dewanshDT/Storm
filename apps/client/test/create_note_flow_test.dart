import 'package:drift/native.dart';
import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:storm/api/storm_api.dart';
import 'package:storm/cache/cache_db.dart';
import 'package:storm/state/app_state.dart';
import 'package:storm/sync/sync_engine.dart';
import 'package:storm/ui/vault_screen.dart';

import 'fake_server.dart';

/// The "new note" flow, end to end through the real screen.
///
/// Creating a note showed a red error screen even though the note was created
/// — the dialog's `TextEditingController` was disposed the moment
/// `showDialog` returned, while the route was still animating out and the
/// `TextField` was still mounted. The next rebuild of that field threw
/// "A TextEditingController was used after being disposed".
///
/// These tests pump *past* the dialog's exit animation, which is what makes
/// the failure visible.
void main() {
  late CacheDb cache;
  late FakeServer server;

  ProviderContainer container() {
    cache = CacheDb(NativeDatabase.memory());
    server = FakeServer();
    final api = StormApi(
      baseUrl: 'http://test',
      token: 't',
      client: server.client,
    );
    return ProviderContainer(
      overrides: [
        cacheProvider.overrideWithValue(cache),
        apiProvider.overrideWithValue(api),
        // Not started: no WebSocket, so no pending reconnect timer.
        syncEngineProvider.overrideWith(
          (ref) => SyncEngine(api: api, cache: cache),
        ),
      ],
    );
  }

  Future<void> pumpScreen(WidgetTester tester, ProviderContainer c) async {
    await tester.pumpWidget(
      UncontrolledProviderScope(
        container: c,
        child: const MaterialApp(home: VaultScreen()),
      ),
    );
    await tester.pumpAndSettle();
  }

  testWidgets('creating a note does not throw', (tester) async {
    tester.view.physicalSize = const Size(1400, 1800);
    tester.view.devicePixelRatio = 1.0;
    addTearDown(tester.view.reset);

    final c = container();
    await pumpScreen(tester, c);

    await tester.tap(find.byTooltip('New note'));
    await tester.pumpAndSettle();
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
      server.notes.values.any((n) => n.path == 'Notes/Fresh.md'),
      isTrue,
      reason: 'and the note should actually be created',
    );

    await tester.pumpWidget(const SizedBox());
    c.dispose();
  });

  testWidgets('cancelling the dialog does not throw either', (tester) async {
    tester.view.physicalSize = const Size(1400, 1800);
    tester.view.devicePixelRatio = 1.0;
    addTearDown(tester.view.reset);

    final c = container();
    await pumpScreen(tester, c);

    await tester.tap(find.byTooltip('New note'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Cancel'));
    await tester.pumpAndSettle();

    expect(tester.takeException(), isNull);

    await tester.pumpWidget(const SizedBox());
    c.dispose();
  });

  testWidgets('an invalid path is rejected without closing the dialog', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(1400, 1800);
    tester.view.devicePixelRatio = 1.0;
    addTearDown(tester.view.reset);

    final c = container();
    await pumpScreen(tester, c);

    await tester.tap(find.byTooltip('New note'));
    await tester.pumpAndSettle();
    await tester.enterText(find.byType(TextField).last, '../escape.md');
    await tester.tap(find.text('OK'));
    await tester.pumpAndSettle();

    expect(tester.takeException(), isNull);
    expect(
      find.text('New note'),
      findsOneWidget,
      reason: 'the dialog stays open so the name can be corrected',
    );
    expect(server.notes.values.any((n) => n.path.contains('escape')), isFalse);

    await tester.pumpWidget(const SizedBox());
    c.dispose();
  });

  group('path validation', () {
    test('mirrors the server rules', () {
      expect(validatePath('Notes/Fresh.md'), isNull);
      expect(validatePath('../escape.md'), isNotNull);
      expect(validatePath('/absolute.md'), isNotNull);
      expect(validatePath('.hidden/x.md'), isNotNull);
      expect(validatePath(''), isNotNull);
    });
  });
}
