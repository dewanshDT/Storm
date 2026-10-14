/// How the client reaches its server — and the one place that knows.
///
/// Above this seam nothing distinguishes a direct LAN dial from a relayed one.
/// A `SaveOutcome` means the same thing whichever path carried it, which is the
/// whole point: CRUD, search and MCP were coupled to one transport, and adding
/// a second would otherwise have meant teaching every layer about both.
///
/// The seam owns three things and no more: which candidates exist, which one
/// currently answers, and whether that one has proved it is the server. It does
/// not own the cache, the outbox, or any policy about *what* to send.
library;

import 'dart:async';

import 'package:http/http.dart' as http;
import 'package:web_socket_channel/web_socket_channel.dart';

import 'auth_api.dart';
import 'auth_models.dart';
import 'connection_client.dart';
import 'connection_log.dart';
import 'relay/srp_http_client.dart';
import 'relay/srp_trunk.dart';
import 'server_verifier.dart';
import 'storm_api.dart';

/// Which kind of path a candidate is — and therefore when it may be raced.
///
/// This is data rather than logic on purpose. No relay exists yet, so today
/// every real deployment has exactly one candidate; modelling the tier now
/// means wiring real relays later is configuration, not a rewrite of the race.
enum CandidateTier {
  /// The stored server address. Plain configuration, not discovery.
  direct,

  /// A relay the operator runs. Sees plaintext, but *their* plaintext.
  selfHosted,

  /// A relay Storm runs. Also sees plaintext — and that is Storm's, not the
  /// operator's, which is the entire reason it is raced last.
  public,
}

/// One way to reach the server.
class ConnectionCandidate {
  const ConnectionCandidate({
    required this.tier,
    required this.baseUrl,
    this.relayUrl,
  });

  final CandidateTier tier;

  /// Base URL for REST, no trailing slash.
  final String baseUrl;

  /// The relay this candidate goes through, or null for [CandidateTier.direct].
  final String? relayUrl;

  @override
  String toString() => '${tier.name}:$baseUrl';
}

/// What the seam currently believes about the connection.
enum ConnectionStatus {
  /// Nothing attempted yet.
  idle,

  /// A race is in flight.
  connecting,

  /// A candidate answered *and* proved its identity.
  connected,

  /// A candidate answered and could **not** prove its identity. Deliberately
  /// not [offline]: something is there, and that is the problem.
  unproven,

  /// Nothing answered — not the server, and not any relay. From where the
  /// device sits, the relays are unavailable too.
  offline,

  /// A relay answered and said the server is not connected to it
  /// (`server_unreachable`, `trunk_lost`, `server_timeout` — SRP §6). The
  /// path is fine; the server is off, asleep, or lost its own connection.
  /// Never [offline], which would send the user to check their network.
  serverOffline,
}

/// Hosts of relays Storm itself runs: the only relays raced last. Empty, as
/// is mcp-connect's `PUBLIC_RELAY_HOSTS`, until a hosted relay ships; then its
/// host goes in both (`apps/relay-cf/DEPLOY.md`). A relay not listed is the
/// operator's, and is raced before any of these.
const Set<String> kPublicRelayHosts = {};

/// Resolves candidates to one working, *proven* transport.
class StormConnection {
  StormConnection({
    required this.localAddress,
    required this.token,
    this.serverId = '',
    this.publicKeyB64 = '',
    List<RelayAdvert> relays = const [],
    this.publicRelayHosts = kPublicRelayHosts,
    this.client,
    this.buildApi,
    this.buildAuthApi,
    this.buildVerifier,
    this.relayConnector,
    this.onRelaysChanged,
    this.publicRelayDelay = const Duration(seconds: 2),
    this.raceTimeout = const Duration(seconds: 15),
    ConnectionLog? log,
  }) : _relays = List.of(relays),
       _log = log ?? ConnectionLog.instance {
    _active = _candidates().first;
  }

  /// One candidate, already built, and no race to run.
  ///
  /// What every caller wants before relays exist, and what tests want always:
  /// it keeps today's behaviour exactly, so the seam is an indirection rather
  /// than a behaviour change until something is actually configured.
  StormConnection.direct({
    required StormApi api,
    AuthApi? authApi,
    ServerVerifier? verifier,
  }) : token = api.token,
       serverId = '',
       publicKeyB64 = '',
       localAddress = api.baseUrl,
       _relays = const [],
       publicRelayHosts = kPublicRelayHosts,
       client = null,
       buildApi = null,
       buildAuthApi = null,
       buildVerifier = null,
       relayConnector = null,
       publicRelayDelay = Duration.zero,
       raceTimeout = Duration.zero,
       onRelaysChanged = null,
       _log = ConnectionLog.instance,
       _pinnedApi = api,
       _pinnedAuthApi = authApi,
       _pinnedVerifier = verifier {
    _active = ConnectionCandidate(
      tier: CandidateTier.direct,
      baseUrl: api.baseUrl,
    );
  }

  final String token;

  final ConnectionLog _log;

  /// Pinned at pairing, and what the identity challenge verifies against.
  /// Empty on an unpaired install, which is the one case with nothing to prove.
  final String serverId;
  final String publicKeyB64;

  /// How long the direct + self-hosted round runs alone before the public
  /// relay may join it. It only gates the public relay: the first round keeps
  /// running past it (decision 113).
  final Duration publicRelayDelay;

  /// How long a race waits for any candidate to answer before it reports the
  /// server unreachable.
  ///
  /// A relayed probe is a TLS dial, the `HELLO`, a stream open and a request,
  /// about seven round trips: 1 s from a desk, 3–5 s over mobile data. Until
  /// decision 113 the race gave up at [publicRelayDelay] (2 s), so a phone
  /// off Wi-Fi never connected and re-raced on every backoff tick. It is a
  /// ceiling, not a wait: a refused candidate fails at once, and the first to
  /// answer wins. It bounds a direct dial to a LAN address that never answers.
  final Duration raceTimeout;

  /// The stored server address. Plain configuration, never discovery.
  final String localAddress;

  /// Hosts belonging to relays *Storm* runs. Everything else is treated as the
  /// operator's, which is the safe direction to err in: it means racing
  /// something they already trust rather than something we do.
  final Set<String> publicRelayHosts;

  final http.Client? client;

  /// Test seams. Production leaves all three null and gets the real clients.
  final StormApi Function(ConnectionCandidate)? buildApi;
  final AuthApi Function(ConnectionCandidate)? buildAuthApi;
  final ServerVerifier Function(AuthApi)? buildVerifier;

  /// Fired when the relay set changes (D3). The owner persists it so the set
  /// survives a restart — the server is the only writer, so this is a
  /// one-way mirror, not a merge.
  final void Function(List<RelayAdvert> relays)? onRelaysChanged;

  /// Test seam: how a relay's socket is opened. Null is the real one.
  final WebSocketChannel Function(Uri)? relayConnector;

  /// Mutable, unlike everything above: this is state the server tells us, not
  /// configuration we were given.
  List<RelayAdvert> _relays;

  StormApi? _pinnedApi;
  AuthApi? _pinnedAuthApi;
  ServerVerifier? _pinnedVerifier;

  /// The active SRP trunk, if any. Owned here — it is born when a relayed
  /// candidate is adopted and dies with [reset] — because its lifetime is the
  /// connection's (D2 consequence): reconnect re-races, so there is no trunk
  /// to migrate.
  ///
  /// Keyed by relay URL rather than candidate because `_candidates()` builds a
  /// fresh object per call: the race dials several relays at once and each must
  /// keep its own trunk, but the *set of URLs* is stable within a session.
  final Map<String, SrpTrunk> _trunks = {};
  String? _activeRelayUrl;

  late ConnectionCandidate _active;
  StormApi? _cachedApi;
  ConnectionCandidate? _cachedApiFor;

  ConnectionStatus _status = ConnectionStatus.idle;
  ConnectionStatus get status => _status;

  final _statusChanges = StreamController<ConnectionStatus>.broadcast();

  /// Every change of [status]. Most come from [connect], but a relay's
  /// `server_unreachable` can arrive after the race has already given up —
  /// the relay holds `HELLO` 5 s and the race does not wait that long.
  Stream<ConnectionStatus> get statusChanges => _statusChanges.stream;

  /// A relay said the server is not there since the last [reset].
  bool _serverBehindRelay = false;

  void _setStatus(ConnectionStatus value) {
    if (_status == value) return;
    _log.add('status ${_status.name} -> ${value.name}');
    _status = value;
    if (!_statusChanges.isClosed) _statusChanges.add(value);
  }

  void _onServerUnreachable(String code) {
    // A verdict about one request on a working connection is that request's
    // failure, not the server's status (decision 113).
    if (_status == ConnectionStatus.connected ||
        _status == ConnectionStatus.unproven) {
      return;
    }
    _log.add('relay says server not there: $code');
    _serverBehindRelay = true;
    if (_status == ConnectionStatus.offline) {
      _setStatus(ConnectionStatus.serverOffline);
    }
  }

  ConnectionStatus get _unreachable => _serverBehindRelay
      ? ConnectionStatus.serverOffline
      : ConnectionStatus.offline;

  /// The candidate currently in use. Above the seam nothing reads this except
  /// to *display* it — behaviour must never branch on it.
  ConnectionCandidate get active => _active;

  /// Replaces the relay set, mirroring it out to the owner (D3).
  void _setRelays(List<RelayAdvert> relays) {
    _relays = List.of(relays);
    onRelaysChanged?.call(List.unmodifiable(_relays));
  }

  /// The relay set as last learned from the server.
  List<RelayAdvert> get relays => List.unmodifiable(_relays);

  /// The API client for whichever candidate is active.
  ///
  /// A getter rather than a field so callers follow the winning transport
  /// without being rebuilt. Session calls use [sessionApi], which waits for a
  /// proven transport; this one is the transport itself.
  StormApi get api {
    final pinned = _pinnedApi;
    if (pinned != null) return pinned;
    if (_cachedApi != null && _cachedApiFor == _active) return _cachedApi!;
    _cachedApiFor = _active;
    return _cachedApi = _apiFor(_active);
  }

  AuthApi get authApi => _pinnedAuthApi ?? _authApiFor(_active);

  /// The API for session calls: addressed to the saved address, and each
  /// request sent on whatever transport is proven when it goes out, joining
  /// the race in flight if there is one ([ConnectionClient], decision 112).
  ///
  /// [api] follows the active candidate, which between a reset and the end
  /// of the re-race is the saved address. A request sent then from off the
  /// LAN was refused, and the engine took that as offline (decision 113).
  StormApi get sessionApi =>
      _pinnedApi ??
      (_sessionApi ??= StormApi(
        baseUrl: localAddress,
        token: token,
        client: ConnectionClient(() => this),
      ));
  StormApi? _sessionApi;

  /// Whether the active candidate is relayed and its trunk is still up.
  bool get hasLiveTrunk {
    final url = _activeRelayUrl;
    final trunk = url == null ? null : _trunks[url];
    return trunk != null && !trunk.isClosed;
  }

  /// Where the change feed lives on the active candidate.
  ///
  /// This is the knowledge the seam exists to hold: `SyncEngine` used to build
  /// it by string-replacing `http` with `ws` on the API's base URL, which is a
  /// transport detail it had no business knowing.
  /// Takes a **ticket**, not the session credential. Both would authenticate,
  /// and only one is safe to put in a URL — see `StormApi.wsTicket`.
  Uri streamUri(String ticket) {
    final ws = _active.baseUrl.replaceFirst(RegExp(r'^http'), 'ws');
    return Uri.parse('$ws/v1/stream?ticket=${Uri.encodeComponent(ticket)}');
  }

  /// SSE endpoint for the change feed on a relayed connection.
  ///
  /// The relay tunnels HTTP, not WebSocket upgrades, so the feed comes over
  /// SSE at the same path. Uses the active candidate's base URL (the relay's
  /// origin) directly — no `ws`/`wss` conversion.
  Uri sseUri(String ticket) {
    return Uri.parse(
      '${_active.baseUrl}/v1/stream?ticket=${Uri.encodeComponent(ticket)}',
    );
  }

  /// Whether the active transport is relayed (vs direct LAN).
  bool get isRelayed => _active.relayUrl != null;

  /// The transport client for the active candidate (for SSE and other
  /// streaming uses that need direct client access).
  http.Client get transportClient => api.client;

  /// Auth headers for the active candidate.
  Map<String, String> get authHeaders => {
    'Authorization': 'Bearer $token',
    'Content-Type': 'application/json',
  };

  StormApi _apiFor(ConnectionCandidate c) =>
      buildApi?.call(c) ??
      StormApi(
        baseUrl: c.baseUrl,
        token: token,
        client: c.relayUrl != null ? _relayClient(c) : client,
      );

  AuthApi _authApiFor(ConnectionCandidate c) =>
      buildAuthApi?.call(c) ??
      AuthApi(
        baseUrl: c.baseUrl,
        client: c.relayUrl != null ? _relayClient(c) : client,
      );

  /// A tunnel transport for a relayed candidate. Each relay URL keeps its own
  /// trunk (`_trunks`), so concurrently probing multiple relays does not tear
  /// down a peer's in-flight open. The one the race crowns is retained; the
  /// rest are released by [connect] once a winner is known.
  http.Client _relayClient(ConnectionCandidate c) {
    final url = c.relayUrl!;
    var trunk = _trunks[url];
    // A dead trunk is replaced, never reused: it refuses every request.
    if (trunk == null || trunk.isClosed) {
      final connector = relayConnector;
      trunk = connector == null
          ? SrpTrunk(url: Uri.parse(url), serverId: serverId, log: _log)
          : SrpTrunk(
              url: Uri.parse(url),
              serverId: serverId,
              log: _log,
              connector: connector,
            );
      trunk.onServerUnreachable.listen(_onServerUnreachable);
      final born = trunk;
      // The winner's trunk dying ends the connection: without this the
      // status stayed `connected` and every request went to a dead trunk
      // until something reset.
      born.onTrunkLost.listen((reason) {
        if (identical(_trunks[url], born) && url == _activeRelayUrl) {
          reset('active trunk lost: $reason');
        }
      });
      _trunks[url] = trunk;
    }
    return SrpHttpClient(trunk);
  }

  /// Releases every trunk except the one behind [winner] — the losers of the
  /// race served their purpose (proving reachability) and must not keep a
  /// heartbeat and a slot on the relay idle.
  void _releaseWinlessTrunks(String? winnerUrl) {
    for (final entry in _trunks.entries.toList()) {
      if (entry.key != winnerUrl) {
        _trunks.remove(entry.key);
        entry.value.dispose();
      }
    }
  }

  ServerVerifier _verifierFor(AuthApi a) =>
      _pinnedVerifier ??
      buildVerifier?.call(a) ??
      ServerVerifier(
        authApi: a,
        serverId: serverId,
        publicKeyB64: publicKeyB64,
      );

  /// Candidates in the order they may be *considered*, direct first.
  List<ConnectionCandidate> _candidates() {
    final out = <ConnectionCandidate>[
      ConnectionCandidate(tier: CandidateTier.direct, baseUrl: localAddress),
    ];
    for (final relay in _relays) {
      out.add(
        ConnectionCandidate(
          tier: _tierOf(relay.url),
          // `baseUrl` is only ever used to build REST paths, which the tunnel
          // carries as method + path — so the origin is nominal. It must be the
          // relay's *origin*, never `publicAddress`: that field is the WebSocket
          // connect endpoint (`wss://…/connect/<id>`) and handing it to an
          // http.Client would build `/connect/<id>/v1/…` paths the server
          // cannot route (per §4.3 it expects a `HELLO`, not an HTTP request).
          baseUrl: relay.url
              .replaceFirst(RegExp(r'^wss'), 'https')
              .replaceFirst(RegExp(r'^ws'), 'http'),
          // The actual dial target: the per-id WebSocket connect endpoint.
          relayUrl: relay.publicAddress,
        ),
      );
    }
    return out;
  }

  /// A relay is public only if it is one Storm runs; everything else is the
  /// operator's. Erring towards self-hosted is the safe direction — it means
  /// racing something the operator already trusts, never Storm's.
  CandidateTier _tierOf(String relayUrl) {
    final host = Uri.tryParse(relayUrl)?.host ?? '';
    return publicRelayHosts.contains(host)
        ? CandidateTier.public
        : CandidateTier.selfHosted;
  }

  /// Races the candidates, refreshes the relay set, and proves the winner.
  ///
  /// Returns what the *identity* check concluded, because that is what the
  /// caller has to act on — reachability alone is not a connection.
  ///
  /// One race at a time: a caller arriving while one is in flight shares it.
  /// A race overtaken by [reset] reports unreachable and changes nothing.
  Future<ServerProof> connect() {
    final running = _connecting;
    if (running != null) return running;
    final race = _connect(_epoch);
    _connecting = race;
    race.whenComplete(() {
      if (identical(_connecting, race)) _connecting = null;
    }).ignore();
    return race;
  }

  Future<ServerProof>? _connecting;
  int _epoch = 0;

  Future<ServerProof> _connect(int epoch) async {
    // Each race reports what *it* found; an earlier verdict is stale.
    _serverBehindRelay = false;
    _setStatus(ConnectionStatus.connecting);

    // A pinned transport has nothing to race and nothing to refresh — probing
    // would put a request on the wire that the old code never sent, which is
    // the sort of change a refactor is not allowed to make.
    if (_pinnedApi != null) {
      final proof = await _verifierFor(authApi).check();
      if (epoch != _epoch) return ServerProof.unreachable;
      _setStatus(switch (proof) {
        ServerProof.verified => ConnectionStatus.connected,
        ServerProof.impostor => ConnectionStatus.unproven,
        ServerProof.unreachable => _unreachable,
      });
      return proof;
    }

    final candidates = _candidates();
    _log.add(
      'race start: ${candidates.map((c) => '${c.tier.name} ${logHost(c.relayUrl ?? c.baseUrl)}').join(', ')}',
    );

    // Round one: direct and every self-hosted relay together. The public relay
    // is excluded here even when it would answer first — see below.
    final firstRound = candidates
        .where((c) => c.tier != CandidateTier.public)
        .toList();
    final publicRound = candidates
        .where((c) => c.tier == CandidateTier.public)
        .toList();

    // **The public relay runs only if nothing the operator controls answered
    // in time.** Not a latency rule: without end-to-end encryption, whichever
    // path carries traffic reads note content. Self-hosted means the operator
    // sees it, public means Storm does. Racing all three from the start would
    // silently prefer Storm's infrastructure over the operator's own — a
    // privacy regression wearing a latency optimisation.
    final winner = await _race(firstRound, publicRound);
    if (epoch != _epoch) return ServerProof.unreachable;

    if (winner == null) {
      _log.add('race: no candidate answered');
      _setStatus(_unreachable);
      return ServerProof.unreachable;
    }

    _log.add('race won by ${winner.candidate.tier.name}');
    _adopt(winner.candidate);

    // The race dialled a trunk for every relayed candidate; only the winner's
    // survives. The rest are released now, not at reset, so a multi-relay race
    // leaves exactly one trunk alive.
    _activeRelayUrl = winner.candidate.relayUrl;
    _releaseWinlessTrunks(_activeRelayUrl);

    // The probe already fetched it, so the relay set refreshes as a side
    // effect of connecting rather than needing a trip of its own. This is
    // what stops a client being stranded when its server changes relays while
    // it is away: the pairing payload is frozen at issuance and this is not.
    if (winner.info != null) _setRelays(winner.info!.relays);

    // **Reachability is not trust.** The race chose a transport; only the
    // challenge decides whether we are connected to the server.
    final proof = await _verifierFor(authApi).check();
    _log.add('identity check: ${proof.name}');
    if (epoch != _epoch) return ServerProof.unreachable;
    _setStatus(switch (proof) {
      ServerProof.verified => ConnectionStatus.connected,
      ServerProof.impostor => ConnectionStatus.unproven,
      ServerProof.unreachable => _unreachable,
    });
    return proof;
  }

  /// Forgets the winner so the next [connect] starts from nothing.
  ///
  /// A dropped connection re-races rather than resuming on whatever won last
  /// time — there is no mid-session transport migration, and reusing a stale
  /// winner is how you get one by accident.
  void reset([String reason = '']) {
    _log.add('reset${reason.isEmpty ? '' : ': $reason'}');
    _epoch++;
    _connecting = null;
    _setStatus(ConnectionStatus.idle);
    _serverBehindRelay = false;
    _cachedApi = null;
    _cachedApiFor = null;
    for (final t in _trunks.values) {
      t.dispose();
    }
    _trunks.clear();
    _activeRelayUrl = null;
    _active = _candidates().first;
  }

  /// Closes every trunk for good. The owner is done with this connection.
  void dispose() {
    reset('disposed');
    _statusChanges.close();
  }

  void _adopt(ConnectionCandidate c) {
    if (_active != c) {
      _cachedApi = null;
      _cachedApiFor = null;
    }
    _active = c;
  }

  /// First candidate to answer `GET /v1/server` wins.
  ///
  /// That endpoint is the right probe for three reasons: it needs no
  /// credential, it is the one a stranded client can always reach, and its
  /// answer *is* the relay-set refresh.
  ///
  /// [public] joins after [publicRelayDelay], or as soon as every one of
  /// [first] has failed. [first] keeps running either way, so a slow
  /// self-hosted relay can still win; nothing is abandoned until
  /// [raceTimeout].
  Future<_Probe?> _race(
    List<ConnectionCandidate> first,
    List<ConnectionCandidate> public,
  ) async {
    if (first.length == 1 && public.isEmpty) {
      // Nothing to race. The probe is best-effort: a server predating the
      // endpoint must still connect, and the challenge remains the real
      // reachability signal.
      final c = first.single;
      final watch = Stopwatch()..start();
      try {
        final info = await _authApiFor(c).serverInfo();
        _log.add('probe ${c.tier.name} ok in ${watch.elapsedMilliseconds} ms');
        return _Probe(c, info);
      } catch (e) {
        _log.add(
          'probe ${c.tier.name} failed in ${watch.elapsedMilliseconds} ms: ${logError(e)}',
        );
        return _Probe(c, null);
      }
    }

    final completer = Completer<_Probe?>();
    final watch = Stopwatch()..start();
    var pending = 0;
    var publicJoined = public.isEmpty;
    Timer? gate;
    late final void Function() joinPublic;

    void probe(ConnectionCandidate c) {
      pending++;
      unawaited(() async {
        try {
          final info = await _authApiFor(c).serverInfo();
          _log.add(
            'probe ${c.tier.name} ok in ${watch.elapsedMilliseconds} ms',
          );
          if (!completer.isCompleted) completer.complete(_Probe(c, info));
        } catch (e) {
          _log.add(
            'probe ${c.tier.name} failed in ${watch.elapsedMilliseconds} ms: ${logError(e)}',
          );
          pending--;
          if (pending > 0 || completer.isCompleted) return;
          if (publicJoined) {
            completer.complete(null);
          } else {
            joinPublic();
          }
        }
      }());
    }

    joinPublic = () {
      if (publicJoined || completer.isCompleted) return;
      publicJoined = true;
      gate?.cancel();
      _log.add('public relay joins after ${watch.elapsedMilliseconds} ms');
      public.forEach(probe);
    };

    first.forEach(probe);
    if (!publicJoined) gate = Timer(publicRelayDelay, joinPublic);
    try {
      return await completer.future.timeout(
        raceTimeout,
        onTimeout: () {
          _log.add('race gave up after ${raceTimeout.inMilliseconds} ms');
          return null;
        },
      );
    } finally {
      gate?.cancel();
    }
  }
}

class _Probe {
  _Probe(this.candidate, this.info);
  final ConnectionCandidate candidate;
  final ServerInfo? info;
}
