import 'dart:async';

import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_riverpod/legacy.dart';
import 'package:shared_preferences/shared_preferences.dart';

import '../state/app_state.dart';
import 'agent_api.dart';
import 'agent_models.dart';
import 'terminal_events.dart';

/// How a session's terminal stream is opened, when a test supplies one.
final terminalStreamFactoryProvider =
    Provider<Stream<TerminalEvent> Function(String sessionId, int offset)?>(
      (ref) => null,
    );

/// How an [AgentApi] is built, when a test supplies one.
final agentApiFactoryProvider = Provider<AgentApi Function()?>((ref) => null);

/// An [AgentApi] for the signed-in session, or null without one.
AgentApi? agentApi(WidgetRef ref) => _agentApi(
  ref.read(agentApiFactoryProvider),
  ref.read(settingsProvider).value,
);

/// The one place an [AgentApi] is built, for widgets and providers alike.
AgentApi? _agentApi(AgentApi Function()? factory, Settings? settings) {
  if (factory != null) return factory();
  if (settings == null || !settings.hasSession) return null;
  return AgentApi(baseUrl: settings.baseUrl, token: settings.accessToken);
}

/// The same, for a provider: waits for settings only when it needs them, so
/// a supplied factory does not hang on a settings load it never uses.
Future<AgentApi?> _agentApiFor(Ref ref) async {
  final factory = ref.read(agentApiFactoryProvider);
  if (factory != null) return factory();
  return _agentApi(null, await ref.watch(settingsProvider.future));
}

/// Whether to show agents at all. **The server's owner check is the
/// answer**: a 403 hides every entry point, so "hidden for non-owners"
/// (AC-S1) cannot drift from what the server enforces (decision 77d).
///
/// Every agent entry point reads this — the dashboard band, the sidebar's
/// space switch, Server settings, and the router's guard (decision 78).
final agentAccessProvider = FutureProvider<bool>((ref) async {
  final api = await _agentApiFor(ref);
  if (api == null) return false;
  try {
    return await api.canUseAgents();
  } catch (_) {
    return false;
  } finally {
    api.dispose();
  }
});

/// Sessions and hosts, as the band, the sidebar and the Agents screen show
/// them (decision 78).
class AgentOverview {
  const AgentOverview({
    required this.sessions,
    required this.hosts,
    this.unreachable = false,
  });

  /// The server could not be asked. **A state, not an error** — Storm's
  /// ground rule for offline — so it is a value the UI draws calmly rather
  /// than an exception it has to catch.
  const AgentOverview.unreachable()
    : sessions = const [],
      hosts = const [],
      unreachable = true;

  /// Newest first.
  final List<AgentSession> sessions;
  final List<AgentHost> hosts;
  final bool unreachable;

  List<AgentSession> get live => [
    for (final s in sessions)
      if (!s.ended) s,
  ];

  List<AgentSession> get ended => [
    for (final s in sessions)
      if (s.ended) s,
  ];

  List<AgentHost> get online => [
    for (final h in hosts)
      if (h.online) h,
  ];

  String hostName(String id) => hosts
      .firstWhere(
        (h) => h.id == id,
        orElse: () => AgentHost(
          id: id,
          name: 'host',
          status: 'offline',
          providers: const [],
          workspaces: const [],
          maxSessions: 0,
          lastSeen: null,
        ),
      )
      .name;
}

/// How often a visible list re-asks the server.
///
/// An open session does not depend on this — its own stream carries every
/// status change — so this only has to keep a *list* from going stale.
const agentRefreshInterval = Duration(seconds: 15);

/// The overview, refreshed every [agentRefreshInterval] while anything is
/// watching it, and not at all once nothing is.
///
/// `autoDispose` is what stops the polling: the timer belongs to this
/// computation, so leaving the screen disposes the provider and cancels it.
/// It never throws. Riverpod 3 retries a failed provider on its own
/// schedule, which would be a second, unbounded poll alongside this one.
final agentOverviewProvider = FutureProvider.autoDispose<AgentOverview>((
  ref,
) async {
  final timer = Timer(agentRefreshInterval, ref.invalidateSelf);
  ref.onDispose(timer.cancel);

  final api = await _agentApiFor(ref);
  if (api == null) return const AgentOverview.unreachable();
  try {
    final sessions = await api.sessions();
    final hosts = await api.hosts();
    return AgentOverview(sessions: sessions.reversed.toList(), hosts: hosts);
  } catch (_) {
    return const AgentOverview.unreachable();
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

/// The host this device launched on last, so the launcher can preselect it.
///
/// Per device rather than per account: the laptop and the phone may well
/// prefer different machines.
const _lastHostKey = 'agent.lastHost';

Future<String?> lastLaunchHost() async =>
    (await SharedPreferences.getInstance()).getString(_lastHostKey);

Future<void> rememberLaunchHost(String hostId) async =>
    (await SharedPreferences.getInstance()).setString(_lastHostKey, hostId);
