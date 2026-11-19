/// Decision 161, slice 161-4: the "This device" settings that are purely
/// local — sound, and macOS's "keep running when closed" — never synced
/// to the server (`device_notification_prefs` holds only what changes
/// what the server *sends*; see `notifications_page.dart`'s doc comment
/// on that split).
library;

import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

/// Writes macOS's "keep running when its window is closed" to the native
/// side (`AppDelegate.swift`'s `StormLifecycle`), which is what
/// `applicationShouldTerminateAfterLastWindowClosed` actually reads —
/// Dart's own copy in [ThisDeviceLocalPrefs] is for the toggle's display,
/// not the source of truth.
abstract final class MacosLifecycle {
  static const _channel = MethodChannel('storm/lifecycle');

  static Future<void> setKeepRunningWhenClosed(bool value) async {
    if (defaultTargetPlatform != TargetPlatform.macOS) return;
    try {
      await _channel.invokeMethod<void>('setKeepRunningWhenClosed', value);
    } catch (_) {
      // An older or stubbed native side (tests): the toggle still shows
      // what was asked for, which is all a best-effort write promises.
    }
  }
}

class ThisDeviceLocalPrefs extends ChangeNotifier {
  static const _kKeepRunning = 'storm.macosKeepRunningWhenClosed';
  static const _kSound = 'storm.notifications.soundOn';

  ThisDeviceLocalPrefs() {
    unawaited(_load());
  }

  bool keepRunningWhenClosed = false;

  /// On by default: a silent desktop notification is a notification
  /// nobody notices.
  bool soundOn = true;
  bool loaded = false;
  bool _disposed = false;

  Future<void> _load() async {
    try {
      final prefs = await SharedPreferences.getInstance();
      keepRunningWhenClosed = prefs.getBool(_kKeepRunning) ?? false;
      soundOn = prefs.getBool(_kSound) ?? true;
    } catch (_) {
      // Tests and previews have no preferences store; the defaults above
      // still work.
    }
    loaded = true;
    if (!_disposed) notifyListeners();
  }

  Future<void> setKeepRunningWhenClosed(bool value) async {
    keepRunningWhenClosed = value;
    if (!_disposed) notifyListeners();
    unawaited(MacosLifecycle.setKeepRunningWhenClosed(value));
    try {
      final prefs = await SharedPreferences.getInstance();
      await prefs.setBool(_kKeepRunning, value);
    } catch (_) {}
  }

  Future<void> setSoundOn(bool value) async {
    soundOn = value;
    if (!_disposed) notifyListeners();
    try {
      final prefs = await SharedPreferences.getInstance();
      await prefs.setBool(_kSound, value);
    } catch (_) {}
  }

  @override
  void dispose() {
    _disposed = true;
    super.dispose();
  }
}

/// One instance for the app's lifetime — not `autoDispose`: the setting
/// (and the native side it mirrors) must stay what was last chosen even
/// while Settings › Notifications is closed.
final thisDeviceLocalPrefsProvider = Provider<ThisDeviceLocalPrefs>((ref) {
  final p = ThisDeviceLocalPrefs();
  ref.onDispose(p.dispose);
  return p;
});
