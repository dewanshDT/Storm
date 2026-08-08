import 'dart:io';

import 'package:flutter_test/flutter_test.dart';

/// The Android app's manifest has to grant network access.
///
/// Sibling of `macos_entitlements_test.dart`, and here for the same reason: a
/// platform permission that is missing does not fail the build, does not fail
/// the install, and does not fail at launch. It fails on the first socket, as
/// "Couldn't reach the server" — which reads as a wrong address, a dead server
/// or a bad token, and sends you to look at all three.
///
/// This one has already happened. Flutter's template declares `INTERNET` in the
/// **debug** and **profile** manifests only, so hot reload can reach the host
/// machine. Every debug build therefore works, and the first release APK put on
/// a phone cannot open a socket at all — the kernel refuses it with `EPERM`
/// before anything of Storm's runs.
void main() {
  final main = File('android/app/src/main/AndroidManifest.xml');

  late String text;

  setUpAll(() {
    expect(
      main.existsSync(),
      isTrue,
      reason: 'no ${main.path} — run this from apps/client',
    );
    text = main.readAsStringSync();
  });

  test('the main manifest grants INTERNET', () {
    // The *main* manifest specifically: debug and profile have their own, and
    // finding it there is exactly the false comfort that let this ship.
    expect(
      text,
      contains('android.permission.INTERNET'),
      reason:
          'a release APK without it installs, launches, and can never reach '
          'the server',
    );
  });

  test('it is a uses-permission, not a mention in a comment', () {
    // The comment above the line names the permission too, so a `contains`
    // check alone would pass with the declaration deleted.
    expect(
      RegExp(
        r'<uses-permission\s+android:name="android\.permission\.INTERNET"\s*/>',
      ).hasMatch(text),
      isTrue,
      reason: 'the permission must actually be declared',
    );
  });
}
