/// Decision 161, slice 161-8: Android closed-app push via UnifiedPush,
/// split by platform the same way `web_notification_raw.dart` splits its
/// own DOM calls — `package:unifiedpush` imports `dart:io`, which does not
/// compile for the web target, so the real implementation lives in
/// `unified_push_io.dart` and `unified_push_stub.dart` gives web (and any
/// other platform with no `dart:io`) the harmless, always-"unavailable"
/// stand-in. Every real caller already gates on being Android first, so
/// the stub is never actually reached for a decision — only resolved so
/// `main.dart` and `notifications_page.dart` (both built for web too)
/// compile at all.
library;

export 'unified_push_io.dart'
    if (dart.library.js_interop) 'unified_push_stub.dart';
