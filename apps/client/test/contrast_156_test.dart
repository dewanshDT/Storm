import 'package:flutter_test/flutter_test.dart';

import 'package:storm/ui/oklch.dart';
import 'package:storm/ui/palette/palette_model.dart' show PaletteTone;
import 'package:storm/ui/palette/palette_rows.dart';
import 'package:storm/ui/tokens.dart';

/// The palette's rows (decision 156q), selected and not, in every theme:
/// each word on the ground it is actually drawn on. The unselected ground
/// is `surface`; the selected one is `surface2`, where `text3` would not
/// pass, so nothing on it is `text3`.
void main() {
  for (final preset in StormPreset.values) {
    group(preset.label, () {
      final t = StormTokens.from(preset);
      for (final selected in [false, true]) {
        for (final tone in PaletteTone.values) {
          final c = paletteRowColors(t, selected: selected, tone: tone);
          final where = '${selected ? 'selected' : 'unselected'} ${tone.name}';
          test('$where row', () {
            expect(
              c.ground,
              selected ? t.surface2 : t.surface,
              reason: 'measured over surface, or the selected surface2',
            );
            for (final (name, fg) in [
              ('label', c.label),
              ('sub-line', c.sub),
              ('hint', c.hint),
            ]) {
              final ratio = contrastRatio(fg, c.ground);
              expect(
                ratio,
                greaterThanOrEqualTo(4.5),
                reason:
                    '$where $name is ${ratio.toStringAsFixed(2)}:1 in '
                    '${preset.label}',
              );
            }
            if (selected) {
              expect([c.label, c.sub, c.hint], isNot(contains(t.text3)));
            }
          });
        }
      }
      test('the group heading and the offline line: text3 on surface', () {
        expect(contrastRatio(t.text3, t.surface), greaterThanOrEqualTo(4.5));
      });
    });
  }
}
