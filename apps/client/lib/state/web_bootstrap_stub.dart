/// Off the web there is no document to carry a nonce, so there is nothing to
/// read and nothing to clear. Native clients pair by QR, unchanged.
String? readWebBootstrapNonce() => null;

void clearWebBootstrapNonce() {}
