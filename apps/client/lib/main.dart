import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart' show rootBundle;
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_web_plugins/url_strategy.dart';
import 'package:window_manager/window_manager.dart';

import 'agent/agent_state.dart' show sessionKeepAliveProvider;
import 'agent/desktop_notifications.dart'
    show
        NotificationTapNone,
        NotificationTapNote,
        NotificationTapSession,
        desktopNotifierProvider,
        notificationTapped;
import 'agent/oauth_links.dart';
import 'router.dart';
import 'state/app_state.dart';
import 'ui/shell/window_buttons.dart' show initWindowChrome;
import 'ui/theme.dart';
import 'ui/themes/theme_library.dart';

Future<void> main() async {
  // Clean URLs on the web, so a link to a note looks like a path rather than
  // a fragment. No effect on any other platform.
  usePathUrlStrategy();
  WidgetsFlutterBinding.ensureInitialized();
  await initWindowChrome();
  try {
    await ThemeLibrary.load(rootBundle).timeout(const Duration(seconds: 2));
  } catch (e) {
    debugPrint('Themes not loaded, using the built-in ones: $e');
  }
  runApp(const ProviderScope(child: StormApp()));
}

class StormApp extends ConsumerStatefulWidget {
  const StormApp({super.key});

  @override
  ConsumerState<StormApp> createState() => _StormAppState();
}

class _StormAppState extends ConsumerState<StormApp>
    with WidgetsBindingObserver {
  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addObserver(this);
    // Integration sign-ins come back as `storm://oauth` links on Android and
    // macOS (spec §10.4, 81l). One this process was not waiting for — the
    // app was killed while the browser was open — opens Integrations, which
    // relays it. Listening before `start`, which may deliver one at once.
    if (!kIsWeb) {
      OAuthLinks.instance.orphans.addListener(_showIntegrations);
      OAuthLinks.instance.start();
    }
    // A tap on a desktop OS notification (decision 161, slice 161-4):
    // this process was not necessarily in front when it happened.
    notificationTapped.addListener(_onNotificationTapped);
    // Once, at startup. **Not `addPostFrameCallback`** — that fires after the
    // first frame, which is well before `settingsProvider` has finished
    // loading preferences and the keychain, so the check ran against a null
    // value and silently did nothing. `ensureSession` now awaits the provider
    // itself, so calling it here is safe and no longer racy.
    _checkSession();
  }

  @override
  void dispose() {
    WidgetsBinding.instance.removeObserver(this);
    OAuthLinks.instance.orphans.removeListener(_showIntegrations);
    notificationTapped.removeListener(_onNotificationTapped);
    super.dispose();
  }

  void _showIntegrations() {
    if (OAuthLinks.instance.orphans.value.isEmpty) return;
    ref.read(routerProvider).go(Routes.integrations);
  }

  /// Brings the window forward, then routes — in that order, so the
  /// person sees Storm come to the front onto the right place rather than
  /// a flash of wherever it last was.
  void _onNotificationTapped() {
    final target = notificationTapped.value;
    if (target == null) return;
    notificationTapped.value = null;
    unawaited(_bringToFront());
    switch (target) {
      case NotificationTapSession(:final sessionId):
        ref.read(routerProvider).go(Routes.agentSession(sessionId));
      case NotificationTapNote(:final vaultId, :final noteId):
        ref.read(routerProvider).go(Routes.note(vaultId, noteId));
      case NotificationTapNone():
        break; // Bringing the window forward above is all there is.
    }
  }

  Future<void> _bringToFront() async {
    try {
      await windowManager.show();
      await windowManager.focus();
    } catch (_) {
      // Not a desktop window (shouldn't happen: only DesktopNotifier sets
      // notificationTapped, and it exists only on desktop) — nothing to
      // bring forward.
    }
  }

  @override
  void didChangeAppLifecycleState(AppLifecycleState state) {
    // **Coming back is when sessions are found dead.** Storm is offline-first
    // and a phone can be away for weeks; the access token is the thing most
    // likely to have lapsed while nothing was watching.
    if (state == AppLifecycleState.resumed) {
      _checkSession();
      // A suspended phone's sockets may be dead without a FIN, and the
      // trunk's silence clock stopped while it slept (§8.7 rule 3).
      unawaited(ref.read(connectionProvider)?.onAppResumed());
      ref.read(sessionKeepAliveProvider).foreground();
    } else if (state == AppLifecycleState.hidden ||
        state == AppLifecycleState.paused) {
      // No terminal stream runs behind the person's back (decision 154).
      ref.read(sessionKeepAliveProvider).background();
    }
  }

  @override
  void didHaveMemoryPressure() =>
      ref.read(sessionKeepAliveProvider).memoryPressure();

  /// Renews a stale session, retires a dead one.
  ///
  /// Fire-and-forget on purpose: nothing here should delay the first frame,
  /// and every outcome is expressed by the settings it saves — the router is
  /// listening and moves to /login by itself when a session is cleared.
  void _checkSession() {
    ref.read(settingsProvider.notifier).ensureSession();
  }

  @override
  Widget build(BuildContext context) {
    final settings = ref.watch(settingsProvider).value;

    final choice = settings?.theme ?? ThemeChoice.initial;
    final fontSize = settings?.fontSize;
    final router = ref.watch(routerProvider);
    // Alive for as long as the app itself is (161-4): watched here, not
    // read, so Riverpod keeps the plugin registration and the events
    // subscription up rather than tearing it down between frames.
    ref.watch(desktopNotifierProvider);

    // Both members of the chosen family, and the mode: with `system`,
    // Flutter follows the platform's brightness live (decision 160). A
    // family with one member gives it for both.
    return ValueListenableBuilder(
      valueListenable: ThemeLibrary.families,
      builder: (context, _, _) {
        final themes = StormTheme.forChoice(choice, fontSize: fontSize);
        return MaterialApp.router(
          title: 'Storm',
          debugShowCheckedModeBanner: false,
          theme: themes.light,
          darkTheme: themes.dark,
          themeMode: choice.mode,
          routerConfig: router,
          scaffoldMessengerKey: rootMessengerKey,
        );
      },
    );
  }
}
