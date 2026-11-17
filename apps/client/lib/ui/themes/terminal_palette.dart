import 'package:flutter/foundation.dart' show immutable;
import 'package:flutter/painting.dart';

import '../oklch.dart';
import '../tokens.dart' show TokenInputs;

/// The terminal's 16 colours as a theme states them: a hue and a chroma per
/// slot, and optionally the lightness the inspiration had. The lightness the
/// terminal draws with is found, never given (decision 160).
@immutable
class AnsiInputs {
  const AnsiInputs({required this.hues, required this.chroma, this.lightness});

  /// Normal slots 0–7, then bright 8–15, in xterm order. Black and white
  /// (0, 7, 8, 15) are greys on the theme's neutral hue whatever is here.
  final List<double> hues;
  final List<double> chroma;

  /// 0–100. A dark theme's colour starts here and only gets lighter, so a
  /// pale inspiration (a pastel yellow) stays pale.
  final List<double>? lightness;

  /// xterm's defaults, the colours Storm's terminal drew before decision 160.
  static const storm = AnsiInputs(
    hues: [
      0, 26.1, 158.8, 109.7, 254.2, 327.9, 221.4, 89.9, //
      89.9, 24.7, 159.5, 109.5, 253.5, 327.3, 219.3, 89.9,
    ],
    chroma: [
      0, 0.16, 0.16, 0.16, 0.153, 0.16, 0.122, 0, //
      0, 0.16, 0.16, 0.16, 0.16, 0.16, 0.123, 0,
    ],
    lightness: [
      0, 55.9, 70.2, 89.3, 55.1, 59.1, 67.8, 92.2, //
      51, 64.9, 76.2, 94.1, 64.1, 69.7, 72.6, 100,
    ],
  );

  static const names = [
    'black',
    'red',
    'green',
    'yellow',
    'blue',
    'magenta',
    'cyan',
    'white',
    'bright black',
    'bright red',
    'bright green',
    'bright yellow',
    'bright blue',
    'bright magenta',
    'bright cyan',
    'bright white',
  ];

  @override
  bool operator ==(Object other) =>
      other is AnsiInputs &&
      _same(other.hues, hues) &&
      _same(other.chroma, chroma) &&
      _same(other.lightness, lightness);

  @override
  int get hashCode => Object.hash(
    Object.hashAll(hues),
    Object.hashAll(chroma),
    lightness == null ? null : Object.hashAll(lightness!),
  );
}

bool _same(List<double>? a, List<double>? b) {
  if (a == null || b == null) return a == b;
  if (a.length != b.length) return false;
  for (var i = 0; i < a.length; i++) {
    if (a[i] != b[i]) return false;
  }
  return true;
}

/// A terminal's ground, text and ANSI 16, compared by value: the terminal
/// keeps one TerminalTheme per palette, and a new instance for the same
/// colours would read to a program as a colour-scheme change.
@immutable
class TerminalPalette {
  const TerminalPalette({
    required this.background,
    required this.foreground,
    required this.ansi,
  });

  final Color background;
  final Color foreground;
  final List<Color> ansi;

  @override
  bool operator ==(Object other) =>
      other is TerminalPalette &&
      other.background == background &&
      other.foreground == foreground &&
      _sameColours(other.ansi, ansi);

  @override
  int get hashCode => Object.hash(background, foreground, Object.hashAll(ansi));
}

bool _sameColours(List<Color> a, List<Color> b) {
  if (a.length != b.length) return false;
  for (var i = 0; i < a.length; i++) {
    if (a[i] != b[i]) return false;
  }
  return true;
}

/// A light view's ground: the page lightened halfway to white (157a).
Color lightTerminalGround(Color bg) =>
    Color.lerp(bg, const Color(0xFFFFFFFF), 0.5)!;

/// The palette [i] draws its terminal with, on a ground of [bg] (dark) or
/// [bg] lightened halfway to white (light).
///
/// Every colour keeps its hue and, inside the gamut, its chroma; only its
/// lightness moves, until it reads at 4.5:1 (normal) or 3:1 (bright) on the
/// ground. A dark theme's brights must reach 6:1, so they still read as the
/// brighter of the pair.
TerminalPalette deriveTerminalPalette(
  TokenInputs i, {
  required Color bg,
  required Color text,
}) => _derived.putIfAbsent(i, () => _derive(i, bg: bg, text: text));

final _derived = <TokenInputs, TerminalPalette>{};

TerminalPalette _derive(
  TokenInputs i, {
  required Color bg,
  required Color text,
}) {
  final dark = i.dark;
  final ground = dark ? bg : lightTerminalGround(bg);
  final a = i.ansi;
  final greyC = i.chroma / 1000 * 2;

  Color fit(double hue, double c, double min, double start, double step) {
    var l = start;
    for (var k = 0; k < 200; k++) {
      final x = oklchMapped(l, c, hue);
      if (contrastRatio(x, ground) >= min) return x;
      l += step;
    }
    return oklchMapped(l, c, hue);
  }

  Color slot(int n) {
    final bright = n >= 8;
    switch (n % 8) {
      case 0:
        return dark
            ? (bright
                  ? fit(i.hue, greyC, 3, 0.45, 0.01)
                  : oklchMapped(0.30, greyC, i.hue))
            : (bright
                  ? fit(i.hue, greyC, 3, 0.60, -0.01)
                  : fit(i.hue, greyC, 4.5, 0.35, -0.01));
      case 7:
        return dark
            ? (bright
                  ? oklchMapped(0.97, i.chroma / 1000, i.hue)
                  : fit(i.hue, greyC, 4.5, 0.78, 0.01))
            : (bright
                  ? fit(i.hue, greyC, 3, 0.55, -0.01)
                  : fit(i.hue, greyC, 4.5, 0.48, -0.01));
    }
    final c = a.chroma[n].clamp(0.0, 0.16);
    final from = (a.lightness?[n] ?? 0) / 100;
    if (dark) {
      return bright
          ? fit(a.hues[n], c, 6, from < 0.72 ? 0.72 : from, 0.01)
          : fit(a.hues[n], c, 4.5, from < 0.62 ? 0.62 : from, 0.01);
    }
    return bright
        ? fit(a.hues[n], c, 3, 0.58, -0.01)
        : fit(a.hues[n], c, 4.5, 0.50, -0.01);
  }

  return TerminalPalette(
    background: ground,
    foreground: text,
    ansi: List.unmodifiable([for (var n = 0; n < 16; n++) slot(n)]),
  );
}
