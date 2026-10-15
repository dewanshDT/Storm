import 'dart:convert';

import 'package:flutter/foundation.dart';

import '../../state/app_state.dart';

/// The environment variable `storm-runtime mcp-connect` reads its key from.
///
/// The key goes in the environment, never in argv: arguments are visible to
/// every other user on the machine through `ps`.
const mcpKeyEnv = 'STORM_MCP_KEY';

/// What `storm-runtime mcp-connect` needs to reach this server and to check it
/// is this server (decision 105e).
@immutable
class McpConnectTarget {
  const McpConnectTarget({
    required this.serverId,
    required this.publicKey,
    required this.addr,
    this.relays = const [],
  });

  final String serverId;

  /// The pinned Ed25519 key, base64url — what mcp-connect verifies the server
  /// against before it sends the key anywhere.
  final String publicKey;

  /// The server's own address, never a relay's: mcp-connect tries it first.
  final String addr;

  /// The relays the server registered with, tried when [addr] is unreachable.
  final List<String> relays;

  /// The target for the server this device is paired with, or null when it has
  /// no pinned identity: mcp-connect cannot run without one, and a key taken
  /// from an unverified answer would defeat its identity check.
  ///
  /// `baseUrl` is the address from pairing, whatever route the device is using
  /// now; the relayed route lives only in the connection.
  static McpConnectTarget? fromSettings(Settings s) {
    if (s.serverId.isEmpty || s.serverPublicKey.isEmpty || s.baseUrl.isEmpty) {
      return null;
    }
    return McpConnectTarget(
      serverId: s.serverId,
      publicKey: s.serverPublicKey,
      addr: s.baseUrl,
      relays: [for (final r in s.relays) r.url],
    );
  }

  List<List<String>> get _identityArgs => [
    ['mcp-connect'],
    _flag('--server', serverId),
    _flag('--pk', publicKey),
  ];

  List<List<String>> get _routeArgs => [
    _flag('--addr', addr),
    for (final r in relays) _flag('--relay', r),
  ];

  /// The `storm-runtime` arguments, without the key.
  List<String> get args => [
    for (final a in [..._identityArgs, ..._routeArgs]) ...a,
  ];

  /// The line to paste into a terminal for Claude Code.
  String claudeCommand(String secret) {
    String words(List<List<String>> groups) =>
        [for (final g in groups) ...g].map(shellQuote).join(' ');
    return 'claude mcp add storm -e ${shellQuote('$mcpKeyEnv=$secret')} -- \\\n'
        '  storm-runtime ${words(_identityArgs)} \\\n'
        '    ${words(_routeArgs)}';
  }

  /// The entry for clients configured by JSON (Claude Desktop, Cursor, …).
  String stdioConfig(String secret) =>
      const JsonEncoder.withIndent('  ').convert({
        'mcpServers': {
          'storm': {
            'command': 'storm-runtime',
            'args': args,
            'env': {mcpKeyEnv: secret},
          },
        },
      });
}

/// `--pk -abc` reads as two flags to most parsers; a base64url key starts with
/// `-` one time in 64.
List<String> _flag(String name, String value) =>
    value.startsWith('-') ? ['$name=$value'] : [name, value];

/// The raw HTTP entry: no adapter needed, but the LAN only, and no check that
/// the server is the one this device pinned.
String httpMcpConfig(String baseUrl, String secret) =>
    const JsonEncoder.withIndent('  ').convert({
      'mcpServers': {
        'storm': {
          'type': 'http',
          'url': '$baseUrl/mcp',
          'headers': {'Authorization': 'Bearer $secret'},
        },
      },
    });

final _shellSafe = RegExp(r'^[A-Za-z0-9_@%+=:,./-]+$');

/// [s] as one POSIX shell word.
String shellQuote(String s) {
  if (_shellSafe.hasMatch(s)) return s;
  return "'${s.replaceAll("'", r"'\''")}'";
}
