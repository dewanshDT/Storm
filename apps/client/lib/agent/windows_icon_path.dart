/// `windowsIconPath()`, split by platform the same way `oauth_flow.dart`
/// splits its own `dart:io` use from the web build (see there).
library;

export 'windows_icon_path_io.dart'
    if (dart.library.js_interop) 'windows_icon_path_web.dart';
