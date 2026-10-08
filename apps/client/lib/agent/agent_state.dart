import 'dart:async';

import 'package:flutter/widgets.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_riverpod/legacy.dart';
import 'package:go_router/go_router.dart';
import 'package:shared_preferences/shared_preferences.dart';

import '../api/models.dart';
import '../router.dart';
import '../state/app_state.dart';
import 'agent_api.dart';
import 'agent_models.dart';
import 'session_controller.dart';
import 'terminal_events.dart';

/// How a session's terminal stream is opened, when a test supplies one.
final terminalStreamFactoryProvider =
    Provider<Stream<TerminalEvent> Function(String sessionId, int offset)?>(
      (ref) => null,
    );

/// The signed-in session's address and token, or null without a session:
/// what every owner-only REST client here is built from.
///
/// Watches only those two fields. Watching the whole settings object re-ran
/// everything downstream on any save — a theme change, a vault switch, every
/// tick of the font-size slider — and each re-run is an owner check over the
/// network.
final sessionCredentialsProvider = Provider<(String, String)?>(
  (ref) => ref.watch(
    settingsProvider.select((a) {
      final s = a.value;
      return s == null || !s.hasSession ? null : (s.baseUrl, s.accessToken);
    }),
  ),
);

/// How this client builds an [AgentApi], or null without a session. Tests
/// override it.
final agentApiFactoryProvider = Provider<AgentApi Function()?>((ref) {
  final session = ref.watch(sessionCredentialsProvider);
  if (session == null) return null;
  final (baseUrl, token) = session;
  return () => AgentApi(baseUrl: baseUrl, token: token);
});

/// An [AgentApi] for the signed-in session, or null without one.
AgentApi? agentApi(WidgetRef ref) => ref.read(agentApiFactoryProvider)?.call();

/// Sessions and hosts, as the rail, the sidebar and the Agents screens show
/// them.
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

  AgentSession? byId(String id) =>
      sessions.where((s) => s.id == id).firstOrNull;
}

/// How often a visible list re-asks the server.
///
/// An open session does not depend on this — its own stream carries every
/// status change — so this only has to keep a *list* from going stale.
const agentRefreshInterval = Duration(seconds: 15);

/// How often an open, live session re-reads its record, for its Wrote count.
const sessionRefreshInterval = Duration(seconds: 4);

/// Re-runs the provider after [every] while it is listened to and the app is
/// in front.
///
/// Two things stop the polling, and neither is `autoDispose` alone. Riverpod
/// pauses a provider whose only listeners are under `TickerMode(false)` — a
/// covered route — and a paused provider skips its refresh. And a tick that
/// lands while the app is backgrounded is skipped here, then made up the
/// moment the app resumes, rather than polling a phone in a pocket.
void _refreshEvery(Ref ref, Duration every) {
  var due = false;
  final timer = Timer(every, () {
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
}

/// The overview, refreshed every [agentRefreshInterval] while it is on screen
/// and the app is in front.
///
/// It never throws. Riverpod 3 retries a failed provider on its own
/// schedule, which would be a second, unbounded poll alongside this one.
final agentOverviewProvider = FutureProvider.autoDispose<AgentOverview>((
  ref,
) async {
  _refreshEvery(ref, agentRefreshInterval);
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

/// One session's stream, terminal and input, shared by whatever shows the
/// session — the desk split and the phone screen are the same route — and
/// closed when nothing does (plan §6.1).
final sessionControllerProvider = ChangeNotifierProvider.autoDispose
    .family<SessionController?, String>((ref, id) {
      final api = ref.watch(agentApiFactoryProvider)?.call();
      if (api == null) return null;
      final open = ref.watch(terminalStreamFactoryProvider);
      final c = SessionController(
        api: api,
        sessionId: id,
        open: open == null ? null : (offset) => open(id, offset),
      )..start();
      // The provider disposes the controller itself.
      ref.onDispose(api.dispose);
      return c;
    });

/// A session's record as the server has it now, re-read while it is live and
/// on screen; null when there is no such session or the server cannot be
/// asked. Its `wrote_count` is what [sessionWritesProvider] follows.
final sessionDetailProvider = FutureProvider.autoDispose
    .family<AgentSession?, String>((ref, id) async {
      final api = ref.watch(agentApiFactoryProvider)?.call();
      if (api == null) return null;
      try {
        final s = await api.session(id);
        if (!s.ended) _refreshEvery(ref, sessionRefreshInterval);
        return s;
      } catch (e) {
        if (!(e is StormApiException && e.statusCode == 404)) {
          _refreshEvery(ref, sessionRefreshInterval);
        }
        return null;
      } finally {
        api.dispose();
      }
    });

/// What a session wrote, newest first. Fetched again only when its record's
/// `wrote_count` moves, so a live session costs one small read per tick and
/// the list itself only when there is something new in it.
final sessionWritesProvider = FutureProvider.autoDispose
    .family<List<SessionWrite>?, String>((ref, id) async {
      await ref.watch(
        sessionDetailProvider(id).selectAsync((s) => s?.wroteCount),
      );
      final api = ref.watch(agentApiFactoryProvider)?.call();
      if (api == null) return null;
      try {
        return await api.writes(id);
      } catch (_) {
        return null;
      } finally {
        api.dispose();
      }
    });

/// Whether Settings › AI access lets a launch choose a write vault at all.
/// Null while unknown; the launcher then treats writes as off.
final agentWritesSettingProvider = FutureProvider.autoDispose<bool?>((
  ref,
) async {
  final api = ref.watch(agentApiFactoryProvider)?.call();
  if (api == null) return null;
  try {
    return await api.agentWrites();
  } catch (_) {
    return null;
  } finally {
    api.dispose();
  }
});

/// A note a session points at (its context, or one it wrote), read from the
/// server whichever vault is active. Null when it is gone or unreachable.
final agentNoteProvider = FutureProvider.autoDispose
    .family<Note?, ({String vaultId, String noteId})>((ref, at) async {
      final api = ref.watch(apiProvider);
      if (api == null) return null;
      try {
        return await api.note(at.vaultId, at.noteId);
      } catch (_) {
        return null;
      }
    });

/// The panel tabs of a session (`?tab=`).
enum SessionTab { context, wrote, about }

SessionTab sessionTabOf(String? raw) => SessionTab.values.firstWhere(
  (t) => t.name == raw,
  orElse: () => SessionTab.context,
);

/// Shows a session. A route, so it is linkable, remembered for launch and
/// backed out of like any other place (plan §6.1).
void openAgentSession(BuildContext context, String id, {SessionTab? tab}) =>
    context.go(Routes.agentSession(id, tab: tab?.name));

/// The host this device launched on last, so the launcher can preselect it.
///
/// Per device rather than per account: the laptop and the phone may well
/// prefer different machines.
const _lastHostKey = 'agent.lastHost';

Future<String?> lastLaunchHost() async =>
    (await SharedPreferences.getInstance()).getString(_lastHostKey);

Future<void> rememberLaunchHost(String hostId) async =>
    (await SharedPreferences.getInstance()).setString(_lastHostKey, hostId);
