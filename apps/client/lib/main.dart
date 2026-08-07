import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_web_plugins/url_strategy.dart';

import 'router.dart';
import 'state/app_state.dart';
import 'ui/theme.dart';

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
    final dark = ref.watch(settingsProvider).value?.darkMode ?? true;

    return MaterialApp.router(
      title: 'Storm',
      debugShowCheckedModeBanner: false,
      theme: StormTheme.light(),
      darkTheme: StormTheme.dark(),
      // Dark by default: the design is dark-first, and a notes app is mostly
      // read at night.
      themeMode: dark ? ThemeMode.dark : ThemeMode.light,
      routerConfig: ref.watch(routerProvider),
    );
  }
}
