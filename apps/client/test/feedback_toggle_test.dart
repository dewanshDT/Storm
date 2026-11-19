import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';

import 'package:storm/api/feedback_api.dart';
import 'package:storm/state/feedback_state.dart'
    show feedbackApiFactoryProvider;
import 'package:storm/ui/controls.dart';
import 'package:storm/ui/settings/feedback_toggle.dart';
import 'package:storm/ui/theme.dart';

/// Settings › About & health's "Send problem reports" toggle (decision
/// 162k), built against a fake `storm-server` whose `GET /v1/feedback`
/// reflects the last `PUT /v1/config/feedback` it was sent.
void main() {
  (MockClient, List<http.Request>) server({bool enabled = true}) {
    var on = enabled;
    final seen = <http.Request>[];
    final client = MockClient((req) async {
      seen.add(req);
      if (req.url.path == '/v1/feedback' && req.method == 'GET') {
        return http.Response(
          jsonEncode({'enabled': on, 'intake_configured': true}),
          200,
        );
      }
      if (req.url.path == '/v1/config/feedback' && req.method == 'PUT') {
        final body = jsonDecode(req.body) as Map<String, dynamic>;
        if (body.containsKey('enabled')) on = body['enabled'] as bool;
        return http.Response(
          jsonEncode({'enabled': on, 'intake_configured': true}),
          200,
        );
      }
      return http.Response(jsonEncode({'error': 'not_found'}), 404);
    });
    return (client, seen);
  }

  Widget harness(MockClient client) => ProviderScope(
    overrides: [
      feedbackApiFactoryProvider.overrideWithValue(
        () => FeedbackApi(baseUrl: 'http://server', token: 't', client: client),
      ),
    ],
    child: MaterialApp(
      theme: StormTheme.light(),
      home: const Scaffold(body: FeedbackToggleRow()),
    ),
  );

  testWidgets('on by default, with the toggle live', (tester) async {
    final (client, _) = server();
    await tester.pumpWidget(harness(client));
    await tester.pumpAndSettle();

    expect(
      tester
          .widget<StormToggle>(find.byKey(const Key('feedback-enabled-toggle')))
          .value,
      isTrue,
    );
    expect(find.textContaining('Lets "Report a problem" send'), findsOneWidget);
  });

  testWidgets('starting off shows the off copy and a false toggle', (
    tester,
  ) async {
    final (client, _) = server(enabled: false);
    await tester.pumpWidget(harness(client));
    await tester.pumpAndSettle();

    expect(
      tester
          .widget<StormToggle>(find.byKey(const Key('feedback-enabled-toggle')))
          .value,
      isFalse,
    );
    expect(find.textContaining('nothing is sent'), findsOneWidget);
  });

  testWidgets('tapping it PUTs the flip and the row updates', (tester) async {
    final (client, seen) = server();
    await tester.pumpWidget(harness(client));
    await tester.pumpAndSettle();

    await tester.tap(find.byKey(const Key('feedback-enabled-toggle')));
    await tester.pumpAndSettle();

    final put = seen.firstWhere(
      (r) => r.method == 'PUT' && r.url.path == '/v1/config/feedback',
    );
    expect(jsonDecode(put.body), {'enabled': false});
    expect(
      tester
          .widget<StormToggle>(find.byKey(const Key('feedback-enabled-toggle')))
          .value,
      isFalse,
    );
  });
}
