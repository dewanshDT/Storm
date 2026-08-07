import 'package:flutter/material.dart';

/// Storm's visual language.
///
/// Dark, low chrome, indigo accent. Amber is reserved — it means "tag or
/// highlight" and nothing else, so it stays legible as a signal rather than
/// decoration.
abstract final class StormTheme {
  /// From the raindrop mark.
  static const indigo = Color(0xFF8FA0F0);
  static const amber = Color(0xFFE8B84B);

  /// The default note-body face. Chrome stays sans, so the two never compete.
  static const bodyFamily = 'Newsreader';

  static ThemeData dark() => _build(Brightness.dark);
  static ThemeData light() => _build(Brightness.light);

  static ThemeData _build(Brightness brightness) {
    final scheme = ColorScheme.fromSeed(
      seedColor: indigo,
      brightness: brightness,
      // Amber is a signal colour, not a third accent; keeping it out of the
      // generated scheme stops Material spending it on chrome.
      tertiary: amber,
    );

    return ThemeData(
      useMaterial3: true,
      colorScheme: scheme,
      scaffoldBackgroundColor: scheme.surface,
      // Low chrome: the app bar should not announce itself.
      appBarTheme: AppBarTheme(
        backgroundColor: scheme.surface,
        surfaceTintColor: Colors.transparent,
        elevation: 0,
        centerTitle: true,
        titleTextStyle: TextStyle(
          color: scheme.onSurface,
          fontSize: 16,
          fontWeight: FontWeight.w600,
        ),
      ),
      dividerTheme: DividerThemeData(
        color: scheme.outlineVariant.withValues(alpha: 0.4),
        thickness: 1,
        space: 1,
      ),
      listTileTheme: const ListTileThemeData(
        dense: true,
        visualDensity: VisualDensity.compact,
      ),
      snackBarTheme: SnackBarThemeData(
        behavior: SnackBarBehavior.floating,
        backgroundColor: scheme.inverseSurface,
      ),
    );
  }
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
