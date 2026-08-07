import 'package:flutter/material.dart';

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
/// works on white is a glare on black and vice versa. The pairs are picked so
/// body text keeps its contrast in both.
enum Accent {
  none('none', Color(0x00000000), Color(0x00000000)),
  coral('coral', Color(0xFFFAD2CF), Color(0xFF5C2B29)),
  peach('peach', Color(0xFFFDE2CE), Color(0xFF614A19)),
  sand('sand', Color(0xFFFFF8B8), Color(0xFF635D19)),
  sage('sage', Color(0xFFE6F4D7), Color(0xFF345920)),
  mint('mint', Color(0xFFD4E4ED), Color(0xFF16504B)),
  sky('sky', Color(0xFFD3E3FD), Color(0xFF2D555E)),
  lavender('lavender', Color(0xFFE9D9FB), Color(0xFF42275E)),
  blossom('blossom', Color(0xFFFDCFE8), Color(0xFF6C394F)),
  clay('clay', Color(0xFFE9E3D4), Color(0xFF4B443A));

  const Accent(this.name, this._light, this._dark);

  /// What goes in the file. Lower case, one word, stable.
  final String name;

  final Color _light;
  final Color _dark;

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

  /// The card or surface fill for [brightness].
  Color surface(Brightness brightness) =>
      brightness == Brightness.dark ? _dark : _light;

  /// A quieter version, for tinting a whole page behind body text.
  ///
  /// The full surface colour is right for a card an inch tall and far too
  /// loud behind a screen of prose.
  Color wash(Brightness brightness) => surface(
    brightness,
  ).withValues(alpha: brightness == Brightness.dark ? 0.38 : 0.45);

  /// The outline that keeps a card readable when its fill is close to the
  /// page.
  Color border(Brightness brightness) => surface(
    brightness,
  ).withValues(alpha: brightness == Brightness.dark ? 1 : 0.9);

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
  Brightness get _brightness => Theme.of(this).brightness;

  Color accentSurface(Accent accent) => accent.isNone
      ? Theme.of(this).colorScheme.surfaceContainerHigh
      : accent.surface(_brightness);

  Color accentBorder(Accent accent) => accent.isNone
      ? Theme.of(this).colorScheme.outlineVariant.withValues(alpha: 0.4)
      : accent.border(_brightness);
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
          _Swatch(
            accent: accent,
            selected: accent == selected,
            size: size,
            onTap: () => onSelected(accent),
          ),
      ],
    );
  }
}

class _Swatch extends StatelessWidget {
  const _Swatch({
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
    final brightness = Theme.of(context).brightness;

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
            color: accent.isNone
                ? scheme.surfaceContainerHighest
                : accent.surface(brightness),
            border: Border.all(
              color: selected ? scheme.primary : scheme.outlineVariant,
              width: selected ? 2 : 1,
            ),
          ),
          child: accent.isNone && !selected
              // A slash, so "no colour" reads as a choice rather than an
              // unfilled swatch.
              ? Icon(
                  Icons.format_color_reset_outlined,
                  size: size * 0.5,
                  color: scheme.onSurfaceVariant,
                )
              : selected
              ? Icon(Icons.check, size: size * 0.55, color: scheme.onSurface)
              : null,
        ),
      ),
    );
  }
}
