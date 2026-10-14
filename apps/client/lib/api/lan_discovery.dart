/// Finding storm-server on the local network (decision 118).
///
/// An advertisement is a hint and nothing more: anyone on the Wi-Fi can
/// publish a `_storm._tcp`. Its key is pinned for the identity challenge
/// (`lan_pairing.dart`), and nothing but that challenge is sent until it
/// verifies.
library;

import 'dart:async';

import 'package:bonsoir/bonsoir.dart';
import 'package:cryptography/dart.dart';
import 'package:flutter/foundation.dart';

import 'ed25519_verify.dart';

const String kStormServiceType = '_storm._tcp';

/// The TXT `v` this client reads. An entry with another is ignored.
const String kLanTxtVersion = '1';

/// `SHA-256(pubkey)`, first 8 bytes, as `6FEC DC12 9A0B 44E1`: the one
/// format a person compares, in every place a key is shown. Null for a key
/// that is not 32 base64url bytes.
String? keyFingerprint(String publicKeyB64) {
  final key = decodeBase64UrlNoPad(publicKeyB64);
  if (key == null || key.length != 32) return null;
  final digest = const DartSha256().hashSync(key).bytes;
  final hex = [
    for (final b in digest.take(8))
      b.toRadixString(16).padLeft(2, '0').toUpperCase(),
  ].join();
  return [for (var i = 0; i < 16; i += 4) hex.substring(i, i + 4)].join(' ');
}

/// One server as its advertisement describes it.
@immutable
class DiscoveredServer {
  const DiscoveredServer({
    required this.name,
    required this.host,
    required this.port,
    required this.serverId,
    required this.publicKey,
    required this.version,
    required this.claimed,
    required this.lanPairing,
  });

  final String name;

  /// The LAN address to dial, never a hostname: `.local` names do not
  /// resolve on Android.
  final String host;
  final int port;
  final String serverId;
  final String publicKey;

  /// The server's release version (`ver`).
  final String version;

  /// The account exists (`claimed=1`).
  final bool claimed;

  /// Password pairing is on (`lan=1`).
  final bool lanPairing;

  String get baseUrl =>
      host.contains(':') ? 'http://[$host]:$port' : 'http://$host:$port';

  /// What a row shows as the address.
  String get address => host;

  String get fingerprint => keyFingerprint(publicKey) ?? '';

  /// Reads an advertisement. Null when the entry is not one this client can
  /// use: an unknown `v`, a missing or malformed `sid` or `pk`, or no LAN
  /// address.
  static DiscoveredServer? fromAdvert({
    required String name,
    required List<String> addresses,
    required int port,
    required Map<String, String> txt,
  }) {
    if (txt['v'] != kLanTxtVersion) return null;
    final sid = txt['sid'] ?? '';
    final pk = txt['pk'] ?? '';
    if (!_validServerId(sid) || keyFingerprint(pk) == null) return null;
    final host = pickLanAddress(addresses);
    if (host == null || port <= 0 || port > 65535) return null;
    return DiscoveredServer(
      name: name.trim().isEmpty ? 'Storm' : name.trim(),
      host: host,
      port: port,
      serverId: sid,
      publicKey: pk,
      version: txt['ver'] ?? '',
      claimed: txt['claimed'] == '1',
      lanPairing: txt['lan'] == '1',
    );
  }

  @override
  bool operator ==(Object other) =>
      other is DiscoveredServer &&
      other.name == name &&
      other.host == host &&
      other.port == port &&
      other.serverId == serverId &&
      other.publicKey == publicKey &&
      other.version == version &&
      other.claimed == claimed &&
      other.lanPairing == lanPairing;

  @override
  int get hashCode => Object.hash(
    name,
    host,
    port,
    serverId,
    publicKey,
    version,
    claimed,
    lanPairing,
  );
}

/// The server's own `validate_server_id` rule, enough to keep a colon out of
/// the signed challenge.
bool _validServerId(String sid) =>
    sid.isNotEmpty &&
    sid.length <= 128 &&
    sid.codeUnits.every((c) => c > 0x20 && c < 0x7F && c != 0x3A && c != 0x22);

/// The address to pair over: a private IPv4 first, then a unique-local IPv6.
/// Never a public address, which the server would refuse anyway, and never a
/// link-local IPv6, which needs a zone the URL cannot carry.
String? pickLanAddress(List<String> addresses) {
  String? ula;
  for (final raw in addresses) {
    final a = raw.split('%').first.trim();
    final v4 = _ipv4(a);
    if (v4 != null) {
      if (_lanV4(v4)) return a;
      continue;
    }
    final lower = a.toLowerCase();
    if (lower.startsWith('fc') || lower.startsWith('fd')) ula ??= a;
  }
  return ula;
}

List<int>? _ipv4(String a) {
  final parts = a.split('.');
  if (parts.length != 4) return null;
  final out = <int>[];
  for (final p in parts) {
    final n = int.tryParse(p);
    if (n == null || n < 0 || n > 255) return null;
    out.add(n);
  }
  return out;
}

bool _lanV4(List<int> o) =>
    o[0] == 10 ||
    (o[0] == 172 && o[1] >= 16 && o[1] < 32) ||
    (o[0] == 192 && o[1] == 168) ||
    (o[0] == 169 && o[1] == 254) ||
    o[0] == 127;

/// The row states the locked UI names.
enum LanServerState {
  available,
  alreadyPaired,
  needsSetup,
  pairingOff,
  updateNeeded,
}

LanServerState lanServerState(
  DiscoveredServer server, {
  required String pairedServerId,
  required bool compatible,
}) {
  if (pairedServerId.isNotEmpty && pairedServerId == server.serverId) {
    return LanServerState.alreadyPaired;
  }
  if (!server.claimed) return LanServerState.needsSetup;
  if (!server.lanPairing) return LanServerState.pairingOff;
  if (!compatible) return LanServerState.updateNeeded;
  return LanServerState.available;
}

/// Browses `_storm._tcp`. One per visible pairing screen: [start] when it
/// appears, [stop] when it goes, so Android's multicast lock is held only
/// while the list is open.
abstract class LanBrowser {
  /// The servers found so far, re-emitted whole on every change.
  Stream<List<DiscoveredServer>> start();

  Future<void> stop();
}

class BonsoirLanBrowser implements LanBrowser {
  BonsoirDiscovery? _discovery;
  StreamSubscription<BonsoirDiscoveryEvent>? _events;
  final Map<String, DiscoveredServer> _found = {};
  StreamController<List<DiscoveredServer>>? _out;

  @override
  Stream<List<DiscoveredServer>> start() {
    final out = _out = StreamController<List<DiscoveredServer>>();
    unawaited(_begin(out));
    return out.stream;
  }

  Future<void> _begin(StreamController<List<DiscoveredServer>> out) async {
    try {
      final discovery = _discovery = BonsoirDiscovery(
        type: kStormServiceType,
        printLogs: false,
      );
      await discovery.initialize();
      _events = discovery.eventStream?.listen((event) {
        switch (event) {
          case BonsoirDiscoveryServiceFoundEvent(:final service):
            unawaited(service.resolve(discovery.serviceResolver));
          case BonsoirDiscoveryServiceResolvedEvent(:final service):
          case BonsoirDiscoveryServiceUpdatedEvent(:final service):
            final server = DiscoveredServer.fromAdvert(
              name: service.name,
              addresses: service.hostAddresses,
              port: service.port,
              txt: service.attributes,
            );
            if (server == null) {
              _found.remove(service.name);
            } else {
              _found[service.name] = server;
            }
            _emit();
          case BonsoirDiscoveryServiceLostEvent(:final service):
            if (_found.remove(service.name) != null) _emit();
          default:
            break;
        }
      });
      await discovery.start();
    } catch (e) {
      if (!out.isClosed) out.addError(e);
    }
  }

  void _emit() {
    final out = _out;
    if (out == null || out.isClosed) return;
    out.add(List.unmodifiable(_found.values));
  }

  @override
  Future<void> stop() async {
    await _events?.cancel();
    _events = null;
    final discovery = _discovery;
    _discovery = null;
    try {
      if (discovery != null && !discovery.isStopped) await discovery.stop();
    } catch (_) {}
    _found.clear();
    await _out?.close();
    _out = null;
  }
}
