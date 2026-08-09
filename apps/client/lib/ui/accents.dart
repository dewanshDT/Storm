import 'package:flutter/material.dart';
import 'package:flutter_lucide/flutter_lucide.dart';

import 'oklch.dart';
import 'tokens.dart';

/// The colours a note or a vault can be tinted with.
///
/// Google Keep's model: a small named palette rather than a colour picker.
/// Names, not hex, is the load-bearing choice — the value is written into a
/// note's frontmatter as `color: sage`, so it has to stay readable, greppable
/// and meaningful when the vault is opened in Obsidian or a text editor. A
/// stored `#B7CDB0` would be none of those, and would pin the vault to one
/// theme forever.
///
/// Each accent carries a light *and* a dark variant, because a tint that
/// works on white is a glare on black and vice versa.
///
/// Both are derived in OKLCH from the same system as everything else (see
/// `tokens.dart`): one hue per name, one small chroma, one lightness per
/// theme. They were sampled from Google Keep before, which made them slabs of
/// unrelated colour sitting on a palette they had never met.
///
/// **They are sized for a tile, not for a card wash.** The design uses an
/// accent as a small rounded square behind a vault's initial, and as a dot
/// beside a note's colour property — never as the fill of a whole card, which
/// is why the earlier values read as heavy blocks of green and teal. Every
/// tile clears 5:1 against the dark ink that sits on it.
enum Accent {
  none('none', 0, 0),
  coral('coral', 25, 1),
  peach('peach', 50, 1),
  sand('sand', 85, 1),
  sage('sage', 135, 1),
  mint('mint', 165, 1),
  sky('sky', 230, 1),
  lavender('lavender', 290, 1),
  blossom('blossom', 340, 1),
  // Deliberately the near-neutral one, so it carries less chroma and leans on
  // lightness alone to stay visible.
  clay('clay', 70, 0.4);

  const Accent(this.name, this.hue, this._chromaScale);

  /// What goes in the file. Lower case, one word, stable.
  final String name;

  /// The only thing an accent really is. Everything else is derived from the
  /// theme it is being drawn in.
  final double hue;

  final double _chromaScale;

  /// Reads an accent from frontmatter, forgivingly.
  ///
  /// An unknown or absent value is [Accent.none] rather than an error: a note
  /// written by hand, or by a future version with more colours, must still
  /// open.
  static Accent parse(String? value) {
    final want = value?.trim().toLowerCase();
    if (want == null || want.isEmpty) return Accent.none;
    for (final accent in Accent.values) {
      if (accent.name == want) return accent;
    }
    return Accent.none;
  }

  bool get isNone => this == Accent.none;

  /// The tile behind a vault's initial, and the dot beside a note's colour.
  ///
  /// Derived from the theme rather than stored, so an accent sits a step off
  /// *this* surface whatever the preset is. Storm light's surface is 0.87 and
  /// SlowFlow's is 0.79; a single stored value vanished into the first while
  /// working on the second, which is what deriving fixes.
  ///
  /// A tile always steps *towards light*, never away from the page in the
  /// theme's own direction. On a dark ground there is a lot of room above the
  /// surface, so the step is large; on a light ground there is very little, so
  /// the step is small and the colour does the work. `clay`, the near-neutral
  /// one, gets a bigger step because it has almost no chroma to be seen by.
  Color tile(StormTokens t) {
    if (isNone) return t.surface2;
    final step = t.dir > 0 ? 0.35 : 0.09 + (1 - _chromaScale) * 0.05;
    return oklch(
      (t.surfaceL + step).clamp(0.0, 0.99),
      0.085 * _chromaScale,
      hue,
    );
  }

  /// A quieter version, for tinting a whole page behind body text.
  ///
  /// A tile colour at tile strength behind a screen of prose is a glare.
  Color wash(StormTokens t) =>
      isNone ? const Color(0x00000000) : tile(t).withValues(alpha: 0.14);

  /// The outline that keeps a tinted card readable against the page.
  Color border(StormTokens t) =>
      isNone ? t.border : tile(t).withValues(alpha: 0.55);

  /// A human label for the picker.
  String get label => switch (this) {
    Accent.none => 'Default',
    Accent.coral => 'Coral',
    Accent.peach => 'Peach',
    Accent.sand => 'Sand',
    Accent.sage => 'Sage',
    Accent.mint => 'Mint',
    Accent.sky => 'Sky',
    Accent.lavender => 'Lavender',
    Accent.blossom => 'Blossom',
    Accent.clay => 'Clay',
  };
}

/// Resolves the accent for the current theme without every caller reaching
/// for `Theme.of(context).brightness`.
extension AccentContext on BuildContext {
  Color accentSurface(Accent accent) => accent.tile(tokens);
  Color accentBorder(Accent accent) => accent.border(tokens);
}

/// A row of swatches, used wherever a colour is chosen.
///
/// One control, three callers: the note's `color` property, the vault menu,
/// and the new-note dialog. A colour picked in one place has to look like the
/// same decision made in another.
class AccentPicker extends StatelessWidget {
  const AccentPicker({
    super.key,
    required this.selected,
    required this.onSelected,
    this.size = 30,
  });

  final Accent selected;
  final ValueChanged<Accent> onSelected;
  final double size;

  @override
  Widget build(BuildContext context) {
    return Wrap(
      spacing: 8,
      runSpacing: 8,
      children: [
        for (final accent in Accent.values)
          AccentSwatch(
            accent: accent,
            selected: accent == selected,
            size: size,
            onTap: () => onSelected(accent),
          ),
      ],
    );
  }
}

/// One swatch. Public because the gallery renders the set on its own.
class AccentSwatch extends StatelessWidget {
  const AccentSwatch({
    super.key,
    required this.accent,
    required this.selected,
    required this.size,
    required this.onTap,
  });

  final Accent accent;
  final bool selected;
  final double size;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;

    return Tooltip(
      message: accent.label,
      child: InkWell(
        onTap: onTap,
        customBorder: const CircleBorder(),
        child: Container(
          width: size,
          height: size,
          decoration: BoxDecoration(
            shape: BoxShape.circle,
            color: accent.tile(context.tokens),
            border: Border.all(
              color: selected ? scheme.primary : scheme.outlineVariant,
              width: selected ? 2 : 1,
            ),
          ),
          child: accent.isNone && !selected
              // A slash, so "no colour" reads as a choice rather than an
              // unfilled swatch.
              ? Icon(
                  LucideIcons.droplet_off,
                  size: size * 0.5,
                  color: scheme.onSurfaceVariant,
                )
              : selected
              ? Icon(
                  LucideIcons.check,
                  size: size * 0.55,
                  color: scheme.onSurface,
                )
              : null,
        ),
      ),
    );
  }
}
