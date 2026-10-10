import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';

import 'package:storm/agent/oauth_links.dart';
import 'package:storm/router.dart';
import 'package:storm/state/app_state.dart';
import 'package:storm/ui/accents.dart';
import 'package:storm/ui/pairing_screen.dart';
import 'package:storm/ui/settings/settings_shell.dart';
import 'package:storm/ui/tokens.dart';

import 'fake_server.dart';
import 'shell_harness.dart';

/// Storm v2 slice 7: every Settings page on its real endpoints, in the shell
/// at both sides of 900.
void main() {
  const phone = Size(390, 844);
  const desk = Size(1280, 800);

  String location(c) => c.read(routerProvider).state.uri.toString();

  Future<void> open(WidgetTester tester, c, String page) async {
    c.read(routerProvider).go(Routes.settingsPage(page));
    await tester.pumpAndSettle();
  }

  Future<void> tapKey(WidgetTester tester, String key) async {
    final f = find.byKey(Key(key));
    await tester.ensureVisible(f);
    await tester.pumpAndSettle();
    await tester.tap(f);
    await tester.pumpAndSettle();
  }

  group('the shell', () {
    testWidgets('phone: list → pushed page → ‹ Settings back to the list', (
      tester,
    ) async {
      final c = shellContainer();
      await pumpShell(tester, c, size: phone);
      c.read(routerProvider).go(Routes.settings);
      await tester.pumpAndSettle();

      await tester.tap(find.byKey(const Key('settings-row-ai')));
      await tester.pumpAndSettle();
      expect(location(c), Routes.settingsPage('ai'));
      expect(find.text('AI access'), findsOneWidget);
      expect(find.byType(AppBar), findsNothing, reason: 'no Material AppBar');
      expect(find.byType(SettingsNav), findsNothing);

      await tester.tap(find.byKey(const Key('settings-back')));
      await tester.pumpAndSettle();
      expect(location(c), Routes.settings);
      expect(find.byKey(const Key('settings-row-ai')), findsOneWidget);
      await disposeShell(tester, c);
    });

    testWidgets('phone: a deep-linked page goes back to the list', (
      tester,
    ) async {
      final c = shellContainer();
      await pumpShell(tester, c, size: phone);
      await open(tester, c, 'storage');
      await tester.tap(find.byKey(const Key('settings-back')));
      await tester.pumpAndSettle();
      expect(location(c), Routes.settings);
      await disposeShell(tester, c);
    });

    testWidgets('desk: the nav selects the page beside it', (tester) async {
      final c = shellContainer();
      await pumpShell(tester, c, size: desk);
      await open(tester, c, 'device');
      expect(find.byType(SettingsNav), findsOneWidget);
      expect(find.byKey(const Key('settings-back')), findsNothing);

      await tester.tap(find.byKey(const Key('settings-nav-storage')));
      await tester.pumpAndSettle();
      expect(location(c), Routes.settingsPage('storage'));
      expect(
        find.text('Where your vaults live on the server.'),
        findsOneWidget,
      );
      Color fillOf(String id) => tester
          .widget<Material>(
            find
                .ancestor(
                  of: find.byKey(Key('settings-nav-$id')),
                  matching: find.byType(Material),
                )
                .first,
          )
          .color!;
      expect(
        fillOf('storage'),
        StormTokens.from(StormPreset.stormDark).surface2,
      );
      expect(fillOf('device'), Colors.transparent);
      await disposeShell(tester, c);
    });

    for (final size in [phone, desk]) {
      testWidgets('every page renders its title at ${size.width}', (
        tester,
      ) async {
        final c = shellContainer();
        await pumpShell(tester, c, size: size);
        for (final p in kSettingsPages) {
          await open(tester, c, p.id);
          expect(
            find.text(p.label),
            findsWidgets,
            reason: '${p.id} shows its title',
          );
          expect(tester.takeException(), isNull);
        }
        await disposeShell(tester, c);
      });
    }
  });

  group('This device', () {
    testWidgets('appearance, text size and note font change real settings', (
      tester,
    ) async {
      final c = shellContainer();
      await pumpShell(tester, c, size: desk);
      await open(tester, c, 'device');
      Settings s() => c.read(settingsProvider).value!;

      await tapKey(tester, 'preset-stormLight');
      expect(s().theme, StormPreset.stormLight);

      final before = s().fontSize;
      await tapKey(tester, 'text-larger');
      expect(s().fontSize, before + 1);
      expect(find.text('${(before + 1).round()} px'), findsOneWidget);

      await tapKey(tester, 'note-font');
      await tapKey(tester, 'font-mono');
      expect(s().bodyFont, BodyFont.mono);

      await tapKey(tester, 'setting-note-id');
      expect(s().showNoteId, isTrue);
      await disposeShell(tester, c);
    });
  });

  group('Devices & access', () {
    Future<dynamic> withDevices(WidgetTester tester, Size size) async {
      final c = shellContainer(
        settings: const Settings(
          baseUrl: 'http://test',
          accessToken: 'sta_test',
          deviceId: 'dev_me',
          activeVault: FakeServer.primaryVault,
        ),
      );
      serverOf(c).devices.addAll([
        {
          'id': 'dev_me',
          'name': 'MacBook',
          'paired': '2026-10-01T00:00:00Z',
          'last_seen': DateTime.now().toUtc().toIso8601String(),
        },
        {
          'id': 'dev_phone',
          'name': 'Pixel 8',
          'paired': '2026-10-01T00:00:00Z',
          'last_seen': DateTime.now()
              .subtract(const Duration(hours: 2))
              .toUtc()
              .toIso8601String(),
        },
        {
          'id': 'dev_old',
          'name': 'Old laptop',
          'paired': '2026-09-01T00:00:00Z',
          'revoked': '2026-09-02T00:00:00Z',
        },
      ]);
      serverOf(c).keys.add({
        'id': 'key_1',
        'user_id': 'u1',
        'name': 'laptop-claude',
        'created': '2026-10-01T00:00:00Z',
      });
      await pumpShell(tester, c, size: size);
      await open(tester, c, 'access');
      return c;
    }

    testWidgets('lists signed-in devices from the server, this one first', (
      tester,
    ) async {
      final c = await withDevices(tester, desk);
      expect(find.text('MacBook (this device)'), findsOneWidget);
      expect(find.text('Pixel 8'), findsOneWidget);
      expect(find.text('active 2h ago'), findsOneWidget);
      expect(find.text('Old laptop'), findsNothing, reason: 'revoked');
      expect(find.byKey(const Key('sign-out')), findsOneWidget);
      expect(
        tester.getTopLeft(find.text('MacBook (this device)')).dy,
        lessThan(tester.getTopLeft(find.text('Pixel 8')).dy),
      );
      await disposeShell(tester, c);
    });

    testWidgets('revoking another device reaches the server', (tester) async {
      final c = await withDevices(tester, phone);
      await tapKey(tester, 'revoke-device-dev_phone');
      await tapKey(tester, 'confirm-revoke');
      final pixel = serverOf(
        c,
      ).devices.firstWhere((d) => d['id'] == 'dev_phone');
      expect(pixel['revoked'], isNotNull);
      expect(find.text('Pixel 8'), findsNothing);
      await disposeShell(tester, c);
    });

    testWidgets('sign out ends this device\'s session', (tester) async {
      final c = await withDevices(tester, desk);
      await tapKey(tester, 'sign-out');
      expect(c.read(settingsProvider).value!.accessToken, isEmpty);
      await disposeShell(tester, c);
    });

    testWidgets('Add a device opens the existing QR page', (tester) async {
      final c = await withDevices(tester, phone);
      await tapKey(tester, 'add-device');
      expect(location(c), Routes.addDevice);
      await disposeShell(tester, c);
    });

    testWidgets('a new key is shown once, then listed without its secret', (
      tester,
    ) async {
      final c = await withDevices(tester, desk);
      expect(find.text('laptop-claude'), findsOneWidget);
      expect(find.text('never used'), findsOneWidget);

      await tapKey(tester, 'new-key');
      await tester.enterText(find.byKey(const Key('prompt-field')), 'ci');
      await tapKey(tester, 'prompt-ok');
      expect(find.text('Copy it now'), findsOneWidget);
      expect(find.text('stk_theonlycopy'), findsOneWidget);
      await tapKey(tester, 'reveal-done');
      expect(find.text('ci'), findsOneWidget);
      expect(find.textContaining('stk_theonlycopy'), findsNothing);
      await disposeShell(tester, c);
    });

    testWidgets('revoking a key reaches the server', (tester) async {
      final c = await withDevices(tester, desk);
      await tapKey(tester, 'revoke-key-key_1');
      await tapKey(tester, 'confirm-revoke');
      expect(serverOf(c).keys.single['revoked'], isNotNull);
      expect(find.text('laptop-claude'), findsNothing);
      await disposeShell(tester, c);
    });
  });

  group('Vaults', () {
    testWidgets('rows carry count and path; a missing vault says so', (
      tester,
    ) async {
      final c = shellContainer();
      serverOf(
        c,
      ).vaults.add(ServerVault(id: 'v-gone', name: 'archive', missing: true));
      await pumpShell(tester, c, size: desk);
      await open(tester, c, 'vaults');
      expect(find.text('Primary'), findsOneWidget);
      expect(
        find.text('${vaultPaths.length} notes · /srv/storm/vaults/primary'),
        findsOneWidget,
      );
      expect(
        find.text('Directory not found. Nothing was deleted.'),
        findsOneWidget,
      );
      expect(find.byKey(const Key('rename-vault-v-gone')), findsNothing);
      await disposeShell(tester, c);
    });

    testWidgets('Q7: the tile opens the AccentPicker and stores the colour', (
      tester,
    ) async {
      final c = shellContainer();
      await pumpShell(tester, c, size: desk);
      await open(tester, c, 'vaults');

      await tester.tap(find.byKey(const Key('vault-tile-v-primary')));
      await tester.pumpAndSettle();
      expect(find.byType(AccentPicker), findsOneWidget);

      await tester.tap(find.byTooltip(Accent.sage.label));
      await tester.pumpAndSettle();
      expect(find.byType(AccentPicker), findsNothing);
      final config = serverOf(
        c,
      ).notes.values.where((n) => n.path == '_storm/vault.md');
      expect(config, hasLength(1));
      expect(config.single.content, contains('storm.color: sage'));
      await disposeShell(tester, c);
    });

    testWidgets('rename and new vault reach the server', (tester) async {
      final c = shellContainer();
      await pumpShell(tester, c, size: phone);
      await open(tester, c, 'vaults');

      await tapKey(tester, 'rename-vault-v-primary');
      await tester.enterText(find.byKey(const Key('prompt-field')), 'Home');
      await tapKey(tester, 'prompt-ok');
      expect(serverOf(c).vaults.first.name, 'Home');
      expect(find.text('Home'), findsOneWidget);

      await tapKey(tester, 'new-vault');
      await tester.enterText(find.byKey(const Key('prompt-field')), 'Work');
      await tapKey(tester, 'prompt-ok');
      expect(serverOf(c).vaults.map((v) => v.name), contains('Work'));
      await disposeShell(tester, c);
    });

    testWidgets('remove asks first, then unregisters', (tester) async {
      final c = shellContainer();
      serverOf(c).addVault('v-2', 'Work');
      await pumpShell(tester, c, size: desk);
      await open(tester, c, 'vaults');
      await tapKey(tester, 'remove-vault-v-2');
      expect(find.textContaining('nothing is deleted'), findsOneWidget);
      await tapKey(tester, 'confirm-remove');
      expect(serverOf(c).vaults.map((v) => v.id), isNot(contains('v-2')));
      await disposeShell(tester, c);
    });
  });

  group('AI access › Storm agents', () {
    testWidgets('an older server: the write toggle is off and says why', (
      tester,
    ) async {
      final c = shellContainer();
      await pumpShell(tester, c, size: desk);
      await open(tester, c, 'ai');
      expect(find.text('Needs a newer server'), findsOneWidget);
      await tapKey(tester, 'agent-writes');
      expect(serverOf(c).agentWrites, isNull, reason: 'nothing was sent');
      await disposeShell(tester, c);
    });

    testWidgets('a server with agent_writes: the toggle reaches it alone', (
      tester,
    ) async {
      final c = shellContainer();
      serverOf(c)
        ..agentWrites = false
        ..mcpEnabled = true;
      await pumpShell(tester, c, size: phone);
      await open(tester, c, 'ai');
      expect(find.text('Needs a newer server'), findsNothing);

      await tapKey(tester, 'agent-writes');
      expect(serverOf(c).agentWrites, isTrue);
      expect(serverOf(c).mcpEnabled, isTrue, reason: 'MCP left alone');
      await disposeShell(tester, c);
    });

    testWidgets('the access-keys link goes to Devices & access', (
      tester,
    ) async {
      final c = shellContainer();
      await pumpShell(tester, c, size: desk);
      await open(tester, c, 'ai');
      await tapKey(tester, 'go-access');
      expect(location(c), Routes.settingsPage('access'));
      await disposeShell(tester, c);
    });
  });

  group('Integrations', () {
    Map<String, dynamic> github(String status) => {
      'id': 'mcc_GH',
      'slug': 'github',
      'display_name': 'GitHub',
      'url': 'https://api.example.com/mcp/',
      'auth_kind': 'oauth',
      'status': status,
      'builtin': false,
      'has_credential': true,
      'tool_allowlist': ['search'],
      'known_tools': ['search'],
      'new_tools': <String>[],
    };

    (MockClient, List<http.Request>) integrations(String status) {
      final seen = <http.Request>[];
      return (
        MockClient((req) async {
          seen.add(req);
          if (req.url.path == '/v1/integrations/connections') {
            return http.Response(jsonEncode([github(status)]), 200);
          }
          if (req.url.path == '/v1/integrations/oauth/callback') {
            return http.Response(
              jsonEncode({
                'ok': true,
                'error_code': null,
                'tool_count': 3,
                'new_tools': <String>[],
                'integration': github('connected'),
              }),
              200,
            );
          }
          return http.Response('{"error":"nope"}', 404);
        }),
        seen,
      );
    }

    for (final size in [phone, desk]) {
      testWidgets('a connection needing sign-in puts a dot on the nav '
          '(${size.width})', (tester) async {
        final (client, _) = integrations('needs_reauth');
        final c = shellContainer(integrationsClient: client);
        await pumpShell(tester, c, size: size);
        c.read(routerProvider).go(Routes.settings);
        await tester.pumpAndSettle();
        expect(find.byKey(const Key('integrations-attention')), findsOneWidget);
        await open(tester, c, 'integrations');
        expect(find.text('Needs sign-in'), findsOneWidget);
        expect(find.text('Sign in again'), findsOneWidget);
        await disposeShell(tester, c);
      });
    }

    testWidgets('a healthy connection shows no dot, and Choose tools', (
      tester,
    ) async {
      final (client, _) = integrations('connected');
      final c = shellContainer(integrationsClient: client);
      await pumpShell(tester, c, size: desk);
      await open(tester, c, 'integrations');
      expect(find.byKey(const Key('integrations-attention')), findsNothing);
      expect(find.text('Signed in · 1 tool on'), findsOneWidget);
      expect(find.byKey(const Key('tools-mcc_GH')), findsOneWidget);
      await disposeShell(tester, c);
    });

    for (final size in [phone, desk]) {
      testWidgets('an OAuth orphan still lands on Integrations and is relayed '
          '(${size.width})', (tester) async {
        final (client, seen) = integrations('connected');
        final links = OAuthLinks.detached()
          ..deliver(Uri.parse('storm://oauth/callback?state=s1&code=c1'));
        final c = shellContainer(integrationsClient: client, oauthLinks: links);
        await pumpShell(tester, c, size: size);
        // What `main.dart` does when an orphan arrives.
        c.read(routerProvider).go(Routes.integrations);
        await tester.pumpAndSettle();
        expect(location(c), '/settings/integrations');
        final relay = seen.where(
          (r) => r.url.path == '/v1/integrations/oauth/callback',
        );
        expect(relay, hasLength(1));
        expect(jsonDecode(relay.single.body), {'state': 's1', 'code': 'c1'});
        expect(links.orphans.value, isEmpty);
        await disposeShell(tester, c);
      });
    }
  });

  group('Hosts & default agent', () {
    (MockClient, List<http.Request>) agent({bool hosts = true}) {
      final seen = <http.Request>[];
      return (
        MockClient((req) async {
          seen.add(req);
          final path = req.url.path;
          if (path == '/v1/agent/hosts') {
            return http.Response(
              jsonEncode([
                if (hosts)
                  {
                    'id': 'hst_1',
                    'name': 'build-vm',
                    'status': 'online',
                    'capabilities': {
                      'providers': [
                        {'id': 'claude-code', 'kind': 'cli', 'available': true},
                        {'id': 'shell', 'kind': 'cli', 'available': true},
                      ],
                    },
                  },
              ]),
              200,
            );
          }
          if (path == '/v1/agent/sessions') return http.Response('[]', 200);
          if (path == '/v1/config/agent') {
            return http.Response('{"default_provider":"claude-code"}', 200);
          }
          return http.Response('{"error":"nope"}', 404);
        }),
        seen,
      );
    }

    testWidgets('host rows and the default agent chips', (tester) async {
      final (client, seen) = agent();
      final c = shellContainer(agentClient: client);
      await pumpShell(tester, c, size: desk);
      await open(tester, c, 'hosts');
      expect(find.text('build-vm'), findsOneWidget);
      expect(
        find.text('online now · Claude Code, Shell · network: host policy'),
        findsOneWidget,
      );
      expect(find.byKey(const Key('rename-host-hst_1')), findsOneWidget);
      await tester.tap(find.text('OpenCode'));
      await tester.pumpAndSettle();
      final put = seen.where(
        (r) => r.method == 'PUT' && r.url.path == '/v1/config/agent',
      );
      expect(jsonDecode(put.single.body), {'default_provider': 'opencode'});
      await disposeShell(tester, c);
    });

    testWidgets('no hosts says so, and About & health offers Enroll', (
      tester,
    ) async {
      final (client, _) = agent(hosts: false);
      final c = shellContainer(agentClient: client);
      await pumpShell(tester, c, size: phone);
      await open(tester, c, 'hosts');
      expect(find.text('No hosts enrolled yet.'), findsOneWidget);
      await open(tester, c, 'health');
      expect(find.text('No hosts enrolled'), findsOneWidget);
      await tester.tap(find.text('Enroll'));
      await tester.pumpAndSettle();
      expect(location(c), Routes.settingsPage('hosts'));
      await disposeShell(tester, c);
    });
  });

  group('Storage', () {
    testWidgets('shows the root and changes it on the server', (tester) async {
      final c = shellContainer();
      await pumpShell(tester, c, size: desk);
      await open(tester, c, 'storage');
      expect(find.text('/srv/storm/vaults'), findsOneWidget);
      expect(find.textContaining('1 vault lives here'), findsOneWidget);
      await tapKey(tester, 'change-root');
      await tester.enterText(find.byKey(const Key('prompt-field')), '/data');
      await tapKey(tester, 'prompt-ok');
      expect(serverOf(c).vaultRoot, '/data');
      expect(find.text('/data'), findsOneWidget);
      await disposeShell(tester, c);
    });
  });

  group('Connection', () {
    testWidgets('address, route, identity and relays on the server', (
      tester,
    ) async {
      final c = shellContainer(
        settings: const Settings(
          baseUrl: 'http://storm.home:7420',
          accessToken: 'sta_test',
          // base64url of 0x4F 0x2A 0x91 …
          serverPublicKey: 'TyqRAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA',
          activeVault: FakeServer.primaryVault,
        ),
      );
      serverOf(c).relays = ['wss://relay.example.net'];
      await pumpShell(tester, c, size: desk);
      await open(tester, c, 'connection');
      expect(find.text('storm.home:7420'), findsOneWidget);
      expect(find.text('verified · 4F:2A:91…'), findsOneWidget);
      expect(find.text('relay.example.net'), findsOneWidget);

      await tapKey(tester, 'add-relay');
      await tester.enterText(
        find.byKey(const Key('prompt-field')),
        'relay.two.net',
      );
      await tapKey(tester, 'prompt-ok');
      expect(serverOf(c).relays, [
        'wss://relay.example.net',
        'wss://relay.two.net',
      ]);
      expect(find.text('relay.two.net'), findsOneWidget);

      await tapKey(tester, 'remove-relay-wss://relay.example.net');
      expect(serverOf(c).relays, ['wss://relay.two.net']);
      await disposeShell(tester, c);
    });

    testWidgets('a refused relay is reported, and nothing changes', (
      tester,
    ) async {
      final c = shellContainer();
      await pumpShell(tester, c, size: phone);
      await open(tester, c, 'connection');
      await tapKey(tester, 'add-relay');
      await tester.enterText(
        find.byKey(const Key('prompt-field')),
        'http://nope',
      );
      await tapKey(tester, 'prompt-ok');
      expect(serverOf(c).relays, isEmpty);
      expect(find.textContaining('wss://'), findsWidgets);
      await disposeShell(tester, c);
    });

    testWidgets('disconnect asks, then forgets the server', (tester) async {
      final c = shellContainer();
      await pumpShell(tester, c, size: desk);
      await open(tester, c, 'connection');
      await tapKey(tester, 'disconnect');
      await tapKey(tester, 'confirm-disconnect');
      expect(c.read(settingsProvider).value!.baseUrl, isEmpty);
      await disposeShell(tester, c);
    });
  });

  group('Advanced', () {
    testWidgets('an older server: endpoint and the client version only', (
      tester,
    ) async {
      final c = shellContainer();
      await pumpShell(tester, c, size: desk);
      await open(tester, c, 'advanced');
      expect(find.text('test/mcp'), findsOneWidget);
      expect(find.text('0.0.0-test'), findsOneWidget);
      await disposeShell(tester, c);
    });

    testWidgets('a server that reports its version shows both', (tester) async {
      final c = shellContainer();
      serverOf(c).version = '0.4.0';
      await pumpShell(tester, c, size: desk);
      await open(tester, c, 'advanced');
      expect(find.text('0.0.0-test · 0.4.0'), findsOneWidget);
      await disposeShell(tester, c);
    });

    testWidgets('Re-pair opens the existing pairing flow', (tester) async {
      final c = shellContainer();
      await pumpShell(tester, c, size: phone);
      await open(tester, c, 'advanced');
      await tapKey(tester, 're-pair');
      expect(find.byType(PairingScreen), findsOneWidget);
      await tapKey(tester, 're-pair-cancel');
      expect(find.byType(PairingScreen), findsNothing);
      expect(location(c), Routes.settingsPage('advanced'));
      await disposeShell(tester, c);
    });
  });

  group('About & health', () {
    testWidgets('sync row first, compatibility when the server says', (
      tester,
    ) async {
      final c = shellContainer();
      serverOf(c).version = '0.0.9';
      await pumpShell(tester, c, size: desk);
      await open(tester, c, 'health');
      expect(find.text('Sync now'), findsOneWidget);
      expect(
        find.text('This client and the server are compatible'),
        findsOneWidget,
      );
      final footer = tester.widget<Text>(find.byKey(const Key('about-footer')));
      expect(footer.data, startsWith('App '));
      expect(footer.data, endsWith(' · Server 0.0.9 · test'));
      await disposeShell(tester, c);
    });

    testWidgets('a different minor version reads as maybe incompatible', (
      tester,
    ) async {
      final c = shellContainer();
      serverOf(c).version = '0.4.0';
      await pumpShell(tester, c, size: phone);
      await open(tester, c, 'health');
      expect(find.textContaining('may not be compatible'), findsOneWidget);
      // B-7: it says which versions it compared.
      expect(find.textContaining('the server (0.4.0)'), findsOneWidget);
      await disposeShell(tester, c);
    });

    testWidgets('an older server shows no compatibility row', (tester) async {
      final c = shellContainer();
      await pumpShell(tester, c, size: desk);
      await open(tester, c, 'health');
      expect(find.textContaining('compatible'), findsNothing);
      await disposeShell(tester, c);
    });
  });
}
