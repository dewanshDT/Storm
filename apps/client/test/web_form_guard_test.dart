import 'dart:io';

import 'package:flutter_test/flutter_test.dart';

/// The web build loads `form_guard.js` before Flutter (decision 90).
///
/// Without it, a native `form.submit()` on Flutter's hidden autofill form — a
/// password manager capturing the sign-in — POSTs the page, and the app reloads
/// mid sign-in: the server logs a success and the browser is back on /login.
/// Nothing in the Dart build would notice it gone; `make test-browser` proves
/// the behaviour in Firefox, and this keeps the wiring from slipping out.
void main() {
  test('index.html loads the guard, before Flutter starts', () {
    final index = File('web/index.html');
    expect(index.existsSync(), isTrue, reason: 'run this from apps/client');
    final html = index.readAsStringSync();
    final guard = html.indexOf('<script src="form_guard.js"></script>');
    final flutter = html.indexOf('<script src="flutter_bootstrap.js"');
    expect(guard, isNonNegative, reason: 'form_guard.js is not loaded');
    expect(guard, lessThan(flutter), reason: 'it must run before Flutter');
  });

  test(
    'the guard sends a native submit of a Flutter form through requestSubmit',
    () {
      final js = File('web/form_guard.js').readAsStringSync();
      expect(js, contains('HTMLFormElement.prototype.submit'));
      expect(js, contains("closest('flutter-view')"));
      expect(js, contains('requestSubmit()'));
    },
  );
}
