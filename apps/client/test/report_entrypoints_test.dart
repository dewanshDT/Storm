import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';

import 'package:storm/api/feedback_api.dart';
import 'package:storm/api/feedback_models.dart';
import 'package:storm/keyboard/storm_commands.dart';
import 'package:storm/state/app_state.dart'
    show rootMessengerKey, rootNavigatorKey;
import 'package:storm/state/feedback_state.dart'
    show feedbackApiFactoryProvider;
import 'package:storm/ui/controls.dart';
import 'package:storm/ui/error_snack.dart';
import 'package:storm/ui/theme.dart';

/// The other ways into the report sheet (decision 162a): the ⌘K command and
/// a "Report this" snackbar action — rather than the sheet's own behaviour,
/// covered in `report_sheet_test.dart`.
void main() {
  MockClient server(List<http.Request> seen) => MockClient((req) async {
    seen.add(req);
    final path = req.url.path;
    if (path == '/v1/feedback' && req.method == 'GET') {
      return http.Response(
        jsonEncode({'enabled': true, 'intake_configured': true}),
        200,
      );
    }
    if (path == '/v1/feedback/preview') {
      return http.Response(
        jsonEncode({
          'preview_id': 'pv_1',
          'expires_at': '2026-11-19T00:10:00Z',
          'client': jsonDecode(req.body)['client'],
          'bundle': <String, dynamic>{},
          'bytes': 10,
        }),
        200,
      );
    }
    return http.Response(jsonEncode({'error': 'not_found'}), 404);
  });

  Widget harness(Widget home, MockClient client) => ProviderScope(
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
          child: Scaffold(body: home),
        ),
      ),
    ),
  );

  testWidgets('the report-problem command opens the sheet', (tester) async {
    final command = stormCommands().firstWhere((c) => c.id == 'report-problem');
    expect(command.label, 'Report a problem');

    final seen = <http.Request>[];
    await tester.pumpWidget(
      harness(
        Consumer(
          builder: (context, ref, _) => Center(
            child: StormButton.primary(
              label: 'Run',
              onPressed: () => command.run(context, ref),
            ),
          ),
        ),
        server(seen),
      ),
    );

    await tester.tap(find.text('Run'));
    await tester.pumpAndSettle();

    expect(find.byKey(const Key('report-text')), findsOneWidget);
  });

  testWidgets(
    'a "Report this" snackbar action opens the sheet with the failure filled in',
    (tester) async {
      final seen = <http.Request>[];
      await tester.pumpWidget(
        harness(
          Center(
            child: StormButton.primary(
              label: 'Fail',
              onPressed: () => showErrorSnack(
                'Could not start the agent.',
                reportContext: const FeedbackErrorContext(
                  action: 'start agent',
                  message: 'host offline',
                ),
              ),
            ),
          ),
          server(seen),
        ),
      );

      await tester.tap(find.text('Fail'));
      await tester.pumpAndSettle();
      expect(find.text('Could not start the agent.'), findsOneWidget);

      await tester.tap(find.text('Report this'));
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('report-text')), findsOneWidget);

      // The prefilled failure reaches the preview request's `client.error`
      // — not just a sheet that happens to have opened.
      await tester.tap(find.byKey(const Key('report-see-sent')));
      await tester.pumpAndSettle();

      final previewRequest = seen.firstWhere(
        (r) => r.url.path == '/v1/feedback/preview',
      );
      final body = jsonDecode(previewRequest.body) as Map<String, dynamic>;
      expect(body['client']['error'], {
        'action': 'start agent',
        'message': 'host offline',
      });
    },
  );

  testWidgets('a snackbar with no reportContext carries no "Report this"', (
    tester,
  ) async {
    final seen = <http.Request>[];
    await tester.pumpWidget(
      harness(
        Center(
          child: StormButton.primary(
            label: 'Info',
            onPressed: () => showErrorSnack('Kept available offline'),
          ),
        ),
        server(seen),
      ),
    );

    await tester.tap(find.text('Info'));
    await tester.pump();
    expect(find.text('Kept available offline'), findsOneWidget);
    expect(find.text('Report this'), findsNothing);
  });
}
