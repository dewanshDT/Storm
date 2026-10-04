import 'dart:async';

import 'package:flutter/widgets.dart';
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

/// How this client builds an [AgentApi]: from the signed-in session's
/// address and token, or null without a session. Tests override it.
///
/// Watches only those two fields. Watching the whole settings object re-ran
/// everything downstream on any save — a theme change, a vault switch, every
/// tick of the font-size slider — and each re-run is an owner check over the
/// network.
final agentApiFactoryProvider = Provider<AgentApi Function()?>((ref) {
  final session = ref.watch(
    settingsProvider.select((a) {
      final s = a.value;
      return s == null || !s.hasSession ? null : (s.baseUrl, s.accessToken);
    }),
  );
  if (session == null) return null;
  final (baseUrl, token) = session;
  return () => AgentApi(baseUrl: baseUrl, token: token);
});

/// An [AgentApi] for the signed-in session, or null without one.
AgentApi? agentApi(WidgetRef ref) => ref.read(agentApiFactoryProvider)?.call();

/// Whether to show agents at all. **The server's owner check is the
/// answer**: a 403 hides every entry point, so "hidden for non-owners"
/// (AC-S1) cannot drift from what the server enforces (decision 77d).
///
/// Every agent entry point reads this — the dashboard band, the sidebar's
/// space switch, Server settings, and the router's guard (decision 78).
final agentAccessProvider = FutureProvider<bool>((ref) async {
  final api = ref.watch(agentApiFactoryProvider)?.call();
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

  String hostName(String id) =>
      hosts.where((h) => h.id == id).firstOrNull?.name ?? 'host';
}

/// How often a visible list re-asks the server.
///
/// An open session does not depend on this — its own stream carries every
/// status change — so this only has to keep a *list* from going stale.
const agentRefreshInterval = Duration(seconds: 15);

/// The overview, refreshed every [agentRefreshInterval] while it is on screen
/// and the app is in front.
///
/// Two things stop the polling, and neither is `autoDispose` alone: the
/// dashboard sits under every phone route and is never disposed. Riverpod
/// pauses a provider whose only listeners are under `TickerMode(false)` — a
/// covered route — and a paused provider skips its refresh. And a tick that
/// lands while the app is backgrounded is skipped here, then made up the
/// moment the app resumes, rather than polling a phone in a pocket.
///
/// It never throws. Riverpod 3 retries a failed provider on its own
/// schedule, which would be a second, unbounded poll alongside this one.
final agentOverviewProvider = FutureProvider.autoDispose<AgentOverview>((
  ref,
) async {
  var due = false;
  final timer = Timer(agentRefreshInterval, () {
    final state = WidgetsBinding.instance.lifecycleState;
    if (state == null || state == AppLifecycleState.resumed) {
      ref.invalidateSelf();
    } else {
      due = true;
    }
  });
  final lifecycle = AppLifecycleListener(
    onResume: () {
      if (due) ref.invalidateSelf();
    },
  );
  ref.onDispose(() {
    timer.cancel();
    lifecycle.dispose();
  });

  final api = ref.watch(agentApiFactoryProvider)?.call();
  if (api == null) return const AgentOverview.unreachable();
  try {
    // Independent, so together: one round trip, not two, which is the
    // difference that shows over the relay.
    final (sessions, hosts) = await (api.sessions(), api.hosts()).wait;
    return AgentOverview(sessions: sessions.reversed.toList(), hosts: hosts);
  } catch (_) {
    return const AgentOverview.unreachable();
  } finally {
    api.dispose();
  }
});

/// Ask the server again now, rather than at the next tick.
Future<void> reloadAgents(WidgetRef ref) async {
  ref.invalidate(agentOverviewProvider);
  await ref.read(agentOverviewProvider.future);
}

/// Opens a session as this device's active tab. Where to *show* it is the
/// caller's business: the dashboard pushes the Agents space, the sidebar is
/// already beside it.
void openAgentSession(WidgetRef ref, String id) {
  ref.read(agentTabsProvider.notifier).open(id);
  ref.read(activeAgentTabProvider.notifier).state = id;
}

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

/// The names agents gave their sessions, by session id, as this device saw
/// them in a terminal title. Only sessions opened here are known; the rest
/// show their provider and workspace. (A host-reported title, so every
/// device knows every name, belongs with the host's terminal state, AM22.)
final agentTitlesProvider = StateProvider<Map<String, String>>((ref) => {});

/// The host this device launched on last, so the launcher can preselect it.
///
/// Per device rather than per account: the laptop and the phone may well
/// prefer different machines.
const _lastHostKey = 'agent.lastHost';

Future<String?> lastLaunchHost() async =>
    (await SharedPreferences.getInstance()).getString(_lastHostKey);

Future<void> rememberLaunchHost(String hostId) async =>
    (await SharedPreferences.getInstance()).setString(_lastHostKey, hostId);
