/// How the client builds a [FeedbackApi], plus the bounded [ActionTrail] a
/// report's diagnostics bundle draws from (decision 162, slice 162-5).
library;

import 'package:flutter/foundation.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../agent/agent_state.dart' show sessionCredentialsProvider;
import '../api/feedback_api.dart';
import '../router.dart';
import 'app_state.dart' show sessionClientProvider;

/// How this client builds a [FeedbackApi], or null without a session. Tests
/// override it.
final feedbackApiFactoryProvider = Provider<FeedbackApi Function()?>((ref) {
  final session = ref.watch(sessionCredentialsProvider);
  if (session == null) return null;
  final (baseUrl, token) = session;
  final client = ref.watch(sessionClientProvider);
  return () => FeedbackApi(baseUrl: baseUrl, token: token, client: client);
});

/// A [FeedbackApi] for the signed-in session, or null without one.
FeedbackApi? feedbackApi(WidgetRef ref) =>
    ref.read(feedbackApiFactoryProvider)?.call();

/// The current screen and the last [limit] action labels — route/screen
/// changes and palette command ids, **never** a note title or path
/// (decision 162c). Fed from `AppShell._remember` and the palette's command
/// dispatch; read only when building a feedback report.
@immutable
class ActionTrail {
  const ActionTrail({this.actions = const [], this.screen = ''});

  static const limit = 20;

  /// Newest last, bounded at [limit].
  final List<String> actions;
  final String screen;

  ActionTrail _record(String label) {
    final kept = [...actions, label];
    final drop = kept.length - limit;
    return ActionTrail(
      actions: drop > 0 ? kept.sublist(drop) : kept,
      screen: screen,
    );
  }

  ActionTrail visitScreen(String label) {
    if (label == screen) return this;
    return ActionTrail(
      actions: _record('opened $label').actions,
      screen: label,
    );
  }

  ActionTrail recordCommand(String id) => _record('ran $id');
}

class ActionTrailNotifier extends Notifier<ActionTrail> {
  @override
  ActionTrail build() => const ActionTrail();

  void visitScreen(String label) => state = state.visitScreen(label);
  void recordCommand(String id) => state = state.recordCommand(id);
}

final actionTrailProvider = NotifierProvider<ActionTrailNotifier, ActionTrail>(
  ActionTrailNotifier.new,
);

/// A route's location as a content-free screen label — the route's shape
/// only, never a vault, note, session or crew id (decision 162c: "the
/// current screen" in a diagnostics bundle).
String screenLabelOf(Uri uri) {
  final seg = uri.pathSegments;
  if (seg.isNotEmpty && seg[0] == 'v') {
    if (seg.length >= 3) {
      return switch (seg[2]) {
        'note' => 'notes/note',
        'browse' => 'notes/browse',
        'tags' => 'notes/tags',
        'trash' => 'notes/trash',
        _ => 'notes',
      };
    }
    return 'notes';
  }
  if (uri.path == Routes.notes) return 'notes';
  if (seg.isNotEmpty && seg[0] == 'agents') {
    if (seg.length >= 2) {
      return switch (seg[1]) {
        'c' => 'agents/crew',
        's' => 'agents/session',
        _ => 'agents',
      };
    }
    return 'agents';
  }
  if (seg.isNotEmpty && seg[0] == 'settings') {
    return seg.length >= 2 ? 'settings/${seg[1]}' : 'settings';
  }
  if (uri.path == Routes.starting) return 'starting';
  if (uri.path == Routes.pairing) return 'pairing';
  if (uri.path == Routes.login) return 'login';
  if (uri.path == Routes.addDevice) return 'add-device';
  if (uri.path == Routes.gallery) return 'gallery';
  return 'other';
}
