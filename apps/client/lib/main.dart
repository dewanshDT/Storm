import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_web_plugins/url_strategy.dart';

import 'agent/oauth_links.dart';
import 'router.dart';
import 'state/app_state.dart';
import 'ui/theme.dart';
import 'ui/tokens.dart';

void main() {
  // Clean URLs on the web, so a link to a note looks like a path rather than
  // a fragment. No effect on any other platform.
  usePathUrlStrategy();
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
    super.dispose();
  }

  void _showIntegrations() {
    if (OAuthLinks.instance.orphans.value.isEmpty) return;
    ref.read(routerProvider).go(Routes.integrations);
  }

  @override
  void didChangeAppLifecycleState(AppLifecycleState state) {
    // **Coming back is when sessions are found dead.** Storm is offline-first
    // and a phone can be away for weeks; the access token is the thing most
    // likely to have lapsed while nothing was watching.
    if (state == AppLifecycleState.resumed) _checkSession();
  }

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

    return MaterialApp.router(
      title: 'Storm',
      debugShowCheckedModeBanner: false,
      // One resolved theme rather than a light/dark pair: there are three
      // identities now, and which one is worn is the user's explicit choice —
      // the OS setting must not override it. Storm dark is the default because
      // the design is dark-first and a notes app is mostly read at night.
      theme: StormTheme.from(
        settings?.theme ?? StormPreset.stormDark,
        fontSize: settings?.fontSize,
      ),
      routerConfig: ref.watch(routerProvider),
    );
  }
}
