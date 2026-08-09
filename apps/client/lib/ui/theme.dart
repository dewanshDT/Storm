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

    final control = RoundedRectangleBorder(
      borderRadius: BorderRadius.circular(t.rControl),
    );

    return ThemeData(
      useMaterial3: true,
      colorScheme: scheme,
      extensions: [t],
      // Chrome is Plex Sans everywhere. Note *bodies* stay Newsreader — set on
      // the editor's own style, not here, so the two never compete.
      fontFamily: StormTokens.sansFamily,
      textTheme: textThemeFrom(t),
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
          fontFamily: StormTokens.sansFamily,
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
      listTileTheme: ListTileThemeData(
        dense: true,
        visualDensity: VisualDensity.compact,
        shape: control,
      ),
      cardTheme: CardThemeData(
        color: t.surface,
        elevation: 0,
        margin: EdgeInsets.zero,
        shape: RoundedRectangleBorder(
          borderRadius: BorderRadius.circular(t.rCard),
          side: BorderSide(color: t.border, width: t.bw),
        ),
      ),
      // Inputs and buttons take `rControl`; cards and sheets take `rCard`. That
      // split is the design's shape grammar, not a rounding preference.
      inputDecorationTheme: InputDecorationTheme(
        filled: true,
        fillColor: t.surface2,
        contentPadding: EdgeInsets.symmetric(
          horizontal: t.sp * 1.5,
          vertical: t.sp * 1.25,
        ),
        border: OutlineInputBorder(
          borderRadius: BorderRadius.circular(t.rControl),
          borderSide: BorderSide(color: t.border, width: t.bw),
        ),
        enabledBorder: OutlineInputBorder(
          borderRadius: BorderRadius.circular(t.rControl),
          borderSide: BorderSide(color: t.border, width: t.bw),
        ),
        focusedBorder: OutlineInputBorder(
          borderRadius: BorderRadius.circular(t.rControl),
          borderSide: BorderSide(color: t.accent, width: t.bw + 0.5),
        ),
        hintStyle: TextStyle(color: t.text3),
      ),
      filledButtonTheme: FilledButtonThemeData(
        style: FilledButton.styleFrom(shape: control),
      ),
      outlinedButtonTheme: OutlinedButtonThemeData(
        style: OutlinedButton.styleFrom(
          shape: control,
          side: BorderSide(color: t.border, width: t.bw),
        ),
      ),
      textButtonTheme: TextButtonThemeData(
        style: TextButton.styleFrom(shape: control),
      ),
      dialogTheme: DialogThemeData(
        backgroundColor: t.surface,
        surfaceTintColor: Colors.transparent,
        shape: RoundedRectangleBorder(
          borderRadius: BorderRadius.circular(t.rCard),
        ),
      ),
      bottomSheetTheme: BottomSheetThemeData(
        backgroundColor: t.surface,
        surfaceTintColor: Colors.transparent,
        shape: RoundedRectangleBorder(
          borderRadius: BorderRadius.vertical(top: Radius.circular(t.rCard)),
        ),
      ),
      popupMenuTheme: PopupMenuThemeData(
        color: t.surface2,
        surfaceTintColor: Colors.transparent,
        shape: RoundedRectangleBorder(
          borderRadius: BorderRadius.circular(t.rControl),
        ),
      ),
      snackBarTheme: SnackBarThemeData(
        behavior: SnackBarBehavior.floating,
        backgroundColor: t.surface2,
        contentTextStyle: TextStyle(color: t.text),
        shape: control,
      ),
    );
  }

  /// The five type roles, sized from the token scale.
  ///
  /// Material has more slots than the design has roles, so the extras are
  /// mapped onto the nearest role rather than invented: everything a widget
  /// might reach for resolves to one of display, heading, body, label or code.
  static TextTheme textThemeFrom(StormTokens t) {
    TextStyle sans(double size, FontWeight weight, Color color) => TextStyle(
      fontFamily: StormTokens.sansFamily,
      fontSize: size,
      fontWeight: weight,
      color: color,
      height: 1.35,
    );
    TextStyle mono(double size, Color color) => TextStyle(
      fontFamily: StormTokens.monoFamily,
      fontSize: size,
      color: color,
      height: 1.35,
    );

    return TextTheme(
      displayLarge: sans(t.displaySize, FontWeight.w600, t.text),
      displayMedium: sans(t.displaySize, FontWeight.w600, t.text),
      displaySmall: sans(t.displaySize, FontWeight.w600, t.text),
      headlineLarge: sans(t.headingSize, FontWeight.w600, t.text),
      headlineMedium: sans(t.headingSize, FontWeight.w600, t.text),
      headlineSmall: sans(t.headingSize, FontWeight.w600, t.text),
      titleLarge: sans(t.headingSize, FontWeight.w600, t.text),
      titleMedium: sans(t.bodySize, FontWeight.w500, t.text),
      titleSmall: sans(t.bodySize, FontWeight.w500, t.text2),
      bodyLarge: sans(t.bodySize, FontWeight.w400, t.text),
      bodyMedium: sans(t.bodySize, FontWeight.w400, t.text),
      bodySmall: sans(t.codeSize, FontWeight.w400, t.text2),
      // Labels are the mono face: metadata, counts, key chips, timestamps.
      labelLarge: sans(t.bodySize, FontWeight.w500, t.text),
      labelMedium: mono(t.labelSize, t.text2),
      labelSmall: mono(t.labelSize, t.text3),
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
  ///    signal, and `StatusDot` already reads it for that.
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

/// The rounded-square corner affordance. `rControl`, `surface`, hairline
/// border — the nav pill is the only round thing, and that difference is the
/// grammar the handoff asks to preserve.
class StormBubble extends StatelessWidget {
  const StormBubble({
    super.key,
    required this.child,
    this.onTap,
    this.onLongPress,
    this.size,
    this.tooltip,
  });

  final Widget child;
  final VoidCallback? onTap;
  final VoidCallback? onLongPress;
  final double? size;
  final String? tooltip;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final side = size ?? t.sp * 5.5;
    final radius = BorderRadius.circular(t.rControl);

    final bubble = Container(
      decoration: BoxDecoration(
        color: t.surface.withValues(alpha: 0.96),
        borderRadius: radius,
        border: Border.all(color: t.border, width: t.bw),
        boxShadow: t.shadow,
      ),
      child: Material(
        color: const Color(0x00000000),
        borderRadius: radius,
        clipBehavior: Clip.antiAlias,
        child: InkWell(
          onTap: onTap,
          onLongPress: onLongPress,
          child: SizedBox(
            width: side,
            height: side,
            child: Center(child: child),
          ),
        ),
      ),
    );
    return tooltip == null ? bubble : Tooltip(message: tooltip!, child: bubble);
  }
}
