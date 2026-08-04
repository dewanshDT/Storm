import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import 'state/app_state.dart';
import 'ui/connect_screen.dart';
import 'ui/vault_screen.dart';

void main() => runApp(const ProviderScope(child: StormApp()));

class StormApp extends ConsumerWidget {
  const StormApp({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final settings = ref.watch(settingsProvider);
    final dark = settings.value?.darkMode ?? false;

    return MaterialApp(
      title: 'Storm',
      debugShowCheckedModeBanner: false,
      theme: ThemeData(
        useMaterial3: true,
        brightness: Brightness.light,
        colorSchemeSeed: const Color(0xFF5B6ABF),
      ),
      darkTheme: ThemeData(
        useMaterial3: true,
        brightness: Brightness.dark,
        colorSchemeSeed: const Color(0xFF8FA0F0),
      ),
      themeMode: dark ? ThemeMode.dark : ThemeMode.light,
      home: switch (settings) {
        AsyncData(:final value) =>
          value.isConfigured ? const VaultScreen() : const ConnectScreen(),
        AsyncError(:final error) => Scaffold(
          body: Center(child: Text('Failed to load settings: $error')),
        ),
        _ => const Scaffold(body: Center(child: CircularProgressIndicator())),
      },
    );
  }
}
