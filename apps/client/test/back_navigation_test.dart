import 'package:flutter/widgets.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:storm/router.dart';
import 'package:storm/state/nav_memory.dart';

import 'shell_harness.dart';
import 'fake_server.dart';

/// The Android back gesture, through `handlePopRoute` (the real system-back
/// signal), against the Q2 contract: pushed pages pop; with nothing to pop a
/// note goes to its folder, a folder to its parent, Agents and Settings to the
/// last Notes location; the vault root exits.
void main() {
  const v = FakeServer.primaryVault;
  const wide = Size(1280, 800);

  Future<bool> systemBack(WidgetTester tester) async {
    final handled = await tester.binding.handlePopRoute();
    await tester.pumpAndSettle();
    return handled;
  }

  String locationOf(ProviderContainer c) =>
      c.read(routerProvider).state.uri.path;

  void go(ProviderContainer c, String location) =>
      c.read(routerProvider).go(location);

  testWidgets('launch with nothing remembered opens the vault root', (
    tester,
  ) async {
    final c = shellContainer();
    await pumpShell(tester, c);
    expect(locationOf(c), Routes.browse(v));
    await disposeShell(tester, c);
  });

  testWidgets('back at the vault root leaves the app', (tester) async {
    final c = shellContainer();
    await pumpShell(tester, c);
    expect(await systemBack(tester), isFalse);
    await disposeShell(tester, c);
  });

  testWidgets('pushed folders and notes pop back the way they came', (
    tester,
  ) async {
    final c = shellContainer();
    await pumpShell(tester, c);

    await tester.tap(find.text('Projects'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Storm'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Design'));
    await tester.pumpAndSettle();
    expect(locationOf(c), contains('/note/'));

    expect(await systemBack(tester), isTrue);
    expect(locationOf(c), Routes.folder(v, 'Projects/Storm'));
    expect(await systemBack(tester), isTrue);
    expect(locationOf(c), Routes.folder(v, 'Projects'));
    expect(await systemBack(tester), isTrue);
    expect(locationOf(c), Routes.browse(v));
    expect(await systemBack(tester), isFalse);

    await disposeShell(tester, c);
  });

  testWidgets('a deep-linked note goes back to its folder, then up', (
    tester,
  ) async {
    final c = shellContainer();
    await pumpShell(tester, c);

    // n3 is Projects/Storm/Design.md in the harness vault.
    go(c, Routes.note(v, 'n3'));
    await tester.pumpAndSettle();

    expect(await systemBack(tester), isTrue);
    expect(locationOf(c), Routes.folder(v, 'Projects/Storm'));
    expect(await systemBack(tester), isTrue);
    expect(locationOf(c), Routes.folder(v, 'Projects'));
    expect(await systemBack(tester), isTrue);
    expect(locationOf(c), Routes.browse(v));
    expect(await systemBack(tester), isFalse);

    await disposeShell(tester, c);
  });

  testWidgets('search and tags go back to the vault root', (tester) async {
    final c = shellContainer();
    await pumpShell(tester, c);

    for (final location in [Routes.search(v), Routes.tags(v)]) {
      go(c, location);
      await tester.pumpAndSettle();
      expect(await systemBack(tester), isTrue);
      expect(locationOf(c), Routes.browse(v));
    }

    await disposeShell(tester, c);
  });

  testWidgets('back from Agents returns to the last Notes location', (
    tester,
  ) async {
    final c = shellContainer();
    await pumpShell(tester, c);

    go(c, Routes.folder(v, 'Daily'));
    await tester.pumpAndSettle();
    go(c, Routes.agents);
    await tester.pumpAndSettle();

    expect(await systemBack(tester), isTrue);
    expect(locationOf(c), Routes.folder(v, 'Daily'));

    await disposeShell(tester, c);
  });

  testWidgets('a phone settings page goes back to the list, then to Notes', (
    tester,
  ) async {
    final c = shellContainer();
    await pumpShell(tester, c);

    go(c, Routes.settingsPage('vaults'));
    await tester.pumpAndSettle();

    expect(await systemBack(tester), isTrue);
    expect(locationOf(c), Routes.settings);
    expect(await systemBack(tester), isTrue);
    expect(locationOf(c), Routes.browse(v));

    await disposeShell(tester, c);
  });

  testWidgets('a pushed phone settings page pops to the list', (tester) async {
    final c = shellContainer();
    await pumpShell(tester, c);

    go(c, Routes.settings);
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('settings-row-device')));
    await tester.pumpAndSettle();
    expect(locationOf(c), Routes.settingsPage('device'));

    expect(await systemBack(tester), isTrue);
    expect(locationOf(c), Routes.settings);

    await disposeShell(tester, c);
  });

  testWidgets('at desk width a settings page goes back to Notes', (
    tester,
  ) async {
    final c = shellContainer();
    await pumpShell(tester, c, size: wide);

    go(c, Routes.settingsPage('vaults'));
    await tester.pumpAndSettle();

    expect(await systemBack(tester), isTrue);
    expect(locationOf(c), Routes.browse(v));

    await disposeShell(tester, c);
  });

  testWidgets('launch restores the last activity and location', (tester) async {
    final c = shellContainer(
      nav: NavMemory(
        activity: Activity.settings,
        notes: Routes.folder(v, 'Daily'),
        settings: Routes.settingsPage('access'),
      ),
    );
    await pumpShell(tester, c);
    expect(locationOf(c), Routes.settingsPage('access'));

    expect(await systemBack(tester), isTrue);
    expect(locationOf(c), Routes.settings);
    expect(await systemBack(tester), isTrue);
    expect(locationOf(c), Routes.folder(v, 'Daily'));

    await disposeShell(tester, c);
  });
}
