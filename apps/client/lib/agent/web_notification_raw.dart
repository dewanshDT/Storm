/// The web notifications round's raw browser calls, split by platform the
/// same way `windows_icon_path.dart` splits its own `dart:js_interop` use
/// from the native build — `package:web` does not compile outside the web
/// target, so every actual DOM call lives in `_web.dart` and `_io.dart`
/// gives the harmless native stand-in (never reached: every caller already
/// gates on `kIsWeb` first).
library;

export 'web_notification_raw_io.dart'
    if (dart.library.js_interop) 'web_notification_raw_web.dart';
