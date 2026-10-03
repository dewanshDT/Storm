import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_riverpod/legacy.dart';
import 'package:shared_preferences/shared_preferences.dart';

import '../state/app_state.dart';
import 'agent_api.dart';
import 'terminal_events.dart';

/// How a session's terminal stream is opened, when a test supplies one.
final terminalStreamFactoryProvider =
    Provider<Stream<TerminalEvent> Function(String sessionId, int offset)?>(
      (ref) => null,
    );

/// How an [AgentApi] is built, when a test supplies one.
final agentApiFactoryProvider = Provider<AgentApi Function()?>((ref) => null);

/// An [AgentApi] for the signed-in session, or null without one.
AgentApi? agentApi(WidgetRef ref) {
  final factory = ref.read(agentApiFactoryProvider);
  if (factory != null) return factory();
  final settings = ref.read(settingsProvider).value;
  if (settings == null || !settings.hasSession) return null;
  return AgentApi(baseUrl: settings.baseUrl, token: settings.accessToken);
}

/// Whether to show agents at all. **The server's owner check is the
/// answer**: a 403 hides every entry point, so "hidden for non-owners"
/// (AC-S1) cannot drift from what the server enforces (decision 77d).
final agentAccessProvider = FutureProvider<bool>((ref) async {
  final settings = await ref.watch(settingsProvider.future);
  if (!settings.hasSession) return false;
  final api = AgentApi(baseUrl: settings.baseUrl, token: settings.accessToken);
  try {
    return await api.canUseAgents();
  } catch (_) {
    return false;
  } finally {
    api.dispose();
  }
});

/// The open tabs on this device: session ids, in order (freeze §10).
///
/// Per device, persisted, and reconciled with the server's list so a
/// dismissed session drops off rather than haunting the strip.
class AgentTabs extends Notifier<List<String>> {
  static const _key = 'agent.tabs';

  @override
  List<String> build() {
    _load();
    return const [];
  }

  Future<void> _load() async {
    final prefs = await SharedPreferences.getInstance();
    final saved = prefs.getStringList(_key) ?? const [];
    if (saved.isNotEmpty && state.isEmpty) state = saved;
  }

  Future<void> _save() async {
    final prefs = await SharedPreferences.getInstance();
    await prefs.setStringList(_key, state);
  }

  void open(String id) {
    if (state.contains(id)) return;
    state = [...state, id];
    _save();
  }

  void close(String id) {
    state = [
      for (final t in state)
        if (t != id) t,
    ];
    _save();
  }

  /// Keeps only tabs whose session still exists.
  void reconcile(Iterable<String> existing) {
    final live = existing.toSet();
    final kept = [
      for (final t in state)
        if (live.contains(t)) t,
    ];
    if (kept.length != state.length) {
      state = kept;
      _save();
    }
  }
}

final agentTabsProvider = NotifierProvider<AgentTabs, List<String>>(
  AgentTabs.new,
);

/// The tab in view on this device.
final activeAgentTabProvider = StateProvider<String?>((ref) => null);
