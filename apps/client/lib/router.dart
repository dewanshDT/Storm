import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';

import 'state/app_state.dart';
import 'ui/browse_screen.dart';
import 'ui/client_settings_screen.dart';
import 'ui/connect_screen.dart';
import 'ui/gallery_screen.dart';
import 'ui/add_device_screen.dart';
import 'ui/login_screen.dart';
import 'ui/signup_screen.dart';
import 'ui/note_screen.dart';
import 'ui/pairing_screen.dart';
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
  static const pairing = '/pairing';

  /// Sign in on a device that is already paired. Distinct from [pairing],
  /// which is first-run only and asks for a QR nobody needs twice.
  static const login = '/login';

  /// Create an account, on a server whose owner has opened registration (A13).
  /// Reachable from [login] only when the server says so.
  static const signup = '/signup';

  /// Reachable without a vault, from the phone's dashboard — which is the
  /// screen you are on when there is no vault yet.
  static const serverSettings = '/settings/server';

  /// Show a pairing QR so another device can join. Session tier: only a
  /// signed-in client can vouch for a new one.
  static const addDevice = '/add-device';

  /// The same two screens, mounted inside the vault shell.
  ///
  /// Two mount points for one screen, deliberately: settings have to be
  /// reachable *without* a vault (there may be none) and *with* the sidebar
  /// beside them (at desk width there is nowhere else to put them). The
  /// screens themselves do not know the difference.
  static String serverSettingsIn(String vaultId) =>
      '${vault(vaultId)}/settings/server';

  static String clientSettingsIn(String vaultId) =>
      '${vault(vaultId)}/settings/client';

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
      final paired = settings.value?.isPaired ?? false;
      final atConnect = state.matchedLocation == Routes.connect;
      final atPairing = state.matchedLocation == Routes.pairing;
      final atLogin = state.matchedLocation == Routes.login;
      final atSignup = state.matchedLocation == Routes.signup;
      final atAuthScreen = atConnect || atPairing || atLogin || atSignup;

      // Settings are still loading; hold still rather than flashing the
      // connect screen at someone who is already set up.
      if (settings.isLoading) return null;

      // **The web client bootstraps its own device rather than showing a QR.**
      // Storm served this page, so the browser already knows the server; the
      // document carries a short-lived nonce and this spends it. Fire-and-
      // forget: it saves settings on success, which notifies `refresh` and
      // re-runs this redirect with `paired` true, so the browser lands on
      // /login instead of /pairing. A returning browser short-circuits inside
      // `bootstrapWebDevice` on `isPaired` and mints nothing.
      //
      // Native clients are untouched — off the web the nonce reader is a stub
      // that returns null, and the QR flow is the only way in.
      if (kIsWeb && !paired && !configured) {
        unawaited(ref.read(settingsProvider.notifier).bootstrapWebDevice());
      }

      // The gallery needs no server, and bouncing it to Connect would make it
      // unreachable on exactly the install where the theme is being judged.
      if (state.matchedLocation == Routes.gallery) return null;

      // Set up — by pairing *or* by the legacy URL+token — so every auth
      // screen is behind us. Pairing is deliberately not required here: an
      // install that predates auth has a token and no device, and sending it
      // to /pairing would lock it out of a vault it can already read.
      if (configured) return atAuthScreen ? Routes.dashboard : null;

      // Paired, but no session — signed out, or the session was revoked. This
      // is /login's whole reason to exist: the device already has credentials,
      // so asking for a pairing QR again would be asking for something the
      // user does not have and does not need.
      // /signup is the same situation as /login — paired, no session — so it
      // has to be reachable from here, or the link on the login screen would
      // bounce straight back to the screen it was offered on.
      if (paired) return (atLogin || atSignup) ? null : Routes.login;

      // Nothing at all. Pairing is the first-run flow, but /connect stays
      // reachable for a server that has no auth yet. /login is not — there is
      // no device credential to log in with.
      if (atConnect || atPairing) return null;
      return Routes.pairing;
    },
    routes: [
      GoRoute(path: Routes.pairing, builder: (_, _) => const PairingScreen()),
      GoRoute(path: Routes.login, builder: (_, _) => const LoginScreen()),
      GoRoute(path: Routes.signup, builder: (_, _) => const SignupScreen()),
      GoRoute(
        path: Routes.addDevice,
        builder: (_, _) => const AddDeviceScreen(),
      ),
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
              // Inside the shell, so the sidebar stays beside them. At desk
              // width a settings screen that replaced the whole window would
              // be the one place the tree disappears.
              GoRoute(
                path: 'v/:vault/settings/server',
                builder: (_, _) => const ServerSettingsScreen(),
              ),
              GoRoute(
                path: 'v/:vault/settings/client',
                builder: (_, _) => const ClientSettingsScreen(),
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
