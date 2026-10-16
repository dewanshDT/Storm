import 'dart:io';

import 'package:flutter_test/flutter_test.dart';

/// The Linux runner is `flutter create` output with a few load-bearing edits
/// (decision 125). A template refresh rewrites these files and the diff looks
/// harmless, so each edit has a guard.
void main() {
  String read(String path) {
    final file = File(path);
    expect(
      file.existsSync(),
      isTrue,
      reason: 'no $path — run this from apps/client',
    );
    return file.readAsStringSync();
  }

  late String app;
  setUpAll(() => app = read('linux/runner/my_application.cc'));

  // path_provider_linux names the app's data directory after the application
  // id, and the drift cache and prefs live in it. The installed .desktop
  // file's StartupWMClass is the same id.
  test('the application id is dev.storm.storm', () {
    expect(
      read('linux/CMakeLists.txt'),
      contains('set(APPLICATION_ID "dev.storm.storm")'),
    );
  });

  // Two instances would open one drift cache and replay one outbox
  // (decision 125b). The template's NON_UNIQUE turns GApplication's D-Bus
  // uniqueness off.
  test('the application is unique', () {
    expect(app, isNot(contains('G_APPLICATION_NON_UNIQUE')));
    expect(app, contains('G_APPLICATION_DEFAULT_FLAGS'));
  });

  // With uniqueness on, a second launch is another activate in this process.
  // Building a window there would start a second engine anyway.
  test('activate raises the existing window before building one', () {
    final raise = app.indexOf('gtk_window_present');
    final build = app.indexOf('gtk_application_window_new');
    expect(raise, isNonNegative);
    expect(build, isNonNegative);
    expect(raise, lessThan(build));
  });

  test('the window is titled Storm and carries the bundled icon', () {
    expect(app, contains('gtk_window_set_title(window, "Storm")'));
    expect(app, contains('"storm_icon.png"'));
    expect(
      read('linux/CMakeLists.txt'),
      contains('assets/icon/storm_icon.png'),
    );
  });

  // Without it a pasted image falls back to text (D15 AM45, decision 125c).
  test('the runner answers storm/clipboard', () {
    expect(
      read('linux/runner/storm_clipboard.cc'),
      contains('"storm/clipboard"'),
    );
    expect(app, contains('storm_clipboard_attach(view)'));
    expect(
      read('linux/runner/CMakeLists.txt'),
      contains('"storm_clipboard.cc"'),
    );
  });
}
