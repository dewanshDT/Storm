import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';

import 'package:storm/api/models.dart';
import 'package:storm/api/storm_api.dart';
import 'package:storm/ui/mcp_keys_screen.dart';

/// A14, client side.
///
/// The property worth pinning is not that the screen renders — it is that the
/// **plaintext exists in exactly one place**. A list must never carry one, and
/// the config snippet has to be pasteable as-is, because assembling it by hand
/// is where a credential gets mangled the way pairing URIs did.
void main() {
  group('the created key is the only place a secret appears', () {
    test('creating parses the secret, listing has none to parse', () async {
      late String seenPath;
      final api = StormApi(
        baseUrl: 'http://server:8484',
        token: 'sta_session',
        client: MockClient((req) async {
          seenPath = req.url.path;
          if (req.method == 'POST') {
            // The shape the server actually returns: the key's fields,
            // flattened, plus `secret`.
            return http.Response(
              jsonEncode({
                'id': 'key_1',
                'user_id': 'u1',
                'name': 'laptop',
                'created': '2026-08-19T12:00:00Z',
                'expires': null,
                'last_used': null,
                'revoked': null,
                'secret': 'stk_theonlycopy',
              }),
              200,
            );
          }
          return http.Response(
            jsonEncode([
              {
                'id': 'key_1',
                'user_id': 'u1',
                'name': 'laptop',
                'created': '2026-08-19T12:00:00Z',
                'expires': null,
                'last_used': '2026-08-19T13:00:00Z',
                'revoked': null,
              },
            ]),
            200,
          );
        }),
      );

      final created = await api.createMcpKey(name: 'laptop');
      expect(seenPath, '/v1/keys');
      expect(created.secret, 'stk_theonlycopy');
      expect(created.key.name, 'laptop');

      final listed = await api.mcpKeys();
      expect(listed, hasLength(1));
      expect(listed.first.lastUsed, isNotNull);
      // `McpKey` has no field that could hold one, which is the point of
      // modelling the created key as a separate type.
      expect(listed.first.name, 'laptop');
    });

    test('a revoked key reports itself as revoked', () {
      final key = McpKey.fromJson({
        'id': 'key_1',
        'user_id': 'u1',
        'name': 'old laptop',
        'created': '2026-08-19T12:00:00Z',
        'revoked': '2026-08-20T09:00:00Z',
      });
      expect(key.isRevoked, isTrue);
    });
  });

  group('the MCP config snippet', () {
    test('is valid JSON carrying the key as a Bearer header', () {
      final snippet = RevealMcpKeyDialog.configSnippet(
        'http://192.168.1.20:8484',
        'stk_abc123',
      );

      // Parsed rather than string-matched: the whole reason this is generated
      // is that a human assembling it by hand gets it subtly wrong, so the
      // test has to check the thing a client will actually parse.
      final parsed = jsonDecode(snippet) as Map<String, dynamic>;
      final storm =
          (parsed['mcpServers'] as Map<String, dynamic>)['storm']
              as Map<String, dynamic>;

      expect(storm['url'], 'http://192.168.1.20:8484/mcp');
      expect(storm['type'], 'http');
      expect(
        (storm['headers'] as Map<String, dynamic>)['Authorization'],
        'Bearer stk_abc123',
        reason:
            'A14.1 — keys travel as an ordinary Bearer token, because most MCP '
            'clients cannot send any other scheme',
      );
    });

    test('points at /mcp, not the REST root', () {
      // A key is accepted on /mcp and nowhere else (A14.2). A snippet aimed at
      // the wrong path would hand someone a credential that 401s and no way to
      // tell why.
      final snippet = RevealMcpKeyDialog.configSnippet(
        'http://server:8484',
        'stk_x',
      );
      expect(snippet, contains('"url": "http://server:8484/mcp"'));
    });
  });
}
