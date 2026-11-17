import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../state/app_state.dart';
import '../themes/theme_audit.dart';
import '../themes/theme_config.dart';
import '../themes/theme_library.dart';
import '../tokens.dart';
import 'settings_widgets.dart';

/// Settings › This device › Appearance: a card per theme family, and whether
/// it follows the system (decision 160).
class ThemePicker extends ConsumerWidget {
  const ThemePicker({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final t = context.tokens;
    final settings = ref.watch(settingsProvider).value ?? const Settings();
    final choice = settings.theme;
    void choose(ThemeChoice next) => ref
        .read(settingsProvider.notifier)
        .save(settings.copyWith(theme: next));
    final shown = switch (choice.mode) {
      ThemeMode.light => Brightness.light,
      ThemeMode.dark => Brightness.dark,
      ThemeMode.system => MediaQuery.platformBrightnessOf(context),
    };

    return ValueListenableBuilder(
      valueListenable: ThemeLibrary.families,
      builder: (context, families, _) => Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          _ModeControl(
            mode: choice.mode,
            onChanged: (m) => choose(choice.copyWith(mode: m)),
          ),
          SizedBox(height: t.sp * 1.5),
          Wrap(
            spacing: t.sp,
            runSpacing: t.sp,
            children: [
              for (final f in families)
                _FamilyCard(
                  family: f,
                  brightness: shown,
                  selected: f.id == ThemeLibrary.family(choice.family).id,
                  onTap: () => choose(choice.copyWith(family: f.id)),
                ),
            ],
          ),
        ],
      ),
    );
  }
}

/// "Low contrast: secondary text, terminal yellow", or null when every pair
/// passes.
String? lowContrastNote(ThemeVariant v) {
  final low = lowContrast(StormTokens.from(v));
  if (low.isEmpty) return null;
  return 'Low contrast: ${low.map((c) => c.part).join(', ')}';
}

class _ModeControl extends StatelessWidget {
  const _ModeControl({required this.mode, required this.onChanged});

  final ThemeMode mode;
  final ValueChanged<ThemeMode> onChanged;

  static const _labels = {
    ThemeMode.system: 'System',
    ThemeMode.light: 'Light',
    ThemeMode.dark: 'Dark',
  };

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final radius = BorderRadius.circular(t.rControl);
    return Semantics(
      container: true,
      label: 'Mode',
      child: DecoratedBox(
        decoration: BoxDecoration(
          borderRadius: radius,
          border: Border.all(color: t.border, width: t.bw),
        ),
        child: Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            for (final MapEntry(key: m, value: label) in _labels.entries)
              Semantics(
                button: true,
                selected: m == mode,
                label: 'Mode, $label',
                onTap: () => onChanged(m),
                excludeSemantics: true,
                child: Material(
                  key: Key('theme-mode-${m.name}'),
                  color: m == mode ? t.accentSoft : Colors.transparent,
                  borderRadius: radius,
                  child: InkWell(
                    onTap: () => onChanged(m),
                    borderRadius: radius,
                    child: Padding(
                      padding: EdgeInsets.symmetric(
                        horizontal: t.sp * 1.75,
                        vertical: t.sp * 0.75,
                      ),
                      child: Text(
                        label,
                        style: TextStyle(
                          fontFamily: StormTokens.sansFamily,
                          fontSize: t.codeSize,
                          fontWeight: m == mode
                              ? FontWeight.w500
                              : FontWeight.w400,
                          color: m == mode ? t.accent : t.text,
                        ),
                      ),
                    ),
                  ),
                ),
              ),
          ],
        ),
      ),
    );
  }
}

class _FamilyCard extends StatelessWidget {
  const _FamilyCard({
    required this.family,
    required this.brightness,
    required this.selected,
    required this.onTap,
  });

  final ThemeFamily family;
  final Brightness brightness;
  final bool selected;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final variant = family.variantFor(brightness);
    final own = StormTokens.from(variant);
    final only = family.has(brightness)
        ? null
        : family.light == null
        ? 'Dark only'
        : 'Light only';
    final credit = family.inspiredBy == null
        ? null
        : 'Inspired by ${family.inspiredBy!.name}';
    final low = lowContrastNote(variant);
    final lines = [
      if (variant.label != family.label) variant.label,
      ?credit,
      ?only,
    ];
    final radius = BorderRadius.circular(t.rCard * 0.75);
    final small = TextStyle(
      fontFamily: StormTokens.sansFamily,
      fontSize: t.labelSize,
      color: t.text2,
    );

    return Semantics(
      button: true,
      selected: selected,
      label: [family.label, ...lines, ?low].join(', '),
      onTap: onTap,
      excludeSemantics: true,
      child: SizedBox(
        width: t.sp * 26,
        child: Material(
          key: Key('theme-family-${family.id}'),
          color: selected ? t.accentSoft : t.surface,
          shape: RoundedRectangleBorder(
            borderRadius: radius,
            side: BorderSide(
              color: selected ? t.accent : t.border,
              width: selected ? t.bw * 2 : t.bw,
            ),
          ),
          child: InkWell(
            onTap: onTap,
            borderRadius: radius,
            child: Padding(
              padding: EdgeInsets.all(t.sp * 1.25),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  ThemePreviewStrip(tokens: own),
                  SizedBox(height: t.sp),
                  Text(
                    family.label,
                    style: TextStyle(
                      fontFamily: StormTokens.sansFamily,
                      fontSize: t.codeSize,
                      fontWeight: FontWeight.w500,
                      color: t.text,
                    ),
                  ),
                  if (lines.isNotEmpty) Text(lines.join(' · '), style: small),
                  if (low != null)
                    Text(
                      low,
                      key: Key('theme-low-${family.id}'),
                      style: small.copyWith(color: t.danger),
                    ),
                ],
              ),
            ),
          ),
        ),
      ),
    );
  }
}

/// A theme in miniature: its page, a panel with its text, its accent, and a
/// few of its terminal colours on the terminal's ground.
class ThemePreviewStrip extends StatelessWidget {
  const ThemePreviewStrip({super.key, required this.tokens});

  final StormTokens tokens;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final o = tokens;
    final h = t.sp * 4;
    final r = BorderRadius.circular(t.rControl * 0.6);
    Widget dot(Color c) => Container(
      width: t.sp,
      height: t.sp,
      margin: EdgeInsets.only(right: t.sp * 0.4),
      decoration: BoxDecoration(color: c, shape: BoxShape.circle),
    );
    return ExcludeSemantics(
      child: Container(
        height: h,
        decoration: BoxDecoration(
          color: o.bg,
          borderRadius: r,
          border: Border.all(color: o.border, width: t.bw),
        ),
        clipBehavior: Clip.antiAlias,
        child: Row(
          children: [
            Container(
              width: t.sp * 7,
              color: o.surface,
              alignment: Alignment.center,
              child: Text(
                'Aa',
                style: TextStyle(
                  fontFamily: StormTokens.sansFamily,
                  fontSize: t.codeSize,
                  fontWeight: FontWeight.w600,
                  color: o.text,
                ),
              ),
            ),
            SizedBox(width: t.sp),
            Container(
              width: t.sp * 3,
              height: t.sp * 1.5,
              decoration: BoxDecoration(
                color: o.accent,
                borderRadius: BorderRadius.circular(t.sp),
              ),
            ),
            const Spacer(),
            Container(
              height: h,
              padding: EdgeInsets.symmetric(horizontal: t.sp * 0.75),
              color: o.terminal.background,
              child: Row(
                children: [
                  for (final i in [1, 2, 3, 4, 5]) dot(o.terminal.ansi[i]),
                ],
              ),
            ),
          ],
        ),
      ),
    );
  }
}

/// Settings › About: who the inspired themes are inspired by.
class ThemeCredits extends StatelessWidget {
  const ThemeCredits({super.key});

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return ValueListenableBuilder(
      valueListenable: ThemeLibrary.families,
      builder: (context, families, _) {
        final credited = [
          for (final f in families)
            if (f.inspiredBy != null) (f, f.inspiredBy!),
        ];
        if (credited.isEmpty) return const SizedBox.shrink();
        return Column(
          key: const Key('theme-credits'),
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            GroupLabel('Theme credits', top: t.sp * 3),
            for (final (f, c) in credited)
              Padding(
                padding: EdgeInsets.only(top: t.sp * 0.5),
                child: SelectableText(
                  '${f.label}: inspired by ${c.name} (${c.url}, ${c.licence})',
                  style: TextStyle(
                    fontFamily: StormTokens.sansFamily,
                    fontSize: t.codeSize,
                    color: t.text2,
                  ),
                ),
              ),
          ],
        );
      },
    );
  }
}
