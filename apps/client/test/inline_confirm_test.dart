import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:storm/ui/panels.dart';
import 'package:storm/ui/theme.dart';

void main() {
  Future<void> pump(WidgetTester tester, double width) => tester.pumpWidget(
    MaterialApp(
      theme: StormTheme.dark(),
      home: Scaffold(
        body: Align(
          alignment: Alignment.topLeft,
          child: SizedBox(
            width: width,
            child: InlineConfirm(
              title: 'End accept-1',
              message:
                  'The agent and everything it started are stopped on the host.',
              confirmLabel: 'End session',
              onConfirm: () {},
              onCancel: () {},
            ),
          ),
        ),
      ),
    ),
  );

  testWidgets('wide: the title, message and actions on one line', (
    tester,
  ) async {
    await pump(tester, 1100);
    final title = tester.getRect(find.byKey(const Key('confirm-title')));
    final message = tester.getRect(
      find.textContaining('The agent and everything'),
    );
    final cancel = tester.getRect(find.byKey(const Key('confirm-cancel')));
    final end = tester.getRect(find.byKey(const Key('confirm-action')));
    expect(message.left - title.right, greaterThanOrEqualTo(16));
    expect(cancel.right, lessThanOrEqualTo(end.left));
    expect((title.center.dy - end.center.dy).abs(), lessThan(2));
    expect(tester.takeException(), isNull);
  });

  testWidgets('narrow: the message wraps above the actions, still compact', (
    tester,
  ) async {
    await pump(tester, 420);
    final title = tester.getRect(find.byKey(const Key('confirm-title')));
    final end = tester.getRect(find.byKey(const Key('confirm-action')));
    expect(end.top, greaterThan(title.bottom));
    expect(end.right, closeTo(420 - 17, 2));
    expect(tester.takeException(), isNull);
  });
}
