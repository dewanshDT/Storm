/// Re-proves the server's identity every time the client connects.
///
/// The public key is pinned at pairing. Until now nothing checked it again:
/// `challenge()` had exactly one caller, in the pairing screen, so a server
/// proved itself once and was trusted forever after. On a LAN that is a
/// defensible shortcut — whatever answers `192.168.1.42` is almost certainly
/// the box that answered yesterday.
///
/// Through a relay it is not. The relay is deliberately dumb infrastructure:
/// it authenticates *servers* so one cannot squat another's id, and it
/// authenticates no clients at all. What stops a malicious or compromised
/// relay impersonating the server to *us* is this check, run end to end over
/// whichever transport happens to be carrying traffic. A relay that cannot
/// forge a signature can drop and delay traffic, and nothing worse.
///
/// So this must run on **every** connect, not once per pairing — including
/// every reconnect, because that is what a transport switch looks like from
/// here.
library;

import 'dart:math';

import 'auth_api.dart';
import 'ed25519_verify.dart';

/// What a connect-time identity check concluded.
///
/// Three outcomes, not two, matching the discipline the rest of the client
/// already keeps: an `AuthApiException` means the server answered and refused,
/// a socket failure means it never answered, and those are different things.
/// A bad signature is a third thing again — the server answered perfectly
/// well, and the answer was wrong.
enum ServerProof {
  /// The signature checked out against the pinned key.
  verified,

  /// Nothing answered. Ordinary offline — the server is down, the wifi is
  /// gone, the relay is unavailable. Says nothing about identity.
  unreachable,

  /// Something answered and could not prove it holds the server's key.
  /// Either the traffic is being intercepted or this is a different server.
  impostor,
}

/// Generates a challenge nonce.
///
/// Sixteen characters from a 62-symbol alphabet, from a CSPRNG. Well inside
/// the server's `validate_nonce` rule (16–128 printable ASCII, no `:` and no
/// `"`) — the exclusions matter because the signed message is colon-delimited
/// and a nonce carrying one could forge its own field boundaries.
String randomNonce() {
  const chars =
      'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789';
  final rng = Random.secure();
  return List.generate(16, (_) => chars[rng.nextInt(chars.length)]).join();
}

/// Asks the server to sign a fresh nonce and checks it against the pinned key.
class ServerVerifier {
  ServerVerifier({
    required this.authApi,
    required this.serverId,
    required this.publicKeyB64,
  });

  final AuthApi authApi;

  /// The server id pinned at pairing. It is part of the signed message, so a
  /// signature captured from one server cannot be replayed for another.
  final String serverId;

  /// The Ed25519 public key pinned at pairing, base64url without padding.
  final String publicKeyB64;

  /// Runs one challenge round.
  ///
  /// Never throws: every failure is one of the [ServerProof] values, because
  /// the caller's job is to decide what to do about it, not to unwind.
  Future<ServerProof> check() async {
    if (serverId.isEmpty || publicKeyB64.isEmpty) {
      // Nothing was ever pinned, so there is nothing to verify against. This
      // is an unpaired install, not an impostor — refusing to sync here would
      // brick a client that simply has not paired yet.
      return ServerProof.verified;
    }

    final nonce = randomNonce();
    final String signature;
    try {
      signature = (await authApi.challenge(nonce)).signature;
    } on AuthApiException {
      // The server answered and refused to sign. Older servers predate the
      // endpoint, and a refusal is not a forgery — treat it as unreachable so
      // the client retries rather than accusing a healthy server.
      return ServerProof.unreachable;
    } catch (_) {
      return ServerProof.unreachable;
    }

    final valid = await verifyChallenge(
      publicKeyB64: publicKeyB64,
      signatureB64: signature,
      serverId: serverId,
      nonce: nonce,
    );
    return valid ? ServerProof.verified : ServerProof.impostor;
  }
}
