import 'package:flutter/material.dart';

import 'tokens.dart';

/// Storm's visual language, built entirely from [StormTokens].
///
/// Nothing here picks a colour. Every value comes from the token derivation, so
/// changing an input in `tokens.dart` moves the whole app and the three
/// identities stay one system rather than three palettes.
///
/// The [ColorScheme] is written out by hand rather than generated with
/// `ColorScheme.fromSeed`. A generated scheme invents a dozen colours from one
/// seed, which is exactly the per-screen drift the token layer exists to end —
/// and it would quietly override the measured contrast guarantees.
abstract final class StormTheme {
  /// The default note-body face. Chrome stays sans, so the two never compete.
  static const bodyFamily = StormTokens.serifFamily;

  static ThemeData dark() => from(StormPreset.stormDark);
  static ThemeData light() => from(StormPreset.stormLight);

  static ThemeData from(StormPreset preset, {double? fontSize}) =>
      _build(StormTokens.from(preset, fs: fontSize));

  static ThemeData _build(StormTokens t) {
    final scheme = schemeFrom(t);

    return ThemeData(
      useMaterial3: true,
      colorScheme: scheme,
      extensions: [t],
      // The page, not the card. `surface` in Material is the base; Storm's
      // cards sit on it as surfaceContainer.
      scaffoldBackgroundColor: t.bg,
      // Low chrome: the app bar should not announce itself.
      appBarTheme: AppBarTheme(
        backgroundColor: t.bg,
        surfaceTintColor: Colors.transparent,
        elevation: 0,
        centerTitle: true,
        titleTextStyle: TextStyle(
          color: t.text,
          fontSize: t.headingSize,
          fontWeight: FontWeight.w600,
        ),
      ),
      dividerTheme: DividerThemeData(
        color: t.border,
        thickness: t.bw,
        space: t.bw,
      ),
      listTileTheme: const ListTileThemeData(
        dense: true,
        visualDensity: VisualDensity.compact,
      ),
      snackBarTheme: SnackBarThemeData(
        behavior: SnackBarBehavior.floating,
        backgroundColor: t.surface2,
        contentTextStyle: TextStyle(color: t.text),
      ),
    );
  }

  /// Maps the tokens onto Material's roles.
  ///
  /// This is what makes the token layer worth landing before any widget is
  /// rewritten: every Material widget in the app — dialogs, switches,
  /// snackbars, text fields — re-skins from it immediately.
  ///
  /// Two mappings are load-bearing rather than arbitrary:
  ///  * `tertiary` is amber, because four existing call sites read amber as
  ///    `colorScheme.tertiary` and amber's meaning (tags and highlight) must
  ///    not become a generated accent.
  ///  * `outline` is [StormTokens.text3], because that is also the offline
  ///    signal, and `StormStatusDot` already reads `outline` for it.
  static ColorScheme schemeFrom(StormTokens t) => ColorScheme(
    brightness: t.brightness,
    primary: t.accent,
    onPrimary: t.onAccent,
    primaryContainer: t.accentSoft,
    onPrimaryContainer: t.text,
    secondary: t.accent,
    onSecondary: t.onAccent,
    secondaryContainer: t.accentSoft,
    onSecondaryContainer: t.text,
    tertiary: t.amber,
    onTertiary: t.bg,
    tertiaryContainer: t.amberSoft,
    onTertiaryContainer: t.text,
    error: t.danger,
    onError: t.brightness == Brightness.dark
        ? const Color(0xFF1A0F0E)
        : const Color(0xFFFFFFFF),
    errorContainer: t.surface2,
    onErrorContainer: t.danger,
    surface: t.bg,
    onSurface: t.text,
    onSurfaceVariant: t.text2,
    surfaceContainerLowest: t.bg,
    surfaceContainerLow: t.surface,
    surfaceContainer: t.surface,
    surfaceContainerHigh: t.surface2,
    surfaceContainerHighest: t.surface2,
    outline: t.text3,
    outlineVariant: t.border,
    inverseSurface: t.text,
    onInverseSurface: t.bg,
    shadow: const Color(0xFF000000),
    scrim: const Color(0xFF000000),
  );
}

/// A rounded-square bubble, the shape used for both corner affordances and
/// the floating navigation.
class StormBubble extends StatelessWidget {
  const StormBubble({
    super.key,
    required this.child,
    this.onTap,
    this.size = 42,
    this.tooltip,
  });

  final Widget child;
  final VoidCallback? onTap;
  final double size;
  final String? tooltip;

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    final bubble = Material(
      color: scheme.surfaceContainerHigh,
      borderRadius: BorderRadius.circular(size * 0.32),
      clipBehavior: Clip.antiAlias,
      child: InkWell(
        onTap: onTap,
        child: SizedBox(
          width: size,
          height: size,
          child: Center(child: child),
        ),
      ),
    );
    return tooltip == null ? bubble : Tooltip(message: tooltip!, child: bubble);
  }
}

/// The status dot shared by the vault bubble and any badged nav slot.
///
/// One visual language for state, so a dot always means the same thing
/// wherever it appears.
class StormStatusDot extends StatelessWidget {
  const StormStatusDot({super.key, required this.status, this.size = 10});

  final StormStatus status;
  final double size;

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    final color = switch (status) {
      StormStatus.synced => const Color(0xFF5CC28F),
      StormStatus.syncing => amberOf(context),
      StormStatus.offline => scheme.outline,
    };
    return Container(
      width: size,
      height: size,
      decoration: BoxDecoration(
        color: color,
        shape: BoxShape.circle,
        border: Border.all(color: scheme.surface, width: 1.5),
      ),
    );
  }

  static Color amberOf(BuildContext context) =>
      Theme.of(context).colorScheme.tertiary;
}

enum StormStatus { synced, syncing, offline }
