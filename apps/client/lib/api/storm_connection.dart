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

import 'auth_api.dart';
import 'auth_models.dart';
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

  /// Nothing answered.
  offline,
}

/// Resolves candidates to one working, *proven* transport.
class StormConnection {
  StormConnection({
    required this.localAddress,
    required this.token,
    this.serverId = '',
    this.publicKeyB64 = '',
    List<RelayAdvert> relays = const [],
    this.publicRelayHosts = const {},
    this.client,
    this.buildApi,
    this.buildAuthApi,
    this.buildVerifier,
    this.onRelaysChanged,
    this.publicRelayDelay = const Duration(seconds: 2),
  }) : _relays = List.of(relays) {
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
       publicRelayHosts = const {},
       client = null,
       buildApi = null,
       buildAuthApi = null,
       buildVerifier = null,
       publicRelayDelay = Duration.zero,
       onRelaysChanged = null,
       _pinnedApi = api,
       _pinnedAuthApi = authApi,
       _pinnedVerifier = verifier {
    _active = ConnectionCandidate(
      tier: CandidateTier.direct,
      baseUrl: api.baseUrl,
    );
  }

  final String token;

  /// Pinned at pairing, and what the identity challenge verifies against.
  /// Empty on an unpaired install, which is the one case with nothing to prove.
  final String serverId;
  final String publicKeyB64;

  /// How long the direct + self-hosted round gets before the public relay is
  /// allowed to run at all.
  final Duration publicRelayDelay;

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
  /// without being rebuilt. `SyncEngine` reads `api.x()` in a dozen places and
  /// none of them had to change.
  StormApi get api {
    final pinned = _pinnedApi;
    if (pinned != null) return pinned;
    if (_cachedApi != null && _cachedApiFor == _active) return _cachedApi!;
    _cachedApiFor = _active;
    return _cachedApi = _apiFor(_active);
  }

  AuthApi get authApi => _pinnedAuthApi ?? _authApiFor(_active);

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
  http.Client get transportClient {
    final pinned = _pinnedApi;
    if (pinned != null) return pinned.client;
    if (_cachedApi != null && _cachedApiFor == _active) {
      return _cachedApi!.client;
    }
    return _apiFor(_active).client;
  }

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
    final trunk = _trunks.putIfAbsent(
      url,
      () => SrpTrunk(url: Uri.parse(url), serverId: serverId),
    );
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
  Future<ServerProof> connect() async {
    _status = ConnectionStatus.connecting;

    // A pinned transport has nothing to race and nothing to refresh — probing
    // would put a request on the wire that the old code never sent, which is
    // the sort of change a refactor is not allowed to make.
    if (_pinnedApi != null) {
      final proof = await _verifierFor(authApi).check();
      _status = switch (proof) {
        ServerProof.verified => ConnectionStatus.connected,
        ServerProof.impostor => ConnectionStatus.unproven,
        ServerProof.unreachable => ConnectionStatus.offline,
      };
      return proof;
    }

    final candidates = _candidates();

    // Round one: direct and every self-hosted relay together. The public relay
    // is excluded here even when it would answer first — see below.
    final firstRound = candidates
        .where((c) => c.tier != CandidateTier.public)
        .toList();
    final publicRound = candidates
        .where((c) => c.tier == CandidateTier.public)
        .toList();

    var winner = await _race(firstRound);

    // **The public relay runs only if nothing the operator controls answered.**
    // Not a latency rule: without end-to-end encryption, whichever path carries
    // traffic reads note content. Self-hosted means the operator sees it,
    // public means Storm does. Racing all three and taking the first answer
    // would silently prefer Storm's infrastructure over the operator's own —
    // a privacy regression wearing a latency optimisation.
    if (winner == null && publicRound.isNotEmpty) {
      winner = await _race(publicRound);
    }

    if (winner == null) {
      _status = ConnectionStatus.offline;
      return ServerProof.unreachable;
    }

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
    _status = switch (proof) {
      ServerProof.verified => ConnectionStatus.connected,
      ServerProof.impostor => ConnectionStatus.unproven,
      ServerProof.unreachable => ConnectionStatus.offline,
    };
    return proof;
  }

  /// Forgets the winner so the next [connect] starts from nothing.
  ///
  /// A dropped connection re-races rather than resuming on whatever won last
  /// time — there is no mid-session transport migration, and reusing a stale
  /// winner is how you get one by accident.
  void reset() {
    _status = ConnectionStatus.idle;
    _cachedApi = null;
    _cachedApiFor = null;
    for (final t in _trunks.values) {
      t.dispose();
    }
    _trunks.clear();
    _activeRelayUrl = null;
    _active = _candidates().first;
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
  Future<_Probe?> _race(List<ConnectionCandidate> candidates) async {
    if (candidates.isEmpty) return null;
    if (candidates.length == 1) {
      // Nothing to race. The probe is best-effort: a server predating the
      // endpoint must still connect, and the challenge remains the real
      // reachability signal.
      final c = candidates.single;
      try {
        final info = await _authApiFor(c).serverInfo();
        return _Probe(c, info);
      } catch (_) {
        return _Probe(c, null);
      }
    }

    final completer = Completer<_Probe?>();
    var pending = candidates.length;
    for (final c in candidates) {
      unawaited(() async {
        try {
          final info = await _authApiFor(c).serverInfo();
          if (!completer.isCompleted) completer.complete(_Probe(c, info));
        } catch (_) {
          pending--;
          if (pending == 0 && !completer.isCompleted) completer.complete(null);
        }
      }());
    }
    return completer.future.timeout(publicRelayDelay, onTimeout: () => null);
  }
}

class _Probe {
  _Probe(this.candidate, this.info);
  final ConnectionCandidate candidate;
  final ServerInfo? info;
}
