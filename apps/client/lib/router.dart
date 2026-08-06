import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';

import 'state/app_state.dart';
import 'ui/browse_screen.dart';
import 'ui/connect_screen.dart';
import 'ui/note_screen.dart';
import 'ui/search_screen.dart';
import 'ui/shell/dashboard.dart';
import 'ui/tags_screen.dart';

/// Where the app can be.
///
/// A real router rather than a screen flag, because the navigation bubble's
/// Context slot has to answer "where am I" and there must be exactly one
/// answer. It also gives the web client working deep links, which the old
/// single-screen shell simply didn't have.
abstract final class Routes {
  static const dashboard = '/';
  static const browse = '/browse';
  static const search = '/search';
  static const tags = '/tags';
  static const connect = '/connect';

  static String note(String id) => '/note/$id';

  /// `/browse/Daily/2026` — the folder path is the rest of the URL, so a
  /// breadcrumb is just the segments of the current location.
  static String folder(String path) =>
      path.isEmpty ? browse : '$browse/${Uri.encodeFull(path)}';

  /// The folder a `/browse/...` location refers to, or `''` for the root.
  static String folderOf(Uri uri) {
    final segments = uri.pathSegments;
    if (segments.isEmpty || segments.first != 'browse') return '';
    return Uri.decodeFull(segments.skip(1).join('/'));
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

      if (!configured) return atConnect ? null : Routes.connect;
      return atConnect ? Routes.dashboard : null;
    },
    routes: [
      GoRoute(
        path: Routes.connect,
        builder: (_, _) => const ConnectScreen(),
      ),
      // Everything else is a *child* of the dashboard, so navigating to it
      // builds a stack with the dashboard underneath rather than replacing it.
      // Flat routes meant `go` left exactly one route on the stack, and the
      // Android back gesture popped it straight out of the app.
      GoRoute(
        path: Routes.dashboard,
        builder: (_, _) => const DashboardScreen(),
        routes: [
          GoRoute(
            path: 'browse/:path(.*)',
            builder: (_, state) =>
                BrowseScreen(folder: Routes.folderOf(state.uri)),
          ),
          GoRoute(
            path: 'browse',
            builder: (_, _) => const BrowseScreen(folder: ''),
          ),
          GoRoute(
            path: 'note/:id',
            builder: (_, state) =>
                NoteScreen(noteId: state.pathParameters['id']!),
          ),
          GoRoute(path: 'search', builder: (_, _) => const SearchScreen()),
          GoRoute(path: 'tags', builder: (_, _) => const TagsScreen()),
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
