/// How the client reaches its server — and the one place that knows.
///
/// Above this seam nothing distinguishes a direct LAN dial from a relayed one.
/// A `SaveOutcome` means the same thing whichever path carried it, which is the
/// whole point: CRUD, search and MCP were coupled to one transport, and adding
/// a second would otherwise have meant teaching every layer about both.
///
/// This is the Connection of SRP §8 (vault *Storm Relay Protocol*): it owns
/// the candidates, the one race in flight, every trunk, the proven transport,
/// the backoff and the reconnect timer. Nothing else starts a race or decides
/// that a transport is lost (decisions 113, 116g, 116h).
library;

import 'dart:async';
import 'dart:math';

import 'package:http/http.dart' as http;
import 'package:web_socket_channel/web_socket_channel.dart';

import 'auth_api.dart';
import 'auth_models.dart';
import 'connection_client.dart';
import 'connection_log.dart';
import 'ed25519_verify.dart';
import 'net.dart';
import 'relay/srp_http_client.dart';
import 'relay/srp_trunk.dart';
import 'server_verifier.dart';
import 'storm_api.dart';

/// Which kind of path a candidate is — and therefore when it may be raced.
enum CandidateTier {
  /// The stored server address. Plain configuration, not discovery.
  direct,

  /// A relay the operator runs. Sees plaintext, but *their* plaintext.
  selfHosted,

  /// A relay someone else runs (§2.1). Also sees plaintext, which is the
  /// entire reason it is raced last.
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

/// The Connection's state (§8.2).
enum ConnectionPhase { idle, racing, connected, unreachable, unproven, closed }

/// What the user is shown, derived from [ConnectionPhase] alone (§8.6).
enum ConnectionStatus {
  /// Nothing attempted yet.
  idle,

  /// A race has run for more than two seconds, or replaced a lost transport.
  connecting,

  /// A candidate answered *and* proved its identity.
  connected,

  /// A candidate answered and could **not** prove its identity. Deliberately
  /// not [offline]: something is there, and that is the problem.
  unproven,

  /// Nothing answered — not the server, and not any relay.
  offline,

  /// In the race that just ended, a relay refused `HELLO` with
  /// `server_unreachable`, or the direct address refused the connection: the
  /// path is fine and the server is not there. Never [offline], which would
  /// send the user to check their network.
  serverOffline,
}

/// Hosts of relays someone other than the operator runs: raced only after
/// every first-round candidate is done (§2.1, decision 114d). Storm's hosted
/// Cloudflare relay is one, whatever its registration mode. mcp-connect's
/// `PUBLIC_RELAY_HOSTS` holds the same list.
const Set<String> kPublicRelayHosts = {'storm-relay.dewansh.space'};

/// Resolves candidates to one working, *proven* transport.
class StormConnection {
  StormConnection({
    required this.localAddress,
    required String token,
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
    this.candidateDeadline = const Duration(seconds: 15),
    this.directConnectTimeout = kDirectConnectTimeout,
    this.probeTimeout = const Duration(seconds: 10),
    this.waitTimeout = const Duration(seconds: 30),
    this.backoffMin = const Duration(seconds: 1),
    this.backoffMax = const Duration(seconds: 60),
    this.stableAfter = const Duration(seconds: 60),
    this.statusGrace = const Duration(seconds: 2),
    Random? random,
    ConnectionLog? log,
  }) : _bearer = token,
       _relays = List.of(relays),
       _random = random ?? Random(),
       _log = log ?? ConnectionLog.instance {
    _active = ConnectionCandidate(
      tier: CandidateTier.direct,
      baseUrl: localAddress,
    );
  }

  /// One candidate, already built, and no race to run but the challenge.
  StormConnection.direct({
    required StormApi api,
    AuthApi? authApi,
    ServerVerifier? verifier,
  }) : _bearer = api.token,
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
       candidateDeadline = const Duration(seconds: 15),
       directConnectTimeout = kDirectConnectTimeout,
       probeTimeout = const Duration(seconds: 10),
       waitTimeout = const Duration(seconds: 30),
       backoffMin = const Duration(seconds: 1),
       backoffMax = const Duration(seconds: 60),
       stableAfter = const Duration(seconds: 60),
       statusGrace = const Duration(seconds: 2),
       onRelaysChanged = null,
       _random = Random(),
       _log = ConnectionLog.instance,
       _pinnedApi = api,
       _pinnedAuthApi = authApi,
       _pinnedVerifier = verifier {
    _active = ConnectionCandidate(
      tier: CandidateTier.direct,
      baseUrl: api.baseUrl,
    );
  }

  final ConnectionLog _log;
  final Random _random;

  /// The session credential. Changing it changes request headers and nothing
  /// else: no re-race, no new trunk (§8.1 rule 3).
  String get token => _bearer;
  String _bearer;
  set token(String value) {
    if (value == _bearer || _pinnedApi != null) return;
    _bearer = value;
    _sessionApi = null;
    _cachedApi = null;
    _cachedApiFor = null;
    _log.add('token changed: connection kept');
  }

  /// Pinned at pairing, and what the identity challenge verifies against.
  /// Empty on an unpaired install, which is the one case with nothing to prove.
  final String serverId;
  final String publicKeyB64;

  /// `T_candidate` (§8.8): first byte to proof, per candidate.
  final Duration candidateDeadline;

  /// `T_connect`: the direct candidate's connect phase.
  final Duration directConnectTimeout;

  /// `T_probe`: the resume probe.
  final Duration probeTimeout;

  /// `T_wait`: how long a request not yet sent waits for `Connected`.
  final Duration waitTimeout;

  /// The race backoff: from [backoffMin], doubling to [backoffMax], ±20 %.
  final Duration backoffMin;
  final Duration backoffMax;

  /// How long a transport must stay connected before the backoff clears.
  final Duration stableAfter;

  /// How long a race may run before the status says `connecting`.
  final Duration statusGrace;

  /// The stored server address. Plain configuration, never discovery.
  final String localAddress;

  /// Hosts of public relays (§2.1). Every other relay is the operator's.
  final Set<String> publicRelayHosts;

  final http.Client? client;

  /// Test seams. Production leaves them null and gets the real clients.
  final StormApi Function(ConnectionCandidate)? buildApi;
  final AuthApi Function(ConnectionCandidate)? buildAuthApi;
  final ServerVerifier Function(AuthApi)? buildVerifier;

  /// Fired when a proven server hands over a new relay set (D3, §4.3). The
  /// owner persists it so the set survives a restart.
  final void Function(List<RelayAdvert> relays)? onRelaysChanged;

  /// Test seam: how a relay's socket is opened. Null is the real one.
  final WebSocketChannel Function(Uri)? relayConnector;

  List<RelayAdvert> _relays;

  StormApi? _pinnedApi;
  AuthApi? _pinnedAuthApi;
  ServerVerifier? _pinnedVerifier;

  /// Trunks by relay URL. A `READY` one outlives a race that did not use it,
  /// so the next race reuses it instead of spending another `HELLO` (§8.3
  /// rule 6).
  final Map<String, SrpTrunk> _trunks = {};
  String? _activeRelayUrl;
  http.Client? _directClient;

  late ConnectionCandidate _active;
  StormApi? _cachedApi;
  ConnectionCandidate? _cachedApiFor;

  ConnectionPhase _phase = ConnectionPhase.idle;
  ConnectionPhase get phase => _phase;
  bool get isConnected => _phase == ConnectionPhase.connected;

  /// Bumped whenever a transport is adopted. A request that failed on an
  /// older one says nothing about the current one.
  int get generation => _generation;
  int _generation = 0;

  ConnectionStatus _status = ConnectionStatus.idle;
  ConnectionStatus get status => _status;
  final _statusChanges = StreamController<ConnectionStatus>.broadcast();
  Stream<ConnectionStatus> get statusChanges => _statusChanges.stream;
  final _phaseChanges = StreamController<ConnectionPhase>.broadcast();

  /// Whether the last race ended with `server_offline` (§8.6).
  bool _serverSide = false;
  ServerProof _lastProof = ServerProof.unreachable;

  Future<ServerProof>? _race;
  int _raceNo = 0;
  int _epoch = 0;

  Duration? _backoff;
  int _failedRaces = 0;
  Timer? _reconnectTimer;
  DateTime? _reconnectDue;
  Timer? _stableTimer;
  Timer? _graceTimer;

  final Set<Object> _demand = {};
  int _waiting = 0;
  bool get _hasDemand => _waiting > 0 || _demand.isNotEmpty;

  /// The candidate that failed the proof in the last race, if one did.
  ConnectionCandidate? get failedIdentity => _failedIdentity;
  ConnectionCandidate? _failedIdentity;

  /// The candidate currently in use. Above the seam nothing reads this except
  /// to *display* it — behaviour must never branch on it.
  ConnectionCandidate get active => _active;

  /// The relay set as last learned from a proven server.
  List<RelayAdvert> get relays => List.unmodifiable(_relays);

  /// Whether a reconnect timer is pending: the Connection is waiting out its
  /// backoff and starts no race on demand alone (§8.3 rule 2).
  bool get inBackoff =>
      (_phase == ConnectionPhase.unreachable ||
          _phase == ConnectionPhase.unproven) &&
      _reconnectTimer != null;

  StormApi get api {
    final pinned = _pinnedApi;
    if (pinned != null) return pinned;
    if (_cachedApi != null && _cachedApiFor == _active) return _cachedApi!;
    _cachedApiFor = _active;
    return _cachedApi = _apiFor(_active);
  }

  AuthApi get authApi => _pinnedAuthApi ?? _authApiFor(_active);

  /// The API for session calls: addressed to the saved address, and each
  /// request sent on whatever transport is proven when it goes out, waiting
  /// for one when there is none ([ConnectionClient], §8.5).
  StormApi get sessionApi =>
      _pinnedApi ??
      (_sessionApi ??= StormApi(
        baseUrl: localAddress,
        token: _bearer,
        client: ConnectionClient(() => this),
      ));
  StormApi? _sessionApi;

  /// Whether the active candidate is relayed and its trunk is still up.
  bool get hasLiveTrunk {
    final url = _activeRelayUrl;
    final trunk = url == null ? null : _trunks[url];
    return trunk != null && !trunk.isClosed;
  }

  /// Takes a **ticket**, not the session credential — see `StormApi.wsTicket`.
  Uri streamUri(String ticket) {
    final ws = _active.baseUrl.replaceFirst(RegExp(r'^http'), 'ws');
    return Uri.parse('$ws/v1/stream?ticket=${Uri.encodeComponent(ticket)}');
  }

  /// SSE endpoint for the change feed on a relayed connection: the relay
  /// tunnels HTTP, not WebSocket upgrades.
  Uri sseUri(String ticket) {
    return Uri.parse(
      '${_active.baseUrl}/v1/stream?ticket=${Uri.encodeComponent(ticket)}',
    );
  }

  bool get isRelayed => _active.relayUrl != null;

  http.Client get transportClient => api.client;

  Map<String, String> get authHeaders => {
    'Authorization': 'Bearer $_bearer',
    'Content-Type': 'application/json',
  };

  http.Client get _direct => client ?? (_directClient ??= directHttpClient());

  StormApi _apiFor(ConnectionCandidate c) =>
      buildApi?.call(c) ??
      StormApi(
        baseUrl: c.baseUrl,
        token: _bearer,
        client: c.relayUrl != null ? _relayClient(c) : _direct,
      );

  AuthApi _authApiFor(ConnectionCandidate c) =>
      buildAuthApi?.call(c) ??
      AuthApi(
        baseUrl: c.baseUrl,
        client: c.relayUrl != null ? _relayClient(c) : _direct,
      );

  /// A tunnel over [c]'s trunk: the live one if there is one, never a second
  /// beside it (one dial per trunk, §5).
  http.Client _relayClient(ConnectionCandidate c) {
    final url = c.relayUrl!;
    var trunk = _trunks[url];
    if (trunk == null || trunk.isClosed) {
      trunk = SrpTrunk(
        url: Uri.parse(url),
        serverId: serverId,
        serverKey: publicKeyB64,
        log: _log,
        connector: relayConnector,
      );
      final born = trunk;
      born.onTrunkLost.listen((reason) {
        if (identical(_trunks[url], born) && url == _activeRelayUrl) {
          _transportLost(
            'trunk lost: ${born.lossReason ?? reason}',
            superseded: born.lossReason == 'trunk_superseded',
          );
        }
      });
      _trunks[url] = trunk;
    }
    return SrpHttpClient(trunk);
  }

  ServerVerifier _verifierFor(AuthApi a) =>
      _pinnedVerifier ??
      buildVerifier?.call(a) ??
      ServerVerifier(
        authApi: a,
        serverId: serverId,
        publicKeyB64: publicKeyB64,
      );

  /// Candidates in the order they may be considered, direct first. A `ws://`
  /// relay outside §4.3's address classes is not one.
  List<ConnectionCandidate> _candidates() {
    final out = <ConnectionCandidate>[
      ConnectionCandidate(tier: CandidateTier.direct, baseUrl: localAddress),
    ];
    for (final relay in _relays) {
      if (!relayUrlAllowed(relay.url) ||
          !relayUrlAllowed(relay.publicAddress)) {
        _log.add(
          'relay ${logHost(relay.url)} skipped: plaintext to a public host',
        );
        continue;
      }
      out.add(
        ConnectionCandidate(
          tier: _tierOf(relay.url),
          // The relay's origin, never `publicAddress`: REST paths go through
          // the tunnel as method + path, so the origin is nominal.
          baseUrl: relay.url
              .replaceFirst(RegExp(r'^wss'), 'https')
              .replaceFirst(RegExp(r'^ws'), 'http'),
          relayUrl: relay.publicAddress,
        ),
      );
    }
    return out;
  }

  CandidateTier _tierOf(String relayUrl) {
    final host = (Uri.tryParse(relayUrl)?.host ?? '').toLowerCase();
    return publicRelayHosts.contains(host)
        ? CandidateTier.public
        : CandidateTier.selfHosted;
  }

  // ---- demand and events ----------------------------------------------

  /// Ensures connected (§8.3 rule 2): joins the race in flight, answers at
  /// once when connected, starts a race from `Idle`, and in backoff reports
  /// the last race's verdict without racing.
  Future<ServerProof> connect() {
    switch (_phase) {
      case ConnectionPhase.closed:
        return Future.value(ServerProof.unreachable);
      case ConnectionPhase.connected:
        return Future.value(ServerProof.verified);
      case ConnectionPhase.racing:
        _log.add('ensure connected: joined race #$_raceNo');
        return _race!;
      case ConnectionPhase.unreachable || ConnectionPhase.unproven
          when _reconnectTimer != null:
        return Future.value(_lastProof);
      case _:
        return _startRace('demand');
    }
  }

  /// For a request not yet sent (§8.5 rule 2): waits up to `T_wait` for
  /// `Connected`, joining a race in flight and the next one after a backoff.
  Future<ServerProof> awaitConnected() async {
    final deadline = DateTime.now().add(waitTimeout);
    _waiting++;
    try {
      final first = await connect().timeout(
        waitTimeout,
        onTimeout: () => ServerProof.unreachable,
      );
      if (first != ServerProof.unreachable ||
          _phase == ConnectionPhase.closed) {
        return first;
      }
      if (_phase != ConnectionPhase.racing && _reconnectTimer == null) {
        unawaited(connect());
      }
      final left = deadline.difference(DateTime.now());
      if (left <= Duration.zero) return ServerProof.unreachable;
      if (_phase == ConnectionPhase.racing) {
        final joined = await _race!.timeout(
          left,
          onTimeout: () => ServerProof.unreachable,
        );
        if (joined != ServerProof.unreachable) return joined;
      }
      final rest = deadline.difference(DateTime.now());
      if (rest <= Duration.zero) return ServerProof.unreachable;
      final next = await _phaseChanges.stream
          .firstWhere(
            (p) =>
                p == ConnectionPhase.connected ||
                p == ConnectionPhase.unproven ||
                p == ConnectionPhase.closed,
            orElse: () => ConnectionPhase.closed,
          )
          .timeout(rest, onTimeout: () => ConnectionPhase.unreachable);
      return switch (next) {
        ConnectionPhase.connected => ServerProof.verified,
        ConnectionPhase.unproven => ServerProof.impostor,
        _ => ServerProof.unreachable,
      };
    } finally {
      _waiting--;
    }
  }

  /// Something wants a transport for as long as it is registered: an open
  /// change feed, a queued write. Backoff only re-races while there is
  /// demand.
  void addDemand(Object who) {
    _demand.add(who);
    final waiting =
        _phase == ConnectionPhase.unreachable ||
        _phase == ConnectionPhase.unproven;
    if (waiting && _reconnectTimer == null) _startRace('demand');
  }

  void removeDemand(Object who) => _demand.remove(who);

  /// A request on this Connection failed. Only the Connection decides what
  /// that means (§8.4): a connection-level error on the direct transport, or
  /// a `trunk_lost`/`trunk_superseded` on a relayed one, loses the transport;
  /// anything stream-scoped and any HTTP status changes nothing.
  void requestFailed(Object error, {int? generation}) {
    if (_phase != ConnectionPhase.connected) return;
    if (generation != null && generation != _generation) return;
    if (isRelayed) {
      if (error is SrpTransportException &&
          (error.code == 'trunk_lost' || error.code == 'trunk_superseded')) {
        _transportLost(
          'stream ${error.code}',
          superseded: error.code == 'trunk_superseded',
        );
      }
    } else if (isConnectionLevel(error)) {
      _transportLost('direct request failed: ${logError(error)}');
    }
  }

  /// The app came back to the foreground (§8.7 rule 3).
  Future<void> onAppResumed() => _onWake('app resumed');

  /// The platform reported a network change (§8.7 rule 3).
  Future<void> onNetworkChanged() => _onWake('network changed');

  Future<void> _onWake(String why) async {
    switch (_phase) {
      case ConnectionPhase.connected:
        final gen = _generation;
        _log.add('$why: probing the transport');
        final alive = await _probe();
        if (!alive && gen == _generation) {
          _transportLost('no answer to the probe after $why');
        } else if (alive) {
          _log.add('$why: probe answered');
        }
      case ConnectionPhase.unreachable || ConnectionPhase.unproven:
        _cancelReconnect();
        _startRace(why);
      case _:
        break;
    }
  }

  Future<bool> _probe() async {
    final url = _activeRelayUrl;
    if (url != null) {
      final trunk = _trunks[url];
      return trunk != null && await trunk.probe(probeTimeout);
    }
    try {
      await api.client
          .get(Uri.parse('${_active.baseUrl}/v1/health'))
          .timeout(probeTimeout);
      return true;
    } catch (_) {
      return false;
    }
  }

  // ---- the race --------------------------------------------------------

  /// Starts the one race (§8.3 rule 1). Only ever called when no race runs.
  Future<ServerProof> _startRace(String cause) {
    assert(
      _phase != ConnectionPhase.racing && _phase != ConnectionPhase.closed,
    );
    final no = ++_raceNo;
    final epoch = _epoch;
    _serverSide = false;
    _setPhase(ConnectionPhase.racing);
    final race = _runRace(no, cause).then((r) => _conclude(epoch, no, r));
    _race = race;
    return race;
  }

  Future<_RaceResult> _runRace(int no, String cause) async {
    final watch = Stopwatch()..start();
    if (_pinnedApi != null) {
      final proof = await _verifierFor(authApi).check();
      _log.add('race #$no: identity ${proof.name}');
      return _RaceResult(
        winner: proof == ServerProof.verified
            ? _Verdict(_active, _Outcome.proven)
            : null,
        identityFailedBy: proof == ServerProof.impostor ? _active : null,
        serverSide: false,
        millis: watch.elapsedMilliseconds,
      );
    }

    final candidates = _candidates();
    final first = [
      for (final c in candidates)
        if (c.tier != CandidateTier.public) c,
    ];
    final public = [
      for (final c in candidates)
        if (c.tier == CandidateTier.public) c,
    ];
    _log.add(
      'race #$no start ($cause): '
      '${candidates.map((c) => '${c.tier.name} ${logHost(c.relayUrl ?? c.baseUrl)}').join(', ')}',
    );

    final verdicts = <_Verdict>[];
    var winner = await _round(first, watch, verdicts);
    // The privacy rule (§2.2): a public relay runs only once every candidate
    // the operator controls has failed or hit its deadline — never on a timer.
    if (winner == null && public.isNotEmpty) {
      _log.add(
        'race #$no: round 1 over at ${watch.elapsedMilliseconds} ms, '
        'public round starts',
      );
      winner = await _round(public, watch, verdicts);
    }
    return _RaceResult(
      winner: winner,
      identityFailedBy: verdicts
          .where((v) => v.outcome == _Outcome.identityFailed)
          .firstOrNull
          ?.candidate,
      serverSide: verdicts.any((v) => v.serverSide),
      millis: watch.elapsedMilliseconds,
    );
  }

  /// Runs [round] together; the first proven candidate wins it.
  Future<_Verdict?> _round(
    List<ConnectionCandidate> round,
    Stopwatch watch,
    List<_Verdict> verdicts,
  ) {
    if (round.isEmpty) return Future.value(null);
    final done = Completer<_Verdict?>();
    var pending = round.length;
    for (final c in round) {
      unawaited(
        _attempt(c, watch).then((v) {
          verdicts.add(v);
          pending--;
          if (done.isCompleted) return;
          if (v.outcome == _Outcome.proven) {
            done.complete(v);
          } else if (pending == 0) {
            done.complete(null);
          }
        }),
      );
    }
    return done.future;
  }

  /// One candidate, proven on its own path (§8.3 rule 3), within
  /// `T_candidate`.
  Future<_Verdict> _attempt(ConnectionCandidate c, Stopwatch watch) {
    final label = '${c.tier.name} ${logHost(c.relayUrl ?? c.baseUrl)}';
    final auth = _authApiFor(c);

    Future<_Verdict> run() async {
      ServerInfo? info;
      try {
        var probe = auth.serverInfo();
        if (c.tier == CandidateTier.direct) {
          probe = probe.timeout(directConnectTimeout);
        }
        info = await probe;
        _log.add('  $label: probe ok at ${watch.elapsedMilliseconds} ms');
      } on AuthApiException catch (e) {
        // An answer, from a server predating the endpoint: the challenge
        // still decides.
        _log.add(
          '  $label: probe answered ${e.statusCode} at ${watch.elapsedMilliseconds} ms',
        );
      } catch (e) {
        final serverSide = _refusedByServerSide(c, e);
        _log.add(
          '  $label: failed at ${watch.elapsedMilliseconds} ms: '
          '${serverSide ? 'server not there' : logError(e)}',
        );
        return _Verdict(c, _Outcome.unreachable, serverSide: serverSide);
      }
      final proof = await _verifierFor(auth).check();
      _log.add(
        '  $label: identity ${proof.name} at ${watch.elapsedMilliseconds} ms',
      );
      return switch (proof) {
        ServerProof.verified => _Verdict(c, _Outcome.proven, info: info),
        ServerProof.impostor => _Verdict(c, _Outcome.identityFailed),
        ServerProof.unreachable => _Verdict(c, _Outcome.unreachable),
      };
    }

    return run().timeout(
      candidateDeadline,
      onTimeout: () {
        _log.add(
          '  $label: deadline (${candidateDeadline.inMilliseconds} ms) reached',
        );
        return _Verdict(c, _Outcome.unreachable);
      },
    );
  }

  bool _refusedByServerSide(ConnectionCandidate c, Object e) {
    final url = c.relayUrl;
    if (url == null) return isConnectionRefused(e);
    return _trunks[url]?.helloRefusal == 'server_unreachable';
  }

  ServerProof _conclude(int epoch, int no, _RaceResult r) {
    // A race the Connection's end overtook adopts nothing (§8.3 rule 1).
    if (epoch != _epoch) return ServerProof.unreachable;
    _race = null;
    final winner = r.winner;
    if (winner != null) {
      _adopt(winner.candidate);
      _activeRelayUrl = winner.candidate.relayUrl;
      _closeTrunks(keep: (url, _) => url == _activeRelayUrl);
      _generation++;
      _log.add(
        'race #$no won by ${winner.candidate.tier.name} in ${r.millis} ms',
      );
      final info = winner.info;
      if (info != null) _adoptRelaySet(info);
      _lastProof = ServerProof.verified;
      _cancelReconnect();
      _stableTimer?.cancel();
      _stableTimer = Timer(stableAfter, () {
        if (_backoff != null) {
          _log.add('connected ${stableAfter.inSeconds} s: backoff cleared');
        }
        _backoff = null;
        _failedRaces = 0;
      });
      _setPhase(ConnectionPhase.connected);
      return ServerProof.verified;
    }

    // Kept: a `READY` trunk, for the next race (§8.3 rule 7).
    _closeTrunks(keep: (_, t) => t.isConnected);
    _activeRelayUrl = null;
    _active = ConnectionCandidate(
      tier: CandidateTier.direct,
      baseUrl: localAddress,
    );
    _cachedApi = null;
    _cachedApiFor = null;
    _serverSide = r.serverSide;
    _failedIdentity = r.identityFailedBy;
    final unproven = r.identityFailed;
    _lastProof = unproven ? ServerProof.impostor : ServerProof.unreachable;
    _log.add(
      'race #$no over in ${r.millis} ms: '
      '${unproven ? 'identity failed' : (_serverSide ? 'server offline' : 'nothing answered')}',
    );
    _setPhase(
      unproven ? ConnectionPhase.unproven : ConnectionPhase.unreachable,
    );
    _scheduleBackoff();
    return _lastProof;
  }

  /// Adopts a relay set only from the proven winner, and only when it
  /// describes the pinned server (§4.3, §8.3 rule 8). It takes effect at the
  /// next race.
  void _adoptRelaySet(ServerInfo info) {
    if (serverId.isNotEmpty && info.serverId != serverId) {
      _log.add('relay set ignored: another server id');
      return;
    }
    if (publicKeyB64.isNotEmpty && !_sameKey(info.publicKey, publicKeyB64)) {
      _log.add('relay set ignored: another key');
      return;
    }
    if (_sameRelays(_relays, info.relays)) return;
    _relays = List.of(info.relays);
    _log.add(
      'relay set adopted: ${_relays.map((r) => logHost(r.url)).join(', ')}',
    );
    onRelaysChanged?.call(List.unmodifiable(_relays));
  }

  void _transportLost(String reason, {bool superseded = false}) {
    if (_phase != ConnectionPhase.connected) return;
    _log.add('transport lost: $reason');
    _stableTimer?.cancel();
    // A lost trunk is never reused, even one whose socket looks open: a
    // failed probe or a `trunk_lost` says it carries nothing.
    final url = _activeRelayUrl;
    if (url != null) _trunks.remove(url)?.dispose();
    _startRace(superseded ? 'trunk superseded' : 'transport lost');
  }

  // ---- timers ----------------------------------------------------------

  void _scheduleBackoff() {
    final base = _backoff ?? backoffMin;
    final jitter = 0.8 + _random.nextDouble() * 0.4;
    final delay = Duration(
      milliseconds: (base.inMilliseconds * jitter).round(),
    );
    _failedRaces++;
    final next = base * 2;
    _backoff = next > backoffMax ? backoffMax : next;
    _scheduleReconnect(delay, 'backoff after $_failedRaces failed race(s)');
  }

  /// At most one pending reconnect timer, and the earliest wins (§8.3).
  void _scheduleReconnect(Duration delay, String why) {
    final due = DateTime.now().add(delay);
    final pending = _reconnectDue;
    if (_reconnectTimer != null && pending != null && !pending.isAfter(due)) {
      _log.add('reconnect timer kept: an earlier one is pending ($why)');
      return;
    }
    _reconnectTimer?.cancel();
    _reconnectDue = due;
    _log.add('reconnect in ${delay.inMilliseconds} ms: $why');
    _reconnectTimer = Timer(delay, _reconnectFired);
  }

  void _cancelReconnect() {
    _reconnectTimer?.cancel();
    _reconnectTimer = null;
    _reconnectDue = null;
  }

  /// A reconnect timer only ensures connected; it never resets (§8.3).
  void _reconnectFired() {
    _reconnectTimer = null;
    _reconnectDue = null;
    switch (_phase) {
      case ConnectionPhase.racing:
        _log.add('reconnect timer: race #$_raceNo in flight, nothing to do');
      case ConnectionPhase.connected:
        _log.add('reconnect timer: connected, nothing to do');
      case ConnectionPhase.closed:
        break;
      case _ when !_hasDemand:
        _log.add('reconnect timer: nothing wants the server, waiting');
      case _:
        _startRace('backoff elapsed');
    }
  }

  // ---- state and status ------------------------------------------------

  void _setPhase(ConnectionPhase next) {
    if (_phase != next) {
      _log.add('state ${_phase.name} -> ${next.name}');
    }
    final previous = _status;
    _phase = next;
    _graceTimer?.cancel();
    switch (next) {
      case ConnectionPhase.connected:
        _setStatus(ConnectionStatus.connected);
      case ConnectionPhase.unproven:
        _setStatus(ConnectionStatus.unproven);
      case ConnectionPhase.unreachable:
        _setStatus(
          _serverSide
              ? ConnectionStatus.serverOffline
              : ConnectionStatus.offline,
        );
      case ConnectionPhase.idle || ConnectionPhase.closed:
        _setStatus(ConnectionStatus.idle);
      case ConnectionPhase.racing:
        // Online only while a transport is connected; anything else keeps
        // its line for two seconds so a quick race does not flicker.
        if (previous == ConnectionStatus.connected) {
          _setStatus(ConnectionStatus.connecting);
        } else {
          _graceTimer = Timer(statusGrace, () {
            if (_phase == ConnectionPhase.racing) {
              _setStatus(ConnectionStatus.connecting);
            }
          });
        }
    }
    if (!_phaseChanges.isClosed) _phaseChanges.add(next);
  }

  void _setStatus(ConnectionStatus value) {
    if (_status == value) return;
    _log.add('status ${_status.name} -> ${value.name}');
    _status = value;
    if (!_statusChanges.isClosed) _statusChanges.add(value);
  }

  void _adopt(ConnectionCandidate c) {
    if (_active != c) {
      _cachedApi = null;
      _cachedApiFor = null;
    }
    _active = c;
  }

  void _closeTrunks({required bool Function(String, SrpTrunk) keep}) {
    for (final entry in _trunks.entries.toList()) {
      if (!keep(entry.key, entry.value)) {
        _trunks.remove(entry.key);
        entry.value.dispose();
      }
    }
  }

  /// Ends the Connection (§8.1 rule 4): every trunk closed, any race void.
  void close([String why = 'closed']) {
    if (_phase == ConnectionPhase.closed) return;
    _log.add('connection closed: $why');
    _epoch++;
    _race = null;
    _cancelReconnect();
    _stableTimer?.cancel();
    _graceTimer?.cancel();
    _closeTrunks(keep: (_, _) => false);
    _activeRelayUrl = null;
    _directClient?.close();
    _directClient = null;
    _setPhase(ConnectionPhase.closed);
  }

  void dispose() {
    close('disposed');
    _statusChanges.close();
    _phaseChanges.close();
  }
}

bool _sameKey(String a, String b) {
  final x = decodeBase64UrlNoPad(a), y = decodeBase64UrlNoPad(b);
  if (x == null || y == null || x.length != y.length) return false;
  for (var i = 0; i < x.length; i++) {
    if (x[i] != y[i]) return false;
  }
  return true;
}

bool _sameRelays(List<RelayAdvert> a, List<RelayAdvert> b) {
  if (a.length != b.length) return false;
  for (var i = 0; i < a.length; i++) {
    if (a[i].url != b[i].url || a[i].publicAddress != b[i].publicAddress) {
      return false;
    }
  }
  return true;
}

enum _Outcome { proven, identityFailed, unreachable }

class _Verdict {
  _Verdict(this.candidate, this.outcome, {this.info, this.serverSide = false});
  final ConnectionCandidate candidate;
  final _Outcome outcome;
  final ServerInfo? info;
  final bool serverSide;
}

class _RaceResult {
  _RaceResult({
    required this.winner,
    required this.identityFailedBy,
    required this.serverSide,
    required this.millis,
  });
  final _Verdict? winner;
  final ConnectionCandidate? identityFailedBy;
  bool get identityFailed => identityFailedBy != null;
  final bool serverSide;
  final int millis;
}
