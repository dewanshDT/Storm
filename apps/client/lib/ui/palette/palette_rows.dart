import 'package:flutter/material.dart';

import '../shell/sidebar_rows.dart' show VaultTag;
import '../states.dart' show SkeletonRows;
import '../tokens.dart';
import 'palette_model.dart';

/// A row's colours. The selected row sits on `surface2`, where `text3` is
/// not readable, so everything quiet on it steps up to `text2` (156q).
class PaletteRowColors {
  const PaletteRowColors({
    required this.ground,
    required this.label,
    required this.sub,
    required this.hint,
    required this.highlight,
  });

  final Color ground;
  final Color label;
  final Color sub;
  final Color hint;

  /// A snippet's `<<match>>`: `accent` fails on `surface2` in Storm dark.
  final Color highlight;
}

PaletteRowColors paletteRowColors(
  StormTokens t, {
  required bool selected,
  PaletteTone tone = PaletteTone.plain,
}) {
  final quiet = selected ? t.text2 : t.text3;
  return PaletteRowColors(
    ground: selected ? t.surface2 : t.surface,
    label: tone == PaletteTone.tag ? t.amber : t.text,
    sub: tone == PaletteTone.loud ? t.text2 : quiet,
    hint: quiet,
    highlight: selected ? t.text : t.accent,
  );
}

class PaletteHeading extends StatelessWidget {
  const PaletteHeading(this.text, {super.key});

  final String text;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Padding(
      padding: EdgeInsets.fromLTRB(t.sp * 2, t.sp * 1.5, t.sp * 2, t.sp * 0.5),
      child: Semantics(
        header: true,
        child: Text(
          text.toUpperCase(),
          style: TextStyle(
            fontFamily: StormTokens.monoFamily,
            fontSize: t.labelSize,
            color: t.text3,
            letterSpacing: 0.6,
          ),
        ),
      ),
    );
  }
}

class PaletteRowTile extends StatelessWidget {
  const PaletteRowTile({
    super.key,
    required this.row,
    required this.selected,
    required this.onTap,
    this.showHint = true,
  });

  final PaletteRow row;
  final bool selected;
  final VoidCallback onTap;

  /// A phone has no keys to hint at (156n).
  final bool showHint;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final c = paletteRowColors(t, selected: selected, tone: row.tone);
    final sub = row.sub;
    final hint = showHint ? row.hint : null;
    final vault = row.vault;
    return Semantics(
      container: true,
      button: true,
      selected: selected,
      label: [
        row.label,
        if (row.snippet case final snippet?) plainSnippet(snippet),
        if (sub != null && sub.isNotEmpty) sub,
        if (vault != null && vault.isNotEmpty) vault,
      ].join(', '),
      hint: hint,
      excludeSemantics: true,
      onTap: onTap,
      child: Material(
        color: c.ground,
        child: InkWell(
          onTap: onTap,
          // The keyboard is how a row is reached; focus stays in the field.
          canRequestFocus: false,
          child: Padding(
            padding: EdgeInsets.symmetric(
              horizontal: t.sp * 2,
              vertical: t.sp * 0.875,
            ),
            child: Row(
              children: [
                Expanded(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    mainAxisSize: MainAxisSize.min,
                    children: [
                      Text(
                        row.label,
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: TextStyle(
                          fontSize: t.uiSize,
                          color: c.label,
                          fontWeight: row.tone == PaletteTone.loud
                              ? FontWeight.w600
                              : FontWeight.w400,
                        ),
                      ),
                      if (row.snippet case final snippet?)
                        PaletteSnippet(
                          raw: snippet,
                          color: c.sub,
                          highlight: c.highlight,
                        )
                      else if (sub != null && sub.isNotEmpty)
                        Text(
                          sub,
                          maxLines: 1,
                          overflow: TextOverflow.ellipsis,
                          style: TextStyle(fontSize: t.labelSize, color: c.sub),
                        ),
                    ],
                  ),
                ),
                if (vault != null && vault.isNotEmpty) ...[
                  SizedBox(width: t.sp),
                  VaultTag(vault, color: selected ? c.hint : null),
                ],
                if (hint != null) ...[
                  SizedBox(width: t.sp * 1.5),
                  Text(
                    hint,
                    style: TextStyle(
                      fontFamily: StormTokens.monoFamily,
                      fontSize: t.labelSize,
                      color: c.hint,
                    ),
                  ),
                ],
              ],
            ),
          ),
        ),
      ),
    );
  }
}

/// A snippet without its `<<match>>` markers, newlines collapsed.
String plainSnippet(String raw) =>
    raw.replaceAll('\n', ' ').replaceAll('<<', '').replaceAll('>>', '');

/// Renders the server's `<<match>>` markers as highlighted spans.
class PaletteSnippet extends StatelessWidget {
  const PaletteSnippet({
    super.key,
    required this.raw,
    required this.color,
    required this.highlight,
  });

  final String raw;
  final Color color;
  final Color highlight;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final base = TextStyle(
      fontFamily: StormTokens.sansFamily,
      fontSize: t.labelSize,
      color: color,
      height: 1.4,
    );
    final spans = <TextSpan>[];
    // A snippet spanning lines would blow up the row.
    var text = raw.replaceAll('\n', ' ');
    while (true) {
      final open = text.indexOf('<<');
      final close = open < 0 ? -1 : text.indexOf('>>', open + 2);
      if (open < 0 || close < 0) {
        spans.add(TextSpan(text: text, style: base));
        break;
      }
      spans.add(TextSpan(text: text.substring(0, open), style: base));
      spans.add(
        TextSpan(
          text: text.substring(open + 2, close),
          style: base.copyWith(color: highlight, fontWeight: FontWeight.w600),
        ),
      );
      text = text.substring(close + 2);
    }
    return Text.rich(
      TextSpan(children: spans),
      maxLines: 2,
      overflow: TextOverflow.ellipsis,
    );
  }
}

/// The In note text slot while its request is out (156l).
class PalettePendingRow extends StatelessWidget {
  const PalettePendingRow({super.key});

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Padding(
      padding: EdgeInsets.symmetric(horizontal: t.sp * 2),
      child: const ExcludeSemantics(child: SkeletonRows(rows: 1)),
    );
  }
}

/// One quiet line in a group: "Couldn't search Home".
class PaletteQuietLine extends StatelessWidget {
  const PaletteQuietLine(this.text, {super.key});

  final String text;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Padding(
      padding: EdgeInsets.fromLTRB(t.sp * 2, t.sp * 0.5, t.sp * 2, t.sp * 0.5),
      child: Text(
        text,
        style: TextStyle(fontSize: t.labelSize, color: t.text3),
      ),
    );
  }
}
