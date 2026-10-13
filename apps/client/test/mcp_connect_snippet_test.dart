import 'dart:convert';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:storm/api/auth_models.dart';
import 'package:storm/api/models.dart';
import 'package:storm/state/app_state.dart';
import 'package:storm/ui/settings/access_page.dart';
import 'package:storm/ui/settings/mcp_connect_config.dart';

import 'shell_harness.dart' show FixedSettings;

/// Decision 105e: the reveal's first offer is `storm-runtime mcp-connect`.
///
/// The properties worth pinning are the ones a pasted line gets wrong without
/// anyone noticing until it fails on another machine: the key in argv, a
/// relay URL as the address, and a word the shell splits.
void main() {
  const secret = 'stk_theonlycopy';
  const target = McpConnectTarget(
    serverId: 'srv_abc',
    publicKey: 'AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8',
    addr: 'http://192.168.1.20:8484',
    relays: ['wss://relay.example.net', 'wss://relay.storm.dev'],
  );

  Map<String, dynamic> stormEntry(String json) =>
      ((jsonDecode(json) as Map<String, dynamic>)['mcpServers']
              as Map<String, dynamic>)['storm']
          as Map<String, dynamic>;

  group('the stdio config', () {
    test('runs storm-runtime mcp-connect with every registered relay', () {
      final storm = stormEntry(target.stdioConfig(secret));
      expect(storm['command'], 'storm-runtime');
      expect(storm['args'], [
        'mcp-connect',
        '--server',
        'srv_abc',
        '--pk',
        'AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8',
        '--addr',
        'http://192.168.1.20:8484',
        '--relay',
        'wss://relay.example.net',
        '--relay',
        'wss://relay.storm.dev',
      ]);
    });

    test('with no relays, has no --relay at all', () {
      const lan = McpConnectTarget(
        serverId: 'srv_abc',
        publicKey: 'key',
        addr: 'http://storm.home:8484',
      );
      final args = stormEntry(lan.stdioConfig(secret))['args'] as List;
      expect(args, isNot(contains('--relay')));
      expect(args.last, 'http://storm.home:8484');
      expect(lan.claudeCommand(secret), isNot(contains('--relay')));
    });

    test('carries the key in env and nowhere else', () {
      final storm = stormEntry(target.stdioConfig(secret));
      expect(storm['env'], {'STORM_MCP_KEY': secret});
      expect(
        (storm['args'] as List).join(' '),
        isNot(contains(secret)),
        reason: 'argv is visible to every user on the machine through ps',
      );
    });
  });

  group('the claude mcp add line', () {
    test('passes the key with -e, before the command, and only there', () {
      final line = target.claudeCommand(secret);
      expect(
        line,
        startsWith('claude mcp add storm -e STORM_MCP_KEY=$secret -- '),
      );
      final command = line.substring(line.indexOf(' -- ') + 4);
      expect(command, isNot(contains(secret)));
      expect(secret.allMatches(line), hasLength(1));
    });

    test('is the documented shape', () {
      expect(
        target.claudeCommand(secret),
        'claude mcp add storm -e STORM_MCP_KEY=$secret -- \\\n'
        '  storm-runtime mcp-connect --server srv_abc --pk '
        'AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8 \\\n'
        '    --addr http://192.168.1.20:8484 --relay wss://relay.example.net '
        '--relay wss://relay.storm.dev',
      );
    });

    test(
      'a shell splits it back into exactly the intended words',
      () async {
        // Values a hand-written line would break: a quote, a space, `&`, `$`.
        const awkward = McpConnectTarget(
          serverId: 'srv_abc',
          publicKey: '-starts-with-a-dash',
          addr: r"http://it's here:8484/?a=1&b=$HOME",
          relays: ['wss://relay one.example'],
        );
        const key = r"stk_a'b c$d";
        final line = awkward.claudeCommand(key);
        final script = line.replaceFirst('claude', r"printf '%s\n'");
        final result = await Process.run('sh', ['-c', script]);
        expect(result.exitCode, 0, reason: '${result.stderr}');
        expect(const LineSplitter().convert(result.stdout as String), [
          'mcp',
          'add',
          'storm',
          '-e',
          'STORM_MCP_KEY=$key',
          '--',
          'storm-runtime',
          'mcp-connect',
          '--server',
          'srv_abc',
          '--pk=-starts-with-a-dash',
          '--addr',
          r"http://it's here:8484/?a=1&b=$HOME",
          '--relay',
          'wss://relay one.example',
        ]);
      },
      skip: Platform.isWindows ? 'needs a POSIX sh' : false,
    );

    test('quotes only what needs it', () {
      expect(shellQuote('wss://relay.example.net'), 'wss://relay.example.net');
      expect(shellQuote('STORM_MCP_KEY=stk_x'), 'STORM_MCP_KEY=stk_x');
      expect(shellQuote('a b'), "'a b'");
      expect(shellQuote("it's"), r"'it'\''s'");
      expect(shellQuote(''), "''");
    });
  });

  group('the target', () {
    const paired = Settings(
      baseUrl: 'http://192.168.1.20:8484',
      serverId: 'srv_abc',
      serverPublicKey: 'pinnedkey',
      relays: [
        RelayAdvert(
          url: 'wss://relay.example.net',
          publicAddress: 'wss://relay.example.net/connect/srv_abc',
        ),
      ],
    );

    test('is the pinned identity, the paired address and the relay URLs', () {
      final t = McpConnectTarget.fromSettings(paired)!;
      expect(t.serverId, 'srv_abc');
      expect(t.publicKey, 'pinnedkey');
      // The server's own address even while this device is relayed: the
      // relayed route is never written to settings.
      expect(t.addr, 'http://192.168.1.20:8484');
      // The relay itself, not the per-server public address: mcp-connect
      // derives that from --server.
      expect(t.relays, ['wss://relay.example.net']);
    });

    test('is absent without a pinned identity', () {
      expect(
        McpConnectTarget.fromSettings(
          const Settings(baseUrl: 'http://x:8484', serverId: 'srv_abc'),
        ),
        isNull,
      );
      expect(
        McpConnectTarget.fromSettings(
          const Settings(baseUrl: 'http://x:8484', serverPublicKey: 'k'),
        ),
        isNull,
      );
    });
  });

  revealTests();
}

void revealTests() {
  final created = CreatedMcpKey(
    key: McpKey.fromJson({
      'id': 'key_1',
      'user_id': 'u1',
      'name': 'laptop',
      'created': '2026-10-13T04:45:38Z',
    }),
    secret: 'stk_theonlycopy',
  );

  late List<MethodCall> calls;

  setUp(() {
    calls = [];
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(SystemChannels.platform, (call) async {
          calls.add(call);
          return null;
        });
  });

  Widget host(Settings settings) => ProviderScope(
    overrides: [settingsProvider.overrideWith(() => FixedSettings(settings))],
    child: MaterialApp(
      home: Scaffold(body: RevealMcpKeyDialog(created: created)),
    ),
  );

  String copied() =>
      (calls.lastWhere((c) => c.method == 'Clipboard.setData').arguments
              as Map)['text']
          as String;

  Future<void> tapCopy(WidgetTester tester, String id) async {
    await tester.ensureVisible(find.byKey(Key('copy-$id')));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(Key('copy-$id')));
    await tester.pumpAndSettle();
  }

  const paired = Settings(
    baseUrl: 'http://192.168.1.20:8484',
    serverId: 'srv_abc',
    serverPublicKey: 'pinnedkey',
    relays: [
      RelayAdvert(
        url: 'wss://relay.example.net',
        publicAddress: 'wss://relay.example.net/connect/srv_abc',
      ),
    ],
  );

  group('the reveal', () {
    testWidgets('offers mcp-connect first, HTTP behind a disclosure', (
      tester,
    ) async {
      await tester.pumpWidget(host(paired));
      await tester.pumpAndSettle();

      expect(find.byKey(const Key('copy-command')), findsOneWidget);
      expect(find.byKey(const Key('copy-config')), findsOneWidget);
      expect(find.byKey(const Key('copy-http')), findsNothing);

      await tapCopy(tester, 'command');
      expect(copied(), startsWith('claude mcp add storm -e STORM_MCP_KEY='));

      await tapCopy(tester, 'config');
      final storm =
          ((jsonDecode(copied()) as Map)['mcpServers'] as Map)['storm'] as Map;
      expect(storm['env'], {'STORM_MCP_KEY': 'stk_theonlycopy'});
      expect(
        storm['args'],
        containsAllInOrder(['--relay', 'wss://relay.example.net']),
      );

      await tester.ensureVisible(find.byKey(const Key('other-clients')));
      await tester.tap(find.byKey(const Key('other-clients')));
      await tester.pumpAndSettle();
      await tapCopy(tester, 'http');
      final http =
          ((jsonDecode(copied()) as Map)['mcpServers'] as Map)['storm'] as Map;
      expect(http['url'], 'http://192.168.1.20:8484/mcp');
    });

    testWidgets('without a pinned identity, offers the HTTP config alone', (
      tester,
    ) async {
      await tester.pumpWidget(
        host(const Settings(baseUrl: 'http://192.168.1.20:8484')),
      );
      await tester.pumpAndSettle();

      expect(find.byKey(const Key('copy-command')), findsNothing);
      expect(find.byKey(const Key('other-clients')), findsNothing);
      await tapCopy(tester, 'http');
      expect(copied(), contains('"Authorization": "Bearer stk_theonlycopy"'));
    });
  });
}
