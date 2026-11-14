import 'dart:async';
import 'dart:convert';
import 'dart:math';

import 'package:flutter/widgets.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:http/http.dart' as http;
import 'package:web_socket_channel/web_socket_channel.dart';

import '../api/connection_client.dart';
import '../api/net.dart' show connectFeed;
import '../api/server_verifier.dart' show ServerProof;
import '../api/storm_connection.dart';
import '../state/app_state.dart' show connectionProvider;
import 'agent_api.dart';
import 'agent_models.dart';
import 'agent_state.dart';

/// One event of `GET /v1/agent/events` (decision 154, AM109).
sealed class AgentEvent {}

/// Every session, as `GET /v1/agent/sessions` lists them (oldest first): the
/// first event of every connection, and what follows a [AgentResync].
/// A new [epoch] is a restarted server.
class AgentSnapshot extends AgentEvent {
  AgentSnapshot(this.epoch, this.sessions);
  final String epoch;
  final List<AgentSession> sessions;
}

/// One session's whole view, replacing the one held; an unknown id is a new
/// session.
class AgentSessionChanged extends AgentEvent {
  AgentSessionChanged(this.session);
  final AgentSession session;
}

class AgentSessionRemoved extends AgentEvent {
  AgentSessionRemoved(this.id);
  final String id;
}

/// The server dropped events for this reader; a snapshot follows on the
/// same stream.
class AgentResync extends AgentEvent {}

/// [name] and its decoded [data], or null for an event this client does not
/// know or cannot read.
AgentEvent? decodeAgentEvent(String name, Object? data) {
  try {
    return switch (name) {
      'snapshot' => AgentSnapshot((data as Map)['epoch'] as String? ?? '', [
        for (final s in data['sessions'] as List)
          AgentSession.fromJson(s as Map<String, dynamic>),
      ]),
      'session' => AgentSessionChanged(
        AgentSession.fromJson(data as Map<String, dynamic>),
      ),
      'removed' => AgentSessionRemoved((data as Map)['id'] as String),
      'resync' => AgentResync(),
      _ => null,
    };
  } catch (_) {
    return null;
  }
}

/// A direct WebSocket's text frame: `{"event": …, "data": …}`.
AgentEvent? decodeAgentFrame(String frame) {
  try {
    final j = jsonDecode(frame) as Map<String, dynamic>;
    return decodeAgentEvent(j['event'] as String, j['data']);
  } catch (_) {
    return null;
  }
}

/// SSE, a line at a time: what the relays carry.
class AgentEventLines {
  String _name = 'message';
  final _data = <String>[];

  AgentEvent? add(String line) {
    if (line.isEmpty) {
      final name = _name;
      final data = _data.join('\n');
      _name = 'message';
      _data.clear();
      if (name == 'resync') return AgentResync();
      if (data.isEmpty) return null;
      try {
        return decodeAgentEvent(name, jsonDecode(data));
      } catch (_) {
        return null;
      }
    }
    if (line.startsWith(':')) return null;
    final colon = line.indexOf(':');
    final field = colon < 0 ? line : line.substring(0, colon);
    var value = colon < 0 ? '' : line.substring(colon + 1);
    if (value.startsWith(' ')) value = value.substring(1);
    switch (field) {
      case 'event':
        _name = value;
      case 'data':
        _data.add(value);
    }
    return null;
  }
}

/// The server answered the stream with [status] and it is not worth asking
/// again: 404 from a server before decision 154, 403 for a non-owner.
class AgentEventsRefused implements Exception {
  AgentEventsRefused(this.status);
  final int status;

  @override
  String toString() => 'AgentEventsRefused($status)';
}

/// Opens one connection's events; errors or ends when it does.
typedef AgentEventsOpen = Stream<AgentEvent> Function();

/// Silence this long on SSE, which carries a keepalive every 15 s, is a dead
/// tunnel.
const agentEventsIdle = Duration(seconds: 45);

/// The events over [connection], however it reaches the server now: a
/// WebSocket when direct, SSE through a relay, which tunnels HTTP and not
/// upgrades (the way the change feed picks).
Stream<AgentEvent> openAgentEvents(
  AgentApi Function() api,
  StormConnection connection,
) {
  late final StreamController<AgentEvent> out;
  StreamSubscription<AgentEvent>? inner;
  AgentApi? a;
  var cancelled = false;

  void end([Object? e]) {
    a?.dispose();
    a = null;
    if (out.isClosed) return;
    if (e != null) out.addError(e);
    out.close();
  }

  Future<void> start() async {
    try {
      final proof = await connection.connect();
      if (cancelled) return;
      if (proof != ServerProof.verified) {
        end(StateError('not connected: ${proof.name}'));
        return;
      }
      final client = a = api();
      final s = connection.isRelayed
          ? _openSse(client)
          : _openSocket(client, connection.active.baseUrl);
      inner = s.listen(out.add, onError: end, onDone: end, cancelOnError: true);
    } catch (e) {
      if (!cancelled) end(e);
    }
  }

  out = StreamController<AgentEvent>(
    onListen: start,
    onCancel: () async {
      cancelled = true;
      await inner?.cancel();
      a?.dispose();
      a = null;
    },
  );
  return out.stream;
}

Stream<AgentEvent> _openSse(AgentApi api) {
  late final StreamController<AgentEvent> out;
  StreamSubscription<String>? body;
  Timer? idle;
  var cancelled = false;

  void fail(Object e) {
    idle?.cancel();
    unawaited(body?.cancel());
    if (!out.isClosed) {
      out.addError(e);
      out.close();
    }
  }

  void heard() {
    idle?.cancel();
    idle = Timer(
      agentEventsIdle,
      () => fail(TimeoutException('agent events went quiet', agentEventsIdle)),
    );
  }

  Future<void> start() async {
    try {
      final request = http.Request('GET', _eventsUri(api.baseUrl))
        ..headers.addAll({
          'Authorization': 'Bearer ${api.token}',
          'Accept': 'text/event-stream',
        });
      final response = await api.client.send(request);
      if (cancelled || response.statusCode != 200) {
        unawaited(response.stream.listen(null).cancel());
        if (!cancelled) fail(_refusal(response.statusCode));
        return;
      }
      heard();
      final lines = AgentEventLines();
      body = response.stream
          .transform(utf8.decoder)
          .transform(const LineSplitter())
          .listen(
            (line) {
              heard();
              final e = lines.add(line);
              if (e != null && !out.isClosed) out.add(e);
            },
            onError: fail,
            onDone: () {
              idle?.cancel();
              if (!out.isClosed) out.close();
            },
            cancelOnError: true,
          );
    } catch (e) {
      if (!cancelled) fail(e);
    }
  }

  out = StreamController<AgentEvent>(
    onListen: start,
    onCancel: () {
      cancelled = true;
      idle?.cancel();
      return body?.cancel();
    },
  );
  return out.stream;
}

Stream<AgentEvent> _openSocket(AgentApi api, String activeBase) {
  late final StreamController<AgentEvent> out;
  WebSocketChannel? channel;
  StreamSubscription<Object?>? frames;
  var ready = false;
  var cancelled = false;

  void fail(Object e) {
    if (!out.isClosed) {
      out.addError(e);
      out.close();
    }
  }

  Future<void> start() async {
    try {
      // A handshake carries no header, so the URL carries a one-use ticket.
      final ticket = await api.streamTicket();
      if (cancelled) return;
      final ws = activeBase.replaceFirst(RegExp(r'^http'), 'ws');
      final c = channel = connectFeed(
        Uri.parse('$ws/v1/agent/events?ticket=${Uri.encodeComponent(ticket)}'),
      );
      frames = c.stream.listen(
        (frame) {
          if (frame is! String) return;
          final e = decodeAgentFrame(frame);
          if (e != null && !out.isClosed) out.add(e);
        },
        onError: (Object e) {
          if (ready) fail(e);
        },
        onDone: () {
          if (ready && !out.isClosed) out.close();
        },
        cancelOnError: true,
      );
      try {
        await c.ready;
        ready = true;
      } catch (e) {
        if (cancelled) return;
        // A failed handshake says nothing of why on the web, so ask plainly.
        final status = await _probe(api);
        fail(status == null || status == 200 ? e : _refusal(status));
      }
    } catch (e) {
      if (!cancelled) fail(e);
    }
  }

  out = StreamController<AgentEvent>(
    onListen: start,
    onCancel: () {
      cancelled = true;
      unawaited(frames?.cancel());
      unawaited(channel?.sink.close());
    },
  );
  return out.stream;
}

Uri _eventsUri(String base) => Uri.parse('$base/v1/agent/events');

Object _refusal(int status) => status == 404 || status == 403
    ? AgentEventsRefused(status)
    : http.ClientException('agent events answered $status');

/// The route's status without opening a stream: `HEAD` has no body.
Future<int?> _probe(AgentApi api) async {
  try {
    final r = await api.client.send(
      http.Request('HEAD', _eventsUri(api.baseUrl))
        ..headers['Authorization'] = 'Bearer ${api.token}',
    );
    unawaited(r.stream.listen(null).cancel());
    return r.statusCode;
  } catch (_) {
    return null;
  }
}

enum AgentEventsState {
  /// Not connected: between attempts, or the app is in the background.
  closed,
  connecting,

  /// A snapshot has arrived on the current connection.
  open,

  /// The server has no such stream (or refuses this caller): lists poll.
  unsupported,
}

/// What one event did, for whoever follows transitions (154-4).
class AgentEventUpdate {
  AgentEventUpdate(this.event, {this.before, this.epochChanged = false});

  final AgentEvent event;

  /// The session as held before an [AgentSessionChanged] or
  /// [AgentSessionRemoved]; null for a new session and other events.
  final AgentSession? before;

  /// A snapshot from a restarted server.
  final bool epochChanged;
}

/// The pushed session state (decision 154): one stream per signed-in
/// session, reconnecting on its own backoff, closed while the app is in the
/// background and a fresh snapshot on return.
///
/// Holds the sessions as last pushed, each stamped with [now] when it came,
/// so a list read that was asked before it never draws an older copy over
/// it ([merge]).
class AgentEvents {
  AgentEvents(
    this._open, {
    this.retryMin = const Duration(seconds: 1),
    this.retryMax = const Duration(seconds: 60),
    Random? random,
  }) : _random = random ?? Random();

  final AgentEventsOpen _open;
  final Duration retryMin;
  final Duration retryMax;
  final Random _random;

  final state = ValueNotifier(AgentEventsState.closed);
  bool get isOpen => state.value == AgentEventsState.open;

  final _updates = StreamController<AgentEventUpdate>.broadcast();

  /// Every event as applied; [AgentSessionChanged] and [AgentSessionRemoved]
  /// are live changes, a snapshot or resync never is.
  Stream<AgentEventUpdate> get updates => _updates.stream;

  /// Newest first.
  final _order = <String>[];
  final _sessions = <String, AgentSession>{};
  final _at = <String, int>{};
  final _removedAt = <String, int>{};
  int? _snapshotAt;
  int _clock = 0;
  String? _epoch;

  /// A point in this stream's history: what a list read is stamped with
  /// when it is asked.
  int get now => _clock;

  String? get epoch => _epoch;

  /// The sessions as last pushed, newest first; null before any snapshot.
  List<AgentSession>? get sessions =>
      _snapshotAt == null ? null : [for (final id in _order) _sessions[id]!];

  AgentSession? byId(String id) => _sessions[id];

  StreamSubscription<AgentEvent>? _sub;
  int _run = 0;
  Timer? _retry;
  Duration? _backoff;
  bool _background = false;
  bool _disposed = false;
  AppLifecycleListener? _lifecycle;

  void start({bool followLifecycle = true}) {
    if (followLifecycle) {
      _lifecycle = AppLifecycleListener(onStateChange: _lifecycleChanged);
    }
    _connect();
  }

  void _lifecycleChanged(AppLifecycleState s) {
    switch (s) {
      case AppLifecycleState.hidden || AppLifecycleState.paused:
        background();
      case AppLifecycleState.resumed:
        foreground();
      case AppLifecycleState.inactive || AppLifecycleState.detached:
        break;
    }
  }

  /// No stream runs behind the person's back.
  void background() {
    if (_disposed || _background) return;
    _background = true;
    _drop();
    if (state.value != AgentEventsState.unsupported) {
      state.value = AgentEventsState.closed;
    }
  }

  /// Back in front: connect now, for a fresh snapshot. A server that had no
  /// stream is asked again, in case it was upgraded meanwhile.
  void foreground() {
    if (_disposed || !_background) return;
    _background = false;
    _backoff = null;
    if (state.value == AgentEventsState.unsupported) {
      state.value = AgentEventsState.closed;
    }
    _connect();
  }

  void _drop() {
    _run++;
    _retry?.cancel();
    _retry = null;
    unawaited(_sub?.cancel());
    _sub = null;
  }

  void _connect() {
    if (_disposed || _background || _sub != null) return;
    if (state.value == AgentEventsState.unsupported) return;
    _retry?.cancel();
    _retry = null;
    final run = ++_run;
    state.value = AgentEventsState.connecting;
    final Stream<AgentEvent> stream;
    try {
      stream = _open();
    } catch (_) {
      _ended(run);
      return;
    }
    _sub = stream.listen(
      (e) {
        if (run == _run) _apply(e);
      },
      onError: (Object e) {
        if (run != _run) return;
        if (e is AgentEventsRefused) {
          _drop();
          state.value = AgentEventsState.unsupported;
          return;
        }
        _ended(run);
      },
      onDone: () => _ended(run),
      cancelOnError: true,
    );
  }

  void _ended(int run) {
    if (_disposed || run != _run) return;
    _sub = null;
    state.value = AgentEventsState.closed;
    final base = _backoff ?? retryMin;
    final next = base * 2;
    _backoff = next > retryMax ? retryMax : next;
    _retry?.cancel();
    _retry = Timer(_jitter(base), _connect);
  }

  Duration _jitter(Duration d) => Duration(
    milliseconds: (d.inMilliseconds * (0.8 + _random.nextDouble() * 0.4))
        .round(),
  );

  void _apply(AgentEvent e) {
    final at = ++_clock;
    switch (e) {
      case AgentSnapshot(:final epoch, :final sessions):
        final changed = _epoch != null && _epoch != epoch;
        _epoch = epoch;
        _order
          ..clear()
          ..addAll([for (final s in sessions.reversed) s.id]);
        _sessions
          ..clear()
          ..addAll({for (final s in sessions) s.id: s});
        _at
          ..clear()
          ..addAll({for (final s in sessions) s.id: at});
        _removedAt.clear();
        _snapshotAt = at;
        _backoff = null;
        state.value = AgentEventsState.open;
        _emit(AgentEventUpdate(e, epochChanged: changed));
      case AgentSessionChanged(:final session):
        final before = _sessions[session.id];
        if (before == null) _order.insert(0, session.id);
        _sessions[session.id] = session;
        _at[session.id] = at;
        _removedAt.remove(session.id);
        _emit(AgentEventUpdate(e, before: before));
      case AgentSessionRemoved(:final id):
        final before = _sessions.remove(id);
        _order.remove(id);
        _at.remove(id);
        _removedAt[id] = at;
        _emit(AgentEventUpdate(e, before: before));
      case AgentResync():
        _emit(AgentEventUpdate(e));
    }
  }

  void _emit(AgentEventUpdate u) {
    if (!_updates.isClosed) _updates.add(u);
  }

  /// [id] as known to whichever is newer, a read asked at [askedAt] that
  /// found [read] (null: no such session) or what was pushed since.
  AgentSession? resolve(String id, AgentSession? read, int askedAt) {
    final at = _at[id];
    if (at != null && at > askedAt) return _sessions[id];
    final gone = _removedAt[id];
    if (gone != null && gone > askedAt) return null;
    final snap = _snapshotAt;
    if (snap != null && snap > askedAt) return _sessions[id];
    return read;
  }

  /// A list read asked at [askedAt] (newest first), with whatever was
  /// pushed after it asked drawn over it.
  List<AgentSession> merge(List<AgentSession> read, int askedAt) {
    final snap = _snapshotAt;
    if (snap == null) return read;
    // A snapshot after the read is the whole list, and newer.
    if (snap > askedAt) return sessions!;
    final ids = <String>{};
    final out = <AgentSession>[];
    for (final s in read) {
      ids.add(s.id);
      final kept = resolve(s.id, s, askedAt);
      if (kept != null) out.add(kept);
    }
    return [
      for (final id in _order)
        if (!ids.contains(id) && _at[id]! > askedAt) _sessions[id]!,
      ...out,
    ];
  }

  void dispose() {
    _disposed = true;
    _lifecycle?.dispose();
    _drop();
    _updates.close();
    state.dispose();
  }
}

/// How this client opens `/v1/agent/events`, or null when it cannot: no
/// session, or not over the app's own connection. Tests override it.
final agentEventsTransportProvider = Provider<AgentEventsOpen?>((ref) {
  final factory = ref.watch(agentApiFactoryProvider);
  final connection = ref.watch(connectionProvider);
  if (factory == null || connection == null) return null;
  final probe = factory();
  final ours = probe.client is ConnectionClient;
  probe.dispose();
  if (!ours) return null;
  return () => openAgentEvents(factory, connection);
});

/// The pushed session state, alive while anything showing the lists is:
/// the overview (rail, sidebar, Agents screens) and an open session's
/// record watch it. Feeds the kept sessions (decision 154): a pushed view
/// judges a kept terminal's offset, a snapshot counts as a list read, and a
/// removal lets a kept session go.
final agentEventsProvider = Provider.autoDispose<AgentEvents?>((ref) {
  final open = ref.watch(agentEventsTransportProvider);
  if (open == null) return null;
  final keep = ref.read(sessionKeepAliveProvider);
  final events = AgentEvents(open)..start();
  final sub = events.updates.listen((u) {
    final at = DateTime.now();
    switch (u.event) {
      case AgentSnapshot(:final sessions):
        keep.overviewSeen(sessions, at);
      case AgentSessionChanged(:final session):
        keep.pushed(session, at);
      case AgentSessionRemoved(:final id):
        keep.removed(id);
      case AgentResync():
        break;
    }
  });
  ref.onDispose(() {
    sub.cancel();
    events.dispose();
  });
  return events;
});
