/// Platform pieces the connection needs from the network stack, and the
/// §4.3 plaintext-relay predicate shared by every route.
library;

import 'package:http/http.dart' as http;
import 'package:web_socket_channel/web_socket_channel.dart';

import 'net_io.dart' if (dart.library.js_interop) 'net_web.dart' as platform;

/// `T_connect` (§8.8): a private IPv4 literal on IPv6-only cellular otherwise
/// waits out the OS SYN timeout.
const kDirectConnectTimeout = Duration(seconds: 10);

/// A client for the direct candidate whose connect phase is bounded.
http.Client directHttpClient() => platform.directHttpClient();

/// The direct change feed, pinged every 30 s where the platform can (§8.7).
WebSocketChannel connectFeed(Uri uri) => platform.connectFeed(uri);

/// The host answered and nothing listened: the server is down, the host up.
bool isConnectionRefused(Object e) => platform.isConnectionRefused(e);

/// A failure of the connection itself, not of one exchange on it (§8.4).
bool isConnectionLevel(Object e) => platform.isConnectionLevel(e);

/// Whether a `ws://` relay on [host] may be used (§4.3, decision 114e as
/// amended): loopback, RFC 1918, link-local or ULA only. `100.64.0.0/10` is
/// carrier space and does not qualify. A hostname other than `localhost`
/// cannot be checked without resolving it, so it is public.
bool plaintextRelayAllowed(String host) {
  var h = host.toLowerCase();
  if (h.startsWith('[') && h.endsWith(']')) h = h.substring(1, h.length - 1);
  if (h == 'localhost') return true;
  try {
    final b = Uri.parseIPv4Address(h);
    return b[0] == 127 ||
        b[0] == 10 ||
        (b[0] == 172 && b[1] >= 16 && b[1] <= 31) ||
        (b[0] == 192 && b[1] == 168) ||
        (b[0] == 169 && b[1] == 254);
  } on FormatException {
    // Not IPv4.
  }
  try {
    final b = Uri.parseIPv6Address(h);
    final loopback = b.take(15).every((x) => x == 0) && b[15] == 1;
    return loopback ||
        (b[0] & 0xfe) == 0xfc ||
        (b[0] == 0xfe && (b[1] & 0xc0) == 0x80);
  } on FormatException {
    return false;
  }
}

/// Whether a relay URL may be dialled at all: `wss://` anywhere, `ws://` only
/// per [plaintextRelayAllowed].
bool relayUrlAllowed(String url) {
  final u = Uri.tryParse(url);
  if (u == null || u.host.isEmpty) return false;
  return switch (u.scheme) {
    'wss' => true,
    'ws' => plaintextRelayAllowed(u.host),
    _ => false,
  };
}
