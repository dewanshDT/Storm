import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';

import 'package:storm/agent/integrations_api.dart';
import 'package:storm/agent/oauth_flow_io.dart';

/// The native sign-in (decision 81h): a loopback listener catches the one
/// redirect, and the client relays `{state, code}` to the server (G-D13).
void main() {
  Map<String, dynamic> checked() => {
    'ok': true,
    'tool_count': 1,
    'new_tools': <String>[],
    'integration': {
      'id': 'mcc_N',
      'slug': 'notion',
      'display_name': 'Notion',
      'auth_kind': 'oauth',
      'status': 'connected',
    },
  };

  test('the redirect is caught on loopback and relayed, never kept', () async {
    String? redirect;
    Map<String, dynamic>? relayed;
    final client = MockClient((req) async {
      if (req.url.path.endsWith('/authorize')) {
        redirect = (jsonDecode(req.body) as Map)['redirect_uri'] as String;
        return http.Response(
          jsonEncode({
            'authorization_url': 'https://as.example/authorize?state=abc',
          }),
          200,
        );
      }
      if (req.url.path == '/v1/integrations/oauth/callback') {
        relayed = jsonDecode(req.body) as Map<String, dynamic>;
        return http.Response(jsonEncode(checked()), 200);
      }
      return http.Response('', 404);
    });
    final api = IntegrationsApi(
      baseUrl: 'http://s',
      token: 't',
      client: client,
    );

    final result = await signIn(
      api,
      'mcc_N',
      openBrowser: (url) async {
        expect(url.host, 'as.example');
        // The "browser": a stray request first, which must not end the
        // sign-in, then the real redirect.
        final target = Uri.parse(redirect!);
        expect(target.host, '127.0.0.1');
        final http1 = HttpClient();
        final stray = await (await http1.getUrl(
          target.replace(path: '/favicon.ico'),
        )).close();
        expect(stray.statusCode, 404);
        final real = await (await http1.getUrl(
          target.replace(queryParameters: {'state': 'abc', 'code': 'xyz'}),
        )).close();
        expect(real.statusCode, 200);
        http1.close();
        return true;
      },
    );
    expect(redirect, startsWith('http://127.0.0.1:'));
    expect(relayed, {'state': 'abc', 'code': 'xyz'});
    expect(result.ok, isTrue);
    // The listener is closed: the port answers nothing now.
    await expectLater(
      HttpClient().getUrl(Uri.parse(redirect!)),
      throwsA(isA<SocketException>()),
    );
  });

  test('a cancelled sign-in says so and relays nothing', () async {
    String? redirect;
    var relayed = false;
    final client = MockClient((req) async {
      if (req.url.path.endsWith('/authorize')) {
        redirect = (jsonDecode(req.body) as Map)['redirect_uri'] as String;
        return http.Response(
          '{"authorization_url":"https://as.example/a"}',
          200,
        );
      }
      relayed = true;
      return http.Response('', 404);
    });
    final api = IntegrationsApi(
      baseUrl: 'http://s',
      token: 't',
      client: client,
    );
    await expectLater(
      signIn(
        api,
        'mcc_N',
        openBrowser: (_) async {
          final c = HttpClient();
          await (await c.getUrl(
            Uri.parse(
              redirect!,
            ).replace(queryParameters: {'error': 'access_denied'}),
          )).close();
          c.close();
          return true;
        },
      ),
      throwsA(
        isA<StateError>().having(
          (e) => e.message,
          'message',
          contains('cancelled'),
        ),
      ),
    );
    expect(relayed, isFalse);
  });
}
