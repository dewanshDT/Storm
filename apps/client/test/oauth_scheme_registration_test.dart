import 'dart:io';

import 'package:flutter_test/flutter_test.dart';

/// Android and macOS receive integration sign-ins as `storm://oauth` links
/// (spec §10.4, decision 81l). Both registrations are configuration, and
/// dropping either fails silently: the browser shows the redirect, nothing
/// opens, and the sign-in waits out its deadline. Run from `apps/client`.
void main() {
  test('Android routes storm://oauth to MainActivity, and only that host', () {
    final manifest = File(
      'android/app/src/main/AndroidManifest.xml',
    ).readAsStringSync();
    expect(
      manifest,
      matches(
        RegExp(r'<data\s+android:scheme="storm"\s+android:host="oauth"\s*/>'),
      ),
    );
    expect(manifest, contains('android.intent.category.BROWSABLE'));
    // Flutter's own deep linking would push the link into the router.
    expect(
      manifest,
      matches(
        RegExp(
          r'android:name="flutter_deeplinking_enabled"\s+android:value="false"',
        ),
      ),
    );
    final activity = File(
      'android/app/src/main/kotlin/dev/storm/storm/MainActivity.kt',
    ).readAsStringSync();
    expect(activity, contains('"storm/links"'));
    expect(activity, contains('"takeLinks"'));
  });

  test('macOS registers the storm scheme and forwards it to Dart', () {
    final plist = File('macos/Runner/Info.plist').readAsStringSync();
    expect(
      plist,
      matches(
        RegExp(
          r'<key>CFBundleURLSchemes</key>\s*<array>\s*<string>storm</string>',
        ),
      ),
    );
    final delegate = File('macos/Runner/AppDelegate.swift').readAsStringSync();
    expect(delegate, contains('open urls: [URL]'));
    expect(delegate, contains('"storm/links"'));
    final window = File(
      'macos/Runner/MainFlutterWindow.swift',
    ).readAsStringSync();
    expect(window, contains('StormLinks.shared.attach'));
  });
}
