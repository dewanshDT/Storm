import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../tokens.dart';
import 'theme_config.dart';

/// Every theme family the app knows, Storm's first (decision 160).
///
/// Starts with the built-in Storm and SlowFlow, so nothing waits on a file;
/// [load] replaces them with what `assets/themes/` holds.
abstract final class ThemeLibrary {
  static const folder = 'assets/themes/';

  static const builtIn = [
    ThemeFamily(
      id: 'storm',
      label: 'Storm',
      light: StormPreset.stormLight,
      dark: StormPreset.stormDark,
    ),
    ThemeFamily(
      id: 'slowflow',
      label: 'SlowFlow',
      light: StormPreset.slowflowEarth,
    ),
  ];

  static final families = ValueNotifier<List<ThemeFamily>>(builtIn);

  static ThemeFamily family(String id) => families.value.firstWhere(
    (f) => f.id == id,
    orElse: () => families.value.first,
  );

  /// Reads every config under [folder]. A file that does not parse is skipped
  /// with a log line; a built-in family it would have replaced stays.
  static Future<void> load(AssetBundle bundle) async {
    final manifest = await AssetManifest.loadFromAssetBundle(bundle);
    final paths =
        manifest
            .listAssets()
            .where((p) => p.startsWith(folder) && p.endsWith('.json'))
            .toList()
          ..sort();
    final loaded = <ThemeFamily>[];
    for (final path in paths) {
      try {
        loaded.add(
          parseThemeFamily(await bundle.loadString(path), source: path),
        );
      } catch (e) {
        debugPrint('Theme skipped: $e');
      }
    }
    families.value = merge(loaded);
  }

  /// [loaded] over the built-ins: Storm, then SlowFlow, then the rest by name.
  static List<ThemeFamily> merge(List<ThemeFamily> loaded) {
    final byId = {for (final f in builtIn) f.id: f};
    for (final f in loaded) {
      byId[f.id] = f;
    }
    int rank(ThemeFamily f) => switch (f.id) {
      'storm' => 0,
      'slowflow' => 1,
      _ => 2,
    };
    return byId.values.toList()..sort(
      (a, b) => rank(a) != rank(b)
          ? rank(a) - rank(b)
          : a.label.toLowerCase().compareTo(b.label.toLowerCase()),
    );
  }
}

/// What the person chose: a family, and whether it follows the system.
@immutable
class ThemeChoice {
  const ThemeChoice(this.family, this.mode);

  /// A new install: Storm, dark-first like the design.
  static const initial = ThemeChoice('storm', ThemeMode.dark);

  final String family;
  final ThemeMode mode;

  /// How it is stored: `family/mode`.
  String get wire => '$family/${mode.name}';

  /// The stored value, migrating the three pre-160 preset names.
  ///
  /// Anything unreadable is Storm following the system rather than an error,
  /// so a downgrade-then-upgrade or a hand-edited pref still opens the app.
  static ThemeChoice parse(String? wire) {
    switch (wire) {
      case 'storm-dark':
        return const ThemeChoice('storm', ThemeMode.dark);
      case 'storm-light':
        return const ThemeChoice('storm', ThemeMode.light);
      case 'slowflow-earth':
        return const ThemeChoice('slowflow', ThemeMode.light);
    }
    const fallback = ThemeChoice('storm', ThemeMode.system);
    final parts = wire?.split('/');
    if (parts == null || parts.length != 2) return fallback;
    final mode = ThemeMode.values.where((m) => m.name == parts[1]);
    if (parts[0].isEmpty || mode.isEmpty) return fallback;
    return ThemeChoice(parts[0], mode.first);
  }

  /// The variant worn when the platform is [platform].
  ThemeVariant resolve(Brightness platform) =>
      ThemeLibrary.family(family).variantFor(switch (mode) {
        ThemeMode.system => platform,
        ThemeMode.light => Brightness.light,
        ThemeMode.dark => Brightness.dark,
      });

  ThemeChoice copyWith({String? family, ThemeMode? mode}) =>
      ThemeChoice(family ?? this.family, mode ?? this.mode);

  @override
  bool operator ==(Object other) =>
      other is ThemeChoice && other.family == family && other.mode == mode;

  @override
  int get hashCode => Object.hash(family, mode);

  @override
  String toString() => wire;
}
