import 'dart:io';

import 'package:flutter_test/flutter_test.dart';

/// The macOS app is sandboxed, so its entitlements are load-bearing.
///
/// This is here because the failure mode is silent. Drop
/// `network.client` and the app still builds, still launches, still shows the
/// connect screen — and then reports the server unreachable no matter what
/// address it is given, which reads as a wrong URL, a dead server or a bad
/// token. Nothing points at a missing entitlement.
///
/// Both files are stock Flutter template output, which is exactly why they
/// need a guard: an Xcode capabilities edit or a `flutter create` refresh
/// rewrites them, and the diff looks harmless.
void main() {
  /// Key → what stops working without it.
  const required = {
    'com.apple.security.network.client':
        'every HTTP request and the sync WebSocket',
    'com.apple.security.files.user-selected.read-only':
        'reading a file chosen in the attachment picker',
  };

  for (final name in const ['DebugProfile', 'Release']) {
    group('$name.entitlements', () {
      final file = File('macos/Runner/$name.entitlements');
      late String text;

      setUpAll(() {
        expect(
          file.existsSync(),
          isTrue,
          reason: 'no ${file.path} — run this from apps/client',
        );
        text = file.readAsStringSync();
      });

      // If this ever fails deliberately — someone turned the sandbox off — the
      // rest of this file is moot and should go with it. Until then the
      // sandbox is what makes the entitlements below necessary at all.
      test('is sandboxed', () {
        expect(text, _grants('com.apple.security.app-sandbox'));
      });

      for (final entry in required.entries) {
        test('grants ${entry.key}', () {
          expect(
            text,
            _grants(entry.key),
            reason: 'without it, ${entry.value} fails at the sandbox',
          );
        });
      }
    });
  }
}

/// Matches a key granted `<true/>`, ignoring the whitespace between them.
///
/// Deliberately not a plist parse: present-but-`<false/>` has to fail, and the
/// comments in these files explain *why* each key is there, so they are worth
/// keeping readable rather than round-tripping through a serializer.
Matcher _grants(String key) =>
    matches(RegExp('<key>${RegExp.escape(key)}</key>\\s*<true/>'));
