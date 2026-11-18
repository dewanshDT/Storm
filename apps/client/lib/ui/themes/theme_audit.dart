import 'package:flutter/material.dart';

import '../oklch.dart';
import '../tokens.dart';

/// One measured pair: what it is in plain words, and how it reads.
@immutable
class ContrastCheck {
  const ContrastCheck(this.part, this.ratio, this.min, {this.hard = false});

  final String part;
  final double ratio;
  final double min;

  /// A variant failing this one does not ship.
  final bool hard;

  bool get passes => ratio >= min;

  @override
  String toString() => '$part ${ratio.toStringAsFixed(2)}:1 (min $min)';
}

/// Every pair a theme has to keep readable (decision 160), computed from its
/// colours so the picker's "Low contrast" note can never drift from them.
///
/// Normal ANSI colours need 4.5:1 on the terminal ground and bright ones 3:1;
/// black on a dark ground and white on a light one are left out, being the
/// ground's own colour by design.
List<ContrastCheck> auditTokens(StormTokens t) {
  final p = t.terminal;
  final dark = t.brightness == Brightness.dark;
  final faint = dark
      ? Color.from(
          alpha: 1,
          red: p.foreground.r * 0.66,
          green: p.foreground.g * 0.66,
          blue: p.foreground.b * 0.66,
        )
      : Color.alphaBlend(p.foreground.withValues(alpha: 0.65), p.background);
  ContrastCheck c(String part, Color fg, Color bg, double min) => ContrastCheck(
    part,
    contrastRatio(fg, bg),
    min,
    hard: part == 'body text',
  );
  return [
    c('body text', t.text, t.bg, 4.5),
    c('body text on panels', t.text, t.surface, 4.5),
    c('secondary text', t.text2, t.bg, 4.5),
    c('secondary text on panels', t.text2, t.surface, 4.5),
    c('muted text', t.text3, t.bg, 4.5),
    c('muted text on panels', t.text3, t.surface, 4.5),
    c('links and accents', t.accent, t.bg, 4.5),
    c('selected rows', t.accent, t.accentSoft, 4.5),
    c('button labels', t.onAccent, t.accent, 4.5),
    c('tags', t.amber, t.bg, 4.5),
    c('synced', t.green, t.bg, 4.5),
    c('errors', t.danger, t.bg, 4.5),
    c('terminal text', p.foreground, p.background, 4.5),
    c('faint terminal text', faint, p.background, 3),
    for (final (i, colour) in p.ansi.indexed)
      if (!(dark && i % 8 == 0) && !(!dark && i % 8 == 7))
        c(
          'terminal ${AnsiInputs.names[i]}',
          colour,
          p.background,
          i < 8 ? 4.5 : 3,
        ),
  ];
}

/// The parts of [t] below their floor; empty for a theme that passes.
List<ContrastCheck> lowContrast(StormTokens t) =>
    auditTokens(t).where((c) => !c.passes).toList();
