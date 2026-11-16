import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'package:storm/agent/delete_request.dart' show showAgentToast;
import 'package:storm/router.dart';
import 'package:storm/ui/oklch.dart';
import 'package:storm/ui/theme.dart';
import 'package:storm/ui/tokens.dart';

import 'shell_harness.dart';

/// Toasts float in the desk's bottom-right corner, run full width on a
/// phone, and carry an action that reads on them in every theme.
void main() {
  setUp(() => SharedPreferences.setMockInitialValues({}));

  Future<Rect> toastAt(WidgetTester t, Size size) async {
    final c = shellContainer();
    await pumpShell(t, c, size: size);
    c.read(routerProvider).go(Routes.settingsPage('device'));
    await t.pumpAndSettle();
    showAgentToast(
      SnackBar(
        content: const Text('Run ls -la in bash stopped working'),
        action: SnackBarAction(label: 'Open', onPressed: () {}),
      ),
    );
    await t.pumpAndSettle();
    final surface = find.descendant(
      of: find.byType(SnackBar),
      matching: find.byType(Material),
    );
    final rect = t.getRect(surface.first);
    final label = t.widget<RichText>(
      find.descendant(
        of: find.byType(SnackBarAction),
        matching: find.byType(RichText),
      ),
    );
    final theme = StormTheme.dark().snackBarTheme;
    expect(label.text.style?.color, theme.actionTextColor);
    expect(
      find.descendant(
        of: find.byType(SnackBarAction),
        matching: find.byType(TextButton),
      ),
      findsOneWidget,
      reason: 'a button, so the keyboard reaches it',
    );
    await disposeShell(t, c);
    return rect;
  }

  testWidgets('at the desk a toast floats bottom right, 400 at most', (
    t,
  ) async {
    const size = Size(1400, 900);
    final rect = await toastAt(t, size);
    expect(rect.width, lessThanOrEqualTo(StormTheme.toastMaxWidth));
    expect(rect.width, greaterThan(StormTheme.toastMaxWidth - 1));
    expect(rect.right, closeTo(size.width - 16, 4));
    expect(rect.bottom, closeTo(size.height - 16, 4));
    expect(rect.left, greaterThan(size.width / 2));
  });

  testWidgets('at 900 it still keeps to the corner', (t) async {
    const size = Size(900, 900);
    final rect = await toastAt(t, size);
    expect(rect.width, lessThanOrEqualTo(StormTheme.toastMaxWidth));
    expect(rect.right, closeTo(size.width - 16, 4));
  });

  testWidgets('on a phone a toast runs the full width', (t) async {
    const size = Size(411, 900);
    final rect = await toastAt(t, size);
    expect(rect.left, lessThan(24));
    expect(rect.right, greaterThan(size.width - 24));
  });

  group('the toast action reads at 4.5:1', () {
    for (final preset in StormPreset.values) {
      test(preset.label, () {
        final toast = StormTheme.from(preset).snackBarTheme;
        final ratio = contrastRatio(
          toast.actionTextColor!,
          toast.backgroundColor!,
        );
        expect(
          ratio,
          greaterThanOrEqualTo(4.5),
          reason: '${preset.label}: ${ratio.toStringAsFixed(2)}:1',
        );
      });
    }
  });
}
