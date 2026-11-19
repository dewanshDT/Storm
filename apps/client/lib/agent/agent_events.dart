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
import 'desktop_platform.dart';
import 'notifications_models.dart';

/// What this device reports on connect, and again on change while connected
/// (decision 161, slice 161-4): whether it is active (visible and
/// focused) or backgrounded, and which session (if any) it currently has
/// open — the server skips the deliver hint entirely when an active
/// device already has the session a notice is about on screen.
@immutable
class AgentPresence {
  const AgentPresence({
    required this.active,
    this.openSessionId,
    this.connectionId,
  });

  final bool active;
  final String? openSessionId;

  /// This client's own id for the connection this presence was (or will
  /// be) reported over — a fresh one each time [AgentEvents] actually
  /// connects (161-4 review item 3), so the server can tell two tabs, or
  /// a reconnect's overlap, apart. `null` only before the very first
  /// connect.
  final String? connectionId;

  static const initial = AgentPresence(active: true);
}

/// A random id for one `/v1/agent/events` connection — never anything
/// that identifies the device or session, just unique enough that the
/// server's presence map keys two connections apart (review item 3).
String newConnectionId(Random random) {
  final bytes = List<int>.generate(16, (_) => random.nextInt(256));
  return bytes.map((b) => b.toRadixString(16).padLeft(2, '0')).join();
}

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

/// Decision 161: one new or replaced unread notice.
class AgentNotificationPushed extends AgentEvent {
  AgentNotificationPushed(this.notification);
  final AgentNotification notification;
}

/// Decision 161: these ids were marked read, by this device or another.
class AgentNotificationsRead extends AgentEvent {
  AgentNotificationsRead(this.ids);
  final List<String> ids;
}

/// Decision 161: the account's notification prefs changed; carries the row
/// as it now is, so a listener need not re-`GET` it.
class AgentPrefsChanged extends AgentEvent {
  AgentPrefsChanged(this.prefs);
  final NotificationPrefs prefs;
}

/// Decision 161: the mutes list changed; carries it as it now is.
class AgentMutesChanged extends AgentEvent {
  AgentMutesChanged(this.mutes);
  final List<NotificationMute> mutes;
}

/// Decision 161: the inbox was cleared elsewhere.
class AgentNotificationsCleared extends AgentEvent {}

/// Decision 161 (#416): these ids were cancelled (a `notify` `key` replace,
/// or `cancel_notify`) and must disappear from the inbox and any shown
/// toast, not merely be marked read.
class AgentNotificationsRemoved extends AgentEvent {
  AgentNotificationsRemoved(this.ids);
  final List<String> ids;
}

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
      'notification' => AgentNotificationPushed(
        AgentNotification.fromJson(data as Map<String, dynamic>),
      ),
      'notification_read' => AgentNotificationsRead([
        for (final id in (data as Map)['ids'] as List) '$id',
      ]),
      'prefs_changed' => AgentPrefsChanged(
        NotificationPrefs.fromJson(data as Map<String, dynamic>),
      ),
      'mutes_changed' => AgentMutesChanged([
        for (final m in (data as Map)['mutes'] as List)
          NotificationMute.fromJson(m as Map<String, dynamic>),
      ]),
      'notifications_cleared' => AgentNotificationsCleared(),
      'notification_removed' => AgentNotificationsRemoved([
        for (final id in (data as Map)['ids'] as List) '$id',
      ]),
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
      if (name == 'notifications_cleared') return AgentNotificationsCleared();
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

/// Opens one connection's events; errors or ends when it does. [presence] is
/// this device's state as of the moment it connects (161-4).
typedef AgentEventsOpen = Stream<AgentEvent> Function(AgentPresence presence);

/// Silence this long on SSE, which carries a keepalive every 15 s, is a dead
/// tunnel.
const agentEventsIdle = Duration(seconds: 45);

/// The events over [connection], however it reaches the server now: a
/// WebSocket when direct, SSE through a relay, which tunnels HTTP and not
/// upgrades (the way the change feed picks).
Stream<AgentEvent> openAgentEvents(
  AgentApi Function() api,
  StormConnection connection,
  AgentPresence presence,
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
          ? _openSse(client, presence)
          : _openSocket(client, connection.active.baseUrl, presence);
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

Stream<AgentEvent> _openSse(AgentApi api, AgentPresence presence) {
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
      final request = http.Request('GET', _eventsUri(api.baseUrl, presence))
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

Stream<AgentEvent> _openSocket(
  AgentApi api,
  String activeBase,
  AgentPresence presence,
) {
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
        _eventsUri(ws, presence).replace(
          queryParameters: {..._presenceQuery(presence), 'ticket': ticket},
        ),
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

/// `?presence=active|background[&open_session=<id>]&connection_id=<id>`
/// (161-4). Absent `presence` entirely is `active` too, server-side —
/// but every connection here states it, so the server need not guess.
Map<String, String> _presenceQuery(AgentPresence presence) => {
  'presence': presence.active ? 'active' : 'background',
  if (presence.openSessionId != null) 'open_session': presence.openSessionId!,
  if (presence.connectionId != null) 'connection_id': presence.connectionId!,
};

Uri _eventsUri(String base, AgentPresence presence) => Uri.parse(
  '$base/v1/agent/events',
).replace(queryParameters: _presenceQuery(presence));

Object _refusal(int status) => status == 404 || status == 403
    ? AgentEventsRefused(status)
    : http.ClientException('agent events answered $status');

/// The route's status without opening a stream: `HEAD` has no body.
Future<int?> _probe(AgentApi api) async {
  try {
    final r = await api.client.send(
      http.Request('HEAD', _eventsUri(api.baseUrl, AgentPresence.initial))
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
    this._apiFactory,
    Random? random,
  }) : _random = random ?? Random();

  final AgentEventsOpen _open;
  final Duration retryMin;
  final Duration retryMax;
  final Random _random;

  /// Used only for the lightweight `POST …/presence` while already
  /// connected (161-4) — never to open the stream itself. Null in tests
  /// that don't exercise presence; a failed POST is swallowed the same way
  /// `NotificationsState.markSessionRead` swallows its own.
  final AgentApi Function()? _apiFactory;
  AgentPresence _presence = AgentPresence.initial;

  /// What [_presence] was the moment the current connection's `_open`
  /// call went out — compared against [_presence] again once the
  /// connection reaches [AgentEventsState.open] (review item 6): a
  /// change made in between (still `connecting`, so [_syncPresence]'s
  /// `POST` could not reach a connection that does not exist yet) is
  /// otherwise lost for good, since nothing else ever re-checks it.
  AgentPresence? _sentPresence;

  final state = ValueNotifier(AgentEventsState.closed);
  bool get isOpen => state.value == AgentEventsState.open;

  /// The session [setOpenSession] last reported, if any.
  String? get openSessionId => _presence.openSessionId;

  /// Whether this device currently reports itself active (visible and
  /// focused) rather than backgrounded.
  bool get isActive => _presence.active;

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
        // Undoes a desktop `inactive`'s presence flip below even when
        // `_background` was never set true by it (so `foreground()`
        // itself no-ops) — a window that only lost and regained focus,
        // never truly hidden.
        if (isDesktopPlatform) _setActive(true);
      case AppLifecycleState.inactive:
        // Desktop only (review item 5): the window lost focus without
        // being hidden, minimized or closed. Background presence, same
        // as `background()`'s desktop branch — but never drop the
        // stream (`_background` stays untouched; this is milder).
        if (isDesktopPlatform) _setActive(false);
      case AppLifecycleState.detached:
        break;
    }
  }

  /// No stream runs behind the person's back — except on desktop (161-4,
  /// amends 154 Q2): there, the stream stays open so a closed-window
  /// session still gets OS notifications, and only presence changes to
  /// `background`. Phones and web still close it.
  void background() {
    if (_disposed || _background) return;
    _background = true;
    if (isDesktopPlatform) {
      _setActive(false);
      return;
    }
    _drop();
    if (state.value != AgentEventsState.unsupported) {
      state.value = AgentEventsState.closed;
    }
  }

  /// Back in front. On desktop the stream never left, so this only reports
  /// presence; everywhere else, connect now for a fresh snapshot — a
  /// server that had no stream is asked again, in case it was upgraded
  /// meanwhile.
  void foreground() {
    if (_disposed || !_background) return;
    _background = false;
    if (isDesktopPlatform) {
      _setActive(true);
      return;
    }
    _backoff = null;
    if (state.value == AgentEventsState.unsupported) {
      state.value = AgentEventsState.closed;
    }
    _connect();
  }

  /// This device now has [id] open (or, `null`, has closed whatever it had
  /// open) — 161-4's "any device active and showing that session: nothing"
  /// rule. Best-effort and silent: nothing here is worth surfacing to the
  /// person looking at a session.
  void setOpenSession(String? id) {
    if (_presence.openSessionId == id) return;
    _presence = AgentPresence(
      active: _presence.active,
      openSessionId: id,
      connectionId: _presence.connectionId,
    );
    _syncPresence();
  }

  void _setActive(bool active) {
    if (_presence.active == active) return;
    _presence = AgentPresence(
      active: active,
      openSessionId: _presence.openSessionId,
      connectionId: _presence.connectionId,
    );
    _syncPresence();
  }

  /// Whether a change arrived while [_pumpPresence] was already sending
  /// an older one — picked up by the same pump once it finishes, so the
  /// server always ends up with the *latest* presence rather than
  /// whichever of several concurrent POSTs happened to land last
  /// (review item B).
  bool _presenceDirty = false;
  Future<void>? _presencePump;

  /// Tells the server about [_presence] without reconnecting — the point
  /// of keeping the stream open at all. A connection not yet open carries
  /// the current [_presence] itself once it does ([_connect]); and once
  /// it does, [_apply]'s snapshot case re-checks for exactly this case —
  /// a change made while still connecting, which this early return would
  /// otherwise drop for good (review item 6).
  void _syncPresence() {
    if (!isOpen || _presence.connectionId == null) return;
    _presenceDirty = true;
    _presencePump ??= _pumpPresence();
  }

  /// One `POST` in flight at a time (review item B): each run sends
  /// whatever [_presence] is *at that moment*, retries once on failure,
  /// and — if another change arrived while it was sending — runs again
  /// immediately rather than trusting the stale request that was already
  /// in flight.
  Future<void> _pumpPresence() async {
    while (_presenceDirty) {
      _presenceDirty = false;
      final snapshot = _presence;
      final connectionId = snapshot.connectionId;
      if (connectionId == null) break;
      for (var attempt = 0; attempt < 2; attempt++) {
        final api = _apiFactory?.call();
        if (api == null) break;
        try {
          await api.setPresence(
            connectionId,
            snapshot.active ? 'active' : 'background',
            openSessionId: snapshot.openSessionId,
          );
          break;
        } catch (_) {
          // One retry; a second failure waits for the next real change
          // (or the next connect, which always carries the latest state
          // anyway) rather than looping forever on a dead connection.
        } finally {
          api.dispose();
        }
      }
    }
    _presencePump = null;
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
    // A fresh connection id every time, including a reconnect (review
    // item 3): the server tells this attempt apart from whatever came
    // before, so an old connection's late drop can never clear this
    // one's presence.
    _presence = AgentPresence(
      active: _presence.active,
      openSessionId: _presence.openSessionId,
      connectionId: newConnectionId(_random),
    );
    _sentPresence = _presence;
    final Stream<AgentEvent> stream;
    try {
      stream = _open(_presence);
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
        // Review item 6: a presence change made while this connection was
        // still `connecting` never reached `_syncPresence` (not `isOpen`
        // yet) — catch up now, the first moment a `POST` can land.
        if (_sentPresence != null &&
            (_sentPresence!.active != _presence.active ||
                _sentPresence!.openSessionId != _presence.openSessionId)) {
          _syncPresence();
        }
        _sentPresence = null;
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
      case AgentNotificationPushed():
      case AgentNotificationsRead():
      case AgentPrefsChanged():
      case AgentMutesChanged():
      case AgentNotificationsCleared():
      case AgentNotificationsRemoved():
        // Not session state; carried on this same stream (one connection
        // per device) so `NotificationsState` can apply it without a
        // second subscription.
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
  return (presence) => openAgentEvents(factory, connection, presence);
});

/// The pushed session state, alive while anything showing the lists is:
/// the overview (rail, sidebar, Agents screens) and an open session's
/// record watch it. Feeds the kept sessions (decision 154): a pushed view
/// judges a kept terminal's offset, a snapshot counts as a list read, and a
/// removal lets a kept session go.
final agentEventsProvider = Provider.autoDispose<AgentEvents?>((ref) {
  final open = ref.watch(agentEventsTransportProvider);
  if (open == null) return null;
  final factory = ref.watch(agentApiFactoryProvider);
  final keep = ref.read(sessionKeepAliveProvider);
  final events = AgentEvents(open, apiFactory: factory)..start();
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
      case AgentNotificationPushed():
      case AgentNotificationsRead():
      case AgentPrefsChanged():
      case AgentMutesChanged():
      case AgentNotificationsCleared():
      case AgentNotificationsRemoved():
        break;
    }
  });
  ref.onDispose(() {
    sub.cancel();
    events.dispose();
  });
  return events;
});
