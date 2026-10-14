import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:storm/api/connection_log.dart';
import 'package:storm/api/lan_discovery.dart';
import 'package:storm/ui/lan_servers.dart';
import 'package:storm/ui/settings/relayed_on_lan.dart';
import 'package:storm/ui/theme.dart';

import 'lan_fakes.dart';

/// Settings › Connection, decision 119 Layer 3: relayed, yet the paired
/// server advertises on this network ⇒ say the firewall is the likely cause.
void main() {
  const sid = 'srv_PAIRED00000000000';
  const pk = 'A6EHv_POEL4dcN0Y50vAmWfk1jCbpQ1fHdyGZBJVMbg';

  DiscoveredServer advert(String serverId) => DiscoveredServer.fromAdvert(
    name: 'dew-omarchy',
    addresses: ['192.168.1.13'],
    port: 19484,
    txt: {
      'v': '1',
      'sid': serverId,
      'pk': pk,
      'ver': '0.0.0',
      'claimed': '1',
      'lan': '1',
    },
  )!;

  late FakeLanBrowser browser;

  Future<void> pump(
    WidgetTester tester, {
    required bool relayed,
    bool supported = true,
  }) async {
    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          lanDiscoverySupportedProvider.overrideWithValue(supported),
          lanBrowserFactoryProvider.overrideWithValue(() => browser),
        ],
        child: MaterialApp(
          theme: StormTheme.dark(),
          home: Scaffold(
            body: RelayedOnLanHint(relayed: relayed, serverId: sid),
          ),
        ),
      ),
    );
    await tester.pump();
  }

  setUp(() {
    browser = FakeLanBrowser();
    ConnectionLog.instance.clear();
  });

  testWidgets(
    'relayed while the server advertises here: the hint, and a log line',
    (tester) async {
      await pump(tester, relayed: true);
      expect(browser.browsing, isTrue);
      expect(find.byKey(const Key('relayed-on-lan')), findsNothing);

      browser.emit([advert('srv_SOMEONEELSE00000000')]);
      await tester.pump();
      await tester.pump();
      expect(
        find.byKey(const Key('relayed-on-lan')),
        findsNothing,
        reason: "another server on the network says nothing about this one",
      );

      browser.emit([advert(sid)]);
      await tester.pump();
      await tester.pump();
      expect(
        find.text(
          "Connected through a relay although you're on the same network as "
          "your server. Allow Storm in that computer's firewall for a faster, "
          'private connection.',
        ),
        findsOneWidget,
      );
      expect(
        ConnectionLog.instance.entries.map((e) => e.event),
        contains(
          'lan: server advertised at 192.168.1.13:19484 on this network, but '
          'the route is relayed (the direct probe failed)',
        ),
      );

      await tester.pumpWidget(const SizedBox());
      expect(browser.stops, 1, reason: 'browsing stops when the page goes');
    },
  );

  testWidgets('direct: no browsing and no hint', (tester) async {
    await pump(tester, relayed: false);
    expect(browser.starts, 0);
    expect(find.byKey(const Key('relayed-on-lan')), findsNothing);
  });

  testWidgets('where discovery is unsupported (web) it never browses', (
    tester,
  ) async {
    await pump(tester, relayed: true, supported: false);
    expect(browser.starts, 0);
    expect(find.byKey(const Key('relayed-on-lan')), findsNothing);
  });
}
