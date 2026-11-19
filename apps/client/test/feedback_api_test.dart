import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';

import 'package:storm/api/feedback_api.dart';
import 'package:storm/api/feedback_models.dart';

/// Decision 162 — feedback API contract, client side (162-5), against a
/// fake `storm-server` rather than a running one: 162-3 lands in parallel.
void main() {
  group('FeedbackApi', () {
    test('GET /v1/feedback reports enabled and intake_configured', () async {
      late Uri seen;
      final api = FeedbackApi(
        baseUrl: 'http://server:8484',
        token: 'sta_session',
        client: MockClient((req) async {
          seen = req.url;
          return http.Response(
            jsonEncode({'enabled': true, 'intake_configured': true}),
            200,
          );
        }),
      );

      final status = await api.status();
      expect(seen.path, '/v1/feedback');
      expect(status.enabled, isTrue);
      expect(status.intakeConfigured, isTrue);
    });

    test(
      'a 403 feedback_disabled on GET surfaces as FeedbackApiException',
      () async {
        final api = FeedbackApi(
          baseUrl: 'http://server:8484',
          token: 't',
          client: MockClient(
            (req) async =>
                http.Response(jsonEncode({'error': 'feedback_disabled'}), 403),
          ),
        );

        await expectLater(
          api.status(),
          throwsA(
            isA<FeedbackApiException>().having(
              (e) => e.isDisabled,
              'isDisabled',
              isTrue,
            ),
          ),
        );
      },
    );

    test('POST /v1/feedback/preview sends the kind, flag and client object, '
        'and returns exactly the client + bundle it echoes', () async {
      Map<String, dynamic>? sentBody;
      final api = FeedbackApi(
        baseUrl: 'http://server:8484',
        token: 't',
        client: MockClient((req) async {
          expect(req.url.path, '/v1/feedback/preview');
          sentBody = jsonDecode(req.body) as Map<String, dynamic>;
          return http.Response(
            jsonEncode({
              'preview_id': 'pv_1',
              'expires_at': '2026-11-19T00:10:00Z',
              'client': {'version': '0.19.0', 'screen': 'settings/health'},
              'bundle': {
                'server': {'version': '0.19.0'},
              },
              'bytes': 512,
            }),
            200,
          );
        }),
      );

      final client = FeedbackClientInfo(
        version: '0.19.0',
        platform: 'linux',
        clientKind: 'desktop',
        screen: 'settings/health',
        actions: const ['opened settings/health'],
        connectionLog: const ['probe ok'],
      );
      final preview = await api.preview(
        kind: 'bug',
        includeDiagnostics: true,
        client: client,
      );

      expect(sentBody!['kind'], 'bug');
      expect(sentBody!['include_diagnostics'], true);
      expect(sentBody!['client']['screen'], 'settings/health');
      expect(sentBody!['client']['actions'], ['opened settings/health']);
      expect(sentBody!['client']['error'], isNull);

      expect(preview.previewId, 'pv_1');
      expect(preview.client, {
        'version': '0.19.0',
        'screen': 'settings/health',
      });
      expect(preview.bundle, {
        'server': {'version': '0.19.0'},
      });
      expect(preview.bytes, 512);
    });

    test('POST /v1/feedback forwards exactly the fields given', () async {
      Map<String, dynamic>? sentBody;
      final api = FeedbackApi(
        baseUrl: 'http://server:8484',
        token: 't',
        client: MockClient((req) async {
          expect(req.url.path, '/v1/feedback');
          sentBody = jsonDecode(req.body) as Map<String, dynamic>;
          return http.Response(
            jsonEncode({'report_id': 'rep_abcdefgh1234', 'queued': true}),
            202,
          );
        }),
      );

      final result = await api.send(
        previewId: 'pv_1',
        kind: 'idea',
        text: 'Add a dark terminal theme toggle here too.',
        email: 'me@example.com',
        screenshotPngB64: 'aGVsbG8=',
      );

      expect(sentBody, {
        'preview_id': 'pv_1',
        'kind': 'idea',
        'text': 'Add a dark terminal theme toggle here too.',
        'email': 'me@example.com',
        'screenshot_png_b64': 'aGVsbG8=',
      });
      expect(result.reportId, 'rep_abcdefgh1234');
      expect(result.queued, isTrue);
    });

    test(
      'a 410 preview_expired on send is a distinguishable exception',
      () async {
        final api = FeedbackApi(
          baseUrl: 'http://server:8484',
          token: 't',
          client: MockClient(
            (req) async =>
                http.Response(jsonEncode({'error': 'preview_expired'}), 410),
          ),
        );

        await expectLater(
          api.send(previewId: 'pv_dead', kind: 'bug', text: 'x'),
          throwsA(
            isA<FeedbackApiException>().having(
              (e) => e.isPreviewExpired,
              'isPreviewExpired',
              isTrue,
            ),
          ),
        );
      },
    );
  });
}
