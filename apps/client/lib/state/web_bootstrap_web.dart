import 'package:web/web.dart' as web;

/// The nonce Storm injected into this document, if it minted one.
///
/// Null is not an error: the server declines to mint behind a proxy, when rate
/// limited, or over its outstanding ceiling, and the pairing screen still works
/// and is what the client falls back to.
String? readWebBootstrapNonce() {
  final meta = web.document.querySelector('meta[name="storm-bootstrap"]');
  final nonce = meta?.getAttribute('content');
  if (nonce == null || nonce.isEmpty) return null;
  return nonce;
}

/// Removes the tag once it has been read.
///
/// The nonce is single-use and already spent by the time this runs; leaving it
/// in the DOM keeps a used credential where any script on the page can read it,
/// for no benefit.
void clearWebBootstrapNonce() {
  web.document.querySelector('meta[name="storm-bootstrap"]')?.remove();
}

/// Reloads the page so the server issues a fresh bootstrap nonce.
///
/// The nonce in this document is single-use and already spent, so recovering
/// from a rejected device credential needs a *new* document — there is no way
/// to ask for another one without one, by design.
void reloadForFreshBootstrap() {
  web.window.location.reload();
}
