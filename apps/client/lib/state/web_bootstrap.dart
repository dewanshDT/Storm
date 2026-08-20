/// Reads the bootstrap nonce Storm injects into the page it served.
///
/// Storm serves its own web client, so asking the person who opened it to scan
/// a QR — from a phone, into a browser, to reach a server the browser already
/// knows because it was served from it — is theatre. The server puts a
/// short-lived, single-use, peer-bound nonce in the document; this reads it,
/// and the client spends it through the ordinary `POST /v1/pair`.
///
/// **This is not a second authentication mechanism.** What comes back is an
/// ordinary device credential: `StormDevice` for the device tier, a normal
/// session after a normal login, listed in Sessions & Devices, individually
/// revocable. See *Storm Web Bootstrap*.
///
/// Split by conditional import because `package:web` exists only on the web,
/// and an unconditional import of it does not compile for Android or macOS.
library;

export 'web_bootstrap_stub.dart'
    if (dart.library.js_interop) 'web_bootstrap_web.dart';
