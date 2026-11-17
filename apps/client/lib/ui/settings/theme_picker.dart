import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../state/app_state.dart';
import '../controls.dart';
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
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          ChoiceChips<ThemeMode>(
            key: const Key('theme-mode'),
            options: [
              for (final (m, label) in const [
                (ThemeMode.system, 'System'),
                (ThemeMode.light, 'Light'),
                (ThemeMode.dark, 'Dark'),
              ])
                ChoiceOption(m, label, key: Key('theme-mode-${m.name}')),
            ],
            selected: choice.mode,
            onSelected: (m) => choose(choice.copyWith(mode: m)),
          ),
          SizedBox(height: t.sp * 2),
          ChoiceCards<String>(
            options: [for (final f in families) _option(f, shown, t)],
            selected: ThemeLibrary.family(choice.family).id,
            onSelected: (id) => choose(choice.copyWith(family: id)),
          ),
        ],
      ),
    );
  }

  ChoiceCardOption<String> _option(
    ThemeFamily family,
    Brightness brightness,
    StormTokens t,
  ) {
    final variant = family.variantFor(brightness);
    final only = family.has(brightness)
        ? null
        : family.light == null
        ? 'Dark only'
        : 'Light only';
    final credit = family.inspiredBy == null
        ? null
        : 'Inspired by ${family.inspiredBy!.name}';
    final note = [?credit, ?only].join(' · ');
    final low = lowContrastNote(variant);
    final title = familyTitle(family);
    return ChoiceCardOption(
      family.id,
      title,
      key: Key('theme-family-${family.id}'),
      subtitle: variant.label,
      note: note.isEmpty ? null : note,
      preview: ThemePreviewStrip(tokens: StormTokens.from(variant)),
      semanticLabel: [title, variant.label, ?credit, ?only, ?low].join(', '),
      alert: low == null
          ? null
          : Text(
              low,
              key: Key('theme-low-${family.id}'),
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: TextStyle(
                fontFamily: StormTokens.sansFamily,
                fontSize: t.labelSize,
                color: t.danger,
              ),
            ),
    );
  }
}

/// The family's name without the "(inspired)" its config label carries; the
/// card says who it is inspired by on its own line.
String familyTitle(ThemeFamily family) {
  const suffix = ' (inspired)';
  final l = family.label;
  return family.inspiredBy != null && l.endsWith(suffix)
      ? l.substring(0, l.length - suffix.length)
      : l;
}

/// "Low contrast: secondary text, terminal yellow", or null when every pair
/// passes.
String? lowContrastNote(ThemeVariant v) {
  final low = lowContrast(StormTokens.from(v));
  if (low.isEmpty) return null;
  return 'Low contrast: ${low.map((c) => c.part).join(', ')}';
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
    final r = BorderRadius.circular(t.rControl * 0.6);
    final dot = t.sp * 0.875;
    return ExcludeSemantics(
      child: Container(
        height: t.sp * 3.5,
        decoration: BoxDecoration(
          color: o.bg,
          borderRadius: r,
          border: Border.all(color: o.border, width: t.bw),
        ),
        clipBehavior: Clip.antiAlias,
        child: Row(
          children: [
            Container(
              width: t.sp * 5,
              color: o.surface,
              padding: EdgeInsets.symmetric(horizontal: t.sp),
              alignment: Alignment.centerLeft,
              child: Container(
                height: t.sp * 0.75,
                decoration: BoxDecoration(
                  color: o.accent,
                  borderRadius: BorderRadius.circular(t.sp),
                ),
              ),
            ),
            const Spacer(),
            Container(
              padding: EdgeInsets.symmetric(horizontal: t.sp),
              color: o.terminal.background,
              alignment: Alignment.center,
              child: Row(
                children: [
                  for (final i in [1, 2, 3, 4, 5])
                    Container(
                      width: dot,
                      height: dot,
                      margin: EdgeInsets.only(left: i == 1 ? 0 : t.sp * 0.5),
                      decoration: BoxDecoration(
                        color: o.terminal.ansi[i],
                        shape: BoxShape.circle,
                      ),
                    ),
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
