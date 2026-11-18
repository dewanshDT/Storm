/// OKLCH → sRGB, because Flutter has no perceptual colour space.
///
/// The design system derives every colour from three numbers — lightness,
/// chroma, hue — and that is the whole reason one token change can move the
/// entire palette coherently. Doing the same arithmetic in HSL would not work:
/// equal lightness steps in HSL are not equal *perceived* steps, so a scale
/// that reads evenly in one hue bunches up in another.
///
/// Ported from the Oklab reference (Björn Ottosson). Every colour in the app
/// flows through this, so it is tested against known values rather than
/// eyeballed.
library;

import 'dart:math' as math;

import 'package:flutter/painting.dart';

/// A colour from OKLCH.
///
/// [l] is perceptual lightness 0–1, [c] chroma (0 is grey; ~0.15 is vivid at
/// these lightnesses), [hueDegrees] the hue angle.
///
/// Components outside sRGB are clipped per channel. That is a deliberate
/// simplification: the design's chroma values stay inside the gamut at the
/// lightnesses used, and a gamut-mapping algorithm would add a lot of
/// machinery to change nothing. If a future token pushes outside, the clip
/// shows up as a flat, slightly wrong colour rather than as a crash — and the
/// contrast test is what would catch it.
Color oklch(double l, double c, double hueDegrees) {
  final (r, g, b) = _linear(l, c, hueDegrees);
  return Color.fromARGB(255, _encode(r), _encode(g), _encode(b));
}

(double, double, double) _linear(double l, double c, double hueDegrees) {
  final h = hueDegrees * math.pi / 180;
  final a = c * math.cos(h);
  final b = c * math.sin(h);

  // Oklab → LMS, cube-rooted.
  final lms0 = l + 0.3963377774 * a + 0.2158037573 * b;
  final lms1 = l - 0.1055613458 * a - 0.0638541728 * b;
  final lms2 = l - 0.0894841775 * a - 1.2914855480 * b;

  final long = lms0 * lms0 * lms0;
  final medium = lms1 * lms1 * lms1;
  final short = lms2 * lms2 * lms2;

  // LMS → linear sRGB.
  final r = 4.0767416621 * long - 3.3077115913 * medium + 0.2309699292 * short;
  final g = -1.2684380046 * long + 2.6097574011 * medium - 0.3413193965 * short;
  final bl =
      -0.0041960863 * long - 0.7034186147 * medium + 1.7076147010 * short;

  return (r, g, bl);
}

/// [oklch], with an out-of-gamut colour brought in by lowering its chroma
/// rather than by clipping channels, so the hue and lightness survive.
Color oklchMapped(double l, double c, double hueDegrees) {
  const tolerance = 1e-4;
  var chroma = c;
  while (chroma > 0) {
    final (r, g, b) = _linear(l, chroma, hueDegrees);
    bool inside(double v) => v >= -tolerance && v <= 1 + tolerance;
    if (inside(r) && inside(g) && inside(b)) break;
    chroma -= 0.002;
  }
  return oklch(l, math.max(0, chroma), hueDegrees);
}

/// sRGB → OKLCH: lightness 0–1, chroma, hue in degrees.
({double l, double c, double h}) toOklch(Color color) {
  double linear(double v) =>
      v <= 0.04045 ? v / 12.92 : math.pow((v + 0.055) / 1.055, 2.4).toDouble();
  final r = linear(color.r);
  final g = linear(color.g);
  final b = linear(color.b);
  double cbrt(double v) => math.pow(v, 1 / 3).toDouble();
  final l = cbrt(0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b);
  final m = cbrt(0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b);
  final s = cbrt(0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b);
  final lightness = 0.2104542553 * l + 0.7936177850 * m - 0.0040720468 * s;
  final a = 1.9779984951 * l - 2.4285922050 * m + 0.4505937099 * s;
  final bb = 0.0259040371 * l + 0.7827717662 * m - 0.8086757660 * s;
  var hue = math.atan2(bb, a) * 180 / math.pi;
  if (hue < 0) hue += 360;
  return (l: lightness, c: math.sqrt(a * a + bb * bb), h: hue);
}

/// Linear light → an 8-bit sRGB channel.
int _encode(double linear) {
  final clamped = linear.clamp(0.0, 1.0);
  final encoded = clamped <= 0.0031308
      ? 12.92 * clamped
      : 1.055 * math.pow(clamped, 1 / 2.4) - 0.055;
  return (encoded * 255).round().clamp(0, 255);
}

/// WCAG relative luminance.
double relativeLuminance(Color color) {
  double channel(double v) =>
      v <= 0.04045 ? v / 12.92 : math.pow((v + 0.055) / 1.055, 2.4).toDouble();
  return 0.2126 * channel(color.r) +
      0.7152 * channel(color.g) +
      0.0722 * channel(color.b);
}

/// WCAG contrast ratio between two opaque colours, 1.0 to 21.0.
///
/// Here rather than in the test, because the rule it measures — every semantic
/// colour readable on the surface it paints on — is a property of the design
/// system, and a reader of `tokens.dart` should be able to find the definition
/// next to the values it governs.
double contrastRatio(Color a, Color b) {
  final la = relativeLuminance(a);
  final lb = relativeLuminance(b);
  final lighter = math.max(la, lb);
  final darker = math.min(la, lb);
  return (lighter + 0.05) / (darker + 0.05);
}
