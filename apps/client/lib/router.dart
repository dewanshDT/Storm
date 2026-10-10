import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';

import 'state/app_state.dart';
import 'state/nav_memory.dart';
import 'ui/add_device_screen.dart';
import 'ui/breakpoints.dart';
import 'ui/browse_screen.dart';
import 'ui/gallery_screen.dart';
import 'ui/login_screen.dart';
import 'agent/agents_screen.dart';
import 'agent/agents_shell.dart';
import 'agent/agent_state.dart' show sessionTabOf;
import 'agent/session_screen.dart';
import 'ui/note_screen.dart';
import 'ui/pairing_screen.dart';
import 'ui/search_screen.dart';
import 'ui/settings/settings_shell.dart';
import 'ui/shell/app_shell.dart';
import 'ui/shell/notes_home.dart';
import 'ui/shell/vault_shell.dart';
import 'ui/starting_screen.dart';
import 'ui/tags_screen.dart';

/// Where the app can be.
///
/// A real router rather than a screen flag, because the navigation bubble's
/// Context slot has to answer "where am I" and there must be exactly one
/// answer. It also gives the web client working deep links, which the old
/// single-screen shell simply didn't have.
abstract final class Routes {
  /// Not a screen: redirects to this device's last location (handoff §1.5).
  static const root = '/';

  /// The Notes activity's entry: the last vault, or the no-vaults state.
  static const notes = '/notes';

  /// Shown while the app decides where to send you. Never navigated to
  /// directly — the redirect holds here instead of guessing.
  static const starting = '/starting';
  static const pairing = '/pairing';

  /// Sign in on a device that is already paired.
  static const login = '/login';

  /// Show a pairing QR so another device can join (from Devices & access).
  static const addDevice = '/add-device';

  static const agents = '/agents';

  /// One session, with its panel tab (`context`, `wrote` or `about`).
  static String agentSession(String id, {String? tab}) =>
      '/agents/s/${Uri.encodeComponent(id)}${tab == null ? '' : '?tab=$tab'}';

  /// The settings list on a phone; the first page at desk width.
  static const settings = '/settings';
  static String settingsPage(String id) => '/settings/$id';

  /// Kept at this path: the OAuth orphan relay in `main.dart` lands here.
  static const integrations = '/settings/integrations';

  /// Every shared widget in all three themes. Not linked from the app — it is
  /// a surface for judging the token layer, reached by typing the path.
  static const gallery = '/gallery';

  /// Everything note-shaped hangs off the vault, so a deep link carries which
  /// vault it means and back always retraces the real path.
  static String vault(String vaultId) => '/v/$vaultId';

  static String browse(String vaultId) => '${vault(vaultId)}/browse';
  static String search(String vaultId) => '${vault(vaultId)}/search';
  static String tags(String vaultId) => '${vault(vaultId)}/tags';

  /// [session] marks a note pushed from that agent session, which its back
  /// link then names.
  static String note(String vaultId, String id, {String? session}) =>
      '${vault(vaultId)}/note/$id'
      '${session == null ? '' : '?session=${Uri.encodeQueryComponent(session)}'}';

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
    initialLocation: Routes.root,
    refreshListenable: refresh,
    redirect: (context, state) {
      final settings = ref.read(settingsProvider);
      final configured = settings.value?.isConfigured ?? false;
      final paired = settings.value?.isPaired ?? false;
      final atPairing = state.matchedLocation == Routes.pairing;
      final atLogin = state.matchedLocation == Routes.login;
      final atAuthScreen = atPairing || atLogin;

      // **Hold on a neutral screen rather than guessing.** `null` means "stay
      // where you are", which on a cold start is the dashboard — so returning
      // it here rendered the vault shell for someone who may not even be
      // signed in. Holding costs a frame; guessing costs a wrong screen.
      if (settings.isLoading) {
        return state.matchedLocation == Routes.starting
            ? null
            : Routes.starting;
      }

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
        final notifier = ref.read(settingsProvider.notifier);

        // **The bootstrap has run and produced nothing — fall through to the
        // pairing screen.** This branch used to return `Routes.starting` on
        // every path, including this one, so a browser that could not
        // bootstrap waited on the brand mark forever with no route out.
        //
        // Failing to mint is *expected*, not exceptional: the server declines
        // behind a reverse proxy (a forwarding header hides the real peer),
        // over the per-peer rate limit, over the outstanding ceiling, and when
        // it has no peer address to bind to. The comments here always claimed
        // the pairing screen was the fallback; nothing implemented it.
        if (notifier.bootstrapFailed) {
          return state.matchedLocation == Routes.pairing
              ? null
              : Routes.pairing;
        }

        // Not paired *yet* — the bootstrap is in flight. Sending them to the
        // QR screen now means showing a pairing screen they must not use and
        // taking it away a moment later, which is the flicker this avoids.
        if (notifier.bootstrapping) {
          return state.matchedLocation == Routes.starting
              ? null
              : Routes.starting;
        }
        unawaited(notifier.bootstrapWebDevice());
        return state.matchedLocation == Routes.starting
            ? null
            : Routes.starting;
      }

      // The gallery needs no server, and bouncing it to Connect would make it
      // unreachable on exactly the install where the theme is being judged.
      if (state.matchedLocation == Routes.gallery) return null;

      // Set up — by pairing *or* by the legacy URL+token — so every auth
      // screen is behind us. Pairing is deliberately not required here: an
      // install that predates auth has a token and no device, and sending it
      // to /pairing would lock it out of a vault it can already read.
      // Anyone who reached /starting and is now settled gets moved on; it is
      // a waiting room, not a destination.
      final atStarting = state.matchedLocation == Routes.starting;

      if (configured) {
        if (atAuthScreen || atStarting) {
          return ref.read(navMemoryProvider).launch;
        }
        return _shellRedirect(state, ref);
      }

      // Paired, but no session — signed out, or the session was revoked. This
      // is /login's whole reason to exist: the device already has credentials,
      // so asking for a pairing QR again would be asking for something the
      // user does not have and does not need.
      if (paired) return atLogin ? null : Routes.login;
      if (atStarting) return Routes.pairing;

      // Nothing at all. Pairing is the only first-run flow now — /connect
      // went with the shared token it existed to collect, and /login needs a
      // device credential this install does not have.
      if (atPairing) return null;
      return Routes.pairing;
    },
    routes: [
      GoRoute(path: Routes.starting, builder: (_, _) => const StartingScreen()),
      GoRoute(path: Routes.pairing, builder: (_, _) => const PairingScreen()),
      GoRoute(path: Routes.login, builder: (_, _) => const LoginScreen()),
      GoRoute(
        path: Routes.addDevice,
        builder: (_, _) => const AddDeviceScreen(),
      ),
      GoRoute(path: Routes.gallery, builder: (_, _) => const GalleryScreen()),
      ShellRoute(
        builder: (_, state, child) =>
            _shell(AppShell(location: state.uri.toString(), child: child)),
        routes: [
          GoRoute(
            path: Routes.notes,
            builder: (_, _) => _leaf(const NotesHome()),
          ),
          // The vault's screens share `VaultShell`, which carries `VaultGate`
          // (the route's vault is active before its children build) and, at
          // desk width, the folder tree.
          ShellRoute(
            builder: (_, _, child) => _shell(VaultShell(child: child)),
            routes: [
              GoRoute(
                path: '/v/:vault/browse/:path(.*)',
                builder: (_, state) =>
                    _leaf(BrowseScreen(folder: Routes.folderOf(state.uri))),
              ),
              GoRoute(
                path: '/v/:vault/browse',
                builder: (_, _) => _leaf(const BrowseScreen(folder: '')),
              ),
              GoRoute(
                path: '/v/:vault/note/:id',
                builder: (_, state) => _leaf(
                  NoteScreen(
                    noteId: state.pathParameters['id']!,
                    fromSession: state.uri.queryParameters['session'],
                  ),
                ),
              ),
              GoRoute(
                path: '/v/:vault/search',
                builder: (_, _) => _leaf(const SearchScreen()),
              ),
              GoRoute(
                path: '/v/:vault/tags',
                builder: (_, _) => _leaf(const TagsScreen()),
              ),
            ],
          ),
          ShellRoute(
            builder: (_, state, child) =>
                _shell(AgentsShell(location: state.uri, child: child)),
            routes: [
              GoRoute(
                path: Routes.agents,
                builder: (_, _) => _leaf(const AgentsScreen()),
              ),
              GoRoute(
                path: '/agents/s/:id',
                builder: (_, state) => _leaf(
                  SessionScreen(
                    sessionId: state.pathParameters['id']!,
                    tab: sessionTabOf(state.uri.queryParameters['tab']),
                  ),
                ),
              ),
            ],
          ),
          ShellRoute(
            builder: (_, state, child) => _shell(
              SettingsShell(page: state.pathParameters['page'], child: child),
            ),
            routes: [
              GoRoute(
                path: Routes.settings,
                builder: (context, _) => _leaf(
                  context.isExpanded
                      ? settingsPageFor('device')
                      : const SettingsListScreen(),
                ),
              ),
              GoRoute(
                path: '/settings/:page',
                builder: (_, state) =>
                    _leaf(settingsPageFor(state.pathParameters['page']!)),
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

/// Retired locations, `/` and the Notes entry, for a signed-in device.
String? _shellRedirect(GoRouterState state, Ref ref) {
  final path = state.uri.path;
  final seg = state.uri.pathSegments;
  final memory = ref.read(navMemoryProvider);

  if (path == Routes.root) return memory.launch;
  if (path == Routes.notes) {
    return memory.notes.isEmpty ? null : memory.notes;
  }
  if (path == '/settings/server') return Routes.settingsPage('vaults');
  if (path == '/settings/mcp-keys') return Routes.settingsPage('access');
  if (path == '/agents/hosts') return Routes.settingsPage('hosts');
  if (seg.length == 4 && seg[0] == 'v' && seg[2] == 'settings') {
    return Routes.settingsPage(seg[3] == 'client' ? 'device' : 'vaults');
  }
  if (seg.length == 2 && seg[0] == 'settings' && !isSettingsPage(seg[1])) {
    return Routes.settings;
  }
  return null;
}

/// Nudges GoRouter to re-run its redirect without replacing the router.
class _RouterRefresh extends ChangeNotifier {
  void notify() => notifyListeners();
}

/// Every signed-in page claims system back while it has somewhere to go.
Widget _leaf(Widget page) => LogicalBack(child: page);

/// And so does each shell around it: a navigator whose pages changed reports
/// for its own top page, and that report can arrive after the page's.
Widget _shell(Widget shell) => LogicalBack(shell: true, child: shell);
