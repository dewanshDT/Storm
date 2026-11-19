import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';

import 'package:storm/api/feedback_api.dart';
import 'package:storm/state/app_state.dart'
    show rootMessengerKey, rootNavigatorKey;
import 'package:storm/state/feedback_state.dart'
    show feedbackApiFactoryProvider;
import 'package:storm/ui/controls.dart';
import 'package:storm/ui/report_sheet.dart';
import 'package:storm/ui/theme.dart';
import 'package:storm/ui/widgets.dart';

/// The report sheet (decision 162a), built against *Decision 162 — feedback
/// API contract* with a fake `storm-server` — 162-3 lands in parallel.
void main() {
  /// A fake server backing `/v1/feedback*`. [sendFailTimes] 410s that many
  /// sends with `preview_expired` before succeeding.
  MockClient server({
    bool enabled = true,
    Map<String, dynamic>? previewClient,
    Map<String, dynamic>? previewBundle,
    int sendFailTimes = 0,
  }) {
    var previewCount = 0;
    var sendCount = 0;
    return MockClient((req) async {
      final path = req.url.path;
      if (path == '/v1/feedback' && req.method == 'GET') {
        return http.Response(
          jsonEncode({'enabled': enabled, 'intake_configured': true}),
          200,
        );
      }
      if (path == '/v1/feedback/preview') {
        previewCount++;
        return http.Response(
          jsonEncode({
            'preview_id': 'pv_$previewCount',
            'expires_at': '2026-11-19T00:10:00Z',
            'client': previewClient ?? {'screen': 'settings/health'},
            'bundle':
                previewBundle ??
                {
                  'server': {'version': '0.19.0'},
                },
            'bytes': 100,
          }),
          200,
        );
      }
      if (path == '/v1/feedback' && req.method == 'POST') {
        sendCount++;
        if (sendCount <= sendFailTimes) {
          return http.Response(jsonEncode({'error': 'preview_expired'}), 410);
        }
        return http.Response(
          jsonEncode({'report_id': 'rep_1234567890', 'queued': true}),
          202,
        );
      }
      return http.Response(jsonEncode({'error': 'not_found'}), 404);
    });
  }

  /// Desk width throughout, so `showReportProblemSheet` takes the dialog
  /// path (`context.isExpanded`) — the bottom sheet is the same content,
  /// `showStormSheet` already covered by its own tests elsewhere.
  Widget harness(MockClient client) => ProviderScope(
    overrides: [
      feedbackApiFactoryProvider.overrideWithValue(
        () => FeedbackApi(baseUrl: 'http://server', token: 't', client: client),
      ),
    ],
    child: MaterialApp(
      navigatorKey: rootNavigatorKey,
      scaffoldMessengerKey: rootMessengerKey,
      theme: StormTheme.light(),
      home: Builder(
        builder: (context) => MediaQuery(
          data: MediaQuery.of(context).copyWith(size: const Size(1280, 800)),
          child: Scaffold(
            body: Center(
              child: StormButton.primary(
                label: 'Open',
                onPressed: () => showReportProblemSheet(context),
              ),
            ),
          ),
        ),
      ),
    ),
  );

  Future<void> open(WidgetTester tester, MockClient client) async {
    await tester.pumpWidget(harness(client));
    await tester.tap(find.text('Open'));
    await tester.pumpAndSettle();
  }

  testWidgets('default state: Bug selected, diagnostics on, Send enabled', (
    tester,
  ) async {
    await open(tester, server());

    expect(
      tester
          .widget<ChoiceChips<String>>(find.byKey(const Key('report-kind')))
          .selected,
      'bug',
    );
    expect(
      tester
          .widget<StormCheckbox>(find.byKey(const Key('report-diagnostics')))
          .value,
      isTrue,
    );
    expect(find.byKey(const Key('report-screenshot')), findsNothing);
    expect(
      tester
          .widget<StormButton>(find.byKey(const Key('report-send')))
          .onPressed,
      isNotNull,
    );
  });

  testWidgets('toggles: picking a kind and unchecking diagnostics stick', (
    tester,
  ) async {
    await open(tester, server());

    await tester.tap(find.text('Idea'));
    await tester.pump();
    expect(
      tester
          .widget<ChoiceChips<String>>(find.byKey(const Key('report-kind')))
          .selected,
      'idea',
    );

    await tester.tap(find.byKey(const Key('report-diagnostics')));
    await tester.pump();
    expect(
      tester
          .widget<StormCheckbox>(find.byKey(const Key('report-diagnostics')))
          .value,
      isFalse,
    );
  });

  testWidgets('a disabled server explains itself and disables Send', (
    tester,
  ) async {
    await open(tester, server(enabled: false));

    expect(find.textContaining('turned off on this server'), findsOneWidget);
    expect(
      tester
          .widget<StormButton>(find.byKey(const Key('report-send')))
          .onPressed,
      isNull,
    );
    expect(
      tester
          .widget<ChoiceChips<String>>(find.byKey(const Key('report-kind')))
          .onSelected,
      isNull,
    );
  });

  testWidgets("'See what's sent' shows exactly the fake's client and bundle", (
    tester,
  ) async {
    final client = {'screen': 'agents/session', 'version': '0.19.0'};
    final bundle = {
      'server': {'version': '0.19.0'},
      'hosts': <dynamic>[],
    };
    await open(tester, server(previewClient: client, previewBundle: bundle));

    await tester.tap(find.byKey(const Key('report-see-sent')));
    await tester.pumpAndSettle();

    final expected = const JsonEncoder.withIndent(
      '  ',
    ).convert({'client': client, 'bundle': bundle});
    final shown = tester
        .widget<CodeBlock>(find.byKey(const Key('report-sent-json')))
        .text;
    expect(shown, expected);
  });

  testWidgets('sending shows "Report sent · <first 8 of id>" and closes', (
    tester,
  ) async {
    await open(tester, server());

    await tester.enterText(
      find.byKey(const Key('report-text')),
      'The terminal lost focus after a resize.',
    );
    await tester.tap(find.byKey(const Key('report-send')));
    await tester.pumpAndSettle();

    expect(find.text('Report sent · rep_1234'), findsOneWidget);
    // The sheet closed: its Send button is gone.
    expect(find.byKey(const Key('report-send')), findsNothing);
  });

  testWidgets('an expired preview on Send is recovered by pressing it again', (
    tester,
  ) async {
    await open(tester, server(sendFailTimes: 1));

    await tester.enterText(
      find.byKey(const Key('report-text')),
      'Agents screen is blank after switching vaults.',
    );
    await tester.tap(find.byKey(const Key('report-send')));
    await tester.pumpAndSettle();

    expect(
      find.text('That preview expired. Press Send again.'),
      findsOneWidget,
    );
    // Still open — nothing was lost, and Send is still there to press again.
    expect(find.byKey(const Key('report-send')), findsOneWidget);

    await tester.tap(find.byKey(const Key('report-send')));
    await tester.pumpAndSettle();

    expect(find.text('Report sent · rep_1234'), findsOneWidget);
  });

  testWidgets('an empty report is refused inline, nothing is sent', (
    tester,
  ) async {
    await open(tester, server());

    await tester.tap(find.byKey(const Key('report-send')));
    await tester.pumpAndSettle();

    expect(find.text('Say what happened.'), findsOneWidget);
    expect(find.byKey(const Key('report-send')), findsOneWidget);
  });
}
