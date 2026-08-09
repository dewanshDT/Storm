import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:storm/ui/oklch.dart';
import 'package:storm/ui/theme.dart';
import 'package:storm/ui/tokens.dart';

/// The design system's two hard rules, and the maths underneath them.
///
/// Both come from the handoff's accessibility review, and both are the kind of
/// thing that passes by eye and fails in use, so they are asserted rather than
/// trusted.
void main() {
  group('OKLCH conversion', () {
    // Every colour in the app flows through this, so it is checked against
    // known values rather than eyeballed. References computed with the Oklab
    // reference implementation.
    test('converts known values', () {
      expect(_hex(oklch(0, 0, 0)), '#000000');
      expect(_hex(oklch(1, 0, 0)), '#ffffff');
      // The design's Storm accent.
      expect(_hex(oklch(0.68, 0.15, 293)), '#9d84ec');
      // Its amber and green, at dark-mode lightness.
      expect(_hex(oklch(0.74, 0.14, 68)), '#e49839');
      expect(_hex(oklch(0.74, 0.13, 148)), '#6dc17b');
    });

    test('a grey has equal channels whatever the hue', () {
      for (final hue in [0.0, 55.0, 148.0, 293.0]) {
        final c = oklch(0.5, 0, hue);
        expect(c.r, closeTo(c.g, 0.004), reason: 'hue $hue');
        expect(c.g, closeTo(c.b, 0.004), reason: 'hue $hue');
      }
    });

    test('clips out-of-gamut chroma instead of overflowing', () {
      // A chroma no sRGB display can show. The channels must stay in range;
      // the contrast test is what would catch the visual consequence.
      final c = oklch(0.6, 0.9, 140);
      for (final channel in [c.r, c.g, c.b]) {
        expect(channel, inInclusiveRange(0.0, 1.0));
      }
    });

    test('contrast ratio matches the WCAG bounds', () {
      expect(contrastRatio(Colors.white, Colors.black), closeTo(21, 0.01));
      expect(contrastRatio(Colors.white, Colors.white), closeTo(1, 0.001));
    });
  });

  group('every semantic colour is readable on the surface it paints on', () {
    // Against `surface`, never `bg`. In light mode surface is *darker* than
    // the page, so checking the page passes while the real composition fails —
    // the mistake the handoff records failing review twice.
    for (final preset in StormPreset.values) {
      test(preset.label, () {
        final t = StormTokens.from(preset);

        for (final entry in t.semanticColors.entries) {
          final ratio = contrastRatio(entry.value, t.surface);
          expect(
            ratio,
            greaterThanOrEqualTo(4.5),
            reason:
                '${entry.key} on surface is ${ratio.toStringAsFixed(2)}:1 in '
                '${preset.label}; the design requires 4.5:1',
          );
        }
      });
    }

    test('light presets really do put surface darker than the page', () {
      // If this ever flips, the check above silently becomes the easy one.
      for (final preset in StormPreset.values) {
        final t = StormTokens.from(preset);
        final darker = relativeLuminance(t.surface) < relativeLuminance(t.bg);
        expect(
          darker,
          t.brightness == Brightness.light,
          reason:
              '${preset.label}: surface should be darker than bg in light '
              'mode and lighter in dark mode',
        );
      }
    });

    test('text on the accent is readable too', () {
      // `onAccent` flips between near-black and white depending on the accent's
      // lightness; a wrong flip is invisible until someone reads a button.
      for (final preset in StormPreset.values) {
        final t = StormTokens.from(preset);
        expect(
          contrastRatio(t.onAccent, t.accent),
          greaterThanOrEqualTo(4.5),
          reason: '${preset.label}: label on a primary button',
        );
      }
    });
  });

  group('the type scale never goes below the floor', () {
    // A ratio applied twice under a small base drives labels into
    // illegibility: at fs 12 and ratio 1.3 the raw value is 7.1px.
    test('at every size the user can choose', () {
      for (final preset in StormPreset.values) {
        for (var fs = 12.0; fs <= 24.0; fs += 1) {
          final t = StormTokens.from(preset, fs: fs);
          expect(
            t.labelSize,
            greaterThanOrEqualTo(StormTokens.minStep),
            reason: '${preset.label} at fs $fs',
          );
        }
      }
    });

    test('and the floor actually binds at the small end', () {
      // Proves the clamp is doing something rather than the raw value already
      // being large enough — which would make the test above vacuous.
      final t = StormTokens.from(StormPreset.slowflowEarth, fs: 12);
      expect(12 / (1.30 * 1.30), lessThan(StormTokens.minStep));
      expect(t.labelSize, StormTokens.minStep);
    });

    test('the scale still grows in the right order', () {
      final t = StormTokens.from(StormPreset.stormDark);
      expect(t.labelSize, lessThan(t.codeSize));
      expect(t.codeSize, lessThan(t.bodySize));
      expect(t.bodySize, lessThan(t.headingSize));
      expect(t.headingSize, lessThan(t.displaySize));
    });
  });

  group('the theme is built from the tokens', () {
    test('every preset carries its tokens on the ThemeData', () {
      for (final preset in StormPreset.values) {
        final theme = StormTheme.from(preset);
        expect(theme.extension<StormTokens>()?.preset, preset);
      }
    });

    test('the scheme maps the meanings that other code already reads', () {
      // Four call sites read amber as `tertiary`, and the offline dot reads
      // `outline`. Losing either mapping changes what a colour *means*.
      final t = StormTokens.from(StormPreset.stormDark);
      final scheme = StormTheme.schemeFrom(t);
      expect(scheme.tertiary, t.amber, reason: 'amber = tags and highlight');
      expect(scheme.outline, t.text3, reason: 'grey = offline and inactive');
      expect(scheme.error, t.danger);
      expect(scheme.primary, t.accent);
      expect(scheme.surface, t.bg, reason: 'Material surface is the page');
      expect(scheme.surfaceContainer, t.surface, reason: 'cards sit on it');
    });

    test('the user font size reaches the tokens', () {
      final theme = StormTheme.from(StormPreset.stormDark, fontSize: 20);
      expect(theme.extension<StormTokens>()!.bodySize, 20);
    });
  });

  group('presets', () {
    test('round-trip through their stored spelling', () {
      for (final preset in StormPreset.values) {
        expect(StormPreset.parse(preset.wire), preset);
      }
    });

    test('an unknown or missing choice falls back to the dark identity', () {
      expect(StormPreset.parse(null), StormPreset.stormDark);
      expect(StormPreset.parse('retired-theme'), StormPreset.stormDark);
    });

    test('Storm light keeps Storm dark identity', () {
      // The point of deriving it rather than adopting SlowFlow: switching mode
      // must not change what product you are looking at.
      final dark = StormPreset.stormDark.inputs;
      final light = StormPreset.stormLight.inputs;
      expect(light.hue, dark.hue);
      expect(light.accentH, dark.accentH);
      expect(light.accentC, dark.accentC);
      expect(light.rCard, dark.rCard);
    });
  });
}

String _hex(Color c) =>
    '#${((c.r * 255).round() << 16 | (c.g * 255).round() << 8 | (c.b * 255).round()).toRadixString(16).padLeft(6, '0')}';
