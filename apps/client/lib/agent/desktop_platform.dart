import 'package:flutter/foundation.dart';

/// Whether this process can run Storm's desktop-only notification features
/// (decision 161, slice 161-4): keeping `/v1/agent/events` open while
/// hidden, OS notifications, and the macOS "keep running" toggle. `dart:io`
/// is deliberately not used here — it does not compile for the web target,
/// which this file is shared with.
bool get isDesktopPlatform =>
    !kIsWeb &&
    (defaultTargetPlatform == TargetPlatform.macOS ||
        defaultTargetPlatform == TargetPlatform.windows ||
        defaultTargetPlatform == TargetPlatform.linux);

/// The `platform` query param the server's device-prefs endpoints want
/// (161-4): `"macos"|"windows"|"linux"|"web"|"android"|"ios"`.
String currentPlatformName() {
  if (kIsWeb) return 'web';
  return switch (defaultTargetPlatform) {
    TargetPlatform.macOS => 'macos',
    TargetPlatform.windows => 'windows',
    TargetPlatform.linux => 'linux',
    TargetPlatform.android => 'android',
    TargetPlatform.iOS => 'ios',
    TargetPlatform.fuchsia => 'web',
  };
}
