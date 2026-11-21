/// The web (and non-`dart:io`) half of `unified_push.dart`'s conditional
/// export — see that file's doc comment. Nothing here is ever reached by
/// a real decision: [unifiedPushControllerProvider] is always `null` off
/// Android, so no code reads [UnifiedPushController] at all there, but
/// the *type* still has to exist for `main.dart` and
/// `ui/settings/notifications_page.dart` (both built for web) to compile
/// against one shared name.
library;

import 'package:flutter/foundation.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import 'notifications_models.dart'
    show AgentNotification, DeviceNotificationPrefs;

enum UnifiedPushStatus {
  off,
  enabling,
  on,
  distributorMissing,
  permissionDenied,
  error,
  unavailable,
}

class UnifiedPushController extends ChangeNotifier {
  UnifiedPushStatus get status => UnifiedPushStatus.unavailable;
  String? get distributorLabel => null;
  String? get errorMessage => null;

  Future<void> enable() async {}
  Future<void> disable() async {}
  Future<void> forceShow(
    AgentNotification n,
    DeviceNotificationPrefs prefs,
  ) async {}
  Future<bool> openSystemSettings() async => false;
}

Future<void> runUnifiedPushBackgroundEntrypoint() async {}

final unifiedPushControllerProvider =
    Provider.autoDispose<UnifiedPushController?>((ref) => null);
