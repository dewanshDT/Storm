/// Signing an integration in with OAuth (spec §10; decision 81h).
///
/// The browser returns to **this client**, which relays `{state, code}` to the
/// server (G-D13): upstreams refuse a LAN `http` redirect, so the server
/// cannot receive it itself. Android and macOS receive it as a
/// `storm://oauth` link; desktop Linux and Windows listen on a loopback port
/// (spec §10.4, decision 81l). The web has no way to, so it connects with a
/// token.
library;

export 'oauth_flow_io.dart' if (dart.library.js_interop) 'oauth_flow_web.dart';
