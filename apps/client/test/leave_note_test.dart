import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:storm/router.dart';
import 'package:storm/state/app_state.dart';

import 'fake_server.dart';
import 'shell_harness.dart';

/// Decision 128b: leaving a note never takes an unsaved edit with it.
void main() {
  testWidgets('a refused save is kept on the device when the note is left, '
      'and the user is told', (tester) async {
    final c = shellContainer();
    await pumpShell(tester, c, size: const Size(411, 900));
    final router = c.read(routerProvider);
    router.go(Routes.note(FakeServer.primaryVault, 'n0'));
    await tester.pumpAndSettle();

    final session = c.read(noteSessionProvider);
    expect(session.noteId, 'n0');
    serverOf(c).failWith = 400;
    session.edit('${session.buffer}the refused edit\n');

    router.go(Routes.note(FakeServer.primaryVault, 'n1'));
    await tester.pumpAndSettle();

    expect(find.textContaining("Couldn't save your last change"), findsOne);
    final queued = await tester.runAsync(
      () => c.read(cacheProvider).outboxFor(FakeServer.primaryVault, 'n0'),
    );
    expect(queued?.content, contains('the refused edit'));

    serverOf(c).failWith = null;
    await disposeShell(tester, c);
  });
}
