import 'package:flutter/material.dart';
import 'package:flutter_lucide/flutter_lucide.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:storm/ui/states.dart';
import 'package:storm/ui/theme.dart';
import 'package:storm/ui/tokens.dart';

void main() {
  Future<void> show(WidgetTester tester, Widget child) async {
    await tester.pumpWidget(
      MaterialApp(
        theme: StormTheme.dark(),
        home: Scaffold(body: SingleChildScrollView(child: child)),
      ),
    );
    await tester.pumpAndSettle();
  }

  group('offline reads as a condition, not a failure', () {
    testWidgets('never uses the danger colour', (tester) async {
      // The whole rule: nothing is broken and nothing is lost. Red here would
      // say the opposite of what is true.
      final t = StormTokens.from(StormPreset.stormDark);
      await show(tester, const OfflineNotice(queued: 2));

      for (final text in tester.widgetList<Text>(find.byType(Text))) {
        expect(text.style?.color, isNot(t.danger));
      }
    });

    testWidgets('says what is queued and when it resolves', (tester) async {
      await show(tester, const OfflineNotice(queued: 2));
      expect(find.textContaining('cached copy'), findsOneWidget);
      expect(find.textContaining('2 edits are queued'), findsOneWidget);
      expect(
        find.textContaining('when the server is reachable'),
        findsOneWidget,
      );
    });

    testWidgets('counts one edit in the singular', (tester) async {
      await show(tester, const OfflineNotice(queued: 1));
      expect(find.textContaining('1 edit is queued'), findsOneWidget);
    });

    testWidgets('claims nothing is queued when nothing is', (tester) async {
      await show(tester, const OfflineNotice());
      expect(find.textContaining('queued'), findsNothing);
    });

    testWidgets('offers a retry when one is possible', (tester) async {
      var retried = false;
      await show(tester, OfflineNotice(onRetry: () => retried = true));
      await tester.tap(find.text('Retry now'));
      expect(retried, isTrue);
    });
  });

  group('conflict', () {
    testWidgets('is the one place danger is right, and explains the fix', (
      tester,
    ) async {
      final t = StormTokens.from(StormPreset.stormDark);
      await show(tester, const ConflictCard());

      final title = tester.widget<Text>(find.text('Conflict in this note'));
      expect(title.style?.color, t.danger);
      // Resolving means editing the note by hand, so the card has to say so.
      expect(find.textContaining('delete the lines'), findsOneWidget);
      expect(find.textContaining('Both are kept'), findsOneWidget);
      expect(find.text('<<<<<<< yours'), findsOneWidget);
    });
  });

  group('empty', () {
    testWidgets('names where here is, and offers the way out', (tester) async {
      var made = false;
      await show(
        tester,
        EmptyState(
          icon: LucideIcons.folder_open,
          title: 'Nothing in this folder',
          detail: 'New notes made here will land in Projects.',
          action: 'New note',
          onAction: () => made = true,
        ),
      );

      expect(find.textContaining('Projects'), findsOneWidget);
      await tester.tap(find.text('New note'));
      expect(made, isTrue);
    });

    testWidgets('works without an action', (tester) async {
      await show(
        tester,
        const EmptyState(icon: LucideIcons.search_x, title: 'No notes match'),
      );
      expect(tester.takeException(), isNull);
      expect(find.byType(FilledButton), findsNothing);
    });
  });

  group('loading', () {
    testWidgets('draws rows of varying width rather than a progress bar', (
      tester,
    ) async {
      await show(tester, const SkeletonRows(rows: 3));
      expect(find.byType(CircularProgressIndicator), findsNothing);
      expect(find.byType(LinearProgressIndicator), findsNothing);

      final widths = tester
          .widgetList<FractionallySizedBox>(find.byType(FractionallySizedBox))
          .map((b) => b.widthFactor)
          .toSet();
      expect(widths.length, greaterThan(1), reason: 'a bar chart, not a block');
    });
  });
}
