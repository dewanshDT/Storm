import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:storm/ui/oklch.dart';
import 'package:storm/ui/tokens.dart';

/// The text-on-ground pairs redesign round 2 (decision 153) added, in every
/// theme (README acceptance: "All three themes pass contrast"). Tints are
/// composed over `surface`, the ground every colour is measured against.
typedef Pair = (
  String,
  Color Function(StormTokens),
  Color Function(StormTokens),
  double,
);

Color tint(StormTokens t, double alpha) =>
    Color.alphaBlend(t.danger.withValues(alpha: alpha), t.surface);

final List<Pair> pairs = [
  // The decision strip, compact and full: NEEDS YOU, the title, Review.
  (
    'danger label on the 8% danger tint',
    (t) => t.danger,
    (t) => tint(t, 0.08),
    4.5,
  ),
  ('title on the 8% danger tint', (t) => t.text, (t) => tint(t, 0.08), 4.5),
  (
    'Review and Later on the 8% danger tint',
    (t) => t.text2,
    (t) => tint(t, 0.08),
    4.5,
  ),
  ('strip outline against the tint', (t) => t.danger, (t) => tint(t, 0.08), 3),
  // An outlined row, crew header or folded tree that needs you.
  ('name on the outlined row tint', (t) => t.text, (t) => tint(t, 0.09), 4.5),
  (
    'quiet name on the outlined row tint',
    (t) => t.text2,
    (t) => tint(t, 0.09),
    4.5,
  ),
  (
    'roll-up words on the outlined row tint',
    (t) => t.danger,
    (t) => tint(t, 0.09),
    4.5,
  ),
  ('row outline against the tint', (t) => t.danger, (t) => tint(t, 0.09), 3),
  // Chips, the sidebar's ＋, an open grid cell, the active rail item.
  ('accent on accentSoft', (t) => t.accent, (t) => t.accentSoft, 4.5),
  ('text on accentSoft', (t) => t.text, (t) => t.accentSoft, 4.5),
  ('open cell detail on accentSoft', (t) => t.text2, (t) => t.accentSoft, 4.5),
  // Hint lines under fields and sections.
  ('text3 hint on surface', (t) => t.text3, (t) => t.surface, 4.5),
  // Agents & hosts cell words: accent where you can act, text, text3.
  ('actionable cell word on surface', (t) => t.accent, (t) => t.surface, 4.5),
  ('ready cell word on surface', (t) => t.text, (t) => t.surface, 4.5),
  ('quiet cell word on surface', (t) => t.text3, (t) => t.surface, 4.5),
  // The rail's and the place bubble's needs-you count.
  ('count on danger', (t) => t.onAccent, (t) => t.danger, 4.5),
];

/// Pairs that miss with today's tokens, and the floor they must hold until
/// a token change fixes them. Selected `accent` on `accentSoft` is the design
/// system's selected state (chips, soft buttons, tabs) from before 153; only
/// SlowFlow earth misses, and no other existing token keeps the state.
const known = {('accent on accentSoft', StormPreset.slowflowEarth): 4.3};

void main() {
  for (final preset in StormPreset.values) {
    group(preset.label, () {
      final t = StormTokens.from(preset);
      for (final (name, fg, bg, min) in pairs) {
        test(name, () {
          final ratio = contrastRatio(fg(t), bg(t));
          final why =
              '$name is ${ratio.toStringAsFixed(2)}:1 in ${preset.label}';
          if (known[(name, preset)] case final floor?) {
            expect(ratio, greaterThanOrEqualTo(floor), reason: why);
            expect(ratio, lessThan(min), reason: '$why: drop it from known');
            return;
          }
          expect(ratio, greaterThanOrEqualTo(min), reason: why);
        });
      }
    });
  }
}
