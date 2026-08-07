import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:storm/router.dart';

import 'shell_harness.dart';
import 'fake_server.dart';

/// The Android back gesture.
///
/// Reported from the phone: back left the app instead of returning to the
/// previous screen. Every navigation used `context.go`, which *replaces* the
/// stack — so there was only ever one route, and popping it exited.
///
/// These drive `handlePopRoute`, which is the real system-back signal, rather
/// than tapping in-app back buttons. Those kept working throughout and would
/// not have caught this.
void main() {
  /// Presses the system back button. True if the app handled it; false means
  /// the app would have been closed.
  Future<bool> systemBack(WidgetTester tester) async {
    final handled = await tester.binding.handlePopRoute();
    await tester.pumpAndSettle();
    return handled;
  }

  String locationOf(ProviderContainer c) =>
      c.read(routerProvider).state.uri.path;

  testWidgets('back from a vault returns to the dashboard', (tester) async {
    // Tapped, not `go`n: opening a vault from the dashboard is what has to
    // push. The dashboard lists vaults now, so this is the first hop.
    final c = shellContainer();
    await pumpShell(tester, c);

    await tester.tap(find.text('Primary'));
    await tester.pumpAndSettle();
    expect(locationOf(c), Routes.browse(FakeServer.primaryVault));

    expect(await systemBack(tester), isTrue, reason: 'must not leave the app');
    expect(locationOf(c), Routes.dashboard);
    expect(find.text('Recently opened'), findsOneWidget);

    await disposeShell(tester, c);
  });

  testWidgets('back from a note returns to the vault it was opened from', (
    tester,
  ) async {
    final c = shellContainer();
    await pumpShell(tester, c);

    await tester.tap(find.text('Primary'));
    await tester.pumpAndSettle();
    // The fake server uses each note's id as its title, so that is the label.
    await tester.tap(find.text('Welcome'));
    await tester.pumpAndSettle();
    expect(locationOf(c), startsWith('/v/'));
    expect(locationOf(c), contains('/note/'));

    expect(await systemBack(tester), isTrue);
    expect(locationOf(c), Routes.browse(FakeServer.primaryVault));

    expect(await systemBack(tester), isTrue);
    expect(locationOf(c), Routes.dashboard);

    await disposeShell(tester, c);
  });

  testWidgets('back from a subfolder returns to its parent', (tester) async {
    final c = shellContainer();
    await pumpShell(tester, c);

    await openVault(tester, c);
    await tester.tap(find.text('Projects'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Storm'));
    await tester.pumpAndSettle();
    expect(
      locationOf(c),
      Routes.folder(FakeServer.primaryVault, 'Projects/Storm'),
    );

    expect(await systemBack(tester), isTrue);
    expect(locationOf(c), Routes.folder(FakeServer.primaryVault, 'Projects'));

    expect(await systemBack(tester), isTrue);
    expect(locationOf(c), Routes.browse(FakeServer.primaryVault));

    await disposeShell(tester, c);
  });

  testWidgets('back from a note opened in a folder returns to that folder', (
    tester,
  ) async {
    final c = shellContainer();
    await pumpShell(tester, c);

    c.read(routerProvider).go(Routes.folder(FakeServer.primaryVault, 'Daily'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('2026-08-05'));
    await tester.pumpAndSettle();
    expect(locationOf(c), contains('/note/'));

    expect(await systemBack(tester), isTrue);
    expect(locationOf(c), Routes.folder(FakeServer.primaryVault, 'Daily'));

    await disposeShell(tester, c);
  });

  testWidgets('a deep link still has the dashboard beneath it', (tester) async {
    // Opening a note from a URL must not strand you with nowhere to go back to.
    final c = shellContainer();
    await pumpShell(tester, c);

    c.read(routerProvider).go(Routes.note(FakeServer.primaryVault, 'n0'));
    await tester.pumpAndSettle();

    expect(await systemBack(tester), isTrue);
    expect(locationOf(c), Routes.dashboard);

    await disposeShell(tester, c);
  });

  testWidgets('switching destinations does not pile up a stack', (
    tester,
  ) async {
    // The nav bubble swaps top-level destinations, so back from any of them
    // goes home rather than replaying everywhere you have been.
    final c = shellContainer();
    await pumpShell(tester, c);

    for (final location in [
      Routes.browse(FakeServer.primaryVault),
      Routes.search(FakeServer.primaryVault),
      Routes.tags(FakeServer.primaryVault),
    ]) {
      c.read(routerProvider).go(location);
      await tester.pumpAndSettle();
    }

    expect(await systemBack(tester), isTrue);
    expect(locationOf(c), Routes.dashboard);

    await disposeShell(tester, c);
  });

  testWidgets('back at the dashboard leaves the app', (tester) async {
    // The one place where exiting is right.
    final c = shellContainer();
    await pumpShell(tester, c);

    expect(await systemBack(tester), isFalse);
    await disposeShell(tester, c);
  });
}
