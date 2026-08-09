import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_web_plugins/url_strategy.dart';

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

class StormApp extends ConsumerWidget {
  const StormApp({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
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
