import 'package:drift/native.dart';
import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:storm/api/models.dart';
import 'package:storm/api/storm_api.dart';
import 'package:storm/cache/cache_db.dart';
import 'package:storm/router.dart';
import 'package:storm/state/app_state.dart';
import 'package:storm/sync/sync_engine.dart';
import 'package:storm/ui/theme.dart';
import 'package:storm/ui/tokens.dart';

import 'fake_server.dart';

/// Mounting the real shell — router, theme and providers — for widget tests.
///
/// Shared so the routing and nav-bubble suites cannot drift into testing two
/// slightly different apps.

/// Paths shaped like a real vault, so the browser has folders to walk.
const vaultPaths = [
  'Welcome.md',
  'Daily/2026-08-05.md',
  'Daily/2026-08-04.md',
  'Projects/Storm/Design.md',
  'Projects/Ideas.md',
];

/// A container wired to a [FakeServer], with the engine never `start()`ed so
/// no reconnect timer is left pending when the test ends.
ProviderContainer shellContainer({bool configured = true}) {
  final cache = CacheDb(NativeDatabase.memory());
  final server = FakeServer();

  var n = 0;
  for (final path in vaultPaths) {
    final id = 'n${n++}';
    server.notes[id] = ServerNote(
      id: id,
      path: path,
      content: '# ${path.split('/').last}\n\nbody\n',
      version: 1,
    );
  }

  final api = StormApi(
    baseUrl: 'http://test',
    token: 't',
    client: server.client,
  );
  final container = ProviderContainer(
    overrides: [
      cacheProvider.overrideWithValue(cache),
      apiProvider.overrideWithValue(configured ? api : null),
      // The engine follows the active vault exactly as it does in the app, so
      // a test that switches vaults exercises the real teardown.
      syncEngineProvider.overrideWith(
        (ref) => SyncEngine(
          api: api,
          cache: cache,
          vaultId: ref.watch(activeVaultProvider),
        ),
      ),
      settingsProvider.overrideWith(() => FakeSettings(configured)),
    ],
  );
  _servers[container] = server;
  return container;
}

/// The fake server behind a container, for asserting what actually reached it.
final _servers = <ProviderContainer, FakeServer>{};

FakeServer serverOf(ProviderContainer c) => _servers[c]!;

Future<void> pumpShell(
  WidgetTester tester,
  ProviderContainer c, {
  Size size = const Size(411, 900),
  double keyboard = 0,
}) async {
  tester.view.physicalSize = size;
  tester.view.devicePixelRatio = 1.0;
  // Driven through the view rather than a wrapping MediaQuery, because
  // MaterialApp builds its own from the view and would overwrite ours.
  tester.view.viewInsets = FakeViewPadding(bottom: keyboard);
  addTearDown(tester.view.reset);

  await tester.pumpWidget(
    UncontrolledProviderScope(
      container: c,
      child: MaterialApp.router(
        theme: StormTheme.dark(),
        routerConfig: c.read(routerProvider),
      ),
    ),
  );
  await tester.pumpAndSettle();
}

/// Unmount before disposing, so nothing rebuilds against a dead container.
Future<void> disposeShell(WidgetTester tester, ProviderContainer c) async {
  await tester.pumpWidget(const SizedBox());
  c.dispose();
  _servers.remove(c);
}

NoteMeta noteMeta(String path) => NoteMeta(
  id: path,
  path: path,
  title: '',
  version: 1,
  modified: '2026-08-05T10:00:00Z',
  size: 0,
);

/// Settings without SharedPreferences, so the router's redirect can be driven
/// directly.
class FakeSettings extends SettingsNotifier {
  FakeSettings(this.configured, {this.activeVault = FakeServer.primaryVault});

  final bool configured;

  /// Which vault starts active, as the real app persists it across launches.
  final String activeVault;

  @override
  Future<Settings> build() async => configured
      ? Settings(
          baseUrl: 'http://test',
          token: 't',
          theme: StormPreset.stormDark,
          activeVault: activeVault,
        )
      : const Settings();

  @override
  Future<void> save(Settings next) async => state = AsyncData(next);
}

/// Opens a vault's browser and waits for `VaultGate` to accept it.
///
/// Navigating is not enough on its own: the gate holds a spinner for one frame
/// while the route's vault becomes active, and anything asserted before that
/// would be asserting against the previous vault.
Future<void> openVault(
  WidgetTester tester,
  ProviderContainer c, {
  String vaultId = FakeServer.primaryVault,
}) async {
  c.read(routerProvider).go(Routes.browse(vaultId));
  await tester.pumpAndSettle();
}
