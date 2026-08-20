import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shared_preferences/shared_preferences.dart';
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

  copyButtonTests();

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

/// The reveal dialog's whole job is getting a string onto the clipboard.
///
/// **The first version of these tests parsed the config snippet and never
/// touched the copying**, so a copy that silently did nothing shipped and was
/// found by hand on a real deployment: Storm's web client is served over plain
/// HTTP on a LAN address, which is not a *secure context*, so the browser does
/// not expose `navigator.clipboard` at all. The button reported success
/// regardless.
///
/// These drive the button and assert what the user is *told*, which is the
/// part that was wrong.
void copyButtonTests() {
  Widget host(CreatedMcpKey created) => ProviderScope(
    child: MaterialApp(
      home: Scaffold(body: RevealMcpKeyDialog(created: created)),
    ),
  );

  final created = CreatedMcpKey(
    key: McpKey.fromJson({
      'id': 'key_1',
      'user_id': 'u1',
      'name': 'laptop',
      'created': '2026-08-20T04:45:38Z',
    }),
    secret: 'stk_theonlycopy',
  );

  group('the copy buttons', () {
    late List<MethodCall> calls;

    setUp(() {
      calls = [];
      SharedPreferences.setMockInitialValues({});
      FlutterSecureStorage.setMockInitialValues({});
    });

    void handleClipboard({required bool succeed}) {
      TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .setMockMethodCallHandler(SystemChannels.platform, (call) async {
            calls.add(call);
            if (call.method == 'Clipboard.setData' && !succeed) {
              throw PlatformException(code: 'unavailable');
            }
            return null;
          });
    }

    testWidgets('a successful copy sends the secret and says so', (
      tester,
    ) async {
      handleClipboard(succeed: true);
      await tester.pumpWidget(host(created));
      await tester.pumpAndSettle();

      await tester.tap(find.byKey(const Key('copy-key')));
      await tester.pumpAndSettle();

      final copy = calls.firstWhere((c) => c.method == 'Clipboard.setData');
      expect(
        (copy.arguments as Map)['text'],
        'stk_theonlycopy',
        reason: 'the button must copy the key, not a label or a preview',
      );
      // Exact, not `textContaining`: the dialog's own dismiss button says
      // "I've copied it", which a loose matcher happily mistakes for the
      // confirmation.
      expect(find.text('Key copied'), findsOneWidget);
    });

    testWidgets('a failed copy says it failed instead of claiming success', (
      tester,
    ) async {
      // The real-world case: a browser with no clipboard API, because the page
      // is served over plain HTTP. Reporting "copied" there is worse than
      // reporting nothing — the text is selectable, but only if you are told.
      handleClipboard(succeed: false);
      await tester.pumpWidget(host(created));
      await tester.pumpAndSettle();

      await tester.tap(find.byKey(const Key('copy-key')));
      await tester.pumpAndSettle();

      expect(find.textContaining("Couldn't copy"), findsOneWidget);
      expect(find.text('Key copied'), findsNothing);
    });

    testWidgets('the config button copies the whole snippet', (tester) async {
      handleClipboard(succeed: true);
      await tester.pumpWidget(host(created));
      await tester.pumpAndSettle();

      // The snippet is tall enough to push its button off the test surface.
      await tester.ensureVisible(find.byKey(const Key('copy-config')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const Key('copy-config')));
      await tester.pumpAndSettle();

      final copied =
          (calls.firstWhere((c) => c.method == 'Clipboard.setData').arguments
              as Map)['text'];
      final parsed = jsonDecode(copied as String) as Map<String, dynamic>;
      expect(((parsed['mcpServers'] as Map)['storm'] as Map)['headers'], {
        'Authorization': 'Bearer stk_theonlycopy',
      });
    });
  });
}
