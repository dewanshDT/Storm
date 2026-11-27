import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:storm/ui/theme.dart';
import 'package:storm/ui/themes/theme_config.dart';
import 'package:storm/ui/themes/theme_library.dart';
import 'package:storm/ui/tokens.dart';
import 'package:widgetbook/widgetbook.dart';

const _themeFolder = 'packages/storm/assets/themes/';

/// Reads the client's theme configs the way its `ThemeLibrary.load` does.
///
/// That loader only matches `assets/themes/`, which is where the files live in
/// the client's own bundle; as a path dependency they are keyed under
/// `packages/storm/`, so it would find nothing here.
Future<void> loadStormThemes(AssetBundle bundle) async {
  final manifest = await AssetManifest.loadFromAssetBundle(bundle);
  final paths =
      manifest
          .listAssets()
          .where((p) => p.startsWith(_themeFolder) && p.endsWith('.json'))
          .toList()
        ..sort();
  final loaded = <ThemeFamily>[];
  for (final path in paths) {
    loaded.add(parseThemeFamily(await bundle.loadString(path), source: path));
  }
  ThemeLibrary.families.value = ThemeLibrary.merge(loaded);
}

/// Every variant of every family. A variant is already light or dark, so
/// there is no separate mode switch: the brightness is part of the name.
List<WidgetbookTheme<ThemeData>> stormThemes() => [
  for (final family in ThemeLibrary.families.value)
    for (final variant in [family.dark, family.light].whereType<ThemeVariant>())
      WidgetbookTheme(
        name:
            '${family.label} · ${variant.inputs.dark ? 'dark' : 'light'}'
            ' (${variant.label})',
        data: StormTheme.from(variant),
      ),
];

ThemeAddon<ThemeData> stormThemeAddon() {
  final themes = stormThemes();
  return ThemeAddon<ThemeData>(
    themes: themes,
    initialTheme: themes.first,
    themeBuilder: (context, theme, child) => Theme(
      data: theme,
      child: Material(
        color: theme.scaffoldBackgroundColor,
        child: DefaultTextStyle(
          style: theme.textTheme.bodyMedium!,
          child: child,
        ),
      ),
    ),
  );
}

const _phone = ViewportData(
  name: 'Phone',
  width: 390,
  height: 844,
  pixelRatio: 3,
  platform: TargetPlatform.android,
);
const _tablet = ViewportData(
  name: 'Tablet',
  width: 820,
  height: 1180,
  pixelRatio: 2,
  platform: TargetPlatform.android,
);
const _desktop = ViewportData(
  name: 'Desktop',
  width: 1280,
  height: 800,
  pixelRatio: 1,
  platform: TargetPlatform.macOS,
);

List<WidgetbookAddon> stormAddons() => [
  stormThemeAddon(),
  ViewportAddon([Viewports.none, _phone, _tablet, _desktop]),
  TextScaleAddon(min: 1.0, max: 2.0, divisions: 10),
];
