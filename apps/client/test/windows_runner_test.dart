import 'dart:io';

import 'package:flutter_test/flutter_test.dart';

/// The Windows runner is `flutter create` output with a few load-bearing
/// edits (decision 125). A template refresh rewrites these files and the diff
/// looks harmless, so each edit has a guard.
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

  // path_provider_windows builds the app's data directory from these two
  // values, and the drift cache, the prefs and the secure store all live in
  // it. Renaming either strands every signed-in device's data.
  test('Runner.rc keeps the company and product that name the data dir', () {
    final rc = read('windows/runner/Runner.rc');
    expect(rc, contains('VALUE "CompanyName", "dev.storm"'));
    expect(rc, contains('VALUE "ProductName", "Storm"'));
  });

  // Two instances would open one drift cache and replay one outbox
  // (decision 125b). The mutex has to be taken before the engine starts.
  test('main.cpp refuses a second instance before the engine starts', () {
    final main = read('windows/runner/main.cpp');
    final guard = main.indexOf('single_instance.IsFirst()');
    final engine = main.indexOf('flutter::DartProject');
    expect(guard, isNonNegative);
    expect(engine, isNonNegative);
    expect(guard, lessThan(engine));
    expect(
      read('windows/runner/single_instance.cpp'),
      contains(r'L"Local\\dev.storm.storm"'),
    );
    expect(
      read('windows/runner/CMakeLists.txt'),
      contains('"single_instance.cpp"'),
    );
  });

  // A second launch finds the first by its window title.
  test('the window is titled Storm', () {
    expect(
      read('windows/runner/single_instance.h'),
      contains('kWindowTitle[] = L"Storm"'),
    );
    expect(
      read('windows/runner/main.cpp'),
      contains('window.Create(kWindowTitle'),
    );
  });

  // Without it a pasted image falls back to text (D15 AM45, decision 125c).
  test('the runner answers storm/clipboard', () {
    expect(
      read('windows/runner/storm_clipboard.cpp'),
      contains('"storm/clipboard"'),
    );
    expect(
      read('windows/runner/flutter_window.cpp'),
      contains('AttachStormClipboard('),
    );
    expect(
      read('windows/runner/CMakeLists.txt'),
      contains('"storm_clipboard.cpp"'),
    );
  });
}
