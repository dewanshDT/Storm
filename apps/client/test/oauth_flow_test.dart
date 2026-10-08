import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:flutter/services.dart';

import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';

import 'package:storm/agent/integrations_api.dart';
import 'package:storm/agent/oauth_flow_io.dart';
import 'package:storm/agent/oauth_links.dart';

/// The native sign-in (decisions 81h, 81l): Linux and Windows catch the one
/// redirect on loopback, Android and macOS receive it as a `storm://oauth`
/// link, and either way the client relays `{state, code}` (G-D13).
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

  group('storm://oauth (Android, macOS; spec §10.4, 81l)', () {
    /// A fake server: `authorize` starts state `abc`, `callback` records
    /// what was relayed, `list` reports `status`.
    (IntegrationsApi, List<String>, Map<String, dynamic> Function()) fake({
      String Function()? status,
    }) {
      final calls = <String>[];
      Map<String, dynamic>? relayed;
      final client = MockClient((req) async {
        final path = req.url.path;
        calls.add('${req.method} $path');
        if (path.endsWith('/authorize')) {
          final redirect = (jsonDecode(req.body) as Map)['redirect_uri'];
          calls.add('redirect $redirect');
          return http.Response(
            jsonEncode({
              'authorization_url': 'https://as.example/authorize?state=abc',
            }),
            200,
          );
        }
        if (path == '/v1/integrations/oauth/callback') {
          relayed = jsonDecode(req.body) as Map<String, dynamic>;
          return http.Response(jsonEncode(checked()), 200);
        }
        if (path == '/v1/integrations/connections' && req.method == 'GET') {
          return http.Response(
            jsonEncode([
              {
                ...(checked()['integration'] as Map),
                'status': status?.call() ?? 'pending_auth',
              },
            ]),
            200,
          );
        }
        if (path.endsWith('/test')) {
          return http.Response(jsonEncode(checked()), 200);
        }
        return http.Response('', 404);
      });
      return (
        IntegrationsApi(baseUrl: 'http://s', token: 't', client: client),
        calls,
        () => relayed ?? {},
      );
    }

    test('the redirect is storm://oauth/callback, matched by its state, and '
        'relayed', () async {
      final (api, calls, relayed) = fake();
      final links = OAuthLinks.detached();
      final result = await signIn(
        api,
        'mcc_N',
        appScheme: true,
        links: links,
        resumes: const Stream.empty(),
        openBrowser: (url) async {
          // Another sign-in's redirect, and a link for some other host:
          // neither finishes this one.
          links.deliver(Uri.parse('storm://oauth/callback?state=zzz&code=1'));
          links.deliver(Uri.parse('storm://pair?state=abc&code=2'));
          links.deliver(
            Uri.parse(
              'storm://oauth/callback?state=abc&code=xyz&iss=https://as',
            ),
          );
          return true;
        },
      );
      expect(calls, contains('redirect storm://oauth/callback'));
      expect(relayed(), {'state': 'abc', 'code': 'xyz', 'iss': 'https://as'});
      expect(result.ok, isTrue);
      // The stray one is kept for the Integrations screen; the pair link is
      // not an OAuth redirect at all.
      expect(links.takeOrphans(), [
        {'state': 'zzz', 'code': '1'},
      ]);
    });

    test('a cancelled sign-in says so and relays nothing', () async {
      final (api, calls, relayed) = fake();
      final links = OAuthLinks.detached();
      await expectLater(
        signIn(
          api,
          'mcc_N',
          appScheme: true,
          links: links,
          resumes: const Stream.empty(),
          openBrowser: (_) async {
            links.deliver(
              Uri.parse('storm://oauth/callback?state=abc&error=access_denied'),
            );
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
      expect(relayed(), isEmpty);
    });

    test('finished by another instance: resuming finds it connected', () async {
      // Android may hand the link to a fresh instance, which relays it as
      // an orphan. This one learns it on resume instead of waiting out the
      // deadline.
      var status = 'pending_auth';
      final (api, calls, relayed) = fake(status: () => status);
      final resumes = StreamController<void>();
      final result = signIn(
        api,
        'mcc_N',
        appScheme: true,
        links: OAuthLinks.detached(),
        resumes: resumes.stream,
        openBrowser: (_) async => true,
      );
      await Future<void>.delayed(const Duration(milliseconds: 50));
      resumes.add(null); // back too early: still pending
      await Future<void>.delayed(const Duration(milliseconds: 50));
      status = 'connected';
      resumes.add(null);
      expect((await result).ok, isTrue);
      expect(relayed(), isEmpty, reason: 'this instance relays nothing');
      await resumes.close();
    });

    test('a link that arrives before Dart listens is taken at start', () async {
      TestWidgetsFlutterBinding.ensureInitialized();
      const channel = MethodChannel('storm/links-test');
      TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .setMockMethodCallHandler(channel, (call) async {
            expect(call.method, 'takeLinks');
            return ['storm://oauth/callback?state=s1&code=c1'];
          });
      final links = OAuthLinks(channel: channel);
      await links.start();
      expect(links.takeOrphans(), [
        {'state': 's1', 'code': 'c1'},
      ]);
    });

    test('a platform without the scheme has nothing to take', () async {
      TestWidgetsFlutterBinding.ensureInitialized();
      final links = OAuthLinks(channel: const MethodChannel('storm/none'));
      await links.start();
      expect(links.orphans.value, isEmpty);
    });
  });
}
