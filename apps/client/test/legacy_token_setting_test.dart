import 'package:flutter_test/flutter_test.dart';

import 'package:storm/api/models.dart';

/// The client half of the A10 migration switch.
///
/// The only thing here that can bite is the default direction, so that is what
/// this pins. Everything else about the switch is server-side and tested there.
void main() {
  group('ServerConfig.legacyTokenEnabled', () {
    test('reads what the server sent', () {
      expect(
        ServerConfig.fromJson({
          'legacy_token_enabled': false,
        }).legacyTokenEnabled,
        isFalse,
      );
      expect(
        ServerConfig.fromJson({
          'legacy_token_enabled': true,
        }).legacyTokenEnabled,
        isTrue,
      );
    });

    test('an older server without the field reads as ON', () {
      // **The opposite of `mcp_enabled` beside it, and deliberately.** A server
      // old enough not to send this key is one where the shared token is the
      // only way in. Reading it as off would tell someone they had already
      // migrated when they had not, and invite them to retire a credential the
      // server has no replacement for.
      expect(ServerConfig.fromJson({}).legacyTokenEnabled, isTrue);
    });

    test('the MCP flags still default off, which is the other direction', () {
      // Guards against someone "tidying up" the three defaults to match.
      final c = ServerConfig.fromJson({});
      expect(c.mcpEnabled, isFalse);
      expect(c.mcpWritable, isFalse);
      expect(c.legacyTokenEnabled, isTrue);
    });
  });
}
