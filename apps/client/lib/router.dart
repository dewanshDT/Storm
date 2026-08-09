import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';

import 'state/app_state.dart';
import 'ui/browse_screen.dart';
import 'ui/connect_screen.dart';
import 'ui/gallery_screen.dart';
import 'ui/note_screen.dart';
import 'ui/search_screen.dart';
import 'ui/server_settings_screen.dart';
import 'ui/shell/dashboard.dart';
import 'ui/shell/vault_shell.dart';
import 'ui/tags_screen.dart';

/// Where the app can be.
///
/// A real router rather than a screen flag, because the navigation bubble's
/// Context slot has to answer "where am I" and there must be exactly one
/// answer. It also gives the web client working deep links, which the old
/// single-screen shell simply didn't have.
abstract final class Routes {
  static const dashboard = '/';
  static const connect = '/connect';
  static const serverSettings = '/settings/server';

  /// Every shared widget in all three themes. Not linked from the app — it is
  /// a surface for judging the token layer, reached by typing the path.
  static const gallery = '/gallery';

  /// Everything note-shaped hangs off the vault, so a deep link carries which
  /// vault it means and back always retraces the real path.
  static String vault(String vaultId) => '/v/$vaultId';

  static String browse(String vaultId) => '${vault(vaultId)}/browse';
  static String search(String vaultId) => '${vault(vaultId)}/search';
  static String tags(String vaultId) => '${vault(vaultId)}/tags';
  static String note(String vaultId, String id) => '${vault(vaultId)}/note/$id';

  /// `/v/<id>/browse/Daily/2026` — the folder path is the rest of the URL, so
  /// a breadcrumb is just the segments of the current location.
  static String folder(String vaultId, String path) => path.isEmpty
      ? browse(vaultId)
      : '${browse(vaultId)}/${Uri.encodeFull(path)}';

  /// The folder a `/v/<id>/browse/...` location refers to, or `''` for the
  /// vault root.
  static String folderOf(Uri uri) {
    final segments = uri.pathSegments;
    final at = segments.indexOf('browse');
    if (at < 0) return '';
    return Uri.decodeFull(segments.skip(at + 1).join('/'));
  }

  /// The vault a location belongs to, or `''` outside one.
  static String vaultOf(Uri uri) {
    final segments = uri.pathSegments;
    if (segments.length < 2 || segments.first != 'v') return '';
    return segments[1];
  }
}

final routerProvider = Provider<GoRouter>((ref) {
  // Listened to, not watched. Watching would recompute this provider and hand
  // back a *different* GoRouter, while the MaterialApp keeps holding the old
  // one — navigation silently stops working, and the stack is thrown away on
  // every settings change. `refreshListenable` re-runs `redirect` on the same
  // router instead, which is the only thing settings actually affect here.
  final refresh = _RouterRefresh();
  ref.listen(settingsProvider, (_, _) => refresh.notify());
  ref.onDispose(refresh.dispose);

  final router = GoRouter(
    initialLocation: Routes.dashboard,
    refreshListenable: refresh,
    redirect: (context, state) {
      final settings = ref.read(settingsProvider);
      final configured = settings.value?.isConfigured ?? false;
      final atConnect = state.matchedLocation == Routes.connect;

      // Settings are still loading; hold still rather than flashing the
      // connect screen at someone who is already set up.
      if (settings.isLoading) return null;

      // The gallery needs no server, and bouncing it to Connect would make it
      // unreachable on exactly the install where the theme is being judged.
      if (state.matchedLocation == Routes.gallery) return null;

      if (!configured) return atConnect ? null : Routes.connect;
      return atConnect ? Routes.dashboard : null;
    },
    routes: [
      GoRoute(path: Routes.connect, builder: (_, _) => const ConnectScreen()),
      GoRoute(path: Routes.gallery, builder: (_, _) => const GalleryScreen()),
      // Everything else is a *child* of the dashboard, so navigating to it
      // builds a stack with the dashboard underneath rather than replacing it.
      // Flat routes meant `go` left exactly one route on the stack, and the
      // Android back gesture popped it straight out of the app.
      GoRoute(
        path: Routes.dashboard,
        builder: (_, _) => const DashboardScreen(),
        routes: [
          GoRoute(
            path: 'settings/server',
            builder: (_, _) => const ServerSettingsScreen(),
          ),
          // Every vault-scoped screen sits inside `VaultShell`, which carries
          // the `VaultGate` — making the route's vault active before its
          // children build, since otherwise there is a frame where the
          // providers still hold the previous vault's notes — and, on a wide
          // screen, the folder tree beside it.
          //
          // A `ShellRoute` rather than wrapping each child: the shell is built
          // once and only the pane inside it changes, which is what lets the
          // sidebar hold its expansion state across opening a note. The paths
          // are unchanged, so the back stack behaves exactly as decision 17
          // describes — `back_navigation_test.dart` is the proof.
          ShellRoute(
            builder: (_, _, child) => VaultShell(child: child),
            routes: [
              GoRoute(
                path: 'v/:vault/browse/:path(.*)',
                builder: (_, state) =>
                    BrowseScreen(folder: Routes.folderOf(state.uri)),
              ),
              GoRoute(
                path: 'v/:vault/browse',
                builder: (_, _) => const BrowseScreen(folder: ''),
              ),
              GoRoute(
                path: 'v/:vault/note/:id',
                builder: (_, state) =>
                    NoteScreen(noteId: state.pathParameters['id']!),
              ),
              GoRoute(
                path: 'v/:vault/search',
                builder: (_, _) => const SearchScreen(),
              ),
              GoRoute(
                path: 'v/:vault/tags',
                builder: (_, _) => const TagsScreen(),
              ),
            ],
          ),
        ],
      ),
    ],
    errorBuilder: (_, state) => Scaffold(
      body: Center(
        child: Padding(
          padding: const EdgeInsets.all(24),
          child: Text('Nothing at ${state.uri}'),
        ),
      ),
    ),
  );

  ref.onDispose(router.dispose);
  return router;
});

/// Nudges GoRouter to re-run its redirect without replacing the router.
class _RouterRefresh extends ChangeNotifier {
  void notify() => notifyListeners();
}
