import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';

import 'package:storm/api/auth_api.dart';
import 'package:storm/api/auth_models.dart';
import 'package:storm/api/server_verifier.dart';
import 'package:storm/api/storm_api.dart';
import 'package:storm/api/storm_connection.dart';

/// The transport seam: which path is chosen, in what order, and what has to be
/// true before the choice counts as a connection.
///
/// The claim under test is not "it connects" — it is that **nothing above the
/// seam can tell which candidate won**, and that the ordering rule is a privacy
/// rule rather than a latency one.
void main() {
  const serverId = 'srv_TESTSERVER';
  final pubkey = 'x' * 43;

  /// `GET /v1/server` for a server advertising [relays].
  http.Client serverAnswering({
    List<Map<String, String>> relays = const [],
    Duration delay = Duration.zero,
    bool dead = false,
  }) => MockClient((request) async {
    if (delay > Duration.zero) await Future<void>.delayed(delay);
    if (dead) throw http.ClientException('Connection refused');
    return http.Response(
      jsonEncode({
        'server_id': serverId,
        'key_id': 'key_1',
        'algorithm': 'ed25519',
        'public_key': pubkey,
        'relays': relays,
      }),
      200,
      headers: {'content-type': 'application/json'},
    );
  });

  /// A verifier that always agrees, so tests about *routing* are not also
  /// tests about crypto.
  ServerVerifier alwaysVerifies() => ServerVerifier(
    authApi: AuthApi(baseUrl: 'http://unused'),
    serverId: '',
    publicKeyB64: '',
  );

  StormConnection connectionWith({
    required Map<String, http.Client> byHost,
    List<RelayAdvert> relays = const [],
    Set<String> publicRelayHosts = const {},
    ServerVerifier? verifier,
    Duration publicRelayDelay = const Duration(milliseconds: 200),
  }) => StormConnection(
    localAddress: 'http://direct.test',
    token: 't',
    serverId: serverId,
    publicKeyB64: pubkey,
    relays: relays,
    publicRelayHosts: publicRelayHosts,
    publicRelayDelay: publicRelayDelay,
    buildAuthApi: (c) {
      final host = Uri.parse(c.baseUrl).host;
      final client = byHost[host];
      if (client == null) {
        return AuthApi(
          baseUrl: c.baseUrl,
          client: MockClient(
            (_) async => throw http.ClientException('no route'),
          ),
        );
      }
      return AuthApi(baseUrl: c.baseUrl, client: client);
    },
    buildVerifier: (_) => verifier ?? alwaysVerifies(),
  );

  const selfHosted = RelayAdvert(
    url: 'wss://mine.example',
    publicAddress: 'wss://mine.example/connect/$serverId',
  );
  const public = RelayAdvert(
    url: 'wss://relay.storm.dev',
    publicAddress: 'wss://relay.storm.dev/connect/$serverId',
  );

  group('candidate tiers', () {
    test('a relay is self-hosted unless it is one Storm runs', () {
      final c = connectionWith(
        byHost: {'direct.test': serverAnswering()},
        relays: const [selfHosted, public],
        publicRelayHosts: const {'relay.storm.dev'},
      );
      expect(c.active.tier, CandidateTier.direct, reason: 'direct comes first');
    });
  });

  group('the race', () {
    test('direct wins when it answers', () async {
      final c = connectionWith(
        byHost: {
          'direct.test': serverAnswering(),
          'mine.example': serverAnswering(),
        },
        relays: const [selfHosted],
      );

      expect(await c.connect(), ServerProof.verified);
      expect(c.status, ConnectionStatus.connected);
      expect(c.active.tier, CandidateTier.direct);
    });

    test('a self-hosted relay is raced with direct, not after it', () async {
      // Direct is dead; the self-hosted relay answers. It must win the *first*
      // round rather than waiting for direct to time out.
      final c = connectionWith(
        byHost: {
          'direct.test': serverAnswering(dead: true),
          'mine.example': serverAnswering(),
        },
        relays: const [selfHosted],
      );

      expect(await c.connect(), ServerProof.verified);
      expect(c.active.tier, CandidateTier.selfHosted);
    });

    test(
      'the public relay is never preferred over a reachable self-hosted one',
      () async {
        // **The privacy rule.** The public relay answers instantly and the
        // self-hosted one is slow; the slow operator-controlled path must still
        // win. Without E2E encryption whichever path carries traffic reads note
        // content, so taking the fastest answer would silently hand it to Storm.
        final c = connectionWith(
          byHost: {
            'direct.test': serverAnswering(dead: true),
            'mine.example': serverAnswering(
              delay: const Duration(milliseconds: 40),
            ),
            'relay.storm.dev': serverAnswering(),
          },
          relays: const [selfHosted, public],
          publicRelayHosts: const {'relay.storm.dev'},
        );

        expect(await c.connect(), ServerProof.verified);
        expect(
          c.active.tier,
          CandidateTier.selfHosted,
          reason: 'a slow self-hosted relay beats a fast public one, always',
        );
      },
    );

    test(
      'the public relay runs when nothing the operator controls answers',
      () async {
        final c = connectionWith(
          byHost: {
            'direct.test': serverAnswering(dead: true),
            'mine.example': serverAnswering(dead: true),
            'relay.storm.dev': serverAnswering(),
          },
          relays: const [selfHosted, public],
          publicRelayHosts: const {'relay.storm.dev'},
        );

        expect(await c.connect(), ServerProof.verified);
        expect(c.active.tier, CandidateTier.public);
      },
    );

    test('nothing answering at all is offline', () async {
      // Two candidates, both dead, so the race genuinely resolves to nothing.
      final c = connectionWith(
        byHost: {
          'direct.test': serverAnswering(dead: true),
          'mine.example': serverAnswering(dead: true),
        },
        relays: const [selfHosted],
      );

      expect(await c.connect(), ServerProof.unreachable);
      expect(c.status, ConnectionStatus.offline);
    });

    test(
      'with one candidate the challenge is the reachability signal',
      () async {
        // There is nothing to race, so the probe is best-effort on purpose — a
        // server predating `GET /v1/server` must still connect. That makes the
        // identity challenge the thing that decides reachability, which is also
        // what the pre-seam client did.
        final c = connectionWith(
          byHost: {'direct.test': serverAnswering(dead: true)},
          verifier: ServerVerifier(
            authApi: AuthApi(
              baseUrl: 'http://direct.test',
              client: MockClient(
                (_) async => throw http.ClientException('Connection refused'),
              ),
            ),
            serverId: serverId,
            publicKeyB64: pubkey,
          ),
        );

        expect(await c.connect(), ServerProof.unreachable);
        expect(c.status, ConnectionStatus.offline);
      },
    );
  });

  group('the identity gate', () {
    test('a winner that fails the challenge is not connected', () async {
      // Racing decides transport only. Something answered — that is exactly
      // the case where "offline" would be the wrong word for it.
      final c = connectionWith(
        byHost: {'direct.test': serverAnswering()},
        verifier: ServerVerifier(
          authApi: AuthApi(
            baseUrl: 'http://test',
            client: MockClient(
              (_) async => http.Response(
                jsonEncode({'signature': 'not-a-signature'}),
                200,
              ),
            ),
          ),
          serverId: serverId,
          publicKeyB64: pubkey,
        ),
      );

      expect(await c.connect(), ServerProof.impostor);
      expect(c.status, ConnectionStatus.unproven);
      expect(
        c.status,
        isNot(ConnectionStatus.offline),
        reason: 'something is answering, and that is the problem',
      );
    });
  });

  group('the relay set', () {
    test('is refreshed from the server that answered', () async {
      // Q11: the pairing payload is frozen at issuance, so this is the only
      // way a client learns its server moved relays while it was away.
      final c = connectionWith(
        byHost: {
          'direct.test': serverAnswering(
            relays: [
              {
                'url': selfHosted.url,
                'public_address': selfHosted.publicAddress,
              },
            ],
          ),
        },
      );
      expect(c.relays, isEmpty, reason: 'precondition: nothing known yet');

      await c.connect();

      expect(c.relays.map((r) => r.url), [selfHosted.url]);
    });

    test('a server predating the field still connects', () async {
      // `relays` absent entirely, not `[]`. This endpoint is what a stranded
      // client reaches, so it must not be the thing that strands it.
      final c = StormConnection(
        localAddress: 'http://direct.test',
        token: 't',
        buildAuthApi: (_) => AuthApi(
          baseUrl: 'http://direct.test',
          client: MockClient(
            (_) async => http.Response(
              jsonEncode({
                'server_id': serverId,
                'key_id': 'key_1',
                'algorithm': 'ed25519',
                'public_key': pubkey,
              }),
              200,
            ),
          ),
        ),
        buildVerifier: (_) => alwaysVerifies(),
      );

      expect(await c.connect(), ServerProof.verified);
      expect(c.relays, isEmpty);
    });
  });

  group('reconnect', () {
    test(
      'reset re-races rather than resuming on the previous winner',
      () async {
        var directAlive = false;
        final c = StormConnection(
          localAddress: 'http://direct.test',
          token: 't',
          relays: const [selfHosted],
          publicRelayDelay: const Duration(milliseconds: 200),
          buildAuthApi: (candidate) => AuthApi(
            baseUrl: candidate.baseUrl,
            client: MockClient((_) async {
              final isDirect = candidate.tier == CandidateTier.direct;
              if (isDirect && !directAlive) {
                throw http.ClientException('Connection refused');
              }
              return http.Response(
                jsonEncode({
                  'server_id': serverId,
                  'key_id': 'key_1',
                  'algorithm': 'ed25519',
                  'public_key': pubkey,
                  'relays': const [],
                }),
                200,
              );
            }),
          ),
          buildVerifier: (_) => alwaysVerifies(),
        );

        await c.connect();
        expect(
          c.active.tier,
          CandidateTier.selfHosted,
          reason: 'direct is down',
        );

        // The LAN comes back. Only a re-race can notice.
        directAlive = true;
        c.reset();
        await c.connect();

        expect(
          c.active.tier,
          CandidateTier.direct,
          reason:
              'a reconnect re-races; it does not resume on last time\'s winner',
        );
      },
    );
  });

  group('above the seam', () {
    test('the stream URI follows the winning candidate', () async {
      // The one piece of transport knowledge SyncEngine used to hold itself,
      // by string-replacing http with ws on the API base URL.
      final c = connectionWith(
        byHost: {
          'direct.test': serverAnswering(dead: true),
          'mine.example': serverAnswering(),
        },
        relays: const [selfHosted],
      );
      await c.connect();

      final uri = c.streamUri('stt_abc');
      expect(uri.scheme, 'wss');
      expect(uri.host, 'mine.example');
      expect(uri.path, endsWith('/v1/stream'));
      expect(uri.queryParameters['ticket'], 'stt_abc');
      // The session token must never reach a URL — that is what the ticket is
      // for, and asserting the ticket alone would not notice it being there
      // as well.
      expect(uri.queryParameters['token'], isNull);
      expect(uri.toString(), isNot(contains('Bearer')));
    });

    test('a pinned connection sends nothing extra on connect', () async {
      // `StormConnection.direct` is the pre-relay world and every existing
      // test. It must not put a probe on the wire that the old code never
      // sent — a refactor is not allowed to add a request.
      var calls = 0;
      final api = AuthApi(
        baseUrl: 'http://test',
        client: MockClient((_) async {
          calls++;
          return http.Response('{}', 404);
        }),
      );
      final c = StormConnection.direct(
        api: StormApi(
          baseUrl: 'http://test',
          token: 't',
          client: MockClient((_) async {
            calls++;
            return http.Response('{}', 404);
          }),
        ),
        authApi: api,
        verifier: alwaysVerifies(),
      );

      expect(await c.connect(), ServerProof.verified);
      expect(calls, 0, reason: 'nothing to race, nothing to refresh');
    });
  });
}
