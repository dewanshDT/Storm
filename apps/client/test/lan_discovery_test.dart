import 'package:flutter/foundation.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:storm/api/lan_discovery.dart';
import 'package:storm/ui/lan_servers.dart' show lanDiscoverySupportedOn;

/// Reading `_storm._tcp` advertisements into the list's rows (decision 118).
void main() {
  const pk = 'A6EHv_POEL4dcN0Y50vAmWfk1jCbpQ1fHdyGZBJVMbg';
  const sid = 'srv_01ARZ3NDEKTSV4RRFFQ69G5FAV';

  Map<String, String> txt({
    String v = '1',
    String? sidValue = sid,
    String? pkValue = pk,
    String ver = '0.1.0',
    String claimed = '1',
    String lan = '1',
  }) => {
    'v': v,
    'sid': ?sidValue,
    'pk': ?pkValue,
    'ver': ver,
    'claimed': claimed,
    'lan': lan,
  };

  DiscoveredServer? read(
    Map<String, String> t, {
    List<String> addresses = const ['192.168.1.13'],
  }) => DiscoveredServer.fromAdvert(
    name: "Dewansh's Storm",
    addresses: addresses,
    port: 8484,
    txt: t,
  );

  group('an advertisement', () {
    test('becomes a server with its fingerprint and direct address', () {
      final s = read(txt())!;
      expect(s.name, "Dewansh's Storm");
      expect(s.baseUrl, 'http://192.168.1.13:8484');
      expect(s.serverId, sid);
      expect(s.publicKey, pk);
      expect(s.version, '0.1.0');
      expect(s.fingerprint, '5647 5AA7 5463 474C');
      expect(s.claimed, isTrue);
      expect(s.lanPairing, isTrue);
    });

    test('with an unknown version is ignored', () {
      expect(read(txt(v: '2')), isNull);
      expect(read(txt(v: '')), isNull);
    });

    test('without a usable id or key is ignored', () {
      expect(read(txt(sidValue: null)), isNull);
      expect(read(txt(pkValue: null)), isNull);
      expect(read(txt(sidValue: 'srv:colon')), isNull);
      // Padded, and the standard alphabet: one key has one spelling.
      expect(read(txt(pkValue: '$pk=')), isNull);
      expect(read(txt(pkValue: pk.replaceAll('_', '/'))), isNull);
      expect(read(txt(pkValue: 'AAAA')), isNull);
    });

    test('is dialled on a LAN address, IPv4 first, never a public one', () {
      expect(
        read(
          txt(),
          addresses: ['fe80::1%wlan0', 'fd00::5', '192.168.1.40'],
        )!.host,
        '192.168.1.40',
      );
      expect(
        read(txt(), addresses: ['fe80::1', 'fd00::5'])!.baseUrl,
        'http://[fd00::5]:8484',
      );
      expect(read(txt(), addresses: ['8.8.8.8', '2606:4700::1111']), isNull);
      expect(read(txt(), addresses: const []), isNull);
      // 100.64.0.0/10 is not LAN-scoped; the server refuses it too.
      expect(pickLanAddress(['100.80.1.2']), isNull);
      expect(pickLanAddress(['100.80.1.2', '10.0.0.4']), '10.0.0.4');
    });
  });

  group('list states from TXT', () {
    LanServerState stateOf(
      Map<String, String> t, {
      String paired = '',
      bool compatible = true,
    }) => lanServerState(
      read(t)!,
      pairedServerId: paired,
      compatible: compatible,
    );

    test('available', () {
      expect(stateOf(txt()), LanServerState.available);
    });
    test('needs setup when claimed=0', () {
      expect(stateOf(txt(claimed: '0')), LanServerState.needsSetup);
    });
    test('pairing off when lan=0', () {
      expect(stateOf(txt(lan: '0')), LanServerState.pairingOff);
    });
    test('a version mismatch is a warning, not a block', () {
      expect(
        stateOf(txt(), compatible: false),
        LanServerState.differentVersion,
      );
    });
    test('already paired wins over everything', () {
      expect(
        stateOf(
          txt(claimed: '0', lan: '0'),
          paired: sid,
        ),
        LanServerState.alreadyPaired,
      );
      expect(stateOf(txt(), paired: 'srv_other'), LanServerState.available);
    });
  });

  test('the web never browses, whatever the platform', () {
    for (final p in TargetPlatform.values) {
      expect(lanDiscoverySupportedOn(web: true, platform: p), isFalse);
    }
    expect(
      lanDiscoverySupportedOn(web: false, platform: TargetPlatform.android),
      isTrue,
    );
    expect(
      lanDiscoverySupportedOn(web: false, platform: TargetPlatform.macOS),
      isTrue,
    );
    expect(
      lanDiscoverySupportedOn(web: false, platform: TargetPlatform.linux),
      isTrue,
    );
  });

  test('a fingerprint is four groups of four uppercase hex', () {
    expect(
      keyFingerprint(pk),
      matches(RegExp(r'^([0-9A-F]{4} ){3}[0-9A-F]{4}$')),
    );
    expect(keyFingerprint(''), isNull);
  });
}
