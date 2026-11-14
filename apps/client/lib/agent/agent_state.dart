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
import 'agent_events.dart';
import 'agent_models.dart';
import 'session_controller.dart';
import 'session_keep_alive.dart';
import 'terminal_events.dart';
import 'terminal_stream.dart' show hiddenStreamGrace;

/// How a session's terminal stream is opened, when a test supplies one.
final terminalStreamFactoryProvider =
    Provider<
      Stream<TerminalEvent> Function(
        String sessionId,
        int offset,
        bool snapshot,
      )?
    >((ref) => null);

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
///
/// Addressed to the saved address but sent over the session's connection,
/// direct or relayed (decision 112). `baseUrl` stays the saved address because
/// it is also what an enrollment tells a runtime to dial.
final agentApiFactoryProvider = Provider<AgentApi Function()?>((ref) {
  final session = ref.watch(sessionCredentialsProvider);
  if (session == null) return null;
  final (baseUrl, token) = session;
  final client = ref.watch(sessionClientProvider);
  return () => AgentApi(baseUrl: baseUrl, token: token, client: client);
});

/// An [AgentApi] for the signed-in session, or null without one.
AgentApi? agentApi(WidgetRef ref) => ref.read(agentApiFactoryProvider)?.call();

/// Sessions, hosts and crews, as the rail, the sidebar and the Agents screens
/// show them.
class AgentOverview {
  const AgentOverview({
    required this.sessions,
    required this.hosts,
    this.crews,
    this.unreachable = false,
    this.pending = const {},
  });

  /// The server could not be asked. **A state, not an error** — Storm's
  /// ground rule for offline — so it is a value the UI draws calmly rather
  /// than an exception it has to catch.
  const AgentOverview.unreachable()
    : sessions = const [],
      hosts = const [],
      crews = null,
      unreachable = true,
      pending = const {};

  /// Newest first.
  final List<AgentSession> sessions;
  final List<AgentHost> hosts;

  /// In the owner's order; null until they have loaded, or from a server
  /// before decision 138. Nothing is drawn as a crew while null.
  final List<Crew>? crews;
  final bool unreachable;

  /// Sessions under a crew or label change this device made and the server
  /// has not answered yet (138j): their crew and role here are ahead of any
  /// other copy of them.
  final Set<String> pending;

  AgentOverview copyWith({
    List<AgentSession>? sessions,
    List<Crew>? crews,
    Set<String>? pending,
  }) => AgentOverview(
    sessions: sessions ?? this.sessions,
    hosts: hosts,
    crews: crews ?? this.crews,
    unreachable: unreachable,
    pending: pending ?? this.pending,
  );

  /// [s] with the crew and role this list has for it while a change of
  /// either is pending, so a session's own record shows the change too.
  AgentSession labelsOf(AgentSession s) {
    if (!pending.contains(s.id)) return s;
    final mine = byId(s.id);
    return mine == null ? s : s.withCrew(mine.crew).withRole(mine.role);
  }

  List<AgentSession> get live => [
    for (final s in sessions)
      if (!s.ended) s,
  ];

  Set<String> get _resumed => {for (final s in sessions) ?s.resumedFrom};

  /// Less any session a later one resumed: that conversation is listed
  /// once, under its latest session.
  List<AgentSession> get ended {
    final resumed = _resumed;
    return [
      for (final s in sessions)
        if (s.ended && !resumed.contains(s.id)) s,
    ];
  }

  /// The crew [s] is drawn in, if any.
  Crew? crewOf(AgentSession s) {
    final id = labelsOf(s).crew?.id;
    return id == null ? null : crews?.where((c) => c.id == id).firstOrNull;
  }

  /// [crew]'s members in their order, resumed predecessors left out as in
  /// [ended]. A crew holds whole trees (decision 145i): a child carries its
  /// root's place, and comes after it.
  List<AgentSession> membersOf(Crew crew) {
    final resumed = _resumed;
    return [
      for (final s in sessions)
        if (s.crew?.id == crew.id && !(s.ended && resumed.contains(s.id))) s,
    ]..sort((a, b) {
      final byOrder = a.crew!.order.compareTo(b.crew!.order);
      return byOrder != 0 ? byOrder : a.spawn.depth.compareTo(b.spawn.depth);
    });
  }

  /// The session a crew change naming [id] acts on: its tree's root, since a
  /// child moves only with the agent that started it (145i).
  String rootIdOf(String id) => byId(id)?.spawn.rootSession ?? id;

  /// [ids] as the roots a crew change acts on, each once, in order.
  List<String> rootIdsOf(Iterable<String> ids) =>
      {for (final id in ids) rootIdOf(id)}.toList();

  /// Sessions in no crew this overview knows: what RUNNING, ENDED and NOT IN
  /// A CREW list.
  List<AgentSession> get looseLive => [
    for (final s in live)
      if (crewOf(s) == null) s,
  ];

  List<AgentSession> get looseEnded => [
    for (final s in ended)
      if (crewOf(s) == null) s,
  ];

  List<AgentHost> get online => [
    for (final h in hosts)
      if (h.online) h,
  ];

  AgentHost? host(String id) => hosts.where((h) => h.id == id).firstOrNull;

  /// A removed host is named by its sessions' record of it (decision 153).
  String hostName(String id) =>
      host(id)?.name ??
      sessions.where((s) => s.hostId == id).firstOrNull?.hostName ??
      'host';

  AgentSession? byId(String id) =>
      sessions.where((s) => s.id == id).firstOrNull;

  /// Why [s]'s conversation can't be reopened now, or null when it can or
  /// there is none to reopen (decision 89).
  String? resumeBlocker(AgentSession s) {
    if (!s.ended || !s.hasConversation) return null;
    final host = this.host(s.hostId);
    if (host == null || !host.online) {
      return "Can't resume while ${host?.name ?? 'its host'} is offline";
    }
    if (!host.resumes.contains(s.provider)) {
      return 'Update storm-runtime on ${host.name} to resume';
    }
    final open = sessions.any(
      (o) => !o.ended && o.harnessSession == s.harnessSession,
    );
    return open ? 'This conversation is open in another session' : null;
  }

  bool canResume(AgentSession s) =>
      s.ended && s.hasConversation && resumeBlocker(s) == null;
}

/// How often a visible list re-asks the server while `/v1/agent/events` is
/// not open: before its first snapshot, between reconnects, or on a server
/// before decision 154.
const agentRefreshInterval = Duration(seconds: 15);

/// While the stream is open the sessions follow it, and the list re-asks
/// only for what is not pushed (hosts, crews' names, colours and order) and
/// as a safety net (decision 154, Q6).
const agentRefreshWhileStreaming = Duration(seconds: 60);

/// How often an open, live session re-reads its record, for its Wrote count,
/// while the stream is not open.
const sessionRefreshInterval = Duration(seconds: 4);

/// While the stream is open its record follows the pushed view.
const sessionRefreshWhileStreaming = Duration(seconds: 30);

/// Re-runs the provider after [every] while it is listened to and the app is
/// in front; with [events], after [whileOpen] instead while their stream is
/// open, and at once when it closes after [every] has already passed.
///
/// Two things stop the polling, and neither is `autoDispose` alone. Riverpod
/// pauses a provider whose only listeners are under `TickerMode(false)` — a
/// covered route — and a paused provider skips its refresh. And a tick that
/// lands while the app is backgrounded is skipped here, then made up the
/// moment the app resumes, rather than polling a phone in a pocket.
void _refreshEvery(
  Ref ref,
  Duration every, {
  AgentEvents? events,
  Duration? whileOpen,
}) {
  var due = false;
  var fired = false;
  void fire() {
    if (fired) return;
    final state = WidgetsBinding.instance.lifecycleState;
    if (state == null || state == AppLifecycleState.resumed) {
      fired = true;
      ref.invalidateSelf();
    } else {
      due = true;
    }
  }

  final streaming = events != null && whileOpen != null;
  var short = false;
  final timers = [
    Timer(every, () {
      short = true;
      if (!streaming || !events.isOpen) fire();
    }),
    if (streaming) Timer(whileOpen, fire),
  ];
  void streamChanged() {
    if (short && !events!.isOpen) fire();
  }

  if (streaming) events.state.addListener(streamChanged);
  final lifecycle = AppLifecycleListener(
    onResume: () {
      if (due) ref.invalidateSelf();
    },
  );
  ref.onDispose(() {
    for (final t in timers) {
      t.cancel();
    }
    if (streaming) events.state.removeListener(streamChanged);
    lifecycle.dispose();
  });
}

/// [_refreshEvery] for the spawning client's providers (`spawn_state.dart`).
void refreshProviderEvery(Ref ref, Duration every) => _refreshEvery(ref, every);

/// One crew or label change this device made that the server has not
/// answered yet (138j), drawn over the list until it does.
class CrewPatch {
  CrewPatch(this.apply, {this.sessions = const {}, this.crews = const {}});

  final AgentOverview Function(AgentOverview o) apply;

  /// What it changes, for the snap-back outline when it is refused.
  final Set<String> sessions;
  final Set<String> crews;
}

/// The changes in flight, oldest first. Kept apart from the list so a poll
/// that lands meanwhile is drawn with them still applied.
final crewPatchesProvider = StateProvider<List<CrewPatch>>((ref) => const []);

AgentOverview _patched(AgentOverview o, List<CrewPatch> patches) {
  if (o.unreachable || patches.isEmpty) return o;
  var out = o;
  for (final p in patches) {
    out = p.apply(out);
  }
  return out.copyWith(pending: {for (final p in patches) ...p.sessions});
}

/// The overview, refreshed every [agentRefreshInterval] while it is on screen
/// and the app is in front, with this device's pending crew changes drawn
/// over it ([crewPatchesProvider]).
///
/// It never throws. Riverpod 3 retries a failed provider on its own
/// schedule, which would be a second, unbounded poll alongside this one.
final agentOverviewProvider =
    AsyncNotifierProvider.autoDispose<AgentOverviewNotifier, AgentOverview>(
      AgentOverviewNotifier.new,
    );

class AgentOverviewNotifier extends AsyncNotifier<AgentOverview> {
  AgentOverview? _server;

  @override
  Future<AgentOverview> build() async {
    final events = ref.watch(agentEventsProvider);
    _refreshEvery(
      ref,
      agentRefreshInterval,
      events: events,
      whileOpen: agentRefreshWhileStreaming,
    );
    ref.listen(crewPatchesProvider, (_, patches) {
      final server = _server;
      if (server != null) state = AsyncData(_patched(server, patches));
    });
    final askedAt = DateTime.now();
    final asked = events?.now ?? 0;
    AgentOverview? read;
    AgentOverview merged(AgentOverview o) => events == null || o.unreachable
        ? o
        : o.copyWith(sessions: events.merge(o.sessions, asked));
    if (events != null) {
      final sub = events.updates.listen((_) {
        final o = read;
        if (o == null) return;
        final server = _server = merged(o);
        state = AsyncData(_patched(server, ref.read(crewPatchesProvider)));
      });
      ref.onDispose(sub.cancel);
    }
    final o = read = await _fetch();
    final server = _server = merged(o);
    if (!o.unreachable) {
      ref.read(sessionKeepAliveProvider).overviewSeen(server.sessions, askedAt);
    }
    return _patched(server, ref.read(crewPatchesProvider));
  }

  Future<AgentOverview> _fetch() async {
    final api = ref.watch(agentApiFactoryProvider)?.call();
    if (api == null) return const AgentOverview.unreachable();
    try {
      // Independent, so together: one round trip, not two, which is the
      // difference that shows over the relay.
      final (sessions, hosts, crews) = await (
        api.sessions(),
        api.hosts(),
        _crewsOrNull(api),
      ).wait;
      return AgentOverview(
        sessions: sessions.reversed.toList(),
        hosts: hosts,
        crews: crews,
      );
    } catch (_) {
      return const AgentOverview.unreachable();
    } finally {
      api.dispose();
    }
  }
}

/// A server before decision 138 has no crews route; its sessions still show.
Future<List<Crew>?> _crewsOrNull(AgentApi api) async {
  try {
    return (await api.crews())..sort((a, b) => a.order.compareTo(b.order));
  } catch (_) {
    return null;
  }
}

/// Kit's role notes (138e), re-read on the list's tick while something that
/// offers them is open; null when they cannot be asked for.
final rolesProvider = FutureProvider.autoDispose<List<RoleNote>?>((ref) async {
  _refreshEvery(ref, agentRefreshInterval);
  final api = ref.watch(agentApiFactoryProvider)?.call();
  if (api == null) return null;
  try {
    return await api.roles();
  } catch (_) {
    return null;
  } finally {
    api.dispose();
  }
});

/// Ask the server again now, rather than at the next tick.
Future<void> reloadAgents(WidgetRef ref) async {
  ref.invalidate(agentOverviewProvider);
  await ref.read(agentOverviewProvider.future);
}

/// How many hidden sessions this device keeps (decision 154). Tests
/// override it.
final sessionKeepAliveBudgetProvider = Provider<KeepAliveBudget>(
  (ref) => KeepAliveBudget.forPlatform(),
);

/// The kept sessions. Only ever read, and it hears credential changes
/// through the container: a hidden controller's own subscriptions are
/// paused, so a `ref.watch` here would not see a logout until something
/// read it again.
final sessionKeepAliveProvider = Provider<SessionKeepAlive>((ref) {
  final keep = SessionKeepAlive(ref.read(sessionKeepAliveBudgetProvider));
  final sub = ref.container.listen(
    sessionCredentialsProvider,
    (_, next) => keep.credentialsChanged(next),
  );
  ref.onDispose(() {
    sub.close();
    keep.dispose();
  });
  return keep;
});

/// One session's stream, terminal and input, shared by whatever shows the
/// session — the desk split and the phone screen are the same route — and
/// kept alive in a bounded LRU once nothing does (decision 154, plan §6.1):
/// hidden, its stream closes after a grace and resumes by offset.
final sessionControllerProvider = ChangeNotifierProvider.autoDispose
    .family<SessionController?, String>((ref, id) {
      final credentials = ref.watch(sessionCredentialsProvider);
      final api = ref.watch(agentApiFactoryProvider)?.call();
      if (api == null) return null;
      final open = ref.watch(terminalStreamFactoryProvider);
      final keep = ref.read(sessionKeepAliveProvider);
      final grace = keep.budget.grace;
      final c = SessionController(
        api: api,
        sessionId: id,
        open: open == null ? null : (o, snapshot) => open(id, o, snapshot),
        hiddenGrace: () => grace ?? hiddenStreamGrace(api),
        onGone: () => keep.gone(id),
      )..start();
      keep.track(id, c, credentials, ref.keepAlive);
      ref.onCancel(() {
        c.hide();
        keep.hidden(id);
      });
      ref.onResume(() {
        c.show();
        keep.shown(id);
      });
      // The provider disposes the controller itself.
      ref.onDispose(() {
        keep.forget(id, c);
        api.dispose();
      });
      return c;
    });

/// A session's record as the server has it now, following its pushed view
/// and re-read while it is live and on screen; null when there is no such
/// session or the server cannot be asked. Its `wrote_count` is what
/// [sessionWritesProvider] follows.
final sessionDetailProvider = AsyncNotifierProvider.autoDispose
    .family<SessionDetailNotifier, AgentSession?, String>(
      SessionDetailNotifier.new,
    );

class SessionDetailNotifier extends AsyncNotifier<AgentSession?> {
  SessionDetailNotifier(this.id);

  final String id;

  @override
  Future<AgentSession?> build() async {
    final events = ref.watch(agentEventsProvider);
    final api = ref.watch(agentApiFactoryProvider)?.call();
    if (api == null) return null;
    void poll() => _refreshEvery(
      ref,
      sessionRefreshInterval,
      events: events,
      whileOpen: sessionRefreshWhileStreaming,
    );
    final asked = events?.now ?? 0;
    var read = false;
    if (events != null) {
      final sub = events.updates.listen((u) {
        if (!read) return;
        switch (u.event) {
          case AgentSessionChanged(:final session) when session.id == id:
            state = AsyncData(session);
          case AgentSessionRemoved(id: final gone) when gone == id:
            state = const AsyncData(null);
          case AgentSnapshot():
            state = AsyncData(events.byId(id));
          default:
            break;
        }
      });
      ref.onDispose(sub.cancel);
    }
    try {
      final fetched = await api.session(id);
      final s = events == null ? fetched : events.resolve(id, fetched, asked);
      if (s != null && !s.ended) poll();
      return s;
    } catch (e) {
      if (!(e is StormApiException && e.statusCode == 404)) poll();
      return events?.resolve(id, null, asked);
    } finally {
      read = true;
      api.dispose();
    }
  }
}

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

/// The inspector tabs of a session (`?tab=`), in the rail's order (153i).
enum SessionTab { context, wrote, agents, about, work }

/// `requests` was the Activity tab before the Agents tab took it in (153i).
SessionTab sessionTabOf(String? raw) => raw == 'requests'
    ? SessionTab.agents
    : SessionTab.values.firstWhere(
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
